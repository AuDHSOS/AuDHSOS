// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A thread that asks the kernel for nothing at all. It counts in its own
//! IPC buffer and never stops.
//!
//! What takes the processor away from it is the timer, and what ends it is
//! the kernel: the point of the program is that neither of those needs its
//! consent. Two of these at one priority only both get anywhere if the
//! time slice runs out and the scheduler rotates them; one of these at a
//! low priority only stops running when somebody of a higher priority
//! becomes ready.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

// This program makes no system call, so it names nothing of the interface.
use audhsos_abi as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// The payload word the count goes into.
pub const COUNT_WORD: usize = 0;

/// Counts, and leaves the count where the kernel can read it.
fn main(ipc_buffer: u64) -> ! {
    let mut count: u64 = 0;
    loop {
        count = count.saturating_add(1);
        // SAFETY: the address is the one the kernel started this thread
        // with, and the borrow ends at the end of the round; this thread
        // holds no other reference to its buffer and makes no call.
        unsafe {
            sys::buffer(ipc_buffer).set_word(COUNT_WORD, count);
        }
    }
}

#[panic_handler]
const fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Nothing of this program panics; the handler is what the language
    // asks for, and a thread that reached it has nothing left to do.
    loop {}
}
