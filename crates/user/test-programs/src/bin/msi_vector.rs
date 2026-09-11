// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A driver at ring three that takes a message interrupt.
//!
//! It creates the interrupt object, binds it to a bit of a notification,
//! reports the address and the data a device would write, and waits for the
//! bit. The test kernel is the device: it raises the vector, because no
//! device of the reference machine writes one yet.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// The payload word the handle of the system control capability is in.
pub const CONTROL_WORD: usize = 0;

/// Where the kernel maps the page the driver and the test share.
const SHARED: u64 = 0x90_0000;

/// The bit of the notification the vector is bound to.
const BIT: u64 = 5;

/// The words of the shared page.
const CREATE_STATUS: usize = 0;
const BIND_STATUS: usize = 1;
const ADDRESS: usize = 2;
const DATA: usize = 3;
const READY: usize = 4;
const WORD: usize = 5;
const DONE: usize = 6;

/// The value the driver writes into [`READY`] once it is about to wait.
const ARMED: u64 = 0x0A_2110;

/// The value the driver writes into [`DONE`] when it has finished.
const FINISHED: u64 = 0x0D_5170;

/// Takes a message interrupt and waits for the bit it was bound to.
fn main(ipc_buffer: u64) -> ! {
    let control = given(ipc_buffer, CONTROL_WORD);

    // SAFETY: the address is the one the kernel started this thread with,
    // and no other reference to the buffer is alive. Every call below has
    // the same argument.
    let (status, notification) = unsafe { sys::call(ipc_buffer, Syscall::NotificationCreate, &[]) };
    if status != 0 {
        write(CREATE_STATUS, status);
        end(ipc_buffer);
    }

    // SAFETY: as above.
    let (status, interrupt) =
        unsafe { sys::call(ipc_buffer, Syscall::InterruptCreateMsi, &[control]) };
    write(CREATE_STATUS, status);
    if status != 0 {
        end(ipc_buffer);
    }
    let (address, data) = message(ipc_buffer);
    write(ADDRESS, address);
    write(DATA, data);

    // SAFETY: as above.
    let (status, _) = unsafe {
        sys::call(
            ipc_buffer,
            Syscall::InterruptBind,
            &[interrupt, notification, BIT],
        )
    };
    write(BIND_STATUS, status);
    if status != 0 {
        end(ipc_buffer);
    }

    write(READY, ARMED);
    // SAFETY: as above.
    let (_, word) = unsafe { sys::call(ipc_buffer, Syscall::NotificationWait, &[notification]) };
    write(WORD, word);
    // The acknowledgement touches no hardware for a message interrupt; what
    // it clears is the flag the kernel keeps.
    // SAFETY: as above.
    unsafe {
        let _ = sys::call(ipc_buffer, Syscall::InterruptAck, &[interrupt]);
    }
    write(DONE, FINISHED);
    end(ipc_buffer)
}

/// The address and the data `interrupt_create_msi` left in the message
/// area.
fn message(ipc_buffer: u64) -> (u64, u64) {
    // SAFETY: as `main`; the borrow ends with this function.
    let buffer = unsafe { sys::buffer(ipc_buffer) };
    let reader = buffer.reader();
    (reader.word(0).unwrap_or(0), reader.word(1).unwrap_or(0))
}

/// The word the kernel left at `index` of the buffer.
fn given(ipc_buffer: u64, index: usize) -> u64 {
    // SAFETY: as `main`.
    unsafe { sys::buffer(ipc_buffer) }
        .reader()
        .word(index)
        .unwrap_or(0)
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
