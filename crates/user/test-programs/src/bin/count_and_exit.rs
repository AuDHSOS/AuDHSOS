// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A thread that writes into its own IPC buffer before it ends, so that a
//! test can tell that the thread ran and reached its buffer, and not only
//! that a system call arrived.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// What the thread leaves in the first payload word of its buffer.
pub const MARK: u64 = 0x5EED_0001;

/// Writes the mark, yields once so that the kernel has to switch, and
/// ends.
fn main(ipc_buffer: u64) -> ! {
    {
        // SAFETY: the address is the one the kernel started this thread
        // with, and the borrow ends before the call.
        let mut buffer = unsafe { sys::buffer(ipc_buffer) };
        buffer.set_word(0, MARK);
    }
    // SAFETY: the address is the one the kernel started this thread with.
    unsafe {
        let _ = sys::call(ipc_buffer, Syscall::ThreadYield, &[]);
    }
    loop {
        // SAFETY: as above. Nothing after the call runs, but a program of
        // this system never falls off its own end.
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
