// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A thread that does nothing but enter the kernel and come back, as many
//! times as it is told to.
//!
//! `thread_yield` is the shortest call the table holds: with one runnable
//! thread the scheduler finds nobody else and the kernel returns to the
//! caller. What is left is the round trip itself — the trap, the dispatch,
//! and the return to ring three — which is what the image measures.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// The payload word the number of rounds stands in.
pub const ROUNDS_WORD: usize = 0;

/// Yields as often as the kernel asked for, then ends.
fn main(ipc_buffer: u64) -> ! {
    let rounds = given(ipc_buffer, ROUNDS_WORD);
    let mut round: u64 = 0;
    while round < rounds {
        // SAFETY: the address is the one the kernel started this thread
        // with, and no other reference to the buffer is alive.
        unsafe {
            let _ = sys::call(ipc_buffer, Syscall::ThreadYield, &[]);
        }
        round = round.saturating_add(1);
    }
    end(ipc_buffer)
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
