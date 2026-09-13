// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The application that breaks on purpose.
//!
//! [6.6.22](../../../../../docs/06-testing-strategy.md#6622-end-to-end-tests-in-qemu)
//! asks that a client which faults be reported by the root task and that
//! the system keep running. Nothing else in the system does that, because
//! everything else is written not to; so one program is written to.
//!
//! It says what it is about to do and then writes to a page nothing has
//! mapped. Its thread stops there — a fault that goes unanswered leaves
//! the thread stopped — and the root task says so on the console. It
//! reports nothing to its parent, because it never finishes.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds thirteen programs and each uses a different part of what
// it depends on; these are the crates this one does not.
use app_canvas as _;
use audhsos_abi as _;
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
use user_proto as _;
use virtio_queue as _;

use user_programs::client::{lookup, write_line};
use user_rt::Startup;
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the console driver registered itself under.
const CONSOLE: &[u8] = b"console";

/// An address in the half of the space that belongs to user programs and
/// that nothing in this process has mapped.
const NOWHERE: u64 = 0x0000_2000_0000_0000;

/// Says so, and then does it.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    if let Some(names) = startup.name_server
        && let Ok(console) = lookup(&mut gate, names, CONSOLE)
    {
        let _said = write_line(&mut gate, console, b"[faulter] about to write to nowhere\n");
    }
    let pointer =
        core::ptr::without_provenance_mut::<u64>(usize::try_from(NOWHERE).unwrap_or(usize::MAX));
    // SAFETY: none, and that is the point: nothing is mapped there, so the
    // write faults and the kernel tells the fault handler of this process.
    // The thread does not come back from it.
    unsafe {
        pointer.write_volatile(1);
    }
    gate.thread_exit()
}
