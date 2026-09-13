// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The program that uses the file system server: it makes a file, writes
//! into it, closes it, opens it again and reads the bytes back.
//!
//! It also proves that a volume survives a boot. The first boot finds no
//! `BOOT.TXT` and writes one; the second finds it and compares the bytes,
//! which is what the scratch disk exists for (D-136).
//!
//! A machine whose file system server was given no disk answers
//! `Unavailable` to everything, and this reports that and ends.

#![no_std]
#![no_main]

// The package holds fifteen programs and each uses a different part of
// what it depends on; these are the crates this one does not.
use app_canvas as _;
use audhsos_time as _;
use driver_i8042 as _;
use driver_uart16550 as _;
use driver_virtio_blk as _;
use fs_fat as _;
use gfx as _;
use pci as _;
use server_console as _;
use server_display as _;
use server_fs as _;
use server_input as _;
use server_memory as _;
use server_name as _;
use user_loader as _;
use virtio_queue as _;

use audhsos_abi::Error;
use user_programs::client::{lookup, write_line};
use user_proto::file::{Data, Name, ROOT, Reply, Request, START};
use user_proto::parent;
use user_rt::{EndpointHandle, Line, Startup};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the file system server registered itself under.
const FILES: &[u8] = b"files";

/// The name the console driver registered itself under.
const CONSOLE: &[u8] = b"console";

/// The file that says a volume survived a boot.
const BOOT: &[u8] = b"BOOT.TXT";

/// What that file holds.
const MARK: &[u8] = b"a volume that survived a boot\n";

/// The file the byte-for-byte test writes, whose length crosses a cluster
/// boundary: a cluster of the scratch volume is one sector.
const LONG: &[u8] = b"LONG.BIN";

/// How many bytes that file holds. More than a sector, so the read and
/// the write both cross a cluster.
const LONG_LEN: usize = 700;

/// How long a line this program writes.
type Report = Line<160>;

/// Writes, reads back, and says what it found.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let voice = voice(&mut gate, &startup);
    match server(&mut gate, &startup) {
        Some(files) => {
            if let Err(error) = work(&mut gate, files, voice) {
                say(
                    &mut gate,
                    voice,
                    &Report::of(format_args!("[files-app] {}\n", error.message())),
                );
            }
        }
        None => say(
            &mut gate,
            voice,
            &Report::of(format_args!("[files-app] no server\n")),
        ),
    }
    finish(&mut gate, &startup);
    gate.thread_exit()
}

/// Everything this program checks.
fn work(
    gate: &mut Gate,
    files: EndpointHandle,
    voice: Option<EndpointHandle>,
) -> Result<(), Error> {
    boot_mark(gate, files, voice)?;
    long_file(gate, files, voice)?;
    listing(gate, files, voice)
}

/// The file that says whether the volume survived a boot.
fn boot_mark(
    gate: &mut Gate,
    files: EndpointHandle,
    voice: Option<EndpointHandle>,
) -> Result<(), Error> {
    let name = Name::new(BOOT)?;
    match call(gate, files, &Request::Open { parent: ROOT, name })? {
        Reply::Opened(Ok(opened)) => {
            let read = read_all(gate, files, opened.file, opened.size)?;
            close(gate, files, opened.file)?;
            let same = read.as_bytes() == MARK;
            say(
                gate,
                voice,
                &Report::of(format_args!(
                    "[files-app] second boot: {} bytes, same={same}\n",
                    read.len()
                )),
            );
            Ok(())
        }
        Reply::Opened(Err(Error::NotFound)) => {
            let file = create(gate, files, name, false)?;
            let written = write_all(gate, files, file, MARK)?;
            close(gate, files, file)?;
            flush(gate, files)?;
            say(
                gate,
                voice,
                &Report::of(format_args!(
                    "[files-app] first boot: wrote {written} bytes\n"
                )),
            );
            Ok(())
        }
        Reply::Opened(Err(error)) => Err(error),
        _ => Err(Error::InvalidArgument),
    }
}

/// A file whose length crosses a cluster, written and read back.
fn long_file(
    gate: &mut Gate,
    files: EndpointHandle,
    voice: Option<EndpointHandle>,
) -> Result<(), Error> {
    let name = Name::new(LONG)?;
    let _removed = call(gate, files, &Request::Remove { parent: ROOT, name })?;
    let mut wanted = [0u8; LONG_LEN];
    for (index, byte) in wanted.iter_mut().enumerate() {
        *byte = u8::try_from(index % 251).unwrap_or(0);
    }
    let file = create(gate, files, name, false)?;
    let written = write_all(gate, files, file, &wanted)?;
    close(gate, files, file)?;
    flush(gate, files)?;

    let opened = match call(gate, files, &Request::Open { parent: ROOT, name })? {
        Reply::Opened(outcome) => outcome?,
        _ => return Err(Error::InvalidArgument),
    };
    let mut read = [0u8; LONG_LEN];
    let taken = read_into(gate, files, opened.file, &mut read)?;
    close(gate, files, opened.file)?;
    let same = taken == LONG_LEN && read == wanted;
    say(
        gate,
        voice,
        &Report::of(format_args!(
            "[files-app] long file: wrote {written} read {taken} same={same}\n"
        )),
    );
    Ok(())
}

/// The entries of the root directory, which is what a listing is.
fn listing(
    gate: &mut Gate,
    files: EndpointHandle,
    voice: Option<EndpointHandle>,
) -> Result<(), Error> {
    let mut cursor = START;
    let mut count = 0usize;
    loop {
        let reply = call(gate, files, &Request::ReadDir { dir: ROOT, cursor })?;
        let Reply::Entry(outcome) = reply else {
            return Err(Error::InvalidArgument);
        };
        let Some(entry) = outcome? else {
            break;
        };
        cursor = entry.cursor;
        count = count.saturating_add(1);
        say(
            gate,
            voice,
            &Report::of(format_args!(
                "[files-app] entry {} of {} bytes\n",
                Text(entry.name.as_bytes()),
                entry.size
            )),
        );
        if count >= MAX_ENTRIES {
            break;
        }
    }
    say(
        gate,
        voice,
        &Report::of(format_args!("[files-app] entries={count}\n")),
    );
    Ok(())
}

/// How many entries this program lists before it stops.
const MAX_ENTRIES: usize = 16;

/// A name as the line writes it.
struct Text<'a>(&'a [u8]);

impl core::fmt::Display for Text<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for byte in self.0 {
            f.write_str(core::str::from_utf8(&[*byte]).unwrap_or("?"))?;
        }
        Ok(())
    }
}

/// Makes a file and answers its handle.
fn create(
    gate: &mut Gate,
    files: EndpointHandle,
    name: Name,
    directory: bool,
) -> Result<u32, Error> {
    match call(
        gate,
        files,
        &Request::Create {
            parent: ROOT,
            name,
            directory,
        },
    )? {
        Reply::Created(outcome) => outcome,
        _ => Err(Error::InvalidArgument),
    }
}

/// Writes every byte of `bytes`, in as many messages as it takes.
fn write_all(
    gate: &mut Gate,
    files: EndpointHandle,
    file: u32,
    bytes: &[u8],
) -> Result<u32, Error> {
    let mut done = 0usize;
    while done < bytes.len() {
        let end = done
            .saturating_add(user_proto::file::MAX_DATA)
            .min(bytes.len());
        let chunk = bytes.get(done..end).unwrap_or(&[]);
        let reply = call(
            gate,
            files,
            &Request::Write {
                file,
                offset: u32::try_from(done).unwrap_or(0),
                data: Data::new(chunk)?,
            },
        )?;
        let Reply::Written(outcome) = reply else {
            return Err(Error::InvalidArgument);
        };
        let taken = usize::try_from(outcome?).unwrap_or(0);
        if taken == 0 {
            return Err(Error::Unavailable);
        }
        done = done.saturating_add(taken);
    }
    u32::try_from(done).map_err(|_| Error::InvalidArgument)
}

/// Reads `size` bytes of `file` into a line-sized buffer.
fn read_all(gate: &mut Gate, files: EndpointHandle, file: u32, size: u32) -> Result<Data, Error> {
    let mut into = [0u8; user_proto::file::MAX_DATA];
    let wanted = usize::try_from(size)
        .unwrap_or(0)
        .min(user_proto::file::MAX_DATA);
    let taken = read_into(gate, files, file, into.get_mut(..wanted).unwrap_or(&mut []))?;
    Data::new(into.get(..taken).unwrap_or(&[])).map_err(Error::from)
}

/// Reads as much of `file` as `into` holds, in as many messages as it
/// takes, and answers how many bytes that was.
fn read_into(
    gate: &mut Gate,
    files: EndpointHandle,
    file: u32,
    into: &mut [u8],
) -> Result<usize, Error> {
    let mut done = 0usize;
    while done < into.len() {
        let want = into.len().saturating_sub(done);
        let reply = call(
            gate,
            files,
            &Request::Read {
                file,
                offset: u32::try_from(done).unwrap_or(0),
                len: u32::try_from(want).unwrap_or(0),
            },
        )?;
        let Reply::Read(outcome) = reply else {
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

/// Gives a handle back.
fn close(gate: &mut Gate, files: EndpointHandle, file: u32) -> Result<(), Error> {
    match call(gate, files, &Request::Close { file })? {
        Reply::Closed(outcome) => outcome,
        _ => Err(Error::InvalidArgument),
    }
}

/// Puts what was written onto the disk.
fn flush(gate: &mut Gate, files: EndpointHandle) -> Result<(), Error> {
    match call(gate, files, &Request::Flush)? {
        Reply::Flushed(outcome) => outcome,
        _ => Err(Error::InvalidArgument),
    }
}

/// Sends one request and reads the reply.
fn call(gate: &mut Gate, files: EndpointHandle, request: &Request) -> Result<Reply, Error> {
    request.encode(&mut gate.writer())?;
    gate.ipc_call(files)?;
    Reply::decode(gate.reader()).map_err(Error::from)
}

/// The endpoint of the file system server, looked up by name.
fn server(gate: &mut Gate, startup: &Startup) -> Option<EndpointHandle> {
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
    let finished = parent::Request::Finished {
        status: parent::SUCCESS,
    };
    if finished.encode(&mut gate.writer()).is_ok() {
        let _reported = gate.ipc_send(parent);
    }
}
