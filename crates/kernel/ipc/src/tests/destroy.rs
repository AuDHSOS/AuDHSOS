// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::destroy` and of `crate::outcome::Waiters`, covering the
//! destruction items of catalog 6.6.8 and D-75.

use audhsos_abi::ipc_buffer::Status;
use audhsos_abi::{Error, ThreadState};
use kernel_objects::object::{AnyObjectId, Wait};
use kernel_objects::store::Destroyed;

use super::fixture::Fixture;
use crate::destroy::{destroy_endpoint, destroy_notification, destroy_reply, destroyed};
use crate::endpoint::{Handover, Intent, open_reply, received, recv, send};
use crate::notify::wait;
use crate::outcome::Waiters;

#[test]
fn the_last_handle_to_an_endpoint_wakes_everyone_on_both_queues() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let server = fixture.process(8);
    let endpoint = fixture.endpoint();

    // Two senders and one receiver: the receiver waits on a second
    // endpoint of its own, so that both queues of this one are filled.
    let first = fixture.running(client, 4);
    send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        first,
        endpoint,
        Intent::call(0),
    )
    .unwrap();
    let second = fixture.thread(client, 2);
    fixture.on_the_processor(second);
    send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        second,
        endpoint,
        Intent::PLAIN,
    )
    .unwrap();
    let waiting = fixture.thread(server, 4);
    fixture.on_the_processor(waiting);
    // The receiver of a queue that already holds senders would meet one, so
    // its queue is filled through a second endpoint and then moved over by
    // hand — which is what the machine looks like the moment before the
    // last handle closes.
    let other = fixture.endpoint();
    recv(
        &mut fixture.objects,
        &mut fixture.scheduler,
        waiting,
        other,
        true,
    )
    .unwrap();
    let moved = *fixture.objects.endpoints.get(other).unwrap();
    fixture
        .objects
        .endpoints
        .get_mut(endpoint)
        .unwrap()
        .receivers = moved.receivers;
    fixture.objects.threads.get_mut(waiting).unwrap().wait = Wait::Endpoint {
        endpoint,
        queue: kernel_objects::object::Queue::Receivers,
        badge: 0,
    };

    // Closing the last handle destroys the endpoint (D-75): a thread that
    // waits on an object holds no reference to it.
    let gone = fixture
        .objects
        .destroy(AnyObjectId::of(endpoint))
        .expect("the last reference went");
    let mut waiters = destroyed(&gone);
    let mut woken = Vec::new();
    while let Some(wakeup) = waiters.wake_next(&mut fixture.objects.threads, &mut fixture.scheduler)
    {
        assert_eq!(wakeup.status, Status::failed(Error::ObjectDestroyed));
        woken.push(wakeup.thread);
    }
    assert_eq!(woken.len(), 3, "everyone on both queues: {woken:?}");
    for thread in woken {
        assert_eq!(fixture.state(thread), Some(ThreadState::Ready));
        assert_eq!(fixture.wait_of(thread), Wait::Nothing);
    }
    assert!(waiters.is_empty());
}

#[test]
fn the_senders_of_a_destroyed_endpoint_wake_in_the_order_they_waited() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let endpoint = fixture.endpoint();
    let mut queued = Vec::new();
    for priority in [1_u8, 8, 4] {
        let sender = fixture.thread(client, priority);
        fixture.on_the_processor(sender);
        send(
            &mut fixture.objects,
            &mut fixture.scheduler,
            sender,
            endpoint,
            Intent::PLAIN,
        )
        .unwrap();
        queued.push((priority, sender));
    }
    queued.sort_by_key(|(priority, _)| core::cmp::Reverse(*priority));
    let expected: Vec<_> = queued.into_iter().map(|(_, id)| id).collect();
    let held = *fixture.objects.endpoints.get(endpoint).unwrap();
    let mut waiters = destroy_endpoint(&held);
    let mut woken = Vec::new();
    while let Some(wakeup) = waiters.wake_next(&mut fixture.objects.threads, &mut fixture.scheduler)
    {
        woken.push(wakeup.thread);
    }
    assert_eq!(woken, expected);
}

#[test]
fn a_destroyed_notification_wakes_its_waiter_with_object_destroyed() {
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
    let gone = fixture
        .objects
        .destroy(AnyObjectId::of(notification))
        .expect("the last reference went");
    let mut waiters = destroyed(&gone);
    let wakeup = waiters
        .wake_next(&mut fixture.objects.threads, &mut fixture.scheduler)
        .unwrap();
    assert_eq!(wakeup.thread, waiter);
    assert_eq!(wakeup.status, Status::failed(Error::ObjectDestroyed));
    assert!(
        waiters
            .wake_next(&mut fixture.objects.threads, &mut fixture.scheduler)
            .is_none()
    );
    assert_eq!(fixture.state(waiter), Some(ThreadState::Ready));
}

#[test]
fn a_notification_nobody_waited_on_owes_nobody_anything() {
    let mut fixture = Fixture::new();
    let notification = fixture.notification();
    let held = *fixture.objects.notifications.get(notification).unwrap();
    let mut waiters = destroy_notification(&held);
    assert!(waiters.is_empty());
    assert!(
        waiters
            .wake_next(&mut fixture.objects.threads, &mut fixture.scheduler)
            .is_none()
    );
}

#[test]
fn a_reply_object_dropped_without_an_answer_wakes_its_caller() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let server = fixture.process(8);
    let caller = fixture.running(client, 4);
    let endpoint = fixture.endpoint();
    send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        caller,
        endpoint,
        Intent::call(0),
    )
    .unwrap();
    let receiver = fixture.running(server, 4);
    recv(
        &mut fixture.objects,
        &mut fixture.scheduler,
        receiver,
        endpoint,
        true,
    )
    .unwrap();
    let (reply, _handle) = open_reply(&mut fixture.objects, caller, server).unwrap();
    received(
        &mut fixture.objects,
        &mut fixture.scheduler,
        caller,
        receiver,
        Handover {
            reply: Some(reply),
            received: [0, 0],
            status: Status::OK,
        },
    )
    .unwrap();

    let gone = fixture
        .objects
        .destroy(AnyObjectId::of(reply))
        .expect("the last reference went");
    let mut waiters = destroyed(&gone);
    let wakeup = waiters
        .wake_next(&mut fixture.objects.threads, &mut fixture.scheduler)
        .unwrap();
    assert_eq!(wakeup.thread, caller);
    assert_eq!(wakeup.status, Status::failed(Error::ReplyDropped));
    assert_eq!(fixture.state(caller), Some(ThreadState::Ready));
}

#[test]
fn a_reply_object_that_was_answered_owes_its_caller_nothing() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let caller = fixture.running(client, 4);
    let mut held = kernel_objects::object::Reply::new(caller);
    held.consumed = true;
    let mut waiters = destroy_reply(&held);
    assert!(waiters.is_empty());
    assert!(
        waiters
            .wake_next(&mut fixture.objects.threads, &mut fixture.scheduler)
            .is_none()
    );
}

#[test]
fn an_object_nobody_could_wait_on_leaves_no_waiters() {
    let mut fixture = Fixture::new();
    let memory = fixture.memory();
    let gone = fixture
        .objects
        .destroy(AnyObjectId::of(memory))
        .expect("the last reference went");
    assert!(matches!(gone, Destroyed::Quietly(_)));
    let mut waiters = destroyed(&gone);
    assert!(waiters.is_empty());
    assert!(
        waiters
            .wake_next(&mut fixture.objects.threads, &mut fixture.scheduler)
            .is_none()
    );
    assert!(Waiters::none().is_empty());
}

#[test]
fn a_waiter_the_pool_no_longer_holds_is_dropped_rather_than_reported() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let endpoint = fixture.endpoint();
    let doomed = fixture.running(client, 4);
    send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        doomed,
        endpoint,
        Intent::PLAIN,
    )
    .unwrap();
    let held = *fixture.objects.endpoints.get(endpoint).unwrap();
    // The thread ends before the endpoint is taken apart, which is what a
    // `process_kill` that races the close of the last handle looks like.
    fixture
        .scheduler
        .exit(&mut fixture.objects.threads, doomed)
        .unwrap();
    let mut waiters = destroy_endpoint(&held);
    assert!(
        waiters
            .wake_next(&mut fixture.objects.threads, &mut fixture.scheduler)
            .is_none(),
        "there is nobody left to tell"
    );
}
