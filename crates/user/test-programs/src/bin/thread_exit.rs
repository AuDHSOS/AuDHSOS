// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A thread that ends itself, which is the shortest program there is: the
//! kernel sees a thread reach user mode, make one system call, and stop.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// Ends the thread. Nothing after the call runs.
fn main(ipc_buffer: u64) -> ! {
    loop {
        // SAFETY: the address is the one the kernel started this thread
        // with, and nothing else holds a reference to the buffer.
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
