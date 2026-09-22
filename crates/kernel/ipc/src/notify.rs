// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Notifications: sixty-four bits that accumulate, and the one thread that
//! may wait for them.
//!
//! Invariants: a signal never loses a bit that was already present, so
//! several signals before a wait are merged; a wait consumes everything that
//! is present and leaves the word at zero; at most one thread waits, and a
//! second is refused rather than queued.

use audhsos_abi::Error;
use kernel_objects::object::{Notification, NotificationId, ThreadId, Wait};
use kernel_objects::store::Objects;
use kernel_sched::Processors as Scheduler;
use kernel_sched::transition::Event;

use crate::outcome::{Outcome, Wakeup, block, wake};

/// `notification_signal`: ORs `bits` into the word and wakes the waiter if
/// there is one and the word is not empty.
///
/// Signalling zero is a no-op that succeeds: it changes no bit, so a waiter
/// that would have been woken by it is still waiting for something that has
/// not happened.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the notification is gone.
pub fn signal<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    notification: NotificationId,
    bits: u64,
) -> Result<Outcome, Error> {
    let held = objects
        .notifications
        .get_mut(notification)
        .map_err(|_| Error::InvalidHandle)?;
    held.word |= bits;
    let held = *held;
    Ok(deliver_word(objects, scheduler, notification, held))
}

/// Hands the word of `held` to its waiter, if there is one and there is
/// anything to hand over.
pub(crate) fn deliver_word<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    notification: NotificationId,
    held: Notification,
) -> Outcome {
    let Some(waiter) = held.waiter else {
        return Outcome::DONE;
    };
    if held.word == 0 {
        return Outcome::DONE;
    }
    // `cancel` clears `waiter` for suspend, kill and process exit, so a
    // waiter that is not blocked here is a stale record from a path that
    // skipped `cancel`; the bits stay in the word for whoever asks next.
    let Some(reschedule) = wake(&mut objects.threads, scheduler, waiter) else {
        return Outcome::DONE;
    };
    let mut word = 0;
    objects.notifications.with(notification, |slot| {
        word = slot.consume();
        slot.waiter = None;
    });
    objects
        .threads
        .with(waiter, |thread| thread.wait = Wait::Nothing);
    Outcome::DONE
        .waking(Wakeup::ok(waiter, [word, 0]))
        .switching(reschedule)
}

/// `notification_wait`: takes what is present, or blocks until something is.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the notification is gone; [`Error::Busy`]
/// when another thread already waits on it; [`Error::InvalidState`] when the
/// caller may not block.
pub fn wait<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    waiter: ThreadId,
    notification: NotificationId,
) -> Result<Outcome, Error> {
    let held = objects
        .notifications
        .get(notification)
        .copied()
        .map_err(|_| Error::InvalidHandle)?;
    if held.word != 0 {
        return poll(objects, notification);
    }
    if held.waiter.is_some() {
        return Err(Error::Busy);
    }
    objects
        .notifications
        .with(notification, |slot| slot.waiter = Some(waiter));
    objects.threads.with(waiter, |thread| {
        thread.wait = Wait::Notification { notification }
    });
    match block(
        &mut objects.threads,
        scheduler,
        waiter,
        Event::BlockNotification,
    ) {
        Ok(reschedule) => Ok(Outcome::blocked().switching(reschedule)),
        Err(error) => {
            objects
                .notifications
                .with(notification, |slot| slot.waiter = None);
            objects
                .threads
                .with(waiter, |thread| thread.wait = Wait::Nothing);
            Err(error)
        }
    }
}

/// `notification_wait_until`: takes what is present, or blocks until
/// something is or until `deadline`, whichever comes first.
///
/// `now` is the clock the caller read, in the microseconds since boot that
/// `clock_now` answers in. Bits that are already there are taken and the
/// deadline is never consulted; a deadline that has passed answers at once
/// with no bits, which is what a thread that woke at its deadline also
/// finds (see [`expire`]).
///
/// # Errors
///
/// As [`wait`].
pub fn wait_until<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    waiter: ThreadId,
    notification: NotificationId,
    deadline: u64,
    now: u64,
) -> Result<Outcome, Error> {
    let held = objects
        .notifications
        .get(notification)
        .copied()
        .map_err(|_| Error::InvalidHandle)?;
    if held.word != 0 {
        return poll(objects, notification);
    }
    if held.waiter.is_some() {
        return Err(Error::Busy);
    }
    if deadline <= now {
        return Ok(Outcome::values(0, 0));
    }
    objects
        .notifications
        .with(notification, |slot| slot.waiter = Some(waiter));
    objects.threads.with(waiter, |thread| {
        thread.wait = Wait::Notification { notification }
    });
    match scheduler.on_block_until(
        &mut objects.threads,
        waiter,
        Event::BlockNotification,
        deadline,
    ) {
        Ok(outcome) => Ok(Outcome::blocked().switching(outcome.reschedule)),
        Err(error) => {
            objects
                .notifications
                .with(notification, |slot| slot.waiter = None);
            objects
                .threads
                .with(waiter, |thread| thread.wait = Wait::Nothing);
            Err(error)
        }
    }
}

/// Makes the next expired waiter ready with no bits. The caller loops until
/// `None`; each removal costs O(log n), and each processor's minimum costs O(1).
pub fn expire<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    now: u64,
) -> Option<Outcome> {
    let waiter = scheduler.expired(&mut objects.threads, now)?;
    let waited_on = objects.threads.get(waiter).ok().map(|thread| thread.wait);
    // Making the thread ready is the step that can fail, so it happens
    // before anything is given up. A wake that did not happen leaves the
    // notification still naming its waiter and the thread still recording
    // what it waits on, which is the only state from which a signal can
    // still reach it; and it answers an outcome rather than `None`, because
    // `None` is how the caller learns that nothing more is due, and one
    // entry it could not use is no reason to leave the rest for the next
    // tick.
    let Some(reschedule) = wake(&mut objects.threads, scheduler, waiter) else {
        return Some(Outcome::DONE);
    };
    if let Some(Wait::Notification { notification }) = waited_on {
        objects
            .notifications
            .with(notification, |slot| slot.waiter = None);
    }
    objects
        .threads
        .with(waiter, |thread| thread.wait = Wait::Nothing);
    Some(
        Outcome::DONE
            .waking(Wakeup::ok(waiter, [0, 0]))
            .switching(reschedule),
    )
}

/// `notification_poll`: takes what is present, which may be nothing.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the notification is gone.
pub fn poll<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    notification: NotificationId,
) -> Result<Outcome, Error> {
    let word = objects
        .notifications
        .get_mut(notification)
        .map(Notification::consume)
        .map_err(|_| Error::InvalidHandle)?;
    Ok(Outcome::values(word, 0))
}
