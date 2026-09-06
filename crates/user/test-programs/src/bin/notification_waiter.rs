// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A driver at ring three: it takes an interrupt line, binds it to a bit of
//! a notification, programs the interval timer through a range of I/O ports,
//! and waits for the bit.
//!
//! The interval timer and not the local APIC timer: that one is the kernel's
//! own, it carries a vector of its own, and the vector plan gives it no
//! global system interrupt, so no interrupt object can name it. ISA line zero
//! reaches the I/O APIC through the interrupt source override the tables
//! carry, and programming it is three port writes — which makes this one
//! program cover the interrupt path, the port path, and the notification path
//! from ring three.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// The payload word the handle of the system control capability is in.
pub const CONTROL_WORD: usize = 0;

/// Where the kernel maps the page the driver and the test share.
const SHARED: u64 = 0x90_0000;

/// The ISA line of the interval timer.
const LINE: u64 = 0;

/// The bit of the notification the line is bound to.
const BIT: u64 = 3;

/// The first port of the interval timer, and how many it has.
const FIRST_PORT: u64 = 0x40;
const PORT_COUNT: u64 = 4;

/// The counter of channel zero, and the command port.
const COUNTER: u64 = 0x40;
const COMMAND: u64 = 0x43;

/// Channel zero, both bytes of the counter, rate generator: what the timer
/// needs to raise its line again and again.
const MODE: u64 = 0x34;

/// The divisor for about a hundred interrupts a second, out of the 1.193182
/// megahertz the interval timer counts at.
const DIVISOR: u64 = 11_932;

/// The words of the shared page the driver writes.
const CREATE_STATUS: usize = 0;
const BIND_STATUS: usize = 1;
const PORTS_STATUS: usize = 2;
const FIRST_WORD: usize = 3;
const SECOND_WORD: usize = 4;
const DONE: usize = 5;

/// The value the driver writes into [`DONE`] when it has finished.
const FINISHED: u64 = 0x0D_21FE;

/// Takes the line, programs the timer, and waits for two interrupts.
fn main(ipc_buffer: u64) -> ! {
    let control = given(ipc_buffer, CONTROL_WORD);

    // SAFETY: the address is the one the kernel started this thread with,
    // and no other reference to the buffer is alive. Every call below has
    // the same argument.
    let (status, notification) = unsafe { sys::call(ipc_buffer, Syscall::NotificationCreate, &[]) };
    if status != 0 {
        write(SHARED, CREATE_STATUS, status);
        end(ipc_buffer);
    }
    // SAFETY: as above.
    let (status, interrupt) =
        unsafe { sys::call(ipc_buffer, Syscall::InterruptCreate, &[control, LINE]) };
    write(SHARED, CREATE_STATUS, status);
    if status != 0 {
        end(ipc_buffer);
    }
    // SAFETY: as above.
    let (status, _) = unsafe {
        sys::call(
            ipc_buffer,
            Syscall::InterruptBind,
            &[interrupt, notification, BIT],
        )
    };
    write(SHARED, BIND_STATUS, status);

    // SAFETY: as above.
    let (status, ports) = unsafe {
        sys::call(
            ipc_buffer,
            Syscall::IoPortCreate,
            &[control, FIRST_PORT, PORT_COUNT],
        )
    };
    write(SHARED, PORTS_STATUS, status);
    if status != 0 {
        end(ipc_buffer);
    }

    // The line is masked until the driver says it is ready for it.
    // SAFETY: as above.
    unsafe {
        let _ = sys::call(ipc_buffer, Syscall::InterruptAck, &[interrupt]);
    }
    program(ipc_buffer, ports);

    // The first interrupt, and then the second, which only arrives because
    // the acknowledgement between them unmasked the line again.
    // SAFETY: as above.
    let (_, word) = unsafe { sys::call(ipc_buffer, Syscall::NotificationWait, &[notification]) };
    write(SHARED, FIRST_WORD, word);
    // SAFETY: as above.
    unsafe {
        let _ = sys::call(ipc_buffer, Syscall::InterruptAck, &[interrupt]);
    }
    // SAFETY: as above.
    let (_, word) = unsafe { sys::call(ipc_buffer, Syscall::NotificationWait, &[notification]) };
    write(SHARED, SECOND_WORD, word);
    write(SHARED, DONE, FINISHED);
    end(ipc_buffer)
}

/// Programs channel zero of the interval timer: the mode byte, then the two
/// bytes of the divisor.
fn program(ipc_buffer: u64, ports: u64) {
    for (port, value) in [
        (COMMAND, MODE),
        (COUNTER, DIVISOR & 0xFF),
        (COUNTER, (DIVISOR >> 8) & 0xFF),
    ] {
        // SAFETY: as `main`.
        unsafe {
            let _ = sys::call(ipc_buffer, Syscall::IoPortWrite, &[ports, port, 1, value]);
        }
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

/// Writes `value` at word `index` of the page at `base`.
fn write(base: u64, index: usize, value: u64) {
    let offset = u64::try_from(index).unwrap_or(0).saturating_mul(8);
    let address = base.saturating_add(offset);
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
