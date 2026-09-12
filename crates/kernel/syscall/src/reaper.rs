// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What is cleared away after a thread has ended.
//!
//! Invariant: the kernel stack of a thread is given back only when nothing
//! stands on it. A thread that ends itself — `thread_exit`, or a
//! `process_kill` of its own process — is still running on its kernel
//! stack while the kernel writes its answer, so the stack, the IPC buffer,
//! and the pool slot stay until the kernel has switched away from it. The
//! state `Exited` is the whole record of what is left to do; there is no
//! second list to keep in step with it.
//!
//! Which thread that is, the caller says: the scheduler has already
//! forgotten it, because a thread that ends leaves the processor in the
//! same breath. Reading `current` instead was enough to make the kernel
//! unmap the stack it was standing on, which the machine answered with a
//! double fault.
//!
//! What the scheduler does keep is how many threads have ended, because
//! this runs after every system call and after every switch and the search
//! is a walk of the thread pool. The count is not a second record of what
//! is left to do — `Exited` is still that — but the answer to whether
//! there is anything to look for, and it has two writers: the one place a
//! thread enters `Exited`, and `clear` below (D-130).

use audhsos_abi::ThreadState;
use kernel_objects::object::ThreadId;

use crate::dispatch::Machine;
use crate::environment::Environment;

/// Gives back what every thread that has ended held, except the one the
/// processor is still on, and returns how many it cleared away.
///
/// The kernel calls this after a switch, and after a system call that
/// asked for none.
pub fn reap<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    running: Option<ThreadId>,
) -> u32 {
    if machine.scheduler.ended() == 0 {
        return 0;
    }
    let mut cleared: u32 = 0;
    // One at a time and looked up again each round: the kernel keeps no
    // second list of what has ended, so there is none to hold across a
    // change to the pool.
    while let Some(id) = next_ended(machine, running) {
        cleared = cleared.saturating_add(clear(machine, id));
    }
    cleared
}

/// The next thread that has ended and is not the one on the processor.
fn next_ended<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
    running: Option<ThreadId>,
) -> Option<ThreadId> {
    // The thread on the processor is the one the kernel is standing on,
    // whether the scheduler still calls it current or not.
    let spared = running.or_else(|| machine.scheduler.current());
    machine
        .objects
        .threads
        .iter()
        .find(|(id, thread)| thread.state == ThreadState::Exited && Some(*id) != spared)
        .map(|(id, _)| id)
}

/// Gives back what the thread `id` held.
fn clear<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    id: ThreadId,
) -> u32 {
    let Ok(thread) = machine.objects.threads.get(id) else {
        return 0;
    };
    let stack = thread.kernel_stack;
    let buffer = thread.ipc_buffer;
    let buffer_object = thread.buffer_object;
    let address = thread.ipc_address;
    let process = thread.process;
    // The page the buffer was mapped at goes with it, so that the slot of
    // the next thread starts with nothing of the one before it.
    if let Ok(root) = machine.objects.processes.get(process).map(|p| p.root)
        && let Ok(page) = kernel_types::Page::from_start(address)
    {
        let _ = machine.environment.unmap(root, page);
    }
    machine.environment.release_kernel_stack(stack);
    // The frame goes back where it came from: to the reserve, or to the
    // memory object the creator supplied, whose mapping was a reference to
    // it (D-91).
    match buffer_object {
        Some(object) => {
            let _gone = machine.objects.memory.release(object);
        }
        None => machine.environment.release_frame(buffer),
    }
    // The slot goes whatever handle still names it: a thread that has
    // been cleared away holds nothing, and there is nothing left to name.
    machine.objects.threads.force_release(id);
    machine.scheduler.cleared();
    1
}

/// `true` if a thread of the machine has ended and is waiting to be
/// cleared away.
#[must_use]
pub fn has_work<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
    running: Option<ThreadId>,
) -> bool {
    machine.scheduler.ended() != 0 && next_ended(machine, running).is_some()
}
