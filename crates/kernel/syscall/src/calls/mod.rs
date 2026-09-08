// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One function per system call, and the match that reaches them.
//!
//! Invariants: a call that returns an error has changed nothing; the
//! arguments of a call are read from the request and never from the buffer
//! again, apart from the message area, which `debug_log` and the endpoint
//! calls read.
//!
//! The argument words of every call of the table:
//!
//! | Call | Arguments |
//! |------|-----------|
//! | `process_create` | process handle, handle capacity, frame quota, object quota, reserved (zero) |
//! | `process_install_handle` | target process handle, handle of the caller, rights |
//! | `process_set_fault_handler` | process handle, endpoint handle (zero clears it) |
//! | `process_kill` | process handle |
//! | `process_watch`, `process_unwatch` | process handle, notification handle, bit index |
//! | `thread_create` | process handle, entry, user stack, priority, maximum priority, memory object for the IPC buffer (zero for one out of the kernel reserve) |
//! | `thread_start`, `thread_suspend`, `thread_resume`, `thread_kill` | thread handle |
//! | `thread_set_priority` | thread handle, priority |
//! | `thread_info` | thread handle |
//! | `thread_exit`, `thread_yield` | none |
//! | `memory_split` | memory handle, offset in bytes |
//! | `memory_merge` | memory handle of the lower part, handle of the upper |
//! | `memory_map` | process handle, memory handle, virtual address, offset, length, permissions |
//! | `memory_unmap` | process handle, virtual address, length |
//! | `memory_protect` | process handle, virtual address, length, permissions |
//! | `memory_info` | memory handle |
//! | `memory_references` | memory handle |
//! | `handle_duplicate` | handle, rights |
//! | `handle_close` | handle |
//! | `endpoint_create`, `notification_create` | none |
//! | `endpoint_badge` | endpoint handle, badge (non-zero) |
//! | `ipc_call`, `ipc_send`, `ipc_recv`, `ipc_try_recv` | endpoint handle |
//! | `ipc_reply` | reply handle |
//! | `ipc_reply_recv` | reply handle, endpoint handle |
//! | `notification_signal` | notification handle, bits |
//! | `notification_wait`, `notification_poll` | notification handle |
//! | `interrupt_create` | system control handle, line |
//! | `interrupt_bind` | interrupt handle, notification handle, bit index |
//! | `interrupt_ack` | interrupt handle |
//! | `ioport_create` | system control handle, first port, count |
//! | `ioport_read` | port range handle, port, width |
//! | `ioport_write` | port range handle, port, width, value |
//! | `memory_create_device` | system control handle, first frame, frame count |
//! | `system_info` | system control handle |
//! | `debug_log` | none; the message area carries the bytes |

pub mod debug;
pub mod device;
pub mod handle;
pub mod ipc;
pub mod memory;
pub mod notify;
pub mod process;
pub mod thread;

use audhsos_abi::ipc_buffer::SIZE;
use audhsos_abi::{Error, Syscall};
use kernel_objects::object::{ProcessId, ThreadId};

use crate::dispatch::{Machine, Reply, Request};
use crate::environment::Environment;

/// Runs the call the request names.
///
/// # Errors
///
/// Whatever the call returns.
pub fn run<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ThreadId,
    process: ProcessId,
    request: &Request,
    buffer: &mut [u8; SIZE],
) -> Result<Reply, Error> {
    match request.call {
        Syscall::ProcessCreate => process::create(machine, process, request),
        Syscall::ProcessInstallHandle => process::install_handle(machine, process, request),
        Syscall::ProcessSetFaultHandler => process::set_fault_handler(machine, process, request),
        Syscall::ProcessKill => process::kill(machine, process, request),
        Syscall::ProcessWatch => process::watch(machine, process, request),
        Syscall::ProcessUnwatch => process::unwatch(machine, process, request),
        Syscall::MemoryReferences => memory::references(machine, process, request),
        Syscall::ThreadCreate => thread::create(machine, caller, process, request),
        Syscall::ThreadStart => thread::start(machine, process, request),
        Syscall::ThreadSuspend => thread::suspend(machine, process, request),
        Syscall::ThreadResume => thread::resume(machine, process, request),
        Syscall::ThreadKill => thread::kill(machine, process, request),
        Syscall::ThreadSetPriority => thread::set_priority(machine, process, request),
        Syscall::ThreadInfo => thread::info(machine, process, request),
        Syscall::ThreadExit => thread::exit(machine, caller),
        Syscall::ThreadYield => thread::yield_now(machine, caller),
        Syscall::MemorySplit => memory::split(machine, process, request),
        Syscall::MemoryMerge => memory::merge(machine, process, request),
        Syscall::MemoryMap => memory::map(machine, process, request),
        Syscall::MemoryUnmap => memory::unmap(machine, process, request),
        Syscall::MemoryProtect => memory::protect(machine, process, request),
        Syscall::MemoryInfo => memory::info(machine, process, request),
        Syscall::HandleDuplicate => handle::duplicate(machine, process, request),
        Syscall::HandleClose => handle::close(machine, process, request),
        Syscall::EndpointCreate => ipc::create(machine, process),
        Syscall::EndpointBadge => ipc::badge(machine, process, request),
        Syscall::IpcSend => ipc::send(machine, caller, process, request, buffer, false),
        Syscall::IpcCall => ipc::send(machine, caller, process, request, buffer, true),
        Syscall::IpcRecv => ipc::receive(machine, caller, process, request, buffer, true),
        Syscall::IpcTryRecv => ipc::receive(machine, caller, process, request, buffer, false),
        Syscall::IpcReply => ipc::reply(machine, process, request, buffer),
        Syscall::IpcReplyRecv => ipc::reply_recv(machine, caller, process, request, buffer),
        Syscall::NotificationCreate => notify::create(machine, process),
        Syscall::NotificationSignal => notify::signal(machine, process, request),
        Syscall::NotificationWait => notify::wait(machine, caller, process, request),
        Syscall::NotificationPoll => notify::poll(machine, process, request),
        Syscall::InterruptCreate => device::interrupt_create(machine, process, request),
        Syscall::InterruptBind => device::interrupt_bind(machine, process, request),
        Syscall::InterruptAck => device::interrupt_ack(machine, process, request),
        Syscall::IoPortCreate => device::ioport_create(machine, process, request),
        Syscall::IoPortRead => device::ioport_read(machine, process, request),
        Syscall::IoPortWrite => device::ioport_write(machine, process, request),
        Syscall::MemoryCreateDevice => device::memory_create_device(machine, process, request),
        Syscall::SystemInfo => device::system_info(machine),
        Syscall::DebugLog => debug::log(machine, buffer),
    }
}

/// Every call the kernel does not implement. The table is complete after
/// Phase 6, so there is none, and a test holds that.
pub const UNIMPLEMENTED: &[Syscall] = &[];

/// Charges one kernel object against the quota of `process`.
///
/// # Errors
///
/// [`Error::QuotaExceeded`] when the process may hold no further object.
pub(crate) fn charge_object<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
) -> Result<(), Error> {
    machine
        .objects
        .processes
        .get_mut(process)?
        .kernel_object_quota
        .charge(1)?;
    Ok(())
}

/// Gives one kernel object back to the quota of `process`.
pub(crate) fn refund_object<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
) {
    machine.objects.with_process(process, |holder| {
        holder.kernel_object_quota.refund(1);
    });
}
