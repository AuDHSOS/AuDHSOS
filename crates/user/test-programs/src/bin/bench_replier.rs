// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The answering half of the measured pair: receive, answer with nothing,
//! receive again.
//!
//! It has no count of its own. It stops when a receive fails, and it waits
//! for ever when the caller has finished — which is what leaves the
//! processor to the image, and how the image learns that the run is over.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// The payload word the handle of the endpoint stands in.
pub const ENDPOINT_WORD: usize = 0;

/// Answers messages until a receive fails.
fn main(ipc_buffer: u64) -> ! {
    let endpoint = given(ipc_buffer, ENDPOINT_WORD);
    loop {
        // SAFETY: the address is the one the kernel started this thread
        // with, and no other reference to the buffer is alive.
        let (status, values) = unsafe { sys::returning(ipc_buffer, Syscall::IpcRecv, &[endpoint]) };
        if status != 0 {
            break;
        }
        empty_message(ipc_buffer);
        // SAFETY: as above.
        let (status, _) = unsafe { sys::call(ipc_buffer, Syscall::IpcReply, &[values[1]]) };
        if status != 0 {
            break;
        }
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
