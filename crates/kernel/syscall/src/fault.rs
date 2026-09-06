// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What happens to a thread the processor stopped.
//!
//! Invariants: a thread that faulted keeps everything it holds — its kernel
//! stack, its IPC buffer, its pool slot, its address space — and loses only
//! the processor, so that whoever created it can look at it and either
//! resume it or kill it; nothing else of the machine ends with it.
//!
//! A process that named a fault handler gets a message on it instead: the
//! kernel builds it in the faulting thread's own buffer, with the reserved
//! label of its kind, and performs a `call`. The thread is then blocked as
//! any caller is, and the reply resumes it at the instruction it faulted on.
//! A process with no handler, a handler endpoint that is gone, and a reply
//! pool with no slot left all end the same way, in [`ThreadState::Faulted`],
//! which is what a fault nobody takes ends in.

use audhsos_abi::ipc_buffer::{BufferMut, SIZE, fault_label};
use audhsos_abi::{Fault, ThreadState};
use kernel_ipc::endpoint::Intent;
use kernel_objects::object::{EndpointId, ThreadId};
use kernel_sched::Outcome;

use crate::calls::ipc;
use crate::dispatch::Machine;
use crate::environment::Environment;

/// Reports `fault` of `thread` to the fault handler of its process, or stops
/// the thread when nobody takes it.
///
/// `buffer` is the IPC buffer of the faulting thread, which the caller
/// reaches the same way the system call gate does: the message the handler
/// receives is built there, and from there it is copied.
pub fn deliver<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    thread: ThreadId,
    fault: Fault,
    buffer: &mut [u8; SIZE],
) -> Outcome {
    machine
        .objects
        .threads
        .with(thread, |held| held.fault = Some(fault));
    let Some((process, handler)) = handler_of(machine, thread) else {
        return stop(machine, thread);
    };
    write_message(buffer, fault);
    let outcome = ipc::deliver(machine, thread, process, handler, Intent::call(0), buffer);
    match outcome {
        Ok(reply) => reply.outcome,
        // Nobody could take the message: no reply slot, no handle slot, or
        // an endpoint that went while the thread was faulting. The thread
        // stops where a fault nobody takes stops it.
        Err(_) => stop(machine, thread),
    }
}

/// The process of `thread` and the endpoint its faults are reported on.
fn handler_of<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
    thread: ThreadId,
) -> Option<(kernel_objects::object::ProcessId, EndpointId)> {
    let process = machine.objects.threads.get(thread).ok()?.process;
    let handler = machine.objects.processes.get(process).ok()?.fault_handler?;
    // An endpoint that is gone takes the handler with it.
    if machine.objects.endpoints.get(handler).is_err() {
        return None;
    }
    Some((process, handler))
}

/// Builds the fault message: the reserved label of its kind, three words,
/// and no handle.
fn write_message(buffer: &mut [u8; SIZE], fault: Fault) {
    let mut writer = BufferMut::new(buffer);
    writer.set_label(fault_label(fault.kind));
    let _ = writer.set_counts(3, 0);
    writer.set_word(0, fault.address);
    writer.set_word(1, fault.instruction_pointer);
    writer.set_word(2, fault.error_code);
}

/// Stops `thread`, which faulted and whose process has no handler for the
/// fault, and says whether the caller has to switch before it returns to
/// user mode.
///
/// A thread that was running always asks for a switch: it cannot be
/// returned to, and the processor stands on its kernel stack. Only a
/// running thread can fault, and the scheduler refuses every other one
/// without moving it, so anything else is left exactly where it was and
/// asks for nothing.
pub fn stop<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    thread: ThreadId,
) -> Outcome {
    machine
        .scheduler
        .fault(&mut machine.objects.threads, thread)
        .unwrap_or(Outcome::NOTHING)
}

/// `true` when `thread` has been stopped by a fault.
#[must_use]
pub fn is_faulted<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
    thread: ThreadId,
) -> bool {
    machine
        .objects
        .threads
        .get(thread)
        .is_ok_and(|entry| entry.state == ThreadState::Faulted)
}
