// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What happens to a thread the processor stopped.
//!
//! Invariants: a thread that faulted keeps everything it holds — its
//! kernel stack, its IPC buffer, its pool slot, its address space — and
//! loses only the processor, so that whoever created it can look at it and
//! either resume it or kill it; nothing else of the machine ends with it.
//!
//! Phase 5 has no fault handler to deliver to: `process_set_fault_handler`
//! names an endpoint, and endpoints are objects of Phase 6. Until then
//! every fault ends the same way, in [`ThreadState::Faulted`], which is
//! what [6.6.21](../../../docs/06-testing-strategy.md) calls the isolation
//! item that does not name a handler. Phase 6 adds the delivery above
//! this; the state stays what a fault nobody took ends in.

use audhsos_abi::ThreadState;
use kernel_objects::object::ThreadId;
use kernel_sched::Outcome;

use crate::dispatch::Machine;
use crate::environment::Environment;

/// Stops `thread`, which faulted and whose process has no handler for the
/// fault, and says whether the caller has to switch before it returns to
/// user mode.
///
/// A thread that was running always asks for a switch: it cannot be
/// returned to, and the processor stands on its kernel stack. Any other
/// thread is left exactly where it is and asks for nothing.
///
/// The state is checked here and not left to the scheduler, which takes a
/// thread out of its run queue before it asks the transition table whether
/// the event is allowed. Only a running thread may fault, so a ready one
/// would come back from a refused fault ready and in no queue, which is
/// the one thing the scheduler says can never be true of a thread. Nothing
/// of this kernel reaches that — the trap handler faults the thread the
/// processor was on — but the check is one line and the invariant is not.
pub fn stop<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    thread: ThreadId,
) -> Outcome {
    if !is_running(machine, thread) {
        return Outcome::NOTHING;
    }
    machine
        .scheduler
        .fault(&mut machine.objects.threads, thread)
        .unwrap_or(Outcome::NOTHING)
}

/// `true` when `thread` is the one on the processor.
fn is_running<
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
        .is_ok_and(|entry| entry.state == ThreadState::Running)
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
