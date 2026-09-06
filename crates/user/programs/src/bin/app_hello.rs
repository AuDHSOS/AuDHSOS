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

// The package holds five programs and each uses a different part of
// what it depends on; these are the crates this one does not.
use audhsos_abi as _;
use driver_uart16550 as _;
use server_console as _;
use server_memory as _;
use server_name as _;
use user_loader as _;
use user_proto as _;

use user_programs::client::{lookup, write_line};
use user_rt::Startup;
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the console driver registered itself under.
const CONSOLE: &[u8] = b"console";

/// What this program has to say.
const GREETING: &[u8] = b"hello from userland\n";

/// Says it, and ends.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let Some(names) = startup.name_server else {
        gate.thread_exit()
    };
    match lookup(&mut gate, names, CONSOLE) {
        Ok(console) => {
            let _said = write_line(&mut gate, console, GREETING);
        }
        Err(error) => {
            // No console to report on; the kernel still owns the port at
            // this point only if the driver never started, and this is what
            // says so.
            let _logged = sys::write_line(&mut gate, startup.log, error.message().as_bytes());
        }
    }
    gate.thread_exit()
}
