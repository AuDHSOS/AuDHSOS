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
use user_proto::file::{BOOT, Name, Reply as FileReply, Request as FileRequest};
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

/// One line of output.
type Report = Line<256>;

#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let voice = voice(&mut gate, &startup);
    let mut table = [0u8; MAX_TABLE];
    match read_table(&mut gate, &startup, &mut table) {
        Ok(None) => say(
            &mut gate,
            voice,
            &Report::of(format_args!("[tls-app] no anchor table on the volume\n")),
        ),
        Ok(Some(len)) => report_anchors(&mut gate, voice, table.get(..len).unwrap_or(&[])),
        Err(error) => say(
            &mut gate,
            voice,
            &Report::of(format_args!(
                "[tls-app] unreadable anchor table: {}\n",
                error.message()
            )),
        ),
    }
    finish(&mut gate, &startup);
    gate.thread_exit()
}

/// Parses the table and says how many anchors it holds.
fn report_anchors(gate: &mut Gate, voice: Option<EndpointHandle>, bytes: &[u8]) {
    let table = match Anchors::parse(bytes) {
        Ok(table) => table,
        Err(error) => {
            say(
                gate,
                voice,
                &Report::of(format_args!("[tls-app] the table is refused: {error}\n")),
            );
            return;
        }
    };
    let mut anchors = [TrustAnchor {
        subject: &[],
        spki: &[],
    }; MAX_ANCHORS];
    match table.read_into(&mut anchors) {
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
        }
        Err(error) => say(
            gate,
            voice,
            &Report::of(format_args!("[tls-app] an anchor is refused: {error}\n")),
        ),
    }
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
