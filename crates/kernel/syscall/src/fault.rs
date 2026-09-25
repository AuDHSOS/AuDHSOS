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
    let Some((process, handler, badge)) = handler_of(machine, thread) else {
        return unreported(machine, thread, fault);
    };
    write_message(buffer, fault);
    let outcome = ipc::deliver(
        machine,
        thread,
        process,
        handler,
        Intent::kernel_call(badge),
        buffer,
    );
    match outcome {
        Ok(reply) => reply.outcome,
        // Nobody could take the message: no reply slot, no handle slot, or
        // an endpoint that went while the thread was faulting. The thread
        // stops where a fault nobody takes stops it.
        Err(_) => unreported(machine, thread, fault),
    }
}

/// Reports `fault` as [`deliver`] does, or stops `thread` with no message
/// for `None`, which is a vector with no [`audhsos_abi::FaultKind`].
pub fn take<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    thread: ThreadId,
    fault: Option<Fault>,
    buffer: &mut [u8; SIZE],
) -> Outcome {
    match fault {
        Some(fault) => deliver(machine, thread, fault, buffer),
        None => stop(machine, thread),
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
) -> Option<(kernel_objects::object::ProcessId, EndpointId, u64)> {
    let process = machine.objects.threads.get(thread).ok()?.process;
    let (handler, badge) = machine.objects.processes.get(process).ok()?.fault_handler?;
    // An endpoint that is gone takes the handler with it.
    if machine.objects.endpoints.get(handler).is_err() {
        return None;
    }
    Some((process, handler, badge))
}

/// Stops `thread`, and says on the console that its fault reached nobody.
///
/// The root task is the fault handler of every process it starts and has
/// none of its own, so a fault of the root task is exactly this case. It
/// takes the machine with it — every server it started waits on a message
/// it will never answer — and without this line the machine goes quiet and
/// says nothing about why.
fn unreported<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    thread: ThreadId,
    fault: Fault,
) -> Outcome {
    let mut line = [0u8; UNREPORTED_LEN];
    let len = say_unreported(&mut line, fault);
    machine.environment.log(line.get(..len).unwrap_or(&[]));
    stop(machine, thread)
}

/// Bytes the line of an unreported fault takes: thirty-seven of words,
/// seventeen of the longest kind name, two addresses of eighteen bytes,
/// the ten that join them, and the newline.
const UNREPORTED_LEN: usize = 104;

/// Writes that line into `into` and answers how many bytes it is.
fn say_unreported(into: &mut [u8; UNREPORTED_LEN], fault: Fault) -> usize {
    let mut len = 0usize;
    let mut put = |bytes: &[u8], len: &mut usize| {
        for byte in bytes {
            if let Some(slot) = into.get_mut(*len) {
                *slot = *byte;
                *len = len.saturating_add(1);
            }
        }
    };
    put(b"[kernel] a fault reached no handler: ", &mut len);
    put(fault.kind.name().as_bytes(), &mut len);
    put(b" at ", &mut len);
    let mut address = [0u8; 18];
    let wrote = hexadecimal(&mut address, fault.address);
    put(address.get(..wrote).unwrap_or(&[]), &mut len);
    put(b" from ", &mut len);
    let wrote = hexadecimal(&mut address, fault.instruction_pointer);
    put(address.get(..wrote).unwrap_or(&[]), &mut len);
    put(b"\n", &mut len);
    len
}

/// Writes `value` as `0x…` into `into` and answers how many bytes it is.
fn hexadecimal(into: &mut [u8; 18], value: u64) -> usize {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    if let Some(head) = into.get_mut(..2) {
        head.copy_from_slice(b"0x");
    }
    let mut len = 2usize;
    let mut seen = false;
    for shift in (0u32..16).rev() {
        let nibble = usize::try_from(value.wrapping_shr(shift.wrapping_mul(4)) & 0xF).unwrap_or(0);
        if nibble != 0 || seen || shift == 0 {
            seen = true;
            if let (Some(slot), Some(digit)) = (into.get_mut(len), DIGITS.get(nibble)) {
                *slot = *digit;
                len = len.saturating_add(1);
            }
        }
    }
    len
}

/// The payload words of a fault message: the address, the instruction
/// pointer, and the error code.
pub(crate) const WORDS: usize = 3;

/// Builds the fault message: the reserved label of its kind, [`WORDS`]
/// words, and no handle.
pub(crate) fn write_message(buffer: &mut [u8; SIZE], fault: Fault) {
    let mut writer = BufferMut::new(buffer);
    writer.set_label(fault_label(fault.kind));
    let _ = writer.set_counts(WORDS, 0);
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
