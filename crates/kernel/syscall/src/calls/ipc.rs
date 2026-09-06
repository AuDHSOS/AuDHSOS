// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The endpoint calls, and the seam to the buffer of a second thread.
//!
//! Invariants: a message is refused before anything is copied when its
//! header does not check or its label lies in the range the kernel keeps
//! for itself; a rendezvous whose copy is refused leaves the peer back in
//! the queue it was taken from, so nobody is lost; a caller that blocks
//! finds nothing in its result area until the thread that meets it writes
//! one.
//!
//! Every transfer has the caller on one side. The dispatcher holds the
//! buffer of the calling thread, and [`Environment::with_buffer`] reaches
//! the other, so a send copies out of the caller's buffer and a receive
//! copies into it.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE, Status};
use audhsos_abi::{Error, Handle, Rights};
use kernel_ipc::endpoint::{Handover, Intent, Meeting, Reception};
use kernel_ipc::outcome::{Outcome as IpcOutcome, Wakeup};
use kernel_ipc::{endpoint, transfer};
use kernel_objects::object::{
    AnyObjectId, Endpoint, EndpointId, ProcessId, Queue, Reply as ReplyObject, ReplyId, ThreadId,
};
use kernel_sched::Outcome;

use crate::dispatch::{Machine, Reply, Request};
use crate::environment::Environment;

/// The rights a capability to a fresh endpoint carries.
const ENDPOINT_RIGHTS: Rights = Rights::SEND
    .union(Rights::RECV)
    .union(Rights::BADGE)
    .union(Rights::DUPLICATE)
    .union(Rights::TRANSFER);

/// The rights a badged capability carries: of the rights of an endpoint,
/// `SEND` alone, so a second badging is refused by the ordinary rights
/// check. The two generic rights come along, or the badged capability could
/// never be handed to the client it was made for.
const BADGED_RIGHTS: Rights = Rights::SEND
    .union(Rights::DUPLICATE)
    .union(Rights::TRANSFER);

/// `endpoint_create`: a rendezvous point of the calling process.
///
/// # Errors
///
/// [`Error::QuotaExceeded`] when the process may hold no further object;
/// [`Error::PoolExhausted`] when no endpoint slot is left;
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
    let id = match machine.objects.endpoints.allocate(Endpoint::new()) {
        Ok(id) => id,
        Err(error) => {
            super::refund_object(machine, process);
            return Err(Error::from(error));
        }
    };
    let entry = kernel_objects::handle_table::Entry::new(AnyObjectId::of(id), ENDPOINT_RIGHTS);
    match machine.objects.install_handle(process, entry) {
        Ok(handle) => Ok(Reply::value(handle.raw())),
        Err(error) => {
            let _ = machine.objects.endpoints.release(id);
            super::refund_object(machine, process);
            Err(error)
        }
    }
}

/// `endpoint_badge`: a send-only capability of the same endpoint that
/// carries a badge the receiver sees.
///
/// # Errors
///
/// [`Error::InvalidArgument`] for a badge of zero, which is what an
/// unbadged capability carries; [`Error::QuotaExceeded`] or
/// [`Error::OutOfHandles`] when the caller has no slot for the second
/// handle. A second badging is refused with [`Error::AccessDenied`] by the
/// rights check of the dispatcher, because the result carries no `BADGE`.
pub fn badge<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let (id, _rights) = machine
        .objects
        .resolve::<Endpoint>(process, handle, Rights::BADGE)?;
    let badge = request.argument(1);
    if badge == 0 {
        return Err(Error::InvalidArgument);
    }
    let entry = kernel_objects::handle_table::Entry::new(AnyObjectId::of(id), BADGED_RIGHTS)
        .with_badge(badge);
    let badged = machine.objects.install_handle(process, entry)?;
    machine.objects.retain(AnyObjectId::of(id))?;
    Ok(Reply::value(badged.raw()))
}

/// `ipc_send` and `ipc_call`, which differ in whether the sender waits for
/// the answer.
///
/// # Errors
///
/// [`Error::InvalidArgument`] for a header whose counts are above what the
/// message area holds and for a label in the range the kernel keeps;
/// [`Error::InvalidHandle`] for an endpoint that is gone;
/// [`Error::AccessDenied`] for a handle of the message without `TRANSFER`;
/// [`Error::PoolExhausted`] when a call finds no reply slot;
/// [`Error::InvalidState`] when the caller may not block.
pub fn send<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ThreadId,
    process: ProcessId,
    request: &Request,
    buffer: &mut [u8; SIZE],
    wants_reply: bool,
) -> Result<Reply, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let entry = machine.objects.entry(process, handle)?;
    let endpoint = entry.object.typed::<Endpoint>()?;
    check_header(buffer)?;
    let intent = Intent {
        wants_reply,
        badge: entry.badge,
    };
    deliver(machine, caller, process, endpoint, intent, buffer)
}

/// The send half of `ipc_send`, `ipc_call`, and the fault message: the
/// rendezvous, the copy between its two steps, and the result.
///
/// # Errors
///
/// As [`send`].
pub fn deliver<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    sender: ThreadId,
    sender_process: ProcessId,
    endpoint: EndpointId,
    intent: Intent,
    buffer: &[u8; SIZE],
) -> Result<Reply, Error> {
    let met = endpoint::send(machine.objects, machine.scheduler, sender, endpoint, intent)?;
    let Meeting::Receiver(receiver) = met else {
        return Ok(Reply::BLOCKED);
    };
    let receiver_process = machine.objects.threads.get(receiver)?.process;
    let receiver_frame = machine.objects.threads.get(receiver)?.ipc_buffer;
    let sender_frame = machine.objects.threads.get(sender)?.ipc_buffer;
    // Two threads never share an IPC buffer, so this is a check on the
    // argument and not a state the kernel can reach.
    if receiver_frame == sender_frame {
        endpoint::undo_meeting(machine.objects, endpoint, receiver, Queue::Receivers, 0);
        return Err(Error::InvalidArgument);
    }
    let opened = if intent.wants_reply {
        match endpoint::open_reply(machine.objects, sender, receiver_process) {
            Ok(pair) => Some(pair),
            Err(error) => {
                endpoint::undo_meeting(machine.objects, endpoint, receiver, Queue::Receivers, 0);
                return Err(error);
            }
        }
    } else {
        None
    };
    let moved = {
        let objects = &mut *machine.objects;
        machine.environment.with_buffer(receiver_frame, |to| {
            transfer::transfer(buffer, to, objects, sender_process, receiver_process)
        })?
    };
    let moved = match moved {
        Ok(moved) => moved,
        Err(error) => {
            if let Some((reply, handle)) = opened {
                endpoint::close_reply(machine.objects, reply, receiver_process, handle);
            }
            endpoint::undo_meeting(machine.objects, endpoint, receiver, Queue::Receivers, 0);
            return Err(error);
        }
    };
    let handover = Handover {
        reply: opened.map(|(reply, _)| reply),
        received: [intent.badge, opened.map_or(0, |(_, handle)| handle.raw())],
        status: status_of(moved.truncated),
    };
    let outcome = endpoint::sent(
        machine.objects,
        machine.scheduler,
        sender,
        receiver,
        handover,
    )?;
    apply(machine, outcome)
}

/// `ipc_recv` and `ipc_try_recv`.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for an endpoint that is gone;
/// [`Error::WouldBlock`] when `ipc_try_recv` finds nobody;
/// [`Error::InvalidState`] when the caller may not block;
/// [`Error::QuotaExceeded`] or [`Error::OutOfHandles`] when a call is
/// waiting and the caller has no slot for its reply handle.
pub fn recv<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ThreadId,
    process: ProcessId,
    endpoint: EndpointId,
    buffer: &mut [u8; SIZE],
    blocking: bool,
) -> Result<Reply, Error> {
    let found = endpoint::recv(
        machine.objects,
        machine.scheduler,
        caller,
        endpoint,
        blocking,
    )?;
    let Reception::Sender {
        sender,
        wants_reply,
        badge,
    } = found
    else {
        if matches!(found, Reception::Empty) {
            return Err(Error::WouldBlock);
        }
        return Ok(Reply::BLOCKED);
    };
    let queue = if wants_reply {
        Queue::Callers
    } else {
        Queue::Senders
    };
    let sender_process = machine.objects.threads.get(sender)?.process;
    let sender_frame = machine.objects.threads.get(sender)?.ipc_buffer;
    let own_frame = machine.objects.threads.get(caller)?.ipc_buffer;
    if sender_frame == own_frame {
        endpoint::undo_meeting(machine.objects, endpoint, sender, queue, badge);
        return Err(Error::InvalidArgument);
    }
    let opened = if wants_reply {
        match endpoint::open_reply(machine.objects, sender, process) {
            Ok(pair) => Some(pair),
            Err(error) => {
                endpoint::undo_meeting(machine.objects, endpoint, sender, queue, badge);
                return Err(error);
            }
        }
    } else {
        None
    };
    let moved = {
        let objects = &mut *machine.objects;
        machine.environment.with_buffer(sender_frame, |from| {
            transfer::transfer(&*from, buffer, objects, sender_process, process)
        })?
    };
    let moved = match moved {
        Ok(moved) => moved,
        Err(error) => {
            if let Some((reply, handle)) = opened {
                endpoint::close_reply(machine.objects, reply, process, handle);
            }
            endpoint::undo_meeting(machine.objects, endpoint, sender, queue, badge);
            return Err(error);
        }
    };
    let handover = Handover {
        reply: opened.map(|(reply, _)| reply),
        received: [badge, opened.map_or(0, |(_, handle)| handle.raw())],
        status: status_of(moved.truncated),
    };
    let outcome = endpoint::received(machine.objects, machine.scheduler, sender, caller, handover)?;
    apply(machine, outcome)
}

/// `ipc_recv` and `ipc_try_recv` as the dispatcher reaches them.
///
/// # Errors
///
/// As [`recv`].
pub fn receive<
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
    buffer: &mut [u8; SIZE],
    blocking: bool,
) -> Result<Reply, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let (endpoint, _rights) = machine
        .objects
        .resolve::<Endpoint>(process, handle, Rights::RECV)?;
    recv(machine, caller, process, endpoint, buffer, blocking)
}

/// `ipc_reply`: the answer to a call, which consumes the reply object.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for a reply object that is gone;
/// [`Error::InvalidState`] for one that has already been answered and for
/// one whose caller is no longer waiting for an answer, which is refused
/// without touching anybody's memory; [`Error::InvalidArgument`] for a
/// header that does not check.
pub fn reply<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
    buffer: &[u8; SIZE],
) -> Result<Reply, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let (id, _rights) = machine
        .objects
        .resolve::<ReplyObject>(process, handle, Rights::EMPTY)?;
    let mut outcome = answer(machine, process, id, buffer)?;
    outcome.reschedule |= consume(machine, process, handle)?;
    apply(machine, outcome)
}

/// Gives up the reply handle once the answer has gone out.
///
/// A reply object is answered once, and 2.6 says the answer consumes it.
/// The handle has to go with it: an entry that names a destroyed object is
/// a slot nobody can use and nobody can free, and a server that leaves one
/// behind per call fills its table and stops receiving. Closing it here
/// also drops the reference the entry held, which is what lets the object
/// itself go back to its pool.
fn consume<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    handle: Handle,
) -> Result<bool, Error> {
    let entry = machine.objects.close_handle(process, handle)?;
    crate::lifetime::release(machine, entry.object)
}

/// `ipc_reply_recv`: the answer and then the next receive, without
/// returning to userland in between. A reply that fails does not become a
/// receive.
///
/// # Errors
///
/// As [`reply`] and [`recv`].
pub fn reply_recv<
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
    buffer: &mut [u8; SIZE],
) -> Result<Reply, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let (id, _rights) = machine
        .objects
        .resolve::<ReplyObject>(process, handle, Rights::EMPTY)?;
    let second = Handle::from_raw(request.argument(1)).ok_or(Error::InvalidHandle)?;
    let (endpoint, _rights) = machine
        .objects
        .resolve::<Endpoint>(process, second, Rights::RECV)?;
    let answered = answer(machine, process, id, buffer)?;
    if let Some(wakeup) = answered.wakeup {
        write_result(machine, wakeup)?;
    }
    let switched = consume(machine, process, handle)?;
    let mut reply = recv(machine, caller, process, endpoint, buffer, true)?;
    if answered.reschedule || switched {
        reply.outcome = Outcome::RESCHEDULE;
    }
    Ok(reply)
}

/// The answer itself: the message of the replier into the buffer of the
/// caller, and the caller awake.
fn answer<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    id: ReplyId,
    buffer: &[u8; SIZE],
) -> Result<IpcOutcome, Error> {
    let target = endpoint::reply_caller(machine.objects, id)?;
    check_header(buffer)?;
    let target_process = machine.objects.threads.get(target)?.process;
    let target_frame = machine.objects.threads.get(target)?.ipc_buffer;
    let moved = {
        let objects = &mut *machine.objects;
        machine.environment.with_buffer(target_frame, |to| {
            transfer::transfer(buffer, to, objects, process, target_process)
        })?
    }?;
    endpoint::replied(
        machine.objects,
        machine.scheduler,
        id,
        status_of(moved.truncated),
    )
}

/// The status a message with a handle that did not fit leaves behind. It is
/// the error flag of 2.6.2 and not an error: the message is delivered.
const fn status_of(truncated: bool) -> Status {
    if truncated {
        Status::PARTIAL
    } else {
        Status::OK
    }
}

/// The header of a message a user thread wrote, checked before anything is
/// copied.
///
/// # Errors
///
/// [`Error::InvalidArgument`] when a count is above what the message area
/// holds, and when the label lies in the range the kernel keeps for its own
/// messages, which is what makes that range reserved.
pub fn check_header(buffer: &[u8; SIZE]) -> Result<(), Error> {
    let message = Buffer::new(buffer).message().map_err(Error::from)?;
    if message.is_kernel_label() {
        return Err(Error::InvalidArgument);
    }
    Ok(())
}

/// Writes the result of a thread that became ready into its own buffer, and
/// turns the rest of the outcome into the reply of the caller.
///
/// # Errors
///
/// [`Error::InvalidArgument`] when the buffer of the woken thread is not
/// reachable, which no buffer of a live thread is.
pub fn apply<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    outcome: IpcOutcome,
) -> Result<Reply, Error> {
    if let Some(wakeup) = outcome.wakeup {
        write_result(machine, wakeup)?;
    }
    let switch = if outcome.reschedule {
        Outcome::RESCHEDULE
    } else {
        Outcome::NOTHING
    };
    if outcome.blocked {
        return Ok(Reply {
            outcome: switch,
            ..Reply::BLOCKED
        });
    }
    Ok(Reply {
        values: outcome.values,
        partial: outcome.status.is_partial(),
        outcome: switch,
        ..Reply::DONE
    })
}

/// Writes the status word and the return words a woken thread finds.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the pool no longer holds the thread;
/// [`Error::InvalidArgument`] when its buffer is not reachable.
pub fn write_result<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    wakeup: Wakeup,
) -> Result<(), Error> {
    let frame = machine.objects.threads.get(wakeup.thread)?.ipc_buffer;
    machine.environment.with_buffer(frame, |bytes| {
        let mut writer = BufferMut::new(bytes);
        writer.clear_result();
        writer.set_status(wakeup.status);
        for (index, value) in wakeup.values.iter().enumerate() {
            writer.set_return_word(index, *value);
        }
    })
}
