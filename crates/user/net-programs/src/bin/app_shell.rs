// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The shell of the desktop: one window of the compositor, a line to type
//! in, and two commands that leave the machine — a Secure Shell session and
//! an HTTP request.
//!
//! What a line means is decided in `app-shell`, which holds the scrollback,
//! the line being typed and the parser, and answers with the draw commands
//! of the window. This is the loop around it: the window it opens at the
//! compositor, the events it takes out of it, and the two protocols it
//! speaks over the socket protocol of `server-net` (D-116).
//!
//! What a session is given, it reads off the volume as `app-ssh` does
//! (D-146): the fingerprints of the host keys it will talk to and its own
//! secret. The account, the host, the port and the command are the line's.
//!
//! It ends when the compositor takes its window away, which the key that
//! ends the desktop and the `quit` command both do.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

// The package holds five programs and each uses a different part of what
// it depends on; these are the crates this one does not.
use audhsos_encoding as _;
use audhsos_time as _;
use audhsos_x509 as _;
use driver_virtio_net as _;
use net_stack as _;
use server_net as _;
use user_net_programs as _;
use virtio_queue as _;

use app_shell::command::{Command, HELP, Locator, Target, parse};
use app_shell::state::{HEIGHT, Line as Typed, Shell, Step, WIDTH};
use audhsos_abi::{Error, Rights};
use audhsos_ssh::auth::ClientKey;
use audhsos_ssh::client::{
    Buffers, Config, Connection, Event as SshEvent, MIN_INCOMING, MIN_OUTGOING,
};
use audhsos_ssh::error::SshError;
use audhsos_ssh::hostkey::{FINGERPRINT_LEN, Fingerprints};
use crypto_rng::ChaChaRng;
use gfx::draw::List;
use net_http::{Decoder, Event as HttpEvent, Method, Request as HttpRequest};
use net_wire::{IpAddr, Ipv4Addr, Writer as WireWriter};
use user_programs::client::{lookup, write_line};
use user_programs::socket::{Idle, Received, Stream};
use user_proto::file::{Name as FileName, ROOT, Reply as FileReply, Request as FileRequest};
use user_proto::socket::{Endpoint, Name, Reply as SocketReply, Request as SocketRequest};
use user_proto::window::{Reply, Request, Title};
use user_rt::{EndpointHandle, Line, Startup, Typed as TypedHandle};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the compositor registered itself under.
const DESK: &[u8] = b"desk";

/// The name the file system server registered itself under.
const FILES: &[u8] = b"files";

/// What stands in the title bar of the window.
const TITLE: &[u8] = b"shell";

/// Where the rings of a connection are mapped. A window of its own: the
/// three programs of the network run in the same boot.
const RINGS: u64 = 0x0000_7600_0000_0000;

/// How long one exchange may take, in microseconds.
const DEADLINE: u64 = 30_000_000;

/// How long a wait sleeps between two asks, in microseconds.
const STEP: u64 = 10_000;

/// How many bytes move between the rings and a protocol at once.
const CHUNK: usize = 1024;

/// How many events one session may take before it is given up.
const MAX_EVENTS: usize = 4096;

/// How many fingerprints the trust file may name.
const MAX_TRUSTED: usize = 8;

/// How many bytes of the trust file this program reads.
const MAX_TRUST_FILE: usize = 1024;

/// The file of fingerprints, one per line, as OpenSSH prints them.
const TRUST: &[u8] = b"SSHTRUST.TXT";

/// The file holding the thirty-two octets the client signs with.
const SECRET: &[u8] = b"SSHKEY.BIN";

/// How many bytes of a body the shell shows.
const MAX_BODY: usize = 512;

/// What a notification handed to a server may be used for.
const SIGNAL_ONLY: Rights = Rights::SIGNAL.union(Rights::TRANSFER);

/// What a process handed to a server may be used for.
const WATCHED: Rights = Rights::INFO.union(Rights::TRANSFER);

/// How many microseconds one second has.
const SECOND: u64 = 1_000_000;

/// How long a line this program writes.
type Report = Line<160>;

/// How long the sentence of a refusal is. It is shorter than a line of
/// the console, because a refusal is read in a window of
/// `app_shell::COLUMNS` characters and the longest message of a system
/// call is eighty-nine.
type Sentence = Line<96>;

/// Why something did not happen, as the sentence the shell writes.
///
/// A step that fails for a reason of its own says that reason; a system
/// call and a protocol say theirs through the conversions below. So one
/// line reaches the scrollback per failure, and it names what was
/// missing rather than which error number stood for it.
struct Refusal(Sentence);

impl Refusal {
    /// The refusal that says `text`.
    fn of(text: &str) -> Self {
        Refusal(Sentence::of(format_args!("{text}")))
    }

    /// The sentence, as the shell writes it.
    fn text(&self) -> &str {
        core::str::from_utf8(self.0.as_bytes()).unwrap_or_default()
    }
}

impl From<Error> for Refusal {
    fn from(error: Error) -> Self {
        Refusal::of(error.message())
    }
}

impl From<SshError> for Refusal {
    fn from(error: SshError) -> Self {
        Refusal(Sentence::of(format_args!("{error}")))
    }
}

/// Opens a window and serves what is typed in it.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    match run(&mut gate, &startup) {
        Ok(()) => say(
            &mut gate,
            &startup,
            &Report::of(format_args!("[shell] done\n")),
        ),
        Err(error) => say(
            &mut gate,
            &startup,
            &Report::of(format_args!("[shell] no window: {}\n", error.message())),
        ),
    }
    report_to_parent(&mut gate, &startup);
    gate.thread_exit()
}

/// Opens the window, then takes what happens in it until it is gone.
fn run(gate: &mut Gate, startup: &Startup) -> Result<(), Error> {
    let process = startup.own_process.ok_or(Error::NotFound)?;
    let desk = match startup.desk_server {
        Some(given) => given,
        None => lookup(gate, startup.name_server.ok_or(Error::NotFound)?, DESK)?,
    };
    let notification = gate.notification_create()?;
    let signal = gate.handle_duplicate(notification.handle(), SIGNAL_ONLY)?;
    let watched = gate.handle_duplicate(process.handle(), WATCHED)?;
    let window = open(gate, desk, signal, watched)?;
    gate.handle_close(signal)?;
    gate.handle_close(watched)?;
    say(
        gate,
        startup,
        &Report::of(format_args!(
            "[shell] window {} of {}x{}\n",
            window.id, window.width, window.height
        )),
    );

    let idle = Idle::new(gate)?.with_step(STEP);
    let mut shell = Shell::new();
    shell.print("AuDHSOS shell. Type help.");
    // The compositor grants what fits its screen, which on a small one is
    // less than this shell draws into. What stands past the edge is
    // clipped there, so the window says so rather than losing rows in
    // silence.
    if window.width < WIDTH || window.height < HEIGHT {
        print(
            &mut shell,
            &Report::of(format_args!(
                "the window is {}x{} of the {WIDTH}x{HEIGHT} this shell draws",
                window.width, window.height
            )),
        );
    }
    paint(gate, desk, window.id, &mut shell)?;
    loop {
        gate.notification_wait(notification)?;
        while let Some(event) = poll(gate, desk, window.id)? {
            match shell.feed(event) {
                Step::Ready => {
                    if let Some(line) = shell.take_line()
                        && act(gate, startup, desk, window.id, &mut shell, &line, idle)
                    {
                        return Ok(());
                    }
                }
                Step::Ended => {
                    // The window is going away; giving it back is what
                    // lets the compositor end without waiting for the
                    // watch on this process.
                    let _closed = close(gate, desk, window.id);
                    return Ok(());
                }
                Step::Nothing | Step::Painted => {}
            }
        }
        paint(gate, desk, window.id, &mut shell)?;
    }
}

/// Runs one line and answers whether it ended the shell.
fn act(
    gate: &mut Gate,
    startup: &Startup,
    desk: EndpointHandle,
    id: u32,
    shell: &mut Shell,
    line: &Typed,
    idle: Idle,
) -> bool {
    let text = core::str::from_utf8(line.as_bytes()).unwrap_or_default();
    say(
        gate,
        startup,
        &Report::of(format_args!("[shell] ran {text}\n")),
    );
    match parse(text) {
        Command::Nothing => {}
        Command::Help => {
            for entry in HELP {
                shell.print(entry);
            }
        }
        Command::Clear => shell.clear(),
        Command::Time => now(gate, shell),
        Command::Echo(what) => shell.print(what),
        Command::Quit => {
            let _closed = close(gate, desk, id);
            return true;
        }
        Command::Usage(usage) => shell.print(usage),
        Command::Unknown(word) => {
            print(shell, &Report::of(format_args!("no such command: {word}")));
        }
        Command::Ssh(target) => {
            if let Err(why) = session(gate, startup, shell, &target, idle) {
                print(shell, &Report::of(format_args!("ssh: {}", why.text())));
            }
        }
        Command::Get(locator) => {
            if let Err(why) = fetch(gate, startup, shell, &locator, idle) {
                print(shell, &Report::of(format_args!("get: {}", why.text())));
            }
        }
    }
    false
}

/// Writes what the clock says into the scrollback.
fn now(gate: &mut Gate, shell: &mut Shell) {
    let Ok((micros, _source)) = gate.clock_wall() else {
        shell.print("no clock on this machine");
        return;
    };
    let seconds = i64::try_from(micros.wrapping_div(SECOND)).unwrap_or(0);
    let Ok(time) = audhsos_time::UnixTime::from_seconds(seconds).to_civil() else {
        shell.print("the clock says nothing this system can read");
        return;
    };
    print(
        shell,
        &Report::of(format_args!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            time.year, time.month, time.day, time.hour, time.minute, time.second
        )),
    );
}

/// Asks for a document over HTTP and writes what came back.
fn fetch(
    gate: &mut Gate,
    startup: &Startup,
    shell: &mut Shell,
    locator: &Locator<'_>,
    idle: Idle,
) -> Result<(), Refusal> {
    if locator.secure {
        shell.print("https is not spoken by this shell yet");
        return Ok(());
    }
    let server = network(gate, startup).ok_or_else(no_network)?;
    let process = startup
        .own_process
        .ok_or_else(|| Refusal::from(Error::NotFound))?;
    let until = gate.clock_now()?.saturating_add(DEADLINE);
    let address = address_of(gate, server, locator.host, idle, until)?;
    let stream = Stream::connect(
        gate,
        server,
        process,
        Endpoint::new(address, locator.port),
        RINGS,
        idle,
        until,
    )?;
    let outcome = exchange(gate, &stream, shell, locator, idle, until);
    let _shut = stream.shutdown_write(gate);
    let _closed = stream.close(gate);
    outcome
}

/// Writes the request and reads the answer over `stream`.
fn exchange(
    gate: &mut Gate,
    stream: &Stream,
    shell: &mut Shell,
    locator: &Locator<'_>,
    idle: Idle,
    until: u64,
) -> Result<(), Refusal> {
    let mut request = [0u8; CHUNK];
    let mut writer = WireWriter::new(&mut request);
    HttpRequest::get(locator.path, locator.host)
        .write(&mut writer)
        .map_err(|_| Refusal::of("the request does not fit one message"))?;
    let len = writer.position();
    stream.write_all(gate, request.get(..len).unwrap_or(&[]), idle, until)?;

    let mut head = [0u8; 1024];
    let mut decoder = Decoder::<8>::new(&mut head, Method::Get);
    let mut body = [0u8; MAX_BODY];
    let mut written = 0usize;
    let mut length = 0usize;
    let mut status = 0u16;
    let mut taken = [0u8; CHUNK];
    loop {
        let read = stream.read(gate, &mut taken, idle, until)?;
        let mut rest = taken.get(..read).unwrap_or(&[]);
        if rest.is_empty() {
            if decoder.ends_at_close() && decoder.finish().is_ok() {
                said(shell, status, length, body.get(..written).unwrap_or(&[]));
                return Ok(());
            }
            return Err(Refusal::of("the server closed before the answer was whole"));
        }
        while !rest.is_empty() {
            let (used, event) = decoder
                .feed(rest)
                .map_err(|_| Refusal::of("the answer is no HTTP response"))?;
            rest = rest.get(used..).unwrap_or(&[]);
            match event {
                HttpEvent::NeedMore => {
                    if used == 0 {
                        break;
                    }
                }
                HttpEvent::Head => status = decoder.head().map_or(0, |head| head.status.get()),
                HttpEvent::Body(bytes) => {
                    length = length.saturating_add(bytes.len());
                    written = keep(&mut body, written, bytes);
                }
                HttpEvent::Done => {
                    said(shell, status, length, body.get(..written).unwrap_or(&[]));
                    return Ok(());
                }
            }
        }
        if decoder.is_done() {
            said(shell, status, length, body.get(..written).unwrap_or(&[]));
            return Ok(());
        }
    }
}

/// Puts as much of `bytes` behind what `body` already holds as fits, and
/// answers with how much stands there.
fn keep(body: &mut [u8; MAX_BODY], written: usize, bytes: &[u8]) -> usize {
    let room = body.len().saturating_sub(written);
    let taking = room.min(bytes.len());
    let end = written.saturating_add(taking);
    if let (Some(slot), Some(source)) = (body.get_mut(written..end), bytes.get(..taking)) {
        slot.copy_from_slice(source);
    }
    end
}

/// Writes the status, the length and the beginning of the body.
fn said(shell: &mut Shell, status: u16, length: usize, body: &[u8]) {
    print(
        shell,
        &Report::of(format_args!("status {status}, {length} bytes")),
    );
    if !body.is_empty() {
        let mut line = Report::of(format_args!(""));
        for byte in body {
            line.put(&[printable(*byte)]);
        }
        print(shell, &line);
    }
}

/// The address `host` names: the four numbers it spells, or what the
/// resolver of the network server answers.
fn address_of(
    gate: &mut Gate,
    server: EndpointHandle,
    host: &str,
    idle: Idle,
    until: u64,
) -> Result<IpAddr, Refusal> {
    if let Some(address) = dotted(host) {
        return Ok(IpAddr::V4(address));
    }
    let name = Name::new(host.as_bytes())
        .map_err(|_| Refusal::of("that name is longer than the resolver takes"))?;
    loop {
        let reply = call(gate, server, &SocketRequest::Resolve { name })?;
        let SocketReply::Resolved(outcome) = reply else {
            return Err(Refusal::of("the network server answered something else"));
        };
        match outcome {
            Ok(addresses) => {
                let Some(address) = addresses.iter().next() else {
                    return Err(Refusal::of("the resolver knows no address for that name"));
                };
                return Ok(address);
            }
            Err(Error::WouldBlock) => idle.wait(gate, until)?,
            Err(error) => return Err(Refusal::from(error)),
        }
    }
}

/// The address four numbers separated by full stops spell, or nothing when
/// `text` is no such thing.
fn dotted(text: &str) -> Option<Ipv4Addr> {
    let mut octets = [0u8; 4];
    let mut count = 0usize;
    for part in text.split('.') {
        let slot = octets.get_mut(count)?;
        *slot = part.parse::<u8>().ok()?;
        count = count.saturating_add(1);
    }
    if count != 4 {
        return None;
    }
    let [first, second, third, fourth] = octets;
    Some(Ipv4Addr::new(first, second, third, fourth))
}

/// Runs a command over a Secure Shell connection and writes both of its
/// streams into the scrollback.
fn session(
    gate: &mut Gate,
    startup: &Startup,
    shell: &mut Shell,
    target: &Target<'_>,
    idle: Idle,
) -> Result<(), Refusal> {
    let server = network(gate, startup).ok_or_else(no_network)?;
    let process = startup
        .own_process
        .ok_or_else(|| Refusal::from(Error::NotFound))?;
    let (trusted, count, secret) = credentials(gate, startup)?;
    let until = gate.clock_now()?.saturating_add(DEADLINE);
    let address = address_of(gate, server, target.host, idle, until)?;
    let stream = Stream::connect(
        gate,
        server,
        process,
        Endpoint::new(address, target.port),
        RINGS,
        idle,
        until,
    )?;
    let outcome = speak(
        gate, &stream, shell, target, &trusted, count, secret, idle, until,
    );
    let _shut = stream.shutdown_write(gate);
    let _closed = stream.close(gate);
    outcome
}

/// The whole of the protocol over one connection.
#[expect(
    clippy::large_stack_arrays,
    clippy::too_many_arguments,
    reason = "the two buffers are the sizes RFC 4253, section 6.1, makes mandatory; the arguments are the connection, what is typed, what is trusted, and how long it may take"
)]
fn speak(
    gate: &mut Gate,
    stream: &Stream,
    shell: &mut Shell,
    target: &Target<'_>,
    trusted: &[[u8; FINGERPRINT_LEN]; MAX_TRUSTED],
    count: usize,
    secret: [u8; 32],
    idle: Idle,
    until: u64,
) -> Result<(), Refusal> {
    let key = ClientKey::new(secret);
    let trust = Fingerprints::new(trusted.get(..count).unwrap_or(&[]));
    let command = target.command.as_bytes();
    let settings = Config {
        user: target.user,
        key: &key,
        trust: &trust,
        command: (!command.is_empty()).then_some(command),
        window: 32_768,
        max_packet: u32::try_from(CHUNK).unwrap_or(u32::MAX),
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
    )?;

    let mut chunk = [0u8; CHUNK];
    for _ in 0..MAX_EVENTS {
        drain(gate, stream, &mut client, &mut chunk, idle, until)?;
        let now = gate.clock_now()?;
        let event = client.poll(now)?;
        match event {
            SshEvent::WantsWrite => {}
            SshEvent::WantsRead => {
                if fill(gate, stream, &mut client, &mut chunk)? == Received::Waiting {
                    idle.wait(gate, until)?;
                }
            }
            SshEvent::Started => client.finish()?,
            SshEvent::Data { stderr, len } => {
                if len > CHUNK {
                    return Err(Refusal::of(
                        "the peer sent more in one message than this client takes",
                    ));
                }
                let taken = client.recv(chunk.get_mut(..len).unwrap_or(&mut []));
                wrote(shell, stderr, chunk.get(..taken).unwrap_or(&[]));
            }
            SshEvent::ExitStatus(code) => {
                print(shell, &Report::of(format_args!("exit status {code}")));
            }
            SshEvent::ExitSignal => shell.print("the command was killed by a signal"),
            SshEvent::Closed => {
                drain(gate, stream, &mut client, &mut chunk, idle, until)?;
                return Ok(());
            }
        }
    }
    Err(Refusal::of("the session made no progress"))
}

/// Writes what one stream of the command wrote.
fn wrote(shell: &mut Shell, stderr: bool, bytes: &[u8]) {
    let mut line = Report::of(format_args!(""));
    if stderr {
        line.put(b"! ");
    }
    for byte in bytes {
        line.put(&[printable(*byte)]);
    }
    print(shell, &line);
}

/// `byte` when it can be written, a line break as itself, and a full stop
/// for everything else.
const fn printable(byte: u8) -> u8 {
    match byte {
        b'\n' => b'\n',
        0x20..=0x7e => byte,
        _other => b'.',
    }
}

/// Writes everything the protocol wants sent.
fn drain<R: crypto_rng::Rng, T: audhsos_ssh::hostkey::Trust>(
    gate: &mut Gate,
    stream: &Stream,
    client: &mut Connection<'_, R, T>,
    chunk: &mut [u8; CHUNK],
    idle: Idle,
    until: u64,
) -> Result<(), Error> {
    loop {
        let len = client.write_ssh(chunk);
        if len == 0 {
            return Ok(());
        }
        stream.write_all(gate, chunk.get(..len).unwrap_or(&[]), idle, until)?;
    }
}

/// Takes what the connection holds into the protocol.
fn fill<R: crypto_rng::Rng, T: audhsos_ssh::hostkey::Trust>(
    gate: &mut Gate,
    stream: &Stream,
    client: &mut Connection<'_, R, T>,
    chunk: &mut [u8; CHUNK],
) -> Result<Received, Error> {
    let received = stream.receive(gate, chunk)?;
    match received {
        Received::Bytes(taken) => {
            let mut rest = chunk.get(..taken).unwrap_or(&[]);
            while !rest.is_empty() {
                let read = client.read_ssh(rest);
                if read == 0 {
                    return Err(Error::BufferTooSmall);
                }
                rest = rest.get(read..).unwrap_or(&[]);
            }
        }
        Received::Ended => return Err(Error::Unavailable),
        Received::Waiting => {}
    }
    Ok(received)
}

/// What the volume says a session may trust and sign with.
#[expect(
    clippy::type_complexity,
    reason = "the fingerprints, how many of them there are, and the secret; a structure for three values used once would be this list under another name"
)]
fn credentials(
    gate: &mut Gate,
    startup: &Startup,
) -> Result<([[u8; FINGERPRINT_LEN]; MAX_TRUSTED], usize, [u8; 32]), Refusal> {
    let files = volume(gate, startup).ok_or_else(|| Refusal::of("this machine has no volume"))?;
    let mut trusted = [[0u8; FINGERPRINT_LEN]; MAX_TRUSTED];
    let mut text = [0u8; MAX_TRUST_FILE];
    let len = read_file(gate, files, TRUST, &mut text)?
        .ok_or_else(|| Refusal::of("the volume carries no SSHTRUST.TXT"))?;
    let count = read_trust(text.get(..len).unwrap_or(&[]), &mut trusted)?;
    if count == 0 {
        return Err(Refusal::of("SSHTRUST.TXT names no fingerprint"));
    }
    let mut secret = [0u8; 32];
    let len = read_file(gate, files, SECRET, &mut secret)?
        .ok_or_else(|| Refusal::of("the volume carries no SSHKEY.BIN"))?;
    if len != secret.len() {
        return Err(Refusal::of("SSHKEY.BIN is not thirty-two octets"));
    }
    Ok((trusted, count, secret))
}

/// The refusal of a machine whose network server is not there.
fn no_network() -> Refusal {
    Refusal::of("this machine has no network server")
}

/// The fingerprints, one `SHA256:` line at a time, and how many there are.
fn read_trust(
    text: &[u8],
    trusted: &mut [[u8; FINGERPRINT_LEN]; MAX_TRUSTED],
) -> Result<usize, Error> {
    let mut count = 0usize;
    for line in text.split(|byte| *byte == b'\n') {
        let Some(body) = after(line, b"SHA256:") else {
            continue;
        };
        if count >= MAX_TRUSTED {
            return Err(Error::BufferTooSmall);
        }
        let digest = fingerprint(body)?;
        let slot = trusted.get_mut(count).ok_or(Error::BufferTooSmall)?;
        *slot = digest;
        count = count.saturating_add(1);
    }
    Ok(count)
}

/// The thirty-two octets the unpadded Base64 of a fingerprint stands for.
fn fingerprint(body: &[u8]) -> Result<[u8; FINGERPRINT_LEN], Error> {
    let mut padded = [b'='; 44];
    let text = trim(body);
    let slot = padded.get_mut(..text.len()).ok_or(Error::InvalidArgument)?;
    slot.copy_from_slice(text);
    let mut digest = [0u8; FINGERPRINT_LEN];
    let len = audhsos_encoding::base64::decode(&padded, &mut digest)
        .map_err(|_| Error::InvalidArgument)?;
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
        } else {
            break;
        }
    }
    bytes
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
/// call behind it needs the gate of the thread, which a source held inside
/// the generator does not have.
struct Entropy;

impl crypto_rng::Entropy for Entropy {
    fn fill(&mut self, _out: &mut [u8]) -> Result<(), crypto_rng::EntropyError> {
        Err(crypto_rng::EntropyError::Unavailable)
    }
}

/// Opens the window at the compositor.
fn open(
    gate: &mut Gate,
    desk: EndpointHandle,
    notification: audhsos_abi::Handle,
    process: audhsos_abi::Handle,
) -> Result<user_proto::window::Window, Error> {
    let request = Request::Open {
        width: WIDTH,
        height: HEIGHT,
        title: Title::new(TITLE)?,
        notification,
        process,
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(desk)?;
    match Reply::decode(gate.reader())? {
        Reply::Opened(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Takes the next event of the window, if one waits.
fn poll(
    gate: &mut Gate,
    desk: EndpointHandle,
    id: u32,
) -> Result<Option<user_proto::window::Event>, Error> {
    Request::Poll { id }.encode(&mut gate.writer())?;
    gate.ipc_call(desk)?;
    match Reply::decode(gate.reader())? {
        Reply::Polled(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Gives the window back.
fn close(gate: &mut Gate, desk: EndpointHandle, id: u32) -> Result<(), Error> {
    Request::Close { id }.encode(&mut gate.writer())?;
    gate.ipc_call(desk)?;
    match Reply::decode(gate.reader())? {
        Reply::Closed(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Sends what the shell wants painted, in as many messages as it takes.
fn paint(gate: &mut Gate, desk: EndpointHandle, id: u32, shell: &mut Shell) -> Result<(), Error> {
    match shell.paint() {
        app_shell::state::Paint::Nothing => return Ok(()),
        app_shell::state::Paint::Input => {
            let mut commands = List::new();
            let _fitted = shell.render_input(&mut commands);
            send(gate, desk, id, &commands)?;
        }
        app_shell::state::Paint::All => {
            let mut from = Some(0);
            while let Some(row) = from {
                let mut commands = List::new();
                let next = shell.render(row, &mut commands);
                send(gate, desk, id, &commands)?;
                from = next;
            }
        }
    }
    shell.painted();
    Ok(())
}

/// Sends one list of draw commands.
fn send(gate: &mut Gate, desk: EndpointHandle, id: u32, commands: &List) -> Result<(), Error> {
    let request = Request::Draw {
        id,
        commands: *commands,
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(desk)?;
    match Reply::decode(gate.reader())? {
        Reply::Drawn(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Writes one line into the scrollback.
fn print(shell: &mut Shell, line: &Report) {
    shell.print(core::str::from_utf8(line.as_bytes()).unwrap_or_default());
}

/// Sends one request of the socket protocol and reads the reply.
fn call(
    gate: &mut Gate,
    server: EndpointHandle,
    request: &SocketRequest,
) -> Result<SocketReply, Error> {
    request.encode(&mut gate.writer())?;
    gate.ipc_call(server)?;
    Ok(SocketReply::decode(gate.reader())?)
}

/// Reads a whole file of the volume into `into`, or nothing when the
/// volume has no such file.
fn read_file(
    gate: &mut Gate,
    files: EndpointHandle,
    name: &[u8],
    into: &mut [u8],
) -> Result<Option<usize>, Error> {
    let name = FileName::new(name)?;
    let opened = match file_call(gate, files, &FileRequest::Open { parent: ROOT, name })? {
        FileReply::Opened(Ok(opened)) => opened,
        FileReply::Opened(Err(Error::NotFound | Error::Unavailable)) => return Ok(None),
        FileReply::Opened(Err(error)) => return Err(error),
        _other => return Err(Error::InvalidArgument),
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

/// Writes one line to the console the root task gave this program.
fn say(gate: &mut Gate, startup: &Startup, line: &Report) {
    let Some(console) = startup.log else {
        return;
    };
    let _said = write_line(gate, console, line.as_bytes());
}

/// Tells the process that started this one that the work is done.
fn report_to_parent(gate: &mut Gate, startup: &Startup) {
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
