// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The Secure Shell client of the image: it opens a connection to an
//! OpenSSH server, authenticates with `publickey`, runs one command, and
//! reports both of its streams and its exit status.
//!
//! This is step S8 of
//! [document 14](../../../../docs/14-secure-shell-as-a-client.md) on the
//! machine. `audhsos-ssh` is sans-I/O: it is given bytes that arrived and
//! a buffer to write into, and what it wants sent goes through the socket
//! protocol of `server-net` (D-116) as `user_programs::socket::Stream`
//! speaks it. Nothing of the protocol and nothing of the transport is
//! decided here.
//!
//! What the program is given, it reads off the scratch volume (D-146):
//! the port the server took and the account to authenticate as, the
//! fingerprints of the host keys it will talk to, and its own secret.
//! A machine whose disk carries none of that reports so and ends, which
//! is every run but the interop one.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

// The package holds three programs and each uses a different part of what
// it depends on; these are the crates this one does not.
use audhsos_time as _;
use driver_virtio_net as _;
use net_http as _;
use net_stack as _;
use server_net as _;
use user_net_programs as _;
use virtio_queue as _;

use audhsos_abi::Error;
use audhsos_encoding::base64;
use audhsos_ssh::auth::ClientKey;
use audhsos_ssh::client::{Buffers, Config, Connection, Event, MIN_INCOMING, MIN_OUTGOING};
use audhsos_ssh::error::SshError;
use audhsos_ssh::hostkey::{FINGERPRINT_LEN, Fingerprints};
use crypto_rng::ChaChaRng;
use net_wire::{IpAddr, Ipv4Addr};
use user_programs::client::{lookup, write_line};
use user_programs::socket::{Idle, Received, Stream};
use user_proto::file::{Name, ROOT, Reply as FileReply, Request as FileRequest};
use user_proto::socket::Endpoint;
use user_rt::{EndpointHandle, Line, Startup};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the file system server registered itself under.
const FILES: &[u8] = b"files";

/// The name the console driver registered itself under.
const CONSOLE: &[u8] = b"console";

/// The file naming the port, the account and the command (D-146).
const CONFIG: &[u8] = b"SSHCONF.TXT";

/// The file of fingerprints, one per line, as OpenSSH prints them.
const TRUST: &[u8] = b"SSHTRUST.TXT";

/// The file holding the thirty-two octets the client signs with.
const SECRET: &[u8] = b"SSHKEY.BIN";

/// The host the connection reaches, which is the gateway of the machine
/// and, under QEMU's user-mode network, the development machine itself.
const HOST: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 2);

/// Where the rings of the socket are mapped. A second window than the one
/// `app-net` takes, because the two programs run in the same boot.
const RINGS: u64 = 0x0000_7500_0000_0000;

/// How long the program waits for one step, in microseconds.
const DEADLINE: u64 = 30_000_000;

/// How many fingerprints the trust file may name.
const MAX_TRUSTED: usize = 8;

/// How long the command may be.
const MAX_COMMAND: usize = 128;

/// How long the account name may be.
const MAX_ACCOUNT: usize = 64;

/// How many bytes of the configuration this program reads.
const MAX_CONFIG: usize = 1024;

/// How many bytes move between the rings and the protocol at once, and
/// the largest channel payload this client advertises.
///
/// The two are one number because `Connection::recv` copies what fits and
/// drops the rest: a peer that keeps to RFC 4254, section 5.1, never
/// sends more in one message than this, and one that does not is refused
/// rather than read short.
const CHUNK: usize = 1024;

/// How many events the run may take before it is given up. A handshake,
/// an authentication and one command are far below this; a peer that
/// makes no progress is what it stops.
const MAX_EVENTS: usize = 4096;

/// How long a line this program writes.
type Report = Line<256>;

/// What the scratch volume says this run is.
struct Setup {
    /// The port on the gateway.
    port: u16,
    /// The account to authenticate as.
    account: [u8; MAX_ACCOUNT],
    /// How many bytes of it there are.
    account_len: usize,
    /// The command to run.
    command: [u8; MAX_COMMAND],
    /// How many bytes of it there are.
    command_len: usize,
    /// The host keys this client will talk to.
    trusted: [[u8; FINGERPRINT_LEN]; MAX_TRUSTED],
    /// How many of them there are.
    trusted_len: usize,
    /// The thirty-two octets the client signs with.
    secret: [u8; 32],
}

/// Reads what the volume says and runs the command it names.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let voice = voice(&mut gate, &startup);
    match setup(&mut gate, &startup) {
        Ok(None) => say(
            &mut gate,
            voice,
            &Report::of(format_args!("[ssh-app] no configuration\n")),
        ),
        Ok(Some(setup)) => {
            if let Err(error) = work(&mut gate, &startup, voice, &setup) {
                say(
                    &mut gate,
                    voice,
                    &Report::of(format_args!("[ssh-app] failed: {}\n", error.message())),
                );
            }
        }
        Err(error) => say(
            &mut gate,
            voice,
            &Report::of(format_args!(
                "[ssh-app] unreadable configuration: {}\n",
                error.message()
            )),
        ),
    }
    finish(&mut gate, &startup);
    gate.thread_exit()
}

/// Opens the connection, drives the protocol over it, and reports.
fn work(
    gate: &mut Gate,
    startup: &Startup,
    voice: Option<EndpointHandle>,
    setup: &Setup,
) -> Result<(), Error> {
    let server = network(gate, startup).ok_or(Error::Unavailable)?;
    let process = startup.own_process.ok_or(Error::Unavailable)?;
    let idle = Idle::new(gate)?;
    let until = gate.clock_now()?.saturating_add(DEADLINE);

    let remote = Endpoint::new(IpAddr::V4(HOST), setup.port);
    let stream = Stream::connect(gate, server, process, remote, RINGS, idle, until)?;
    say(
        gate,
        voice,
        &Report::of(format_args!(
            "[ssh-app] connected to 10.0.2.2:{}\n",
            setup.port
        )),
    );

    let outcome = speak(gate, &stream, voice, setup, until, idle);
    let _shut = stream.shutdown_write(gate);
    let _closed = stream.close(gate);
    outcome
}

/// The whole of the protocol over one connection.
#[expect(
    clippy::large_stack_arrays,
    reason = "the two buffers are the sizes RFC 4253, section 6.1, makes mandatory and 14.11 makes the cost of one connection; a thread of this system has sixty-four pages of stack (D-145)"
)]
fn speak(
    gate: &mut Gate,
    stream: &Stream,
    voice: Option<EndpointHandle>,
    setup: &Setup,
    until: u64,
    idle: Idle,
) -> Result<(), Error> {
    let account = core::str::from_utf8(setup.account.get(..setup.account_len).unwrap_or(&[]))
        .map_err(|_| Error::InvalidArgument)?;
    let key = ClientKey::new(setup.secret);
    let trust = Fingerprints::new(setup.trusted.get(..setup.trusted_len).unwrap_or(&[]));
    let settings = Config {
        user: account,
        key: &key,
        trust: &trust,
        command: Some(setup.command.get(..setup.command_len).unwrap_or(&[])),
        window: 32_768,
        max_packet: u32::try_from(CHUNK).map_err(|_| Error::InvalidArgument)?,
    };
    let mut generator = seed(gate)?;
    let mut incoming = [0u8; MIN_INCOMING];
    let mut outgoing = [0u8; MIN_OUTGOING];
    let now = gate.clock_now()?;
    let mut client = Connection::new(
        &settings,
        &mut generator,
        Buffers {
            incoming: &mut incoming,
            outgoing: &mut outgoing,
        },
        now,
    )
    .map_err(|error| refused(gate, voice, error))?;

    let mut chunk = [0u8; CHUNK];
    let mut status: Option<u32> = None;
    let talk = Talk {
        stream,
        voice,
        until,
        idle,
    };
    for _ in 0..MAX_EVENTS {
        drain(gate, &talk, &mut client, &mut chunk)?;
        let now = gate.clock_now()?;
        let event = match client.poll(now) {
            Ok(event) => event,
            Err(error) => return Err(refused(gate, voice, error)),
        };
        if act(gate, &talk, &mut client, &mut chunk, &mut status, event)? {
            return Ok(());
        }
    }
    Err(Error::Cancelled)
}

/// What one event of the protocol means to this program, and whether it
/// was the last.
fn act<R: crypto_rng::Rng, T: audhsos_ssh::hostkey::Trust>(
    gate: &mut Gate,
    talk: &Talk<'_>,
    client: &mut Connection<'_, R, T>,
    chunk: &mut [u8; CHUNK],
    status: &mut Option<u32>,
    event: Event,
) -> Result<bool, Error> {
    let voice = talk.voice;
    match event {
        Event::WantsWrite => {}
        Event::WantsRead => {
            if fill(gate, talk, client, chunk)? == Received::Waiting {
                talk.idle.wait(gate, talk.until)?;
            }
        }
        Event::Started => {
            say(
                gate,
                voice,
                &Report::of(format_args!("[ssh-app] command started\n")),
            );
            if let Err(error) = client.finish() {
                return Err(refused(gate, voice, error));
            }
        }
        Event::Data { stderr, len } => {
            // A payload longer than this side advertised is a peer that
            // ignored the maximum packet size of RFC 4254, section 5.1.
            // `recv` would copy what fits and drop the rest, so the
            // connection ends here rather than losing bytes in silence.
            if len > CHUNK {
                return Err(refused(gate, voice, SshError::Channel));
            }
            let taken = client.recv(chunk.get_mut(..len).unwrap_or(&mut []));
            report_data(gate, voice, stderr, chunk.get(..taken).unwrap_or(&[]));
        }
        Event::ExitStatus(code) => *status = Some(code),
        Event::ExitSignal => say(
            gate,
            voice,
            &Report::of(format_args!("[ssh-app] exit signal\n")),
        ),
        Event::Closed => {
            drain(gate, talk, client, chunk)?;
            match status {
                Some(code) => say(
                    gate,
                    voice,
                    &Report::of(format_args!("[ssh-app] exit status {code}\n")),
                ),
                None => say(
                    gate,
                    voice,
                    &Report::of(format_args!("[ssh-app] no exit status\n")),
                ),
            }
            return Ok(true);
        }
    }
    Ok(false)
}

/// What the protocol is spoken through, and how long it may take.
struct Talk<'a> {
    /// The connection.
    stream: &'a Stream,
    /// Where the lines go.
    voice: Option<EndpointHandle>,
    /// When the run is given up.
    until: u64,
    /// What a wait sleeps on.
    idle: Idle,
}

/// Says what the protocol refused and answers the error the program ends
/// with.
fn refused(gate: &mut Gate, voice: Option<EndpointHandle>, error: SshError) -> Error {
    say(
        gate,
        voice,
        &Report::of(format_args!("[ssh-app] protocol: {error}\n")),
    );
    Error::InvalidArgument
}

/// Moves everything the protocol wants sent into the connection.
fn drain<R: crypto_rng::Rng, T: audhsos_ssh::hostkey::Trust>(
    gate: &mut Gate,
    talk: &Talk<'_>,
    client: &mut Connection<'_, R, T>,
    chunk: &mut [u8; CHUNK],
) -> Result<(), Error> {
    loop {
        let len = client.write_ssh(chunk);
        if len == 0 {
            return Ok(());
        }
        talk.stream
            .write_all(gate, chunk.get(..len).unwrap_or(&[]), talk.idle, talk.until)?;
    }
}

/// Takes what the connection holds into the protocol, and says what the
/// one ask found.
fn fill<R: crypto_rng::Rng, T: audhsos_ssh::hostkey::Trust>(
    gate: &mut Gate,
    talk: &Talk<'_>,
    client: &mut Connection<'_, R, T>,
    chunk: &mut [u8; CHUNK],
) -> Result<Received, Error> {
    let received = talk.stream.receive(gate, chunk)?;
    match received {
        Received::Bytes(taken) => give(client, chunk.get(..taken).unwrap_or(&[]))?,
        // A connection the peer ended while the protocol still wants a
        // packet is a handshake or a command cut short.
        Received::Ended => return Err(Error::Unavailable),
        Received::Waiting => {}
    }
    Ok(received)
}

/// Hands every byte of `bytes` to the protocol, which takes them unless
/// its buffer is full — and a buffer that is full while the caller holds
/// more is a packet longer than RFC 4253, section 6.1, allows.
fn give<R: crypto_rng::Rng, T: audhsos_ssh::hostkey::Trust>(
    client: &mut Connection<'_, R, T>,
    bytes: &[u8],
) -> Result<(), Error> {
    let mut rest = bytes;
    while !rest.is_empty() {
        let read = client.read_ssh(rest);
        if read == 0 {
            return Err(Error::BufferTooSmall);
        }
        rest = rest.get(read..).unwrap_or(&[]);
    }
    Ok(())
}

/// What one stream of the command wrote, as one line.
fn report_data(gate: &mut Gate, voice: Option<EndpointHandle>, stderr: bool, bytes: &[u8]) {
    let stream = if stderr { "stderr" } else { "stdout" };
    let mut line = Report::of(format_args!("[ssh-app] {stream}: "));
    for byte in bytes {
        let printable = match *byte {
            b'\n' => b'|',
            0x20..=0x7e => *byte,
            _ => b'.',
        };
        line.put(&[printable]);
    }
    line.put(b"\n");
    say(gate, voice, &line);
}

/// What the scratch volume says, or `None` when it says nothing.
fn setup(gate: &mut Gate, startup: &Startup) -> Result<Option<Setup>, Error> {
    let Some(files) = volume(gate, startup) else {
        return Ok(None);
    };
    let mut text = [0u8; MAX_CONFIG];
    let Some(len) = read_file(gate, files, CONFIG, &mut text)? else {
        return Ok(None);
    };
    let mut setup = Setup {
        port: 0,
        account: [0u8; MAX_ACCOUNT],
        account_len: 0,
        command: [0u8; MAX_COMMAND],
        command_len: 0,
        trusted: [[0u8; FINGERPRINT_LEN]; MAX_TRUSTED],
        trusted_len: 0,
        secret: [0u8; 32],
    };
    read_config(text.get(..len).unwrap_or(&[]), &mut setup)?;

    let mut trust = [0u8; MAX_CONFIG];
    let Some(len) = read_file(gate, files, TRUST, &mut trust)? else {
        return Err(Error::NotFound);
    };
    read_trust(trust.get(..len).unwrap_or(&[]), &mut setup)?;
    if setup.trusted_len == 0 {
        return Err(Error::NotFound);
    }

    let mut secret = [0u8; 32];
    let Some(len) = read_file(gate, files, SECRET, &mut secret)? else {
        return Err(Error::NotFound);
    };
    if len != secret.len() {
        return Err(Error::InvalidArgument);
    }
    setup.secret = secret;
    Ok(Some(setup))
}

/// The three lines of the configuration.
fn read_config(text: &[u8], setup: &mut Setup) -> Result<(), Error> {
    for line in text.split(|byte| *byte == b'\n') {
        if let Some(value) = after(line, b"port ") {
            setup.port = number(value)?;
        } else if let Some(value) = after(line, b"user ") {
            setup.account_len = copy(value, &mut setup.account)?;
        } else if let Some(value) = after(line, b"command ") {
            setup.command_len = copy(value, &mut setup.command)?;
        }
    }
    if setup.port == 0 || setup.account_len == 0 || setup.command_len == 0 {
        return Err(Error::InvalidArgument);
    }
    Ok(())
}

/// The fingerprints, one `SHA256:` line at a time.
fn read_trust(text: &[u8], setup: &mut Setup) -> Result<(), Error> {
    for line in text.split(|byte| *byte == b'\n') {
        let Some(body) = after(line, b"SHA256:") else {
            continue;
        };
        if setup.trusted_len >= MAX_TRUSTED {
            return Err(Error::BufferTooSmall);
        }
        let digest = decode_fingerprint(body)?;
        let slot = setup
            .trusted
            .get_mut(setup.trusted_len)
            .ok_or(Error::BufferTooSmall)?;
        *slot = digest;
        setup.trusted_len = setup.trusted_len.saturating_add(1);
    }
    Ok(())
}

/// The thirty-two octets the unpadded Base64 of a fingerprint stands for.
///
/// OpenSSH prints the digest with the padding cut off, so it goes back on
/// before the decoder of RFC 4648 sees it.
fn decode_fingerprint(body: &[u8]) -> Result<[u8; FINGERPRINT_LEN], Error> {
    let mut padded = [b'='; 44];
    let text = trim(body);
    let slot = padded.get_mut(..text.len()).ok_or(Error::InvalidArgument)?;
    slot.copy_from_slice(text);
    let mut digest = [0u8; FINGERPRINT_LEN];
    let len = base64::decode(&padded, &mut digest).map_err(|_| Error::InvalidArgument)?;
    if len != FINGERPRINT_LEN {
        return Err(Error::InvalidArgument);
    }
    Ok(digest)
}

/// What follows `prefix` in `line`, or nothing where the line does not
/// begin with it.
fn after<'a>(line: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    line.get(..prefix.len())
        .filter(|head| *head == prefix)
        .and_then(|_| line.get(prefix.len()..))
}

/// The line without the carriage return and the spaces around it.
const fn trim(line: &[u8]) -> &[u8] {
    let mut bytes = line;
    while let Some((last, rest)) = bytes.split_last() {
        if matches!(*last, b'\r' | b' ' | b'\t') {
            bytes = rest;
            continue;
        }
        break;
    }
    bytes
}

/// The number `text` spells.
fn number(text: &[u8]) -> Result<u16, Error> {
    let mut value: u16 = 0;
    let digits = trim(text);
    if digits.is_empty() {
        return Err(Error::InvalidArgument);
    }
    for byte in digits {
        let digit = byte.checked_sub(b'0').ok_or(Error::InvalidArgument)?;
        if digit > 9 {
            return Err(Error::InvalidArgument);
        }
        value = value
            .checked_mul(10)
            .and_then(|scaled| scaled.checked_add(u16::from(digit)))
            .ok_or(Error::InvalidArgument)?;
    }
    Ok(value)
}

/// Copies `text` into `into` and answers how many bytes that was.
fn copy(text: &[u8], into: &mut [u8]) -> Result<usize, Error> {
    let bytes = trim(text);
    let slot = into.get_mut(..bytes.len()).ok_or(Error::BufferTooSmall)?;
    slot.copy_from_slice(bytes);
    Ok(bytes.len())
}

/// Reads one file of the volume, or answers `None` where it is not there.
fn read_file(
    gate: &mut Gate,
    files: EndpointHandle,
    name: &[u8],
    into: &mut [u8],
) -> Result<Option<usize>, Error> {
    let name = Name::new(name)?;
    let opened = match file_call(gate, files, &FileRequest::Open { parent: ROOT, name })? {
        FileReply::Opened(Ok(opened)) => opened,
        FileReply::Opened(Err(Error::NotFound | Error::Unavailable)) => return Ok(None),
        FileReply::Opened(Err(error)) => return Err(error),
        _ => return Err(Error::InvalidArgument),
    };
    let wanted = usize::try_from(opened.size).unwrap_or(0).min(into.len());
    let taken = read_into(
        gate,
        files,
        opened.file,
        into.get_mut(..wanted).unwrap_or(&mut []),
    );
    let _closed = file_call(gate, files, &FileRequest::Close { file: opened.file })?;
    Ok(Some(taken?))
}

/// Reads as much of `file` as `into` holds, in as many messages as it
/// takes.
fn read_into(
    gate: &mut Gate,
    files: EndpointHandle,
    file: u32,
    into: &mut [u8],
) -> Result<usize, Error> {
    let mut done = 0usize;
    while done < into.len() {
        let want = into.len().saturating_sub(done);
        let FileReply::Read(outcome) = file_call(
            gate,
            files,
            &FileRequest::Read {
                file,
                offset: u32::try_from(done).unwrap_or(0),
                len: u32::try_from(want).unwrap_or(0),
            },
        )?
        else {
            return Err(Error::InvalidArgument);
        };
        let data = outcome?;
        if data.is_empty() {
            break;
        }
        let end = done.saturating_add(data.len()).min(into.len());
        let slot = into.get_mut(done..end).ok_or(Error::BufferTooSmall)?;
        let source = data
            .as_bytes()
            .get(..slot.len())
            .ok_or(Error::BufferTooSmall)?;
        slot.copy_from_slice(source);
        done = end;
    }
    Ok(done)
}

/// A generator seeded from the machine, as 13.4 asks of a process.
fn seed(gate: &mut Gate) -> Result<ChaChaRng<Entropy>, Error> {
    let words = gate.random_bytes()?;
    let mut bytes = [0u8; 32];
    for (chunk, word) in bytes.chunks_mut(8).zip(words) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    Ok(ChaChaRng::from_seed(&bytes, Entropy))
}

/// The entropy source of this program, which never delivers: the system
/// call behind it needs the gate of the thread, which a source held
/// inside the generator does not have. One handshake draws a few hundred
/// bytes and a reseed is due after a mebibyte.
struct Entropy;

impl crypto_rng::Entropy for Entropy {
    fn fill(&mut self, _out: &mut [u8]) -> Result<(), crypto_rng::EntropyError> {
        Err(crypto_rng::EntropyError::Unavailable)
    }
}

/// The endpoint of the network server.
fn network(gate: &mut Gate, startup: &Startup) -> Option<EndpointHandle> {
    if let Some(endpoint) = startup.net_server {
        return Some(endpoint);
    }
    let names = startup.name_server?;
    lookup(gate, names, b"net").ok()
}

/// The endpoint of the file system server.
fn volume(gate: &mut Gate, startup: &Startup) -> Option<EndpointHandle> {
    let names = startup.name_server?;
    lookup(gate, names, FILES).ok()
}

/// The endpoint the lines go to.
fn voice(gate: &mut Gate, startup: &Startup) -> Option<EndpointHandle> {
    let looked_up = startup
        .name_server
        .and_then(|names| lookup(gate, names, CONSOLE).ok());
    looked_up.or(startup.log)
}

/// Sends one request of the file protocol and reads the reply.
fn file_call(
    gate: &mut Gate,
    files: EndpointHandle,
    request: &FileRequest,
) -> Result<FileReply, Error> {
    request.encode(&mut gate.writer())?;
    gate.ipc_call(files)?;
    FileReply::decode(gate.reader()).map_err(Error::from)
}

/// Writes one line, if there is anywhere to write it.
fn say(gate: &mut Gate, voice: Option<EndpointHandle>, line: &Report) {
    let Some(endpoint) = voice else {
        return;
    };
    let _said = write_line(gate, endpoint, line.as_bytes());
}

/// Tells the process that started this one that the work is done.
fn finish(gate: &mut Gate, startup: &Startup) {
    let Some(parent) = startup.parent else {
        return;
    };
    let finished = user_proto::parent::Request::Finished {
        status: user_proto::parent::SUCCESS,
    };
    if finished.encode(&mut gate.writer()).is_ok() {
        let _reported = gate.ipc_send(parent);
    }
}
