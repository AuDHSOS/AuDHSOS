// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Telling whoever asked that a process has ended.
//!
//! A server that holds something of a program — the display server holds a
//! surface, and the memory of it — has no way of learning that the program
//! is gone: the kernel tears the program down, and what the server holds is
//! its own. So the kernel says so, through the one thing it can say
//! something with when there is no thread left to say it: a notification.
//! That is what interrupts use, and for the same reason.
//!
//! Invariants: a process ends once, so its watchers are signalled once; a
//! watch that names a notification that is gone signals nothing and is no
//! error, because a notification is destroyed by whoever held it and the
//! process it watched cannot answer for that.

use audhsos_abi::Error;
use audhsos_abi::layout::WATCHERS_PER_PROCESS;
use kernel_ipc::notify;
use kernel_objects::object::{Process, ProcessId, Watch};

use crate::calls::ipc::write_result;
use crate::dispatch::Machine;
use crate::environment::Environment;

/// Signals the watchers of `process` if the thread that just left was its
/// last one, and answers whether a thread that woke should take the
/// processor.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the buffer of a thread that has to be woken
/// is not reachable, which no buffer of a live thread is.
pub fn thread_left<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
) -> Result<bool, Error> {
    let last = machine
        .objects
        .processes
        .get(process)
        .is_ok_and(|holder| holder.threads().next().is_none());
    if !last {
        return Ok(false);
    }
    ended(machine, process)
}

/// Signals the watchers of `process` and answers whether a thread that woke
/// should take the processor. A process whose end has already been told
/// tells nobody a second time.
///
/// # Errors
///
/// As [`thread_left`].
pub fn ended<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
) -> Result<bool, Error> {
    // A process that is gone was told of when it went, and one that has
    // been told of is not told again: both answer the same way here, so
    // they are read out of the pool as one.
    let (told, watchers) = machine
        .objects
        .processes
        .get(process)
        .map_or((true, [None; WATCHERS_PER_PROCESS]), |holder| {
            (holder.has_ended(), watchers_of(holder))
        });
    if told {
        return Ok(false);
    }
    machine.objects.with_process(process, Process::mark_ended);
    let mut switch = false;
    for watch in watchers.into_iter().flatten() {
        switch |= signal_one(machine, watch)?;
    }
    Ok(switch)
}

/// The watchers of `holder`, as an array this function can hold across the
/// changes that signalling them makes to the pool.
fn watchers_of(holder: &Process) -> [Option<Watch>; WATCHERS_PER_PROCESS] {
    let mut watchers = [None; WATCHERS_PER_PROCESS];
    for (slot, watch) in watchers.iter_mut().zip(holder.watchers()) {
        *slot = Some(watch);
    }
    watchers
}

/// Signals the bit of one watch and answers whether the thread it woke
/// should take the processor.
///
/// # Errors
///
/// As [`thread_left`].
pub fn signal_one<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    watch: Watch,
) -> Result<bool, Error> {
    let bits = 1_u64.wrapping_shl(u32::from(watch.bit));
    let Ok(outcome) = notify::signal(machine.objects, machine.scheduler, watch.notification, bits)
    else {
        return Ok(false);
    };
    if let Some(wakeup) = outcome.wakeup {
        write_result(machine, wakeup)?;
    }
    Ok(outcome.reschedule)
}
