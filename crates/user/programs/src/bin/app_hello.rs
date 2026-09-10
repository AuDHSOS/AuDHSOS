// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The application: it looks the console up by name, says one line through
//! it, and ends.
//!
//! It is the smallest program that proves the whole of the userland works.
//! For the line to arrive, the root task must have read the archive and
//! started three servers, the name server must hold what the console driver
//! registered, the message must reach the driver and the answer come back,
//! and the driver must own the port and put the bytes on the line. Nothing
//! of that is asserted here; the line either appears or it does not.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds twelve programs and each uses a different part of
// what it depends on; these are the crates this one does not.
use app_canvas as _;
use audhsos_abi as _;
use driver_i8042 as _;
use driver_uart16550 as _;
use gfx as _;
use server_console as _;
use server_display as _;
use server_input as _;
use server_memory as _;
use server_name as _;
use user_loader as _;

use user_programs::client::{lookup, read_bytes, write_line};
use user_proto::parent;
use user_rt::Startup;
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the console driver registered itself under.
const CONSOLE: &[u8] = b"console";

/// What this program has to say.
const GREETING: &[u8] = b"hello from userland\n";

/// What it says when it is ready to be typed at, which is what the
/// end-to-end run waits for before it sends anything: bytes sent earlier
/// would reach a controller whose receive path is not up yet.
const READY: &[u8] = b"[hello] ready\n";

/// How many bytes it reads back before it gives up waiting for a line.
const INPUT: usize = 64;

/// Says it, and ends.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    // The log is the console the root task gave this program, which is the
    // one place it can say anything before it has looked one up itself.
    if let Some(log) = startup.log {
        let _running = write_line(&mut gate, log, b"[hello] running\n");
    }
    let Some(names) = startup.name_server else {
        if let Some(log) = startup.log {
            let _said = write_line(&mut gate, log, b"[hello] no name server\n");
        }
        gate.thread_exit()
    };
    match lookup(&mut gate, names, CONSOLE) {
        Ok(console) => {
            let _said = write_line(&mut gate, console, GREETING);
            echo(&mut gate, console);
        }
        Err(error) => {
            if let Some(log) = startup.log {
                let mut line = user_rt::Line::<96>::new();
                line.put(b"[hello] no console: ");
                line.put(error.message().as_bytes());
                line.put(b"\n");
                let _said = write_line(&mut gate, log, line.as_bytes());
            }
        }
    }
    report(&mut gate, &startup);
    gate.thread_exit()
}

/// Tells the process that started this one that the work is done.
///
/// The endpoint is the one the kernel sends this program's faults to, so
/// its parent hears of a program that finished and of one that broke at
/// the same place. Nothing is expected back: this is a send, and the
/// thread exits right after it.
fn report(gate: &mut Gate, startup: &Startup) {
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

/// Says it is ready, waits for a line, and says it back.
///
/// The wait is the call itself: a read of an empty console is held by the
/// driver until a byte arrives, so this thread stands in `ipc_call` and the
/// processor goes to whoever else can use it. Asking again in a loop is
/// what this did before, and it kept the machine at full load for as long
/// as nobody typed: this system has no timer a program can ask for, so a
/// thread that keeps asking is always runnable and the kernel never halts.
fn echo(gate: &mut Gate, console: user_rt::EndpointHandle) {
    let _ready = write_line(gate, console, READY);
    let mut line = [0u8; INPUT];
    let mut have = 0usize;
    loop {
        let mut chunk = [0u8; INPUT];
        let Ok(taken) = read_bytes(gate, console, u64::try_from(INPUT).unwrap_or(0), &mut chunk)
        else {
            return;
        };
        for byte in chunk.get(..taken).unwrap_or(&[]) {
            if let Some(slot) = line.get_mut(have) {
                *slot = *byte;
                have = have.wrapping_add(1);
            }
            if *byte == b'\n' || have >= INPUT {
                say_back(gate, console, line.get(..have).unwrap_or(&[]));
                return;
            }
        }
    }
}

/// Says back what was typed, so that whoever typed it can see it arrived.
fn say_back(gate: &mut Gate, console: user_rt::EndpointHandle, line: &[u8]) {
    let mut said = user_rt::Line::<96>::new();
    said.put(b"[hello] echo: ");
    said.put(line);
    if line.last() != Some(&b'\n') {
        said.put(b"\n");
    }
    let _echoed = write_line(gate, console, said.as_bytes());
}
