// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Interrupt delivery: from a vector to the bit of a notification.
//!
//! Invariants: a line is masked from the moment its interrupt arrives until
//! `interrupt_ack`, so a device that keeps asserting cannot flood the
//! machine; exactly one bit of the bound notification is signalled, the one
//! the binding names; at most one interrupt object names a line, which is
//! what `interrupt_create` checks.
//!
//! The masking, the end-of-interrupt, and the unmasking themselves are the
//! adapter's: this decides what to signal and whom to wake.

use audhsos_abi::Error;
use kernel_objects::object::{InterruptId, NotificationId};
use kernel_objects::store::Objects;
use kernel_sched::Scheduler;

use crate::notify::deliver_word;
use crate::outcome::Outcome;

/// The interrupt object `vector` names, or `None` when no object does.
///
/// The pool is scanned, which is sixty-four slots: a table from vector to
/// object would have to be kept in step with the pool, and a scan of that
/// length in an interrupt handler is cheaper than that.
#[must_use]
pub fn interrupt_for<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &Objects<NP, NT, NM, NH>,
    vector: u8,
) -> Option<InterruptId> {
    objects
        .interrupts
        .iter()
        .find(|(_, held)| held.vector == vector)
        .map(|(id, _)| id)
}

/// The interrupt object that already names `line`, if one does.
#[must_use]
pub fn interrupt_of_line<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &Objects<NP, NT, NM, NH>,
    line: u8,
) -> Option<InterruptId> {
    objects
        .interrupts
        .iter()
        .find(|(_, held)| held.line == line)
        .map(|(id, _)| id)
}

/// An interrupt arrived on `vector`: the line is marked masked, the bound
/// bit is signalled, and the waiter of the notification wakes.
///
/// `None` says that no interrupt object names the vector, which is what a
/// line the kernel routed for itself looks like; the caller acknowledges it
/// at the hardware and does nothing else.
pub fn deliver<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    vector: u8,
) -> Option<Outcome> {
    let id = interrupt_for(objects, vector)?;
    let bound = {
        let held = objects.interrupts.get_mut(id).ok()?;
        held.masked = true;
        held.notification
    };
    let Some((notification, bit)) = bound else {
        return Some(Outcome::DONE);
    };
    Some(raise(objects, scheduler, notification, bit))
}

/// Signals bit `bit` of `notification` and wakes its waiter.
fn raise<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    notification: NotificationId,
    bit: u8,
) -> Outcome {
    let Ok(held) = objects.notifications.get_mut(notification) else {
        return Outcome::DONE;
    };
    held.word |= 1_u64.wrapping_shl(u32::from(bit));
    let held = *held;
    deliver_word(objects, scheduler, notification, held)
}

/// `interrupt_ack`: the line is no longer masked as far as the kernel is
/// concerned, and the adapter unmasks it. Returns the line.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the interrupt object is gone.
pub fn acknowledge<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    interrupt: InterruptId,
) -> Result<u8, Error> {
    let held = objects
        .interrupts
        .get_mut(interrupt)
        .map_err(|_| Error::InvalidHandle)?;
    held.masked = false;
    Ok(held.line)
}

/// `interrupt_bind`: the interrupt signals bit `bit` of `notification`.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when either object is gone;
/// [`Error::InvalidArgument`] for a bit index above sixty-three.
///
/// A notification another interrupt already signals into is not refused:
/// several interrupts may name one notification, each on a bit of its own
/// (D-108). A controller with two lines and one output buffer is drained by
/// one thread, and that thread waits on one notification.
pub fn bind<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    interrupt: InterruptId,
    notification: NotificationId,
    bit: u8,
) -> Result<(), Error> {
    if bit >= 64 {
        return Err(Error::InvalidArgument);
    }
    objects
        .notifications
        .get(notification)
        .map_err(|_| Error::InvalidHandle)?;
    let held = objects
        .interrupts
        .get_mut(interrupt)
        .map_err(|_| Error::InvalidHandle)?;
    held.notification = Some((notification, bit));
    Ok(())
}
