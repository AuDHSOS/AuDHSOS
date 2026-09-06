// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The rendezvous on an endpoint: who meets whom, and what becomes of both.
//!
//! Invariants: a thread is in the senders queue of an endpoint exactly while
//! its state is `BlockedSend` and its record names that queue, and in the
//! receivers queue exactly while its state is `BlockedRecv`; a rendezvous
//! takes the head of a queue and no other thread, so the order of the queue
//! is the order in which threads are served; a caller that blocks has no
//! result until the thread that meets it writes one.
//!
//! Every operation is two steps, because the message moves through two IPC
//! buffers and this crate reaches neither. The first step finds the peer or
//! queues the caller; the caller then copies; the second step says what
//! became of the two threads. Between the two steps the endpoint holds
//! neither of them, which is what makes a refused copy leave nothing behind:
//! the peer is off its queue and the caller is still running.

use audhsos_abi::ipc_buffer::Status;
use audhsos_abi::{Error, Rights, ThreadState};
use kernel_objects::handle_table::Entry;
use kernel_objects::object::{
    AnyObjectId, EndpointId, ProcessId, Queue, Reply, ReplyId, ThreadId, Wait,
};
use kernel_objects::store::Objects;
use kernel_sched::Scheduler;
use kernel_sched::transition::Event;

use crate::outcome::{Outcome, Wakeup, block, state_of, wake};

/// What a thread that wants to send found at the endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Meeting {
    /// A receiver was waiting and has left its queue. The caller hands the
    /// message over and then calls [`sent`].
    Receiver(ThreadId),
    /// Nobody was waiting; the caller is in the senders queue and blocks.
    Queued(Outcome),
}

/// What a thread that wants to receive found at the endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reception {
    /// A sender was waiting and has left its queue. The caller takes the
    /// message and then calls [`received`].
    Sender {
        /// The thread whose message it is.
        sender: ThreadId,
        /// Whether it used `ipc_call` and waits for the answer.
        wants_reply: bool,
    },
    /// Nobody was waiting; the caller is in the receivers queue and blocks.
    Queued(Outcome),
    /// Nobody was waiting and the caller asked not to block. Both queues
    /// are as they were.
    Empty,
}

/// What the two sides of a completed rendezvous get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handover {
    /// The reply object the sender waits on, for a call.
    pub reply: Option<ReplyId>,
    /// The return words of the receiving side: the badge of the capability
    /// the sender used, and the reply handle or zero.
    pub received: [u64; 2],
    /// The status of the side that does not block, which is
    /// [`Status::PARTIAL`] when a handle of the message did not fit.
    pub status: Status,
}

impl Handover {
    /// A handover with no reply object and no badge.
    pub const PLAIN: Handover = Handover {
        reply: None,
        received: [0, 0],
        status: Status::OK,
    };
}

/// Finds a receiver on `endpoint` for `sender`, or queues the sender.
///
/// `wants_reply` is what tells the receiver that arrives later whether this
/// was `ipc_send` or `ipc_call`, and it is what the record of the queued
/// thread carries.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the endpoint or the sender is gone;
/// [`Error::InvalidState`] when the sender may not block, which the
/// transition table decides.
pub fn send<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    sender: ThreadId,
    endpoint: EndpointId,
    wants_reply: bool,
) -> Result<Meeting, Error> {
    let mut held = objects
        .endpoints
        .get(endpoint)
        .copied()
        .map_err(|_| Error::InvalidHandle)?;
    if let Some(receiver) = held.receivers.dequeue_front(&mut objects.threads) {
        forget(objects, receiver);
        write_back(objects, endpoint, held);
        return Ok(Meeting::Receiver(receiver));
    }
    let queue = if wants_reply {
        Queue::Callers
    } else {
        Queue::Senders
    };
    held.senders.enqueue(&mut objects.threads, sender)?;
    record(objects, sender, Wait::Endpoint { endpoint, queue });
    let blocked = block(&mut objects.threads, scheduler, sender, Event::BlockSend);
    match blocked {
        Ok(reschedule) => {
            write_back(objects, endpoint, held);
            Ok(Meeting::Queued(Outcome::blocked().switching(reschedule)))
        }
        Err(error) => {
            // The table refused the block, so the thread never waited: it
            // leaves the queue again and the endpoint is untouched.
            held.senders.unlink(&mut objects.threads, sender);
            record(objects, sender, Wait::Nothing);
            Err(error)
        }
    }
}

/// Finds a sender on `endpoint` for `receiver`, or queues the receiver.
///
/// `blocking` is `false` for `ipc_try_recv`, which leaves both queues as
/// they were and answers [`Reception::Empty`].
///
/// # Errors
///
/// As [`send`].
pub fn recv<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    receiver: ThreadId,
    endpoint: EndpointId,
    blocking: bool,
) -> Result<Reception, Error> {
    let mut held = objects
        .endpoints
        .get(endpoint)
        .copied()
        .map_err(|_| Error::InvalidHandle)?;
    if let Some(sender) = held.senders.dequeue_front(&mut objects.threads) {
        let wants_reply = matches!(
            objects.threads.get(sender).map(|thread| thread.wait),
            Ok(Wait::Endpoint {
                queue: Queue::Callers,
                ..
            })
        );
        write_back(objects, endpoint, held);
        return Ok(Reception::Sender {
            sender,
            wants_reply,
        });
    }
    if !blocking {
        return Ok(Reception::Empty);
    }
    held.receivers.enqueue(&mut objects.threads, receiver)?;
    record(
        objects,
        receiver,
        Wait::Endpoint {
            endpoint,
            queue: Queue::Receivers,
        },
    );
    match block(&mut objects.threads, scheduler, receiver, Event::BlockRecv) {
        Ok(reschedule) => {
            write_back(objects, endpoint, held);
            Ok(Reception::Queued(Outcome::blocked().switching(reschedule)))
        }
        Err(error) => {
            held.receivers.unlink(&mut objects.threads, receiver);
            record(objects, receiver, Wait::Nothing);
            Err(error)
        }
    }
}

/// Closes a rendezvous the sender began: the receiver becomes ready, and
/// the sender blocks for the answer when the handover carries a reply
/// object.
///
/// # Errors
///
/// [`Error::InvalidState`] when the sender may not block for the answer.
pub fn sent<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    sender: ThreadId,
    receiver: ThreadId,
    handover: Handover,
) -> Result<Outcome, Error> {
    let woken = wake(&mut objects.threads, scheduler, receiver).unwrap_or(false);
    let wakeup = Wakeup::ok(receiver, handover.received);
    let outcome = match handover.reply {
        Some(reply) => {
            record(objects, sender, Wait::Reply { reply });
            let reschedule = block(&mut objects.threads, scheduler, sender, Event::BlockReply)?;
            Outcome::blocked().switching(reschedule)
        }
        None => Outcome::DONE.with_status(handover.status).switching(woken),
    };
    Ok(outcome.waking(wakeup))
}

/// Closes a rendezvous the receiver began: the sender either wakes, because
/// its `ipc_send` is done, or stays blocked and waits for the answer.
///
/// # Errors
///
/// [`Error::InvalidState`] when the sender may not wait for the answer.
pub fn received<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    sender: ThreadId,
    receiver: ThreadId,
    handover: Handover,
) -> Result<Outcome, Error> {
    let _ = receiver;
    let mine =
        Outcome::values(handover.received[0], handover.received[1]).with_status(handover.status);
    if let Some(reply) = handover.reply {
        record(objects, sender, Wait::Reply { reply });
        let _ = block(&mut objects.threads, scheduler, sender, Event::BlockReply)?;
        return Ok(mine);
    }
    // The sender only sent: its call is done, and it wakes with the status
    // its own message earned.
    forget(objects, sender);
    let switch = wake(&mut objects.threads, scheduler, sender).unwrap_or(false);
    Ok(mine.waking(Wakeup::ok(sender, [0, 0])).switching(switch))
}

/// Puts the caller of a queued rendezvous back where it was, for a copy
/// that was refused: the peer is off its queue and has to go back into it,
/// or nobody can ever meet it again.
///
/// The peer keeps its place at the front, because it was the head when it
/// was taken and no other thread has been served since.
pub fn undo_meeting<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    endpoint: EndpointId,
    peer: ThreadId,
    queue: Queue,
) {
    {
        let Objects {
            endpoints, threads, ..
        } = &mut *objects;
        let Ok(held) = endpoints.get_mut(endpoint) else {
            return;
        };
        // The peer has just left this queue, so its links are clear and the
        // pool holds it: `requeue` refuses only a thread that is in a queue
        // already.
        let _ = if queue.is_sender() {
            held.senders.requeue(threads, peer)
        } else {
            held.receivers.requeue(threads, peer)
        };
    }
    record(objects, peer, Wait::Endpoint { endpoint, queue });
}

/// A reply object for `caller`, with a handle to it in `receiver`.
///
/// # Errors
///
/// [`Error::PoolExhausted`] when no reply slot is left;
/// [`Error::QuotaExceeded`] or [`Error::OutOfHandles`] when the receiver has
/// no handle slot. Nothing is left behind in either case.
pub fn open_reply<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    caller: ThreadId,
    receiver: ProcessId,
) -> Result<(ReplyId, audhsos_abi::Handle), Error> {
    let reply = objects
        .replies
        .allocate(Reply::new(caller))
        .map_err(Error::from)?;
    let entry = Entry::new(AnyObjectId::of(reply), Rights::EMPTY);
    match objects.install_handle(receiver, entry) {
        Ok(handle) => Ok((reply, handle)),
        Err(error) => {
            let _ = objects.replies.release(reply);
            Err(error)
        }
    }
}

/// Takes back what [`open_reply`] made, for a rendezvous that was refused
/// after it.
pub fn close_reply<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    reply: ReplyId,
    receiver: ProcessId,
    handle: audhsos_abi::Handle,
) {
    let _ = objects.close_handle(receiver, handle);
    let _ = objects.replies.release(reply);
}

/// The thread a reply object answers, if it is still waiting for one.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the reply object is gone;
/// [`Error::InvalidState`] when it has already been answered, or when its
/// caller is no longer waiting for one — a thread that was killed,
/// suspended, or resumed. Nobody's memory is touched in that case.
pub fn reply_caller<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &Objects<NP, NT, NM, NH>,
    reply: ReplyId,
) -> Result<ThreadId, Error> {
    let held = objects
        .replies
        .get(reply)
        .map_err(|_| Error::InvalidHandle)?;
    if held.consumed {
        return Err(Error::InvalidState);
    }
    let caller = held.caller;
    if state_of(&objects.threads, caller) != Some(ThreadState::BlockedReply) {
        return Err(Error::InvalidState);
    }
    if objects.threads.get(caller).map(|thread| thread.wait) != Ok(Wait::Reply { reply }) {
        return Err(Error::InvalidState);
    }
    Ok(caller)
}

/// Consumes the reply object and wakes the caller with `status`.
///
/// # Errors
///
/// As [`reply_caller`].
pub fn replied<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    reply: ReplyId,
    status: Status,
) -> Result<Outcome, Error> {
    let caller = reply_caller(objects, reply)?;
    objects.replies.with(reply, |held| held.consumed = true);
    forget(objects, caller);
    let switch = wake(&mut objects.threads, scheduler, caller).unwrap_or(false);
    Ok(Outcome::DONE
        .waking(Wakeup {
            thread: caller,
            status,
            values: [0, 0],
        })
        .switching(switch))
}

/// Writes the endpoint back into its slot.
fn write_back<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    endpoint: EndpointId,
    held: kernel_objects::object::Endpoint,
) {
    objects.endpoints.with(endpoint, |slot| *slot = held);
}

/// Records what `thread` waits on.
fn record<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    thread: ThreadId,
    wait: Wait,
) {
    objects.threads.with(thread, |held| held.wait = wait);
}

/// Clears what `thread` waits on.
fn forget<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    thread: ThreadId,
) {
    record(objects, thread, Wait::Nothing);
}
