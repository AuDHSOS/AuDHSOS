// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::notify`, covering the notification items of catalog
//! 6.6.8.

use audhsos_abi::{Error, ThreadState};
use kernel_objects::object::Wait;

use super::fixture::Fixture;
use crate::cancel::cancel;
use crate::notify::{expire, poll, signal, wait, wait_until};
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

#[test]
fn a_wait_until_takes_what_is_present_and_never_looks_at_the_deadline() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let thread = fixture.running(owner, 4);
    let notification = fixture.notification();
    signal(
        &mut fixture.objects,
        &mut fixture.scheduler,
        notification,
        0b101,
    )
    .unwrap();
    // A deadline long past, which a call that takes bits never consults.
    let outcome = wait_until(
        &mut fixture.objects,
        &mut fixture.scheduler,
        thread,
        notification,
        0,
        9_000,
    )
    .unwrap();
    assert!(!outcome.blocked);
    assert_eq!(outcome.values, [0b101, 0]);
    assert_eq!(fixture.state(thread), Some(ThreadState::Running));
}

#[test]
fn a_deadline_that_has_passed_answers_at_once_with_no_bits() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let thread = fixture.running(owner, 4);
    let notification = fixture.notification();
    let outcome = wait_until(
        &mut fixture.objects,
        &mut fixture.scheduler,
        thread,
        notification,
        1_000,
        1_000,
    )
    .unwrap();
    assert!(!outcome.blocked);
    assert_eq!(outcome.values, [0, 0]);
    assert_eq!(fixture.state(thread), Some(ThreadState::Running));
    assert_eq!(
        fixture
            .objects
            .notifications
            .get(notification)
            .unwrap()
            .waiter,
        None,
        "nobody was recorded as waiting"
    );
}

#[test]
fn a_signal_before_the_deadline_wakes_the_waiter_with_the_word() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let waiter = fixture.running(owner, 4);
    let notification = fixture.notification();
    let outcome = wait_until(
        &mut fixture.objects,
        &mut fixture.scheduler,
        waiter,
        notification,
        5_000,
        0,
    )
    .unwrap();
    assert!(outcome.blocked);
    assert_eq!(
        fixture.objects.threads.get(waiter).unwrap().deadline,
        Some(5_000)
    );

    let outcome = signal(
        &mut fixture.objects,
        &mut fixture.scheduler,
        notification,
        0b10,
    )
    .unwrap();
    assert_eq!(outcome.wakeup, Some(Wakeup::ok(waiter, [0b10, 0])));
    assert_eq!(
        fixture.objects.threads.get(waiter).unwrap().deadline,
        None,
        "the entry left with the wait"
    );
    // The tick that follows the deadline finds nobody, so the thread is
    // not woken a second time.
    assert!(expire(&mut fixture.objects, &mut fixture.scheduler, 9_000).is_none());
}

#[test]
fn a_deadline_that_comes_first_wakes_the_waiter_with_nothing() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let waiter = fixture.running(owner, 4);
    let notification = fixture.notification();
    wait_until(
        &mut fixture.objects,
        &mut fixture.scheduler,
        waiter,
        notification,
        5_000,
        0,
    )
    .unwrap();
    assert!(
        expire(&mut fixture.objects, &mut fixture.scheduler, 4_999).is_none(),
        "a tick before the deadline wakes nobody"
    );
    let outcome = expire(&mut fixture.objects, &mut fixture.scheduler, 5_000).unwrap();
    assert_eq!(outcome.wakeup, Some(Wakeup::ok(waiter, [0, 0])));
    assert_eq!(fixture.state(waiter), Some(ThreadState::Ready));
    assert_eq!(fixture.wait_of(waiter), Wait::Nothing);
    assert_eq!(
        fixture
            .objects
            .notifications
            .get(notification)
            .unwrap()
            .waiter,
        None,
        "the notification has let go of it"
    );
    assert!(expire(&mut fixture.objects, &mut fixture.scheduler, 9_000).is_none());
}

#[test]
fn a_second_waiter_with_a_deadline_is_refused_as_one_without_is() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let first = fixture.running(owner, 4);
    let notification = fixture.notification();
    wait_until(
        &mut fixture.objects,
        &mut fixture.scheduler,
        first,
        notification,
        5_000,
        0,
    )
    .unwrap();
    let second = fixture.thread(owner, 4);
    fixture.on_the_processor(second);
    assert_eq!(
        wait_until(
            &mut fixture.objects,
            &mut fixture.scheduler,
            second,
            notification,
            5_000,
            0
        ),
        Err(Error::Busy)
    );
}

#[test]
fn a_wait_until_on_a_notification_that_is_gone_answers_nothing() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let thread = fixture.running(owner, 4);
    let stale = kernel_objects::pool::ObjectId::new(6, 4);
    assert_eq!(
        wait_until(
            &mut fixture.objects,
            &mut fixture.scheduler,
            thread,
            stale,
            1,
            0
        ),
        Err(Error::InvalidHandle)
    );
}

#[test]
fn a_thread_that_may_not_block_leaves_the_notification_as_it_found_it_with_a_deadline() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let inactive = fixture.thread(owner, 4);
    let notification = fixture.notification();
    assert_eq!(
        wait_until(
            &mut fixture.objects,
            &mut fixture.scheduler,
            inactive,
            notification,
            5_000,
            0
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
    assert_eq!(
        fixture.objects.threads.get(inactive).unwrap().deadline,
        None
    );
}

#[test]
fn the_deadlines_of_several_waiters_come_out_in_their_order() {
    let mut fixture = Fixture::new();
    let owner = fixture.process(8);
    let mut waiters = Vec::new();
    for (step, deadline) in [3_000_u64, 1_000, 2_000].iter().enumerate() {
        let thread = fixture.thread(owner, 4);
        fixture.on_the_processor(thread);
        let notification = fixture.notification();
        wait_until(
            &mut fixture.objects,
            &mut fixture.scheduler,
            thread,
            notification,
            *deadline,
            0,
        )
        .unwrap();
        waiters.push((*deadline, thread));
        let _ = step;
    }
    waiters.sort_by_key(|(deadline, _)| *deadline);
    let mut woken = Vec::new();
    while let Some(outcome) = expire(&mut fixture.objects, &mut fixture.scheduler, 3_000) {
        woken.push(outcome.wakeup.unwrap().thread);
    }
    let expected: Vec<_> = waiters.iter().map(|(_, thread)| *thread).collect();
    assert_eq!(woken, expected);
}
