// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::endpoint` and `crate::cancel`, covering the endpoint
//! items of catalog 6.6.8.

use audhsos_abi::ipc_buffer::Status;
use audhsos_abi::{Error, Rights, ThreadState};
use kernel_objects::object::{AnyObjectId, Queue, Wait};
use test_support::generators::{range, vec};
use test_support::property::check;

use super::fixture::Fixture;
use crate::cancel::cancel;
use crate::endpoint::{
    Handover, Intent, Meeting, Reception, close_reply, open_reply, received, recv, replied,
    reply_caller, send, sent, undo_meeting,
};
use crate::outcome::Wakeup;

/// The badge a receiver sees in these tests.
const BADGE: u64 = 0x0BAD_6E00;

#[test]
fn a_call_with_no_receiver_blocks_the_caller_and_a_later_receive_completes_it() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let server = fixture.process(8);
    let caller = fixture.running(client, 4);
    let endpoint = fixture.endpoint();

    let met = send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        caller,
        endpoint,
        Intent::call(BADGE),
    )
    .unwrap();
    let Meeting::Queued(outcome) = met else {
        panic!("a call with no receiver met somebody");
    };
    assert!(outcome.blocked);
    assert!(outcome.reschedule);
    assert_eq!(fixture.state(caller), Some(ThreadState::BlockedSend));
    assert_eq!(
        fixture.wait_of(caller),
        Wait::Endpoint {
            endpoint,
            queue: Queue::Callers,
            badge: BADGE
        }
    );
    assert_eq!(fixture.senders(endpoint), vec![caller]);

    // The receiver arrives and finds the call.
    let receiver = fixture.running(server, 4);
    let found = recv(
        &mut fixture.objects,
        &mut fixture.scheduler,
        receiver,
        endpoint,
        true,
    )
    .unwrap();
    let Reception::Sender {
        sender,
        wants_reply,
        badge,
    } = found
    else {
        panic!("the receiver did not find the queued caller");
    };
    assert_eq!(sender, caller);
    assert!(wants_reply, "the caller used `ipc_call`");
    assert_eq!(badge, BADGE, "the badge waited with the caller");
    assert!(fixture.senders(endpoint).is_empty());

    let (reply, handle) = open_reply(&mut fixture.objects, caller, server).unwrap();
    let outcome = received(
        &mut fixture.objects,
        &mut fixture.scheduler,
        caller,
        receiver,
        Handover {
            reply: Some(reply),
            received: [BADGE, handle.raw()],
            status: Status::OK,
        },
    )
    .unwrap();
    assert!(!outcome.blocked, "the receiver runs on");
    assert_eq!(outcome.values, [BADGE, handle.raw()]);
    assert_eq!(outcome.wakeup, None, "the caller waits for the answer");
    assert_eq!(fixture.state(caller), Some(ThreadState::BlockedReply));
    assert_eq!(fixture.wait_of(caller), Wait::Reply { reply });
}

#[test]
fn a_receive_with_no_sender_blocks_and_a_later_call_completes_it() {
    let mut fixture = Fixture::new();
    let server = fixture.process(8);
    let client = fixture.process(8);
    let receiver = fixture.running(server, 4);
    let endpoint = fixture.endpoint();

    let found = recv(
        &mut fixture.objects,
        &mut fixture.scheduler,
        receiver,
        endpoint,
        true,
    )
    .unwrap();
    let Reception::Queued(outcome) = found else {
        panic!("an empty endpoint had a sender");
    };
    assert!(outcome.blocked);
    assert_eq!(fixture.state(receiver), Some(ThreadState::BlockedRecv));
    assert_eq!(fixture.receivers(endpoint), vec![receiver]);

    let caller = fixture.running(client, 4);
    let met = send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        caller,
        endpoint,
        Intent::call(BADGE),
    )
    .unwrap();
    assert_eq!(met, Meeting::Receiver(receiver));
    assert!(
        fixture.receivers(endpoint).is_empty(),
        "the receiver left its queue"
    );
    assert_eq!(fixture.wait_of(receiver), Wait::Nothing);

    let (reply, handle) = open_reply(&mut fixture.objects, caller, server).unwrap();
    let outcome = sent(
        &mut fixture.objects,
        &mut fixture.scheduler,
        caller,
        receiver,
        Handover {
            reply: Some(reply),
            received: [BADGE, handle.raw()],
            status: Status::OK,
        },
    )
    .unwrap();
    assert!(outcome.blocked, "the caller waits for the answer");
    assert_eq!(
        outcome.wakeup,
        Some(Wakeup::ok(receiver, [BADGE, handle.raw()]))
    );
    assert_eq!(fixture.state(receiver), Some(ThreadState::Ready));
    assert_eq!(fixture.state(caller), Some(ThreadState::BlockedReply));
}

#[test]
fn a_send_that_meets_a_receiver_answers_the_sender_and_wakes_the_receiver() {
    let mut fixture = Fixture::new();
    let server = fixture.process(8);
    let client = fixture.process(8);
    let receiver = fixture.running(server, 4);
    let endpoint = fixture.endpoint();
    recv(
        &mut fixture.objects,
        &mut fixture.scheduler,
        receiver,
        endpoint,
        true,
    )
    .unwrap();
    let sender = fixture.running(client, 4);
    let met = send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        sender,
        endpoint,
        Intent::PLAIN,
    )
    .unwrap();
    assert_eq!(met, Meeting::Receiver(receiver));
    let outcome = sent(
        &mut fixture.objects,
        &mut fixture.scheduler,
        sender,
        receiver,
        Handover {
            reply: None,
            received: [BADGE, 0],
            status: Status::OK,
        },
    )
    .unwrap();
    assert!(!outcome.blocked, "a send is done when the message is taken");
    assert_eq!(outcome.status, Status::OK);
    assert_eq!(outcome.wakeup, Some(Wakeup::ok(receiver, [BADGE, 0])));
    assert_eq!(fixture.state(sender), Some(ThreadState::Running));
}

#[test]
fn a_queued_sender_wakes_when_a_receiver_takes_its_message() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let server = fixture.process(8);
    let sender = fixture.running(client, 4);
    let endpoint = fixture.endpoint();
    send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        sender,
        endpoint,
        Intent::PLAIN,
    )
    .unwrap();
    assert_eq!(
        fixture.wait_of(sender),
        Wait::Endpoint {
            endpoint,
            queue: Queue::Senders,
            badge: 0
        }
    );
    let receiver = fixture.running(server, 4);
    let found = recv(
        &mut fixture.objects,
        &mut fixture.scheduler,
        receiver,
        endpoint,
        true,
    )
    .unwrap();
    assert_eq!(
        found,
        Reception::Sender {
            sender,
            wants_reply: false,
            badge: 0
        }
    );
    let outcome = received(
        &mut fixture.objects,
        &mut fixture.scheduler,
        sender,
        receiver,
        Handover {
            reply: None,
            received: [BADGE, 0],
            status: Status::PARTIAL,
        },
    )
    .unwrap();
    assert_eq!(outcome.status, Status::PARTIAL, "a handle did not fit");
    assert_eq!(outcome.wakeup, Some(Wakeup::ok(sender, [0, 0])));
    assert_eq!(fixture.state(sender), Some(ThreadState::Ready));
    assert_eq!(fixture.wait_of(sender), Wait::Nothing);
}

#[test]
fn try_recv_on_an_empty_endpoint_leaves_both_queues_as_they_were() {
    let mut fixture = Fixture::new();
    let server = fixture.process(8);
    let receiver = fixture.running(server, 4);
    let endpoint = fixture.endpoint();
    let found = recv(
        &mut fixture.objects,
        &mut fixture.scheduler,
        receiver,
        endpoint,
        false,
    )
    .unwrap();
    assert_eq!(found, Reception::Empty);
    assert!(fixture.senders(endpoint).is_empty());
    assert!(fixture.receivers(endpoint).is_empty());
    assert_eq!(fixture.state(receiver), Some(ThreadState::Running));
    assert_eq!(fixture.wait_of(receiver), Wait::Nothing);
}

#[test]
fn several_senders_are_served_by_priority_and_then_by_arrival() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let endpoint = fixture.endpoint();
    let mut queued = Vec::new();
    for priority in [2_u8, 7, 2, 9, 7] {
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
    let mut expected: Vec<_> = queued.clone();
    expected.sort_by_key(|(priority, _)| core::cmp::Reverse(*priority));
    let expected: Vec<_> = expected.into_iter().map(|(_, id)| id).collect();
    assert_eq!(fixture.senders(endpoint), expected);

    let server = fixture.process(8);
    let receiver = fixture.running(server, 1);
    for wanted in expected {
        let found = recv(
            &mut fixture.objects,
            &mut fixture.scheduler,
            receiver,
            endpoint,
            true,
        )
        .unwrap();
        assert_eq!(
            found,
            Reception::Sender {
                sender: wanted,
                wants_reply: false,
                badge: 0
            }
        );
        received(
            &mut fixture.objects,
            &mut fixture.scheduler,
            wanted,
            receiver,
            Handover::PLAIN,
        )
        .unwrap();
        fixture.on_the_processor(receiver);
    }
}

#[test]
fn the_order_of_the_senders_queue_is_priority_then_arrival_for_any_arrival() {
    // The property behind the test above, over sequences of priorities the
    // generator picks: the queue is always the stable sort of what arrived.
    check(
        "senders_are_ordered_by_priority_then_arrival",
        &vec(range(0_u8..=9), 0..=8),
        |priorities: &Vec<u8>| -> Result<(), String> {
            let mut fixture = Fixture::new();
            let client = fixture.process(8);
            let endpoint = fixture.endpoint();
            let mut arrived = Vec::new();
            for &priority in priorities {
                let sender = fixture.thread(client, priority);
                fixture.on_the_processor(sender);
                send(
                    &mut fixture.objects,
                    &mut fixture.scheduler,
                    sender,
                    endpoint,
                    Intent::PLAIN,
                )
                .map_err(|error| format!("the send was refused: {error}"))?;
                arrived.push((priority, sender));
            }
            let mut wanted = arrived;
            wanted.sort_by_key(|(priority, _)| core::cmp::Reverse(*priority));
            let wanted: Vec<_> = wanted.into_iter().map(|(_, id)| id).collect();
            if fixture.senders(endpoint) == wanted {
                Ok(())
            } else {
                Err(format!(
                    "{:?} arrived and {:?} waits",
                    priorities,
                    fixture.senders(endpoint)
                ))
            }
        },
    );
}

#[test]
fn a_reply_wakes_its_caller_and_a_second_one_is_refused() {
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
        Intent::call(BADGE),
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
            received: [BADGE, 0],
            status: Status::OK,
        },
    )
    .unwrap();

    assert_eq!(reply_caller(&fixture.objects, reply), Ok(caller));
    let outcome = replied(
        &mut fixture.objects,
        &mut fixture.scheduler,
        reply,
        Status::OK,
    )
    .unwrap();
    assert_eq!(
        outcome.wakeup,
        Some(Wakeup {
            thread: caller,
            status: Status::OK,
            values: [0, 0]
        })
    );
    assert_eq!(fixture.state(caller), Some(ThreadState::Ready));
    assert_eq!(fixture.wait_of(caller), Wait::Nothing);
    assert_eq!(
        replied(
            &mut fixture.objects,
            &mut fixture.scheduler,
            reply,
            Status::OK
        ),
        Err(Error::InvalidState),
        "a consumed reply object answers nothing twice"
    );
}

#[test]
fn a_reply_object_whose_caller_is_gone_is_refused_without_touching_memory() {
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
        Intent::call(BADGE),
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
            received: [BADGE, 0],
            status: Status::OK,
        },
    )
    .unwrap();

    // The caller is killed: the wait is cancelled and the thread ends.
    assert!(cancel(&mut fixture.objects, caller));
    fixture
        .scheduler
        .exit(&mut fixture.objects.threads, caller)
        .unwrap();
    assert_eq!(
        reply_caller(&fixture.objects, reply),
        Err(Error::InvalidState)
    );
    assert_eq!(
        replied(
            &mut fixture.objects,
            &mut fixture.scheduler,
            reply,
            Status::OK
        ),
        Err(Error::InvalidState)
    );
}

#[test]
fn a_reply_object_that_names_nothing_is_no_handle() {
    let mut fixture = Fixture::new();
    let stale = kernel_objects::pool::ObjectId::new(3, 9);
    assert_eq!(
        reply_caller(&fixture.objects, stale),
        Err(Error::InvalidHandle)
    );
    assert_eq!(
        replied(
            &mut fixture.objects,
            &mut fixture.scheduler,
            stale,
            Status::OK
        ),
        Err(Error::InvalidHandle)
    );
}

#[test]
fn a_reply_object_the_receiver_has_no_slot_for_leaves_nothing_behind() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let server = fixture.process(2);
    let caller = fixture.running(client, 4);
    let filler = fixture.memory();
    fixture.fill_handles(server, AnyObjectId::of(filler));
    assert_eq!(
        open_reply(&mut fixture.objects, caller, server),
        Err(Error::QuotaExceeded)
    );
    assert_eq!(fixture.objects.replies.live(), 0, "no slot was kept");
}

#[test]
fn a_reply_object_that_is_taken_back_leaves_neither_slot_nor_handle() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let server = fixture.process(8);
    let caller = fixture.running(client, 4);
    let before = fixture.handle_count(server);
    let (reply, handle) = open_reply(&mut fixture.objects, caller, server).unwrap();
    assert_eq!(fixture.handle_count(server), before + 1);
    close_reply(&mut fixture.objects, reply, server, handle);
    assert_eq!(fixture.handle_count(server), before);
    assert_eq!(fixture.objects.replies.live(), 0);
}

#[test]
fn a_sender_killed_while_it_waits_is_no_longer_in_the_queue() {
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
    let other = fixture.thread(client, 4);
    fixture.on_the_processor(other);
    send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        other,
        endpoint,
        Intent::PLAIN,
    )
    .unwrap();
    assert_eq!(fixture.senders(endpoint), vec![doomed, other]);

    assert!(cancel(&mut fixture.objects, doomed));
    fixture
        .scheduler
        .exit(&mut fixture.objects.threads, doomed)
        .unwrap();
    assert_eq!(
        fixture.senders(endpoint),
        vec![other],
        "a later receive does not see it"
    );
    let server = fixture.process(8);
    let receiver = fixture.running(server, 4);
    let found = recv(
        &mut fixture.objects,
        &mut fixture.scheduler,
        receiver,
        endpoint,
        true,
    )
    .unwrap();
    assert_eq!(
        found,
        Reception::Sender {
            sender: other,
            wants_reply: false,
            badge: 0
        }
    );
}

#[test]
fn a_sender_stays_blocked_when_the_receiver_that_waited_is_killed() {
    let mut fixture = Fixture::new();
    let server = fixture.process(8);
    let client = fixture.process(8);
    let endpoint = fixture.endpoint();
    let receiver = fixture.running(server, 4);
    recv(
        &mut fixture.objects,
        &mut fixture.scheduler,
        receiver,
        endpoint,
        true,
    )
    .unwrap();
    assert!(cancel(&mut fixture.objects, receiver));
    fixture
        .scheduler
        .exit(&mut fixture.objects.threads, receiver)
        .unwrap();

    let sender = fixture.running(client, 4);
    send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        sender,
        endpoint,
        Intent::PLAIN,
    )
    .unwrap();
    assert_eq!(
        fixture.state(sender),
        Some(ThreadState::BlockedSend),
        "there was nobody left to meet"
    );
    assert_eq!(fixture.senders(endpoint), vec![sender]);
}

#[test]
fn a_thread_suspended_while_it_waits_leaves_the_queue_it_waited_in() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let endpoint = fixture.endpoint();
    let waiting = fixture.running(client, 4);
    send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        waiting,
        endpoint,
        Intent::call(BADGE),
    )
    .unwrap();
    assert!(cancel(&mut fixture.objects, waiting));
    fixture
        .scheduler
        .suspend(&mut fixture.objects.threads, waiting)
        .unwrap();
    assert_eq!(fixture.state(waiting), Some(ThreadState::Suspended));
    assert_eq!(fixture.wait_of(waiting), Wait::Nothing);
    assert!(fixture.senders(endpoint).is_empty());
    fixture
        .scheduler
        .resume(&mut fixture.objects.threads, waiting)
        .unwrap();
    assert_eq!(fixture.state(waiting), Some(ThreadState::Ready));
    assert_eq!(
        fixture.wait_of(waiting),
        Wait::Nothing,
        "and it waits on nothing when it runs again"
    );
}

#[test]
fn cancelling_a_receiver_takes_it_out_of_the_receivers_queue() {
    let mut fixture = Fixture::new();
    let server = fixture.process(8);
    let endpoint = fixture.endpoint();
    let receiver = fixture.running(server, 4);
    recv(
        &mut fixture.objects,
        &mut fixture.scheduler,
        receiver,
        endpoint,
        true,
    )
    .unwrap();
    assert!(cancel(&mut fixture.objects, receiver));
    assert!(fixture.receivers(endpoint).is_empty());
    assert_eq!(fixture.wait_of(receiver), Wait::Nothing);
}

#[test]
fn cancelling_a_thread_that_waits_on_nothing_changes_nothing() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let running = fixture.running(client, 4);
    assert!(!cancel(&mut fixture.objects, running));
    let stale = kernel_objects::pool::ObjectId::new(9, 9);
    assert!(!cancel(&mut fixture.objects, stale));
}

#[test]
fn cancelling_a_caller_that_waits_for_an_answer_clears_only_its_record() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let server = fixture.process(8);
    let caller = fixture.running(client, 4);
    let (reply, _handle) = open_reply(&mut fixture.objects, caller, server).unwrap();
    fixture.objects.threads.get_mut(caller).unwrap().wait = Wait::Reply { reply };
    fixture
        .scheduler
        .on_block(
            &mut fixture.objects.threads,
            caller,
            kernel_sched::transition::Event::BlockReply,
        )
        .unwrap();
    assert!(cancel(&mut fixture.objects, caller));
    assert_eq!(fixture.wait_of(caller), Wait::Nothing);
    assert!(
        fixture.objects.replies.get(reply).is_ok(),
        "the reply object is the receiver's and stays"
    );
}

#[test]
fn cancelling_a_thread_whose_endpoint_is_gone_still_clears_its_record() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let endpoint = fixture.endpoint();
    let waiting = fixture.running(client, 4);
    send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        waiting,
        endpoint,
        Intent::PLAIN,
    )
    .unwrap();
    fixture
        .objects
        .destroy(AnyObjectId::of(endpoint))
        .expect("the last reference went");
    assert!(cancel(&mut fixture.objects, waiting));
    assert_eq!(fixture.wait_of(waiting), Wait::Nothing);
}

#[test]
fn an_endpoint_that_is_gone_can_neither_be_sent_on_nor_received_from() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let thread = fixture.running(client, 4);
    let stale = kernel_objects::pool::ObjectId::new(5, 3);
    assert_eq!(
        send(
            &mut fixture.objects,
            &mut fixture.scheduler,
            thread,
            stale,
            Intent::PLAIN
        ),
        Err(Error::InvalidHandle)
    );
    assert_eq!(
        recv(
            &mut fixture.objects,
            &mut fixture.scheduler,
            thread,
            stale,
            true
        ),
        Err(Error::InvalidHandle)
    );
}

#[test]
fn a_thread_that_may_not_block_leaves_the_endpoint_as_it_found_it() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let endpoint = fixture.endpoint();
    // A thread that was never started cannot block: the table has no row
    // out of `Inactive` for it.
    let inactive = fixture.thread(client, 4);
    assert_eq!(
        send(
            &mut fixture.objects,
            &mut fixture.scheduler,
            inactive,
            endpoint,
            Intent::PLAIN
        ),
        Err(Error::InvalidState)
    );
    assert!(fixture.senders(endpoint).is_empty());
    assert_eq!(fixture.wait_of(inactive), Wait::Nothing);
    assert_eq!(
        recv(
            &mut fixture.objects,
            &mut fixture.scheduler,
            inactive,
            endpoint,
            true
        ),
        Err(Error::InvalidState)
    );
    assert!(fixture.receivers(endpoint).is_empty());
}

#[test]
fn a_peer_that_was_taken_for_a_message_nobody_could_copy_goes_back_in_front() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let endpoint = fixture.endpoint();
    let first = fixture.running(client, 4);
    send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        first,
        endpoint,
        Intent::badged(BADGE),
    )
    .unwrap();
    let second = fixture.thread(client, 4);
    fixture.on_the_processor(second);
    send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        second,
        endpoint,
        Intent::PLAIN,
    )
    .unwrap();

    let server = fixture.process(8);
    let receiver = fixture.running(server, 4);
    let found = recv(
        &mut fixture.objects,
        &mut fixture.scheduler,
        receiver,
        endpoint,
        true,
    )
    .unwrap();
    assert_eq!(
        found,
        Reception::Sender {
            sender: first,
            wants_reply: false,
            badge: BADGE
        }
    );
    undo_meeting(&mut fixture.objects, endpoint, first, Queue::Senders, BADGE);
    assert_eq!(
        fixture.senders(endpoint),
        vec![first, second],
        "it kept its place at the front"
    );
    assert_eq!(
        fixture.wait_of(first),
        Wait::Endpoint {
            endpoint,
            queue: Queue::Senders,
            badge: BADGE
        }
    );
}

#[test]
fn undoing_a_meeting_on_an_endpoint_that_is_gone_does_nothing() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let thread = fixture.running(client, 4);
    let stale = kernel_objects::pool::ObjectId::new(4, 2);
    undo_meeting(&mut fixture.objects, stale, thread, Queue::Receivers, 0);
    assert_eq!(fixture.wait_of(thread), Wait::Nothing);
}

#[test]
fn a_receiver_that_was_taken_for_a_refused_message_goes_back_in_its_queue() {
    let mut fixture = Fixture::new();
    let server = fixture.process(8);
    let client = fixture.process(8);
    let endpoint = fixture.endpoint();
    let receiver = fixture.running(server, 4);
    recv(
        &mut fixture.objects,
        &mut fixture.scheduler,
        receiver,
        endpoint,
        true,
    )
    .unwrap();
    let sender = fixture.running(client, 4);
    assert_eq!(
        send(
            &mut fixture.objects,
            &mut fixture.scheduler,
            sender,
            endpoint,
            Intent::PLAIN
        ),
        Ok(Meeting::Receiver(receiver))
    );
    undo_meeting(
        &mut fixture.objects,
        endpoint,
        receiver,
        Queue::Receivers,
        0,
    );
    assert_eq!(fixture.receivers(endpoint), vec![receiver]);
    assert_eq!(
        fixture.wait_of(receiver),
        Wait::Endpoint {
            endpoint,
            queue: Queue::Receivers,
            badge: 0
        }
    );
    assert_eq!(fixture.state(receiver), Some(ThreadState::BlockedRecv));
}

#[test]
fn a_reply_object_carries_no_rights_of_its_own() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let server = fixture.process(8);
    let caller = fixture.running(client, 4);
    let (_reply, handle) = open_reply(&mut fixture.objects, caller, server).unwrap();
    let entry = fixture.objects.handles.lookup(server, handle).unwrap();
    assert_eq!(entry.rights, Rights::EMPTY, "the right is the object");
}

#[test]
fn a_reply_object_of_an_older_call_answers_nothing_after_a_second_one() {
    // The record of the caller and not only the consumed flag is what makes
    // this safe: the first reply object was never answered, because the
    // caller was suspended out of its wait, and it must not answer the call
    // the caller made afterwards.
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let server = fixture.process(8);
    let caller = fixture.running(client, 4);
    let endpoint = fixture.endpoint();
    let receiver = fixture.running(server, 4);

    let first = call_and_receive(&mut fixture, caller, receiver, server, endpoint);
    // The caller is suspended out of its wait and started again, which
    // clears its record and leaves the first reply object unanswered.
    assert!(cancel(&mut fixture.objects, caller));
    fixture
        .scheduler
        .suspend(&mut fixture.objects.threads, caller)
        .unwrap();
    fixture
        .scheduler
        .resume(&mut fixture.objects.threads, caller)
        .unwrap();
    fixture.on_the_processor(caller);
    fixture.on_the_processor(receiver);
    let second = call_and_receive(&mut fixture, caller, receiver, server, endpoint);

    assert_ne!(first, second);
    assert_eq!(
        reply_caller(&fixture.objects, first),
        Err(Error::InvalidState),
        "the older object answers nothing"
    );
    assert_eq!(reply_caller(&fixture.objects, second), Ok(caller));
}

#[test]
fn a_reply_object_whose_caller_was_cleared_away_is_no_handle() {
    let mut fixture = Fixture::new();
    let client = fixture.process(8);
    let server = fixture.process(8);
    let caller = fixture.running(client, 4);
    let endpoint = fixture.endpoint();
    let receiver = fixture.running(server, 4);
    let reply = call_and_receive(&mut fixture, caller, receiver, server, endpoint);
    assert!(cancel(&mut fixture.objects, caller));
    fixture
        .scheduler
        .exit(&mut fixture.objects.threads, caller)
        .unwrap();
    // The reaper gives the slot back, which is what a thread that ended
    // loses once the kernel has switched off its stack.
    fixture.objects.threads.release(caller).unwrap();
    assert_eq!(
        reply_caller(&fixture.objects, reply),
        Err(Error::InvalidState)
    );
}

/// A whole call and the receive that meets it, returning the reply object.
fn call_and_receive(
    fixture: &mut Fixture,
    caller: kernel_objects::object::ThreadId,
    receiver: kernel_objects::object::ThreadId,
    server: kernel_objects::object::ProcessId,
    endpoint: kernel_objects::object::EndpointId,
) -> kernel_objects::object::ReplyId {
    fixture.on_the_processor(caller);
    send(
        &mut fixture.objects,
        &mut fixture.scheduler,
        caller,
        endpoint,
        Intent::call(BADGE),
    )
    .unwrap();
    fixture.on_the_processor(receiver);
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
            received: [BADGE, 0],
            status: Status::OK,
        },
    )
    .unwrap();
    reply
}
