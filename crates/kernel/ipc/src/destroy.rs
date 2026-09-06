// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a destroyed object owes the threads that waited on it.
//!
//! Invariant: every thread that waited on an object which is gone is woken
//! with a status that says so, and none is left blocked on a slot that has
//! been handed out again. A thread blocked on an object holds no reference to
//! it (D-75), so closing the last handle to an endpoint destroys it while
//! threads still wait on it, and this is the path that reaches them.

use audhsos_abi::Error;
use kernel_objects::object::{Endpoint, Notification, Reply};
use kernel_objects::store::Destroyed;

use crate::outcome::Waiters;

/// Everyone waiting on an endpoint that is gone, waking with
/// [`Error::ObjectDestroyed`].
#[must_use]
pub const fn destroy_endpoint(endpoint: &Endpoint) -> Waiters {
    Waiters::queues(endpoint.senders, endpoint.receivers, Error::ObjectDestroyed)
}

/// The waiter of a notification that is gone.
#[must_use]
pub const fn destroy_notification(notification: &Notification) -> Waiters {
    Waiters::one(notification.waiter, Error::ObjectDestroyed)
}

/// The caller of a reply object that was dropped without an answer. One
/// that was answered has already been woken, and there is nobody left.
#[must_use]
pub const fn destroy_reply(reply: &Reply) -> Waiters {
    let caller = if reply.consumed {
        None
    } else {
        Some(reply.caller)
    };
    Waiters::one(caller, Error::ReplyDropped)
}

/// Everyone the destruction of one object left waiting, whatever it was.
#[must_use]
pub const fn destroyed(what: &Destroyed) -> Waiters {
    match what {
        Destroyed::Quietly(_) => Waiters::none(),
        Destroyed::Endpoint(_, endpoint) => destroy_endpoint(endpoint),
        Destroyed::Notification(_, notification) => destroy_notification(notification),
        Destroyed::Reply(_, reply) => destroy_reply(reply),
    }
}
