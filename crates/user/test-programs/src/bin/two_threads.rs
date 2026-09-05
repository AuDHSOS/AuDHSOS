// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Two threads of one process, taking turns at a counter they share.
//!
//! Both threads run this program in one address space. The kernel maps one
//! page read and write into that space and leaves its address in
//! [`SHARED_WORD`] of the buffer of every thread before it starts them:
//! the buffer is the only thing the kernel and a thread of this system
//! both reach.
//!
//! A round is one ticket: read the counter, write it back one higher,
//! write the ticket down, and give up the processor. Two threads of equal
//! priority take alternate tickets, because a thread that yields goes to
//! the end of the queue of its priority and there is exactly one other
//! thread in it. That is what the test reads back out of the two buffers.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// The payload word the kernel left the address of the shared page in.
pub const SHARED_WORD: usize = 0;

/// The payload word the first ticket goes into; the rounds follow.
pub const FIRST_TICKET: usize = 1;

/// How many tickets a thread takes before it ends itself.
pub const ROUNDS: usize = 4;

/// Takes [`ROUNDS`] tickets, one per turn, and ends.
fn main(ipc_buffer: u64) -> ! {
    // SAFETY: the address is the one the kernel started this thread with,
    // and the borrow ends before the first call.
    let shared = unsafe { sys::buffer(ipc_buffer) }
        .reader()
        .word(SHARED_WORD)
        .unwrap_or(0);
    for round in 0..ROUNDS {
        let ticket = take_ticket(shared);
        {
            // SAFETY: as above; nothing else holds a reference to the
            // buffer while this one is alive.
            let mut buffer = unsafe { sys::buffer(ipc_buffer) };
            buffer.set_word(FIRST_TICKET.saturating_add(round), ticket);
        }
        // SAFETY: as above. The yield is what makes the turn end: without
        // it a thread would hold the processor for its whole time slice.
        unsafe {
            let _ = sys::call(ipc_buffer, Syscall::ThreadYield, &[]);
        }
    }
    loop {
        // SAFETY: as above. Nothing after the call runs, but a program of
        // this system never falls off its own end.
        unsafe {
            let _ = sys::call(ipc_buffer, Syscall::ThreadExit, &[]);
        }
    }
}

/// Reads the counter of the shared page and writes it back one higher,
/// returning what it was. Nothing interrupts the two accesses: the thread
/// gives up the processor of its own accord and the image runs no timer.
fn take_ticket(shared: u64) -> u64 {
    let counter = core::ptr::without_provenance_mut::<u64>(usize::try_from(shared).unwrap_or(0));
    // SAFETY: the kernel mapped one page of memory there, read and write,
    // for the threads of this process, and this is the first word of it.
    let ticket = unsafe { core::ptr::read_volatile(counter) };
    // SAFETY: as above, and the thread holds the counter until it gives
    // the processor up of its own accord.
    unsafe {
        core::ptr::write_volatile(counter, ticket.saturating_add(1));
    }
    ticket
}

#[panic_handler]
const fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Nothing of this program panics; the handler is what the language
    // asks for, and a thread that reached it has nothing left to do.
    loop {}
}
