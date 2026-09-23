// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A thread that panics. The handler calls `sys::stop`, as the handler
//! `sys::program!` writes does: `ud2` raises an invalid-opcode fault and
//! the kernel stops the thread (D-193).
//!
//! The entry is `sys::entry!`, because `sys::program!` reads the startup
//! message into a value larger than the two stack pages of a test thread.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

// This program makes no system call, so it names nothing of the interface.
use audhsos_abi as _;
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// Panics, which is the point of the program.
#[expect(clippy::panic, reason = "the panic handler is what this program tests")]
fn main(_ipc_buffer: u64) -> ! {
    panic!("panic_in_user panics on purpose")
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    sys::stop()
}
