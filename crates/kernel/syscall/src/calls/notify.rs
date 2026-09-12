// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The notification calls.
//!
//! Invariant: a signal never loses a bit, and a wait or a poll takes
//! everything that is present and leaves nothing.

use audhsos_abi::{Error, Rights};
use kernel_ipc::notify;
use kernel_objects::handle_table::Entry;
use kernel_objects::object::{AnyObjectId, Notification, ProcessId, ThreadId};

use crate::calls::ipc::apply;
use crate::dispatch::{Machine, Reply, Request};
use crate::environment::Environment;

/// The rights a capability to a fresh notification carries.
const NOTIFICATION_RIGHTS: Rights = Rights::SIGNAL
    .union(Rights::WAIT)
    .union(Rights::BIND)
    .union(Rights::DUPLICATE)
    .union(Rights::TRANSFER);

/// `notification_create`.
///
/// # Errors
///
/// [`Error::QuotaExceeded`] when the process may hold no further object;
/// [`Error::PoolExhausted`] when no notification slot is left;
/// [`Error::OutOfHandles`] when the caller has no slot for the handle.
pub fn create<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
) -> Result<Reply, Error> {
    super::charge_object(machine, process)?;
    let id = match machine.objects.notifications.allocate(Notification::new()) {
        Ok(id) => id,
        Err(error) => {
            super::refund_object(machine, process);
            return Err(Error::from(error));
        }
    };
    let entry = Entry::new(AnyObjectId::of(id), NOTIFICATION_RIGHTS);
    match machine.objects.install_handle(process, entry) {
        Ok(handle) => Ok(Reply::value(handle.raw())),
        Err(error) => {
            let _ = machine.objects.notifications.release(id);
            super::refund_object(machine, process);
            Err(error)
        }
    }
}

/// `notification_signal`: ORs the bits into the word.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for a notification that is gone.
pub fn signal<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let id = notification_of(machine, process, request, Rights::SIGNAL)?;
    let outcome = notify::signal(machine.objects, machine.scheduler, id, request.argument(1))?;
    apply(machine, outcome)
}

/// `notification_wait`: takes what is present, or blocks until something is.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for a notification that is gone;
/// [`Error::Busy`] when another thread already waits on it;
/// [`Error::InvalidState`] when the caller may not block.
pub fn wait<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ThreadId,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let id = notification_of(machine, process, request, Rights::WAIT)?;
    let outcome = notify::wait(machine.objects, machine.scheduler, caller, id)?;
    apply(machine, outcome)
}

/// `notification_wait_until`: takes what is present, or blocks until
/// something is or until the deadline of the second argument, whichever
/// comes first. A thread that woke at its deadline finds zero bits, which
/// a caller that must tell the two apart separates by reading the clock.
///
/// # Errors
///
/// As [`wait`].
pub fn wait_until<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ThreadId,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let id = notification_of(machine, process, request, Rights::WAIT)?;
    let now = machine.environment.now_micros();
    let outcome = notify::wait_until(
        machine.objects,
        machine.scheduler,
        caller,
        id,
        request.argument(1),
        now,
    )?;
    apply(machine, outcome)
}

/// `notification_poll`: takes what is present, which may be nothing.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for a notification that is gone.
pub fn poll<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let id = notification_of(machine, process, request, Rights::WAIT)?;
    let outcome = notify::poll(machine.objects, id)?;
    apply(machine, outcome)
}

/// The notification a handle names.
fn notification_of<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
    required: Rights,
) -> Result<kernel_objects::object::NotificationId, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let (id, _rights) = machine
        .objects
        .resolve::<Notification>(process, handle, required)?;
    Ok(id)
}
