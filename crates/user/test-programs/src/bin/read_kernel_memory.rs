// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A thread that reads an address of the kernel half. The page tables of
//! its address space carry that half, but not for user mode, so the
//! processor raises a page fault and the kernel stops the thread.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::layout::KERNEL_BASE;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// Reads the first byte of the kernel image.
fn main(_ipc_buffer: u64) -> ! {
    loop {
        let address =
            core::ptr::without_provenance::<u8>(usize::try_from(KERNEL_BASE).unwrap_or(0));
        // SAFETY: this is the point of the program. The page is mapped,
        // but not for user mode, so the read faults and the kernel stops
        // this thread; nothing is ever read.
        let byte = unsafe { core::ptr::read_volatile(address) };
        core::hint::black_box(byte);
    }
}

#[panic_handler]
const fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Nothing of this program panics; the handler is what the language
    // asks for, and a thread that reached it has nothing left to do.
    loop {}
}
