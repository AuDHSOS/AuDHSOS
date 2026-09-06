// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::notify`, covering the notification items of catalog
//! 6.6.8.

use audhsos_abi::{Error, ThreadState};
use kernel_objects::object::Wait;

use super::fixture::Fixture;
use crate::cancel::cancel;
use crate::notify::{poll, signal, wait};
use crate::outcome::Wakeup;

#[test]
fn a_signal_without_a_waiter_accumulates_and_a_wait_takes_it_all() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let thread = fixture.running(owner, 4);
    let notification = fixture.notification();
    let outcome = signal(
        &mut fixture.objects,
        &mut fixture.scheduler,
        notification,
        0b0010,
    )
    .unwrap();
    assert_eq!(outcome.wakeup, None, "nobody waited");
    signal(
        &mut fixture.objects,
        &mut fixture.scheduler,
        notification,
        0b1000,
    )
    .unwrap();
    let outcome = wait(
        &mut fixture.objects,
        &mut fixture.scheduler,
        thread,
        notification,
    )
    .unwrap();
    assert!(!outcome.blocked, "there was something to take");
    assert_eq!(outcome.values, [0b1010, 0], "the two signals are merged");
    assert_eq!(
        fixture
            .objects
            .notifications
            .get(notification)
            .unwrap()
            .word,
        0,
        "a wait consumes everything present"
    );
}

#[test]
fn signalling_zero_is_a_success_that_changes_nothing() {
    let mut fixture = Fixture::new();
    let notification = fixture.notification();
    let outcome = signal(
        &mut fixture.objects,
        &mut fixture.scheduler,
        notification,
        0,
    )
    .unwrap();
    assert_eq!(outcome.wakeup, None);
    assert_eq!(
        fixture
            .objects
            .notifications
            .get(notification)
            .unwrap()
            .word,
        0
    );
}

#[test]
fn a_signal_that_arrives_while_a_thread_waits_wakes_it_with_the_word() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let waiter = fixture.running(owner, 4);
    let notification = fixture.notification();
    let outcome = wait(
        &mut fixture.objects,
        &mut fixture.scheduler,
        waiter,
        notification,
    )
    .unwrap();
    assert!(outcome.blocked);
    assert_eq!(
        fixture.state(waiter),
        Some(ThreadState::BlockedNotification)
    );
    assert_eq!(fixture.wait_of(waiter), Wait::Notification { notification });

    let outcome = signal(
        &mut fixture.objects,
        &mut fixture.scheduler,
        notification,
        0b0100,
    )
    .unwrap();
    assert_eq!(outcome.wakeup, Some(Wakeup::ok(waiter, [0b0100, 0])));
    assert_eq!(fixture.state(waiter), Some(ThreadState::Ready));
    assert_eq!(fixture.wait_of(waiter), Wait::Nothing);
    let held = fixture.objects.notifications.get(notification).unwrap();
    assert_eq!(held.word, 0, "the waiter took it");
    assert_eq!(held.waiter, None);
}

#[test]
fn a_second_waiter_is_refused_rather_than_queued() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let first = fixture.running(owner, 4);
    let notification = fixture.notification();
    wait(
        &mut fixture.objects,
        &mut fixture.scheduler,
        first,
        notification,
    )
    .unwrap();
    let second = fixture.thread(owner, 4);
    fixture.on_the_processor(second);
    assert_eq!(
        wait(
            &mut fixture.objects,
            &mut fixture.scheduler,
            second,
            notification
        ),
        Err(Error::Busy)
    );
    assert_eq!(fixture.state(second), Some(ThreadState::Running));
}

#[test]
fn a_poll_takes_what_is_there_and_never_blocks() {
    let mut fixture = Fixture::new();
    let notification = fixture.notification();
    assert_eq!(
        poll(&mut fixture.objects, notification).unwrap().values,
        [0, 0],
        "a poll on nothing returns zero"
    );
    signal(
        &mut fixture.objects,
        &mut fixture.scheduler,
        notification,
        0b11,
    )
    .unwrap();
    let outcome = poll(&mut fixture.objects, notification).unwrap();
    assert!(!outcome.blocked);
    assert_eq!(outcome.values, [0b11, 0]);
    assert_eq!(
        poll(&mut fixture.objects, notification).unwrap().values,
        [0, 0],
        "and it cleared what it took"
    );
}

#[test]
fn a_notification_that_is_gone_answers_no_operation() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let thread = fixture.running(owner, 4);
    let stale = kernel_objects::pool::ObjectId::new(6, 4);
    assert_eq!(
        signal(&mut fixture.objects, &mut fixture.scheduler, stale, 1),
        Err(Error::InvalidHandle)
    );
    assert_eq!(
        wait(&mut fixture.objects, &mut fixture.scheduler, thread, stale),
        Err(Error::InvalidHandle)
    );
    assert_eq!(poll(&mut fixture.objects, stale), Err(Error::InvalidHandle));
}

#[test]
fn a_thread_that_may_not_block_leaves_the_notification_as_it_found_it() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let inactive = fixture.thread(owner, 4);
    let notification = fixture.notification();
    assert_eq!(
        wait(
            &mut fixture.objects,
            &mut fixture.scheduler,
            inactive,
            notification
        ),
        Err(Error::InvalidState)
    );
    assert_eq!(
        fixture
            .objects
            .notifications
            .get(notification)
            .unwrap()
            .waiter,
        None
    );
    assert_eq!(fixture.wait_of(inactive), Wait::Nothing);
}

#[test]
fn a_signal_for_a_waiter_that_stopped_waiting_stays_in_the_word() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let waiter = fixture.running(owner, 4);
    let notification = fixture.notification();
    wait(
        &mut fixture.objects,
        &mut fixture.scheduler,
        waiter,
        notification,
    )
    .unwrap();
    // The thread is suspended out of the wait, which clears the record and
    // the waiter of the notification.
    assert!(cancel(&mut fixture.objects, waiter));
    fixture
        .scheduler
        .suspend(&mut fixture.objects.threads, waiter)
        .unwrap();
    let outcome = signal(
        &mut fixture.objects,
        &mut fixture.scheduler,
        notification,
        0b1,
    )
    .unwrap();
    assert_eq!(outcome.wakeup, None);
    assert_eq!(
        fixture
            .objects
            .notifications
            .get(notification)
            .unwrap()
            .word,
        0b1,
        "the bit waits for whoever asks next"
    );
}

#[test]
fn a_waiter_that_is_no_longer_blocked_is_not_woken_twice() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let waiter = fixture.running(owner, 4);
    let notification = fixture.notification();
    wait(
        &mut fixture.objects,
        &mut fixture.scheduler,
        waiter,
        notification,
    )
    .unwrap();
    // The scheduler wakes the thread without the notification knowing, the
    // way a `thread_resume` of a suspended thread does.
    fixture
        .scheduler
        .on_wake(&mut fixture.objects.threads, waiter)
        .unwrap();
    let outcome = signal(
        &mut fixture.objects,
        &mut fixture.scheduler,
        notification,
        0b1,
    )
    .unwrap();
    assert_eq!(outcome.wakeup, None);
    assert_eq!(
        fixture
            .objects
            .notifications
            .get(notification)
            .unwrap()
            .word,
        0b1
    );
}

#[test]
fn a_waiter_of_higher_priority_takes_the_processor_from_the_signaller() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let waiter = fixture.running(owner, 9);
    let notification = fixture.notification();
    wait(
        &mut fixture.objects,
        &mut fixture.scheduler,
        waiter,
        notification,
    )
    .unwrap();
    let signaller = fixture.thread(owner, 2);
    fixture.on_the_processor(signaller);
    let outcome = signal(
        &mut fixture.objects,
        &mut fixture.scheduler,
        notification,
        1,
    )
    .unwrap();
    assert!(outcome.reschedule);

    // The other way round it does not.
    let low = fixture.running(owner, 1);
    let second = fixture.notification();
    wait(&mut fixture.objects, &mut fixture.scheduler, low, second).unwrap();
    let high = fixture.thread(owner, 9);
    fixture.on_the_processor(high);
    let outcome = signal(&mut fixture.objects, &mut fixture.scheduler, second, 1).unwrap();
    assert!(!outcome.reschedule);
}

#[test]
fn signalling_zero_leaves_a_waiter_waiting() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let waiter = fixture.running(owner, 4);
    let notification = fixture.notification();
    wait(
        &mut fixture.objects,
        &mut fixture.scheduler,
        waiter,
        notification,
    )
    .unwrap();
    let outcome = signal(
        &mut fixture.objects,
        &mut fixture.scheduler,
        notification,
        0,
    )
    .unwrap();
    assert_eq!(outcome.wakeup, None, "no bit was set, so nothing happened");
    assert_eq!(
        fixture.state(waiter),
        Some(ThreadState::BlockedNotification)
    );
}
