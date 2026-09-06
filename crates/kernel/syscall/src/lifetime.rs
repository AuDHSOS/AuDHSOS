// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What happens when a reference to an object goes.
//!
//! Invariants: a handle is a reference, so the close of one drops it and the
//! close of the last one destroys what it named; a thread blocked on an
//! object holds no reference to it (D-75), so the last handle to an endpoint
//! can close under waiters — and then every one of them is woken with a
//! status that says the object is gone.

use audhsos_abi::Error;
use kernel_objects::object::AnyObjectId;

use crate::calls::ipc::write_result;
use crate::dispatch::Machine;
use crate::environment::Environment;

/// Drops one reference to `object` and wakes whoever waited on it if that
/// was the last one. Returns whether one of them should take the processor.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the buffer of a thread that has to be woken
/// is not reachable, which no buffer of a live thread is.
pub fn release<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    object: AnyObjectId,
) -> Result<bool, Error> {
    let Some(gone) = machine.objects.destroy(object) else {
        return Ok(false);
    };
    let mut waiters = kernel_ipc::destroyed(&gone);
    while let Some(wakeup) = waiters.wake_next(&mut machine.objects.threads, machine.scheduler) {
        write_result(machine, wakeup)?;
    }
    Ok(waiters.wants_switch())
}

/// Drops the reference every handle of `process` held, one at a time, and
/// returns whether a thread that woke should take the processor.
///
/// One at a time is what makes this right: a handle is a reference, so every
/// entry has to pass through [`release`], and a count of how many were
/// closed says nothing about which objects lost their last one.
pub fn close_every_handle<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: kernel_objects::object::ProcessId,
) -> bool {
    let mut switch = false;
    while let Some(entry) = machine.objects.close_next_handle(process) {
        switch |= release(machine, entry.object).unwrap_or(false);
    }
    switch
}
