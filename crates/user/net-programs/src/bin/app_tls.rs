// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The program that holds the trust anchors of the image: it reads the
//! table the boot volume carries and reports what it found (D-148).
//!
//! The anchors are what a certificate path is validated against, so a
//! program that speaks TLS needs them before it opens a connection. The
//! table is written by `cargo xtask image` out of the directory
//! `anchors/` and read here with the same code, `audhsos-x509::anchors`.
//!
//! The acceptance run of Phase 15 adds one anchor the image cannot carry:
//! the root of the chain its server presents, which reaches this program
//! over the scratch volume together with the port and the name (D-150).
//! Every run but that one finds no such file and holds the image's
//! anchors alone.
//!
//! What this program does not do yet is the handshake: the transport glue
//! between `audhsos-tls` and a connection of `server-net` is step T8 of
//! [document 11](../../../../docs/11-cryptography-and-tls.md) and Phase
//! 15. This is the half of it that the anchors are: without them a
//! validated chain has nothing to reach.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

// The package holds four programs and each uses a different part of what
// it depends on; these are the crates this one does not.
use audhsos_encoding as _;
use audhsos_ssh as _;
use audhsos_time as _;
use crypto_rng as _;
use driver_virtio_net as _;
use net_http as _;
use net_stack as _;
use net_wire as _;
use server_net as _;
use user_net_programs as _;
use virtio_queue as _;

use audhsos_abi::Error;
use audhsos_x509::TrustAnchor;
use audhsos_x509::anchors::Anchors;
use user_programs::client::{lookup, write_line};
use user_proto::file::{BOOT, Name, ROOT, Reply as FileReply, Request as FileRequest};
use user_rt::{EndpointHandle, Line, Startup};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the file system server registered itself under.
const FILES: &[u8] = b"files";

/// The name the console driver registered itself under.
const CONSOLE: &[u8] = b"console";

/// The directory of the volume the table lies in.
const DIRECTORY: &[u8] = b"AUDHSOS";

/// The file the table lies in.
const TABLE: &[u8] = b"ANCHORS.BIN";

/// Room for the table. The five public roots of `anchors/` are about five
/// kibibytes together, and this is the largest local array the lint set
/// allows, so an operator has room for roughly ten more.
const MAX_TABLE: usize = 16384;

/// How many anchors a program of this system reads.
const MAX_ANCHORS: usize = 32;

/// The file of the scratch volume naming the port and the name (D-150).
const RUN_CONFIG: &[u8] = b"TLSCONF.TXT";

/// The file of the scratch volume holding the root of the run's chain.
const RUN_ROOT: &[u8] = b"TLSROOT.DER";

/// Room for that root. A certificate of the test builder is under a
/// kibibyte; a root with an RSA key of 4096 bits stays under this.
const MAX_RUN_ROOT: usize = 2048;

/// How long a name the run may ask for.
const MAX_NAME: usize = 64;

/// How many bytes of the run configuration this program reads.
const MAX_RUN_CONFIG: usize = 256;

/// One line of output.
type Report = Line<256>;

#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let voice = voice(&mut gate, &startup);
    let mut table = [0u8; MAX_TABLE];
    let mut root = [0u8; MAX_RUN_ROOT];
    // The trust set a handshake validates a chain against: the anchors of
    // the image first, the root of the run behind them. Counting it is
    // what this program does with it until the glue of Phase 15 arrives.
    let mut anchors = [TrustAnchor {
        subject: &[],
        spki: &[],
    }; MAX_ANCHORS];

    let carried = read_table(&mut gate, &startup, &mut table);
    let run = read_run(&mut gate, &startup, &mut root);

    let mut held = 0usize;
    match carried {
        Ok(None) => say(
            &mut gate,
            voice,
            &Report::of(format_args!("[tls-app] no anchor table on the volume\n")),
        ),
        Ok(Some(len)) => {
            held = report_anchors(
                &mut gate,
                voice,
                table.get(..len).unwrap_or(&[]),
                &mut anchors,
            );
        }
        Err(error) => say(
            &mut gate,
            voice,
            &Report::of(format_args!(
                "[tls-app] unreadable anchor table: {}\n",
                error.message()
            )),
        ),
    }
    match run {
        Ok(None) => say(
            &mut gate,
            voice,
            &Report::of(format_args!("[tls-app] no run configuration\n")),
        ),
        Ok(Some(run)) => {
            held = report_run(
                &mut gate,
                voice,
                &run,
                root.get(..run.root_len).unwrap_or(&[]),
                &mut anchors,
                held,
            );
        }
        Err(error) => say(
            &mut gate,
            voice,
            &Report::of(format_args!(
                "[tls-app] unreadable run configuration: {}\n",
                error.message()
            )),
        ),
    }
    say(
        &mut gate,
        voice,
        &Report::of(format_args!("[tls-app] trust anchors={held}\n")),
    );
    finish(&mut gate, &startup);
    gate.thread_exit()
}

/// What the scratch volume says the run is (D-150).
struct Run {
    /// The port the server took on the gateway.
    port: u16,
    /// The name its certificate carries.
    name: [u8; MAX_NAME],
    /// How many bytes of that name there are.
    name_len: usize,
    /// How many bytes of the root certificate the run wrote.
    root_len: usize,
}

/// Parses the table, writes its anchors into `anchors`, and says how many
/// there are. Answers how many of `anchors` are filled.
fn report_anchors<'a>(
    gate: &mut Gate,
    voice: Option<EndpointHandle>,
    bytes: &'a [u8],
    anchors: &mut [TrustAnchor<'a>; MAX_ANCHORS],
) -> usize {
    let table = match Anchors::parse(bytes) {
        Ok(table) => table,
        Err(error) => {
            say(
                gate,
                voice,
                &Report::of(format_args!("[tls-app] the table is refused: {error}\n")),
            );
            return 0;
        }
    };
    match table.read_into(anchors) {
        Ok(count) => {
            say(
                gate,
                voice,
                &Report::of(format_args!(
                    "[tls-app] anchors={count} bytes={}\n",
                    bytes.len()
                )),
            );
            for anchor in anchors.get(..count).unwrap_or(&[]) {
                say(
                    gate,
                    voice,
                    &Report::of(format_args!(
                        "[tls-app] anchor subject={} key={}\n",
                        anchor.subject.len(),
                        anchor.spki.len()
                    )),
                );
            }
            count
        }
        Err(error) => {
            say(
                gate,
                voice,
                &Report::of(format_args!("[tls-app] an anchor is refused: {error}\n")),
            );
            0
        }
    }
}

/// Says where the run's server is and adds the root of its chain behind
/// the `held` anchors of the image. Answers how many anchors there are
/// then, which is `held` again where the root is refused.
fn report_run<'a>(
    gate: &mut Gate,
    voice: Option<EndpointHandle>,
    run: &Run,
    root: &'a [u8],
    anchors: &mut [TrustAnchor<'a>; MAX_ANCHORS],
    held: usize,
) -> usize {
    let name = run.name.get(..run.name_len).unwrap_or(&[]);
    let mut line = Report::of(format_args!(
        "[tls-app] run server=10.0.2.2:{} name=",
        run.port
    ));
    line.put(name);
    line.put(b"\n");
    say(gate, voice, &line);

    let anchor = match TrustAnchor::from_certificate(root) {
        Ok(anchor) => anchor,
        Err(error) => {
            say(
                gate,
                voice,
                &Report::of(format_args!("[tls-app] the run root is refused: {error}\n")),
            );
            return held;
        }
    };
    let Some(slot) = anchors.get_mut(held) else {
        say(
            gate,
            voice,
            &Report::of(format_args!("[tls-app] no room for the run root\n")),
        );
        return held;
    };
    *slot = anchor;
    say(
        gate,
        voice,
        &Report::of(format_args!(
            "[tls-app] run anchor subject={} key={}\n",
            anchor.subject.len(),
            anchor.spki.len()
        )),
    );
    held.saturating_add(1)
}

/// What the run wrote onto the scratch volume, or `None` where no run
/// wrote anything: the port, the name, and the root in `into` (D-150).
fn read_run(gate: &mut Gate, startup: &Startup, into: &mut [u8]) -> Result<Option<Run>, Error> {
    let Some(files) = volume(gate, startup) else {
        return Ok(None);
    };
    let mut text = [0u8; MAX_RUN_CONFIG];
    let Some(len) = read_root_file(gate, files, RUN_CONFIG, &mut text)? else {
        return Ok(None);
    };
    let mut run = Run {
        port: 0,
        name: [0u8; MAX_NAME],
        name_len: 0,
        root_len: 0,
    };
    read_config(text.get(..len).unwrap_or(&[]), &mut run)?;
    let Some(root_len) = read_root_file(gate, files, RUN_ROOT, into)? else {
        return Err(Error::NotFound);
    };
    run.root_len = root_len;
    Ok(Some(run))
}

/// The two lines of the run configuration.
fn read_config(text: &[u8], run: &mut Run) -> Result<(), Error> {
    for line in text.split(|byte| *byte == b'\n') {
        if let Some(value) = after(line, b"port ") {
            run.port = number(value)?;
        } else if let Some(value) = after(line, b"name ") {
            run.name_len = copy(value, &mut run.name)?;
        }
    }
    if run.port == 0 || run.name_len == 0 {
        return Err(Error::InvalidArgument);
    }
    Ok(())
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
    let digits = trim(text);
    if digits.is_empty() {
        return Err(Error::InvalidArgument);
    }
    let mut value: u16 = 0;
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

/// Reads one file of the root of the scratch volume, or answers `None`
/// where the volume carries no such file.
fn read_root_file(
    gate: &mut Gate,
    files: EndpointHandle,
    name: &[u8],
    into: &mut [u8],
) -> Result<Option<usize>, Error> {
    let Some(file) = open(gate, files, ROOT, name)? else {
        return Ok(None);
    };
    let taken = read_into(gate, files, file, into);
    let _closed = file_call(gate, files, &FileRequest::Close { file })?;
    Ok(Some(taken?))
}

/// The bytes of the table, or `None` where the volume carries none.
fn read_table(gate: &mut Gate, startup: &Startup, into: &mut [u8]) -> Result<Option<usize>, Error> {
    let Some(files) = volume(gate, startup) else {
        return Ok(None);
    };
    let Some(directory) = open(gate, files, BOOT, DIRECTORY)? else {
        return Ok(None);
    };
    let opened = open(gate, files, directory, TABLE)?;
    let _closed = file_call(gate, files, &FileRequest::Close { file: directory })?;
    let Some(file) = opened else {
        return Ok(None);
    };
    let taken = read_into(gate, files, file, into);
    let _closed = file_call(gate, files, &FileRequest::Close { file })?;
    Ok(Some(taken?))
}

/// The handle of `name` under `parent`, or `None` where there is no such
/// entry.
fn open(
    gate: &mut Gate,
    files: EndpointHandle,
    parent: u32,
    name: &[u8],
) -> Result<Option<u32>, Error> {
    let name = Name::new(name)?;
    match file_call(gate, files, &FileRequest::Open { parent, name })? {
        FileReply::Opened(Ok(opened)) => Ok(Some(opened.file)),
        FileReply::Opened(Err(Error::NotFound | Error::Unavailable)) => Ok(None),
        FileReply::Opened(Err(error)) => Err(error),
        _ => Err(Error::InvalidArgument),
    }
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

/// The file system server, if this machine has one.
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

/// Writes one line, where there is anything to write it to.
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
        let _sent = gate.ipc_send(parent);
    }
}
