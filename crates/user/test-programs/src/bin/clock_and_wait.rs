// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The clock and a wait that ends at its deadline.
//!
//! The thread reads the clock, waits fifty milliseconds on a notification
//! nothing ever signals, and reads it again. What it reports is the two
//! readings, the difference, and the bits the wait answered — which are
//! none, because nothing signalled.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// Where the kernel maps the page the program and the test share.
const SHARED: u64 = 0x90_0000;

/// How long the wait is, in microseconds.
const WAIT: u64 = 50_000;

/// The words of the shared page.
const BEFORE: usize = 0;
const AFTER: usize = 1;
const ELAPSED: usize = 2;
const BITS: usize = 3;
const STATUS: usize = 4;
const DONE: usize = 5;

/// The value the program writes into [`DONE`] when it has finished.
const FINISHED: u64 = 0x0C_10CC;

/// Reads the clock around a wait that nothing ends but its deadline.
fn main(ipc_buffer: u64) -> ! {
    // SAFETY: the address is the one the kernel started this thread with,
    // and no other reference to the buffer is alive. Every call below has
    // the same argument.
    let (_, notification) = unsafe { sys::call(ipc_buffer, Syscall::NotificationCreate, &[]) };

    // SAFETY: as above.
    let (_, before) = unsafe { sys::call(ipc_buffer, Syscall::ClockNow, &[]) };
    write(BEFORE, before);

    let deadline = before.saturating_add(WAIT);
    // SAFETY: as above.
    let (status, bits) = unsafe {
        sys::call(
            ipc_buffer,
            Syscall::NotificationWaitUntil,
            &[notification, deadline],
        )
    };
    write(STATUS, status);
    write(BITS, bits);

    // SAFETY: as above.
    let (_, after) = unsafe { sys::call(ipc_buffer, Syscall::ClockNow, &[]) };
    write(AFTER, after);
    write(ELAPSED, after.saturating_sub(before));
    write(DONE, FINISHED);
    end(ipc_buffer)
}

/// Writes `value` at word `index` of the shared page.
fn write(index: usize, value: u64) {
    let offset = u64::try_from(index).unwrap_or(0).saturating_mul(8);
    let address = SHARED.saturating_add(offset);
    let pointer = core::ptr::without_provenance_mut::<u64>(usize::try_from(address).unwrap_or(0));
    // SAFETY: the kernel mapped the page read and write for this process
    // alone, and the index stays inside it.
    unsafe {
        pointer.write_volatile(value);
    }
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
