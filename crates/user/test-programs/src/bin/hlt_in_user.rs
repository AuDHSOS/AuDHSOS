// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A thread that runs an instruction only the kernel may run. The
//! processor raises a general protection fault and the kernel stops the
//! thread; nothing of the machine stops with it.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use core::arch::asm;

// This program makes no system call, so it names nothing of the interface.
use audhsos_abi as _;
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// Halts, which a user thread may not do.
fn main(_ipc_buffer: u64) -> ! {
    loop {
        // SAFETY: this is the point of the program. The instruction is
        // privileged, so the processor raises a fault instead of running
        // it, and the kernel stops this thread.
        unsafe {
            asm!("hlt", options(nomem, nostack));
        }
    }
}

#[panic_handler]
const fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Nothing of this program panics; the handler is what the language
    // asks for, and a thread that reached it has nothing left to do.
    loop {}
}
