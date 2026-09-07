// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The calling half of the measured pair: an empty message, sent over and
//! over to whoever is receiving on the endpoint, each one waited for.
//!
//! The message carries no words and no handles, so what the figure holds is
//! the rendezvous and the two switches it takes, and not the copying of a
//! payload whose size the caller chose.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// The payload word the handle of the endpoint stands in.
pub const ENDPOINT_WORD: usize = 0;

/// The payload word the number of rounds stands in.
pub const ROUNDS_WORD: usize = 1;

/// Calls as often as the kernel asked for, then ends.
fn main(ipc_buffer: u64) -> ! {
    let endpoint = given(ipc_buffer, ENDPOINT_WORD);
    let rounds = given(ipc_buffer, ROUNDS_WORD);
    let mut round: u64 = 0;
    while round < rounds {
        empty_message(ipc_buffer);
        // SAFETY: the address is the one the kernel started this thread
        // with, and no other reference to the buffer is alive.
        let (status, _) = unsafe { sys::call(ipc_buffer, Syscall::IpcCall, &[endpoint]) };
        if status != 0 {
            break;
        }
        round = round.saturating_add(1);
    }
    end(ipc_buffer)
}

/// Writes the header of a message with nothing in it.
fn empty_message(ipc_buffer: u64) {
    // SAFETY: as `main`; the borrow ends at the end of the block.
    unsafe {
        let mut buffer = sys::buffer(ipc_buffer);
        buffer.set_label(0);
        let _ = buffer.set_counts(0, 0);
    }
}

/// The word the kernel left at `index` of the buffer.
fn given(ipc_buffer: u64, index: usize) -> u64 {
    // SAFETY: as `main`.
    unsafe { sys::buffer(ipc_buffer) }
        .reader()
        .word(index)
        .unwrap_or(0)
}

/// Ends the thread. Nothing after this runs.
fn end(ipc_buffer: u64) -> ! {
    loop {
        // SAFETY: as `main`.
        unsafe {
            let _ = sys::call(ipc_buffer, Syscall::ThreadExit, &[]);
        }
    }
}

#[panic_handler]
const fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Nothing of this program panics; the handler is what the language
    // asks for, and a thread that reached it has nothing left to do.
    loop {}
}
