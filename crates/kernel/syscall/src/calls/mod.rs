// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One function per system call, and the match that reaches them.
//!
//! Invariants: a call that returns an error has changed nothing; the
//! arguments of a call are read from the request and never from the buffer
//! again, apart from the message area, which only `debug_log` reads in this
//! phase.
//!
//! The argument words of every call this phase implements:
//!
//! | Call | Arguments |
//! |------|-----------|
//! | `process_create` | process handle, handle capacity, frame quota, object quota, reserved (zero) |
//! | `process_install_handle` | target process handle, handle of the caller, rights |
//! | `process_kill` | process handle |
//! | `thread_create` | process handle, entry, user stack, priority, maximum priority, reserved (zero) |
//! | `thread_start`, `thread_suspend`, `thread_resume`, `thread_kill` | thread handle |
//! | `thread_set_priority` | thread handle, priority |
//! | `thread_info` | thread handle |
//! | `thread_exit`, `thread_yield` | none |
//! | `memory_split` | memory handle, offset in bytes |
//! | `memory_map` | process handle, memory handle, virtual address, offset, length, permissions |
//! | `memory_unmap` | process handle, virtual address, length |
//! | `memory_protect` | process handle, virtual address, length, permissions |
//! | `memory_info` | memory handle |
//! | `handle_duplicate` | handle, rights |
//! | `handle_close` | handle |
//! | `debug_log` | none; the message area carries the bytes |

pub mod debug;
pub mod handle;
pub mod memory;
pub mod process;
pub mod thread;

use audhsos_abi::ipc_buffer::SIZE;
use audhsos_abi::{Error, Syscall};
use kernel_objects::object::{ProcessId, ThreadId};

use crate::dispatch::{Machine, Reply, Request};
use crate::environment::Environment;

/// Runs the call the request names. Every call Phase 6 owns returns
/// [`Error::Unsupported`] until then, and a test holds that this is the
/// only thing they do.
///
/// # Errors
///
/// Whatever the call returns; [`Error::Unsupported`] for a call this phase
/// does not implement.
pub fn run<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ThreadId,
    process: ProcessId,
    request: &Request,
    buffer: &[u8; SIZE],
) -> Result<Reply, Error> {
    match request.call {
        Syscall::ProcessCreate => process::create(machine, process, request),
        Syscall::ProcessInstallHandle => process::install_handle(machine, process, request),
        Syscall::ProcessKill => process::kill(machine, process, request),
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
        Syscall::MemoryMap => memory::map(machine, process, request),
        Syscall::MemoryUnmap => memory::unmap(machine, process, request),
        Syscall::MemoryProtect => memory::protect(machine, process, request),
        Syscall::MemoryInfo => memory::info(machine, process, request),
        Syscall::HandleDuplicate => handle::duplicate(machine, process, request),
        Syscall::HandleClose => handle::close(machine, process, request),
        Syscall::DebugLog => debug::log(machine, buffer),
        // Phase 6 owns the rest: endpoints, replies, notifications,
        // interrupts, port ranges, device memory, the system information,
        // and the fault handler, whose endpoint is an object of that phase.
        Syscall::ProcessSetFaultHandler
        | Syscall::EndpointCreate
        | Syscall::EndpointBadge
        | Syscall::IpcCall
        | Syscall::IpcSend
        | Syscall::IpcRecv
        | Syscall::IpcTryRecv
        | Syscall::IpcReply
        | Syscall::IpcReplyRecv
        | Syscall::NotificationCreate
        | Syscall::NotificationSignal
        | Syscall::NotificationWait
        | Syscall::NotificationPoll
        | Syscall::InterruptCreate
        | Syscall::InterruptBind
        | Syscall::InterruptAck
        | Syscall::IoPortCreate
        | Syscall::IoPortRead
        | Syscall::IoPortWrite
        | Syscall::MemoryCreateDevice
        | Syscall::SystemInfo => Err(Error::Unsupported),
    }
}

/// Every call this phase does not implement.
pub const UNIMPLEMENTED: &[Syscall] = &[
    Syscall::ProcessSetFaultHandler,
    Syscall::EndpointCreate,
    Syscall::EndpointBadge,
    Syscall::IpcCall,
    Syscall::IpcSend,
    Syscall::IpcRecv,
    Syscall::IpcTryRecv,
    Syscall::IpcReply,
    Syscall::IpcReplyRecv,
    Syscall::NotificationCreate,
    Syscall::NotificationSignal,
    Syscall::NotificationWait,
    Syscall::NotificationPoll,
    Syscall::InterruptCreate,
    Syscall::InterruptBind,
    Syscall::InterruptAck,
    Syscall::IoPortCreate,
    Syscall::IoPortRead,
    Syscall::IoPortWrite,
    Syscall::MemoryCreateDevice,
    Syscall::SystemInfo,
];
