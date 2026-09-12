// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The wall clock, as a user thread reaches it.
//!
//! The thread reads `clock_wall` twice around a wait of fifty milliseconds
//! and reports both readings, the source the machine named, and the
//! monotonic clock beside the first reading. What the test above it checks
//! is that the date is a plausible one, that it moved forward by the wait,
//! and that it is the boot moment plus the monotonic count.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// Where the kernel maps the page the program and the test share.
const SHARED: u64 = 0x90_0000;

/// How long the wait between the two readings is, in microseconds.
const WAIT: u64 = 50_000;

/// The words of the shared page.
const STATUS: usize = 0;
const BEFORE: usize = 1;
const SOURCE: usize = 2;
const AFTER: usize = 3;
const MONOTONIC: usize = 4;
const DONE: usize = 5;

/// The value the program writes into [`DONE`] when it has finished.
const FINISHED: u64 = 0x0A_11C1;

/// Reads the wall clock around a wait that nothing ends but its deadline.
fn main(ipc_buffer: u64) -> ! {
    // SAFETY: the address is the one the kernel started this thread with,
    // and no other reference to the buffer is alive. Every call below has
    // the same argument.
    let (status, values) = unsafe { sys::returning(ipc_buffer, Syscall::ClockWall, &[]) };
    write(STATUS, status);
    write(BEFORE, values[0]);
    write(SOURCE, values[1]);

    // SAFETY: as above.
    let (_, monotonic) = unsafe { sys::call(ipc_buffer, Syscall::ClockNow, &[]) };
    write(MONOTONIC, monotonic);

    // SAFETY: as above.
    let (_, notification) = unsafe { sys::call(ipc_buffer, Syscall::NotificationCreate, &[]) };
    let deadline = monotonic.saturating_add(WAIT);
    // SAFETY: as above.
    let _ = unsafe {
        sys::call(
            ipc_buffer,
            Syscall::NotificationWaitUntil,
            &[notification, deadline],
        )
    };

    // SAFETY: as above.
    let (_, after) = unsafe { sys::returning(ipc_buffer, Syscall::ClockWall, &[]) };
    write(AFTER, after[0]);
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
