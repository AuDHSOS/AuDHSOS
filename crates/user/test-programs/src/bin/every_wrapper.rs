// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every wrapper of the gate, once, in the order of the system call table.
//!
//! `Gate` carries one method per call, written out by hand (D-92), and the
//! constant assertion beside them holds a *list* to the table: it cannot
//! see whether a method exists for each entry, nor whether a method passes
//! the entry it belongs to. This program is what sees it. The number a
//! wrapper writes into the buffer is what the kernel dispatches on, so a
//! run of the wrappers in table order arrives in the kernel as the table,
//! in order — and a wrapper that passed another call of the same shape,
//! which nothing else in the system would notice, arrives as that call.
//!
//! Every call is made so that it is refused: the handles are one handle
//! that names nothing, and the calls which would otherwise wait for a
//! partner or end this thread are refused before they can. What is being
//! checked is the number the kernel saw, and a call the kernel refused was
//! a call the kernel saw. The three that take no handle succeed —
//! `endpoint_create`, `notification_create`, `thread_yield` — and so does
//! `debug_log`, which is given an empty message so that it writes nothing.
//!
//! `thread_exit` is last and not twelfth: it does not come back, and the
//! test kernel expects the table with that one call moved to the end.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::{Handle, Rights};
use user_rt::{
    EndpointHandle, InterruptHandle, IoPortHandle, MemoryHandle, NotificationHandle, ProcessHandle,
    ReplyHandle, SystemControlHandle, ThreadHandle, Typed as _,
};
use user_sys_x86_64::{self as sys, Gate};

sys::entry!(main);

/// Runs every wrapper once and ends.
///
/// # Safety
///
/// The kernel started this thread with the address of its own IPC buffer,
/// and this is the only gate over it.
fn main(ipc_buffer: u64) -> ! {
    // SAFETY: as above.
    let mut gate = unsafe { Gate::adopt(ipc_buffer) };
    // A handle whose index lies beyond the arena, so no generation could
    // make it name anything. Every call that takes one gets this, and is
    // refused for it.
    let Some(nothing) = Handle::new(0x00FF_FFFF, 1) else {
        gate.thread_exit()
    };
    let process = ProcessHandle::from_handle(nothing);
    let thread = ThreadHandle::from_handle(nothing);
    let memory = MemoryHandle::from_handle(nothing);
    let endpoint = EndpointHandle::from_handle(nothing);
    let reply = ReplyHandle::from_handle(nothing);
    let notification = NotificationHandle::from_handle(nothing);
    let interrupt = InterruptHandle::from_handle(nothing);
    let ports = IoPortHandle::from_handle(nothing);
    let system = SystemControlHandle::from_handle(nothing);

    // In the order of `Syscall::ALL`, one line per entry.
    let _ = gate.process_create(process, 16, 16, 16);
    let _ = gate.process_install_handle(process, nothing, Rights::EMPTY);
    let _ = gate.process_set_fault_handler(process, Some(endpoint));
    let _ = gate.process_kill(process);
    let _ = gate.thread_create(process, 0x40_0000, 0x50_0000, 8, 8, None);
    let _ = gate.thread_start(thread);
    let _ = gate.thread_suspend(thread);
    let _ = gate.thread_resume(thread);
    let _ = gate.thread_kill(thread);
    let _ = gate.thread_set_priority(thread, 8);
    let _ = gate.thread_info(thread);
    // `thread_exit` is the twelfth call of the table and the last one here.
    let _ = gate.thread_yield();
    let _ = gate.memory_split(memory, 0x1000);
    let _ = gate.memory_map(process, memory, 0x50_0000, 0, 0x1000, 0b11);
    let _ = gate.memory_unmap(process, 0x50_0000, 0x1000);
    let _ = gate.memory_protect(process, 0x50_0000, 0x1000, 0b1);
    let _ = gate.memory_info(memory);
    let _ = gate.handle_duplicate(nothing, Rights::EMPTY);
    let _ = gate.handle_close(nothing);
    let _ = gate.endpoint_create();
    let _ = gate.endpoint_badge(endpoint, 7);
    let _ = gate.ipc_call(endpoint);
    let _ = gate.ipc_send(endpoint);
    let _ = gate.ipc_recv(endpoint);
    let _ = gate.ipc_try_recv(endpoint);
    let _ = gate.ipc_reply(reply);
    let _ = gate.ipc_reply_recv(reply, endpoint);
    let _ = gate.notification_create();
    let _ = gate.notification_signal(notification, 1);
    let _ = gate.notification_wait(notification);
    let _ = gate.notification_poll(notification);
    let _ = gate.interrupt_create(system, 0);
    let _ = gate.interrupt_bind(interrupt, notification, 0);
    let _ = gate.interrupt_ack(interrupt);
    let _ = gate.ioport_create(system, 0x40, 4);
    let _ = gate.ioport_read(ports, 0x40, 1);
    let _ = gate.ioport_write(ports, 0x40, 1, 0);
    let _ = gate.memory_create_device(system, 0, 1);
    let _ = gate.system_info(system);
    // An empty message, so that the kernel writes nothing to the console.
    {
        let mut buffer = gate.writer();
        buffer.set_label(0);
        let _emptied = buffer.set_counts(0, 0);
    }
    let _ = gate.debug_log();
    let _ = gate.memory_merge(memory, memory);
    let _ = gate.process_watch(process, notification, 0);
    let _ = gate.process_unwatch(process, notification, 0);
    let _ = gate.memory_references(memory);

    gate.thread_exit()
}

#[panic_handler]
const fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Nothing of this program panics; the handler is what the language
    // asks for, and a thread that reached it has nothing left to do.
    loop {}
}
