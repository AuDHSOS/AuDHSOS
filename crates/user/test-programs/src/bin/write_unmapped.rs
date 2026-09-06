// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A thread that writes to an address of its own half that nothing is mapped
//! at, and then carries on.
//!
//! This is the fault a handler can do something about: the address is the
//! program's own, and a page mapped there is all it needs. What the test
//! reads is the marker after the write — a thread that reached it wrote
//! through a mapping that did not exist when it started.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// Where the kernel maps the page the program and the test share.
const SHARED: u64 = 0x90_0000;

/// The address nothing is mapped at when the thread starts.
const UNMAPPED: u64 = 0x0200_0000;

/// The word the program writes there, and the one it writes into the shared
/// page once it has.
const WROTE: u64 = 0x0A11_0CA7;
const FINISHED: u64 = 0x0DEA_1000;

/// Writes where nothing is mapped, then says it got through, and ends.
fn main(ipc_buffer: u64) -> ! {
    write(UNMAPPED, 0, WROTE);
    write(SHARED, 0, read(UNMAPPED, 0));
    write(SHARED, 1, FINISHED);
    loop {
        // SAFETY: the address is the one the kernel started this thread
        // with, and nothing else holds a reference to the buffer.
        unsafe {
            let _ = sys::call(ipc_buffer, Syscall::ThreadExit, &[]);
        }
    }
}

/// Writes `value` at word `index` of the page at `base`.
fn write(base: u64, index: usize, value: u64) {
    let offset = u64::try_from(index).unwrap_or(0).saturating_mul(8);
    let address = base.saturating_add(offset);
    let pointer = core::ptr::without_provenance_mut::<u64>(usize::try_from(address).unwrap_or(0));
    // SAFETY: the shared page the kernel mapped for this process is read and
    // write, and the address the thread faults at is one a handler maps
    // before the write is retried.
    unsafe {
        pointer.write_volatile(value);
    }
}

/// Reads word `index` of the page at `base`.
fn read(base: u64, index: usize) -> u64 {
    let offset = u64::try_from(index).unwrap_or(0).saturating_mul(8);
    let address = base.saturating_add(offset);
    let pointer = core::ptr::without_provenance::<u64>(usize::try_from(address).unwrap_or(0));
    // SAFETY: as `write`.
    unsafe { pointer.read_volatile() }
}

#[panic_handler]
const fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Nothing of this program panics; the handler is what the language
    // asks for, and a thread that reached it has nothing left to do.
    loop {}
}
