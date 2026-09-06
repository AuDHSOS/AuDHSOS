// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Taking a thread out of whatever it waits on.
//!
//! Invariant: after this the thread is in no wait queue and no object names
//! it, so the scheduler operation that follows — suspend, kill, or the exit
//! of a whole process — cannot splice a queue it knows nothing about.
//!
//! The record the thread carries is what makes this cheap: `Wait` names the
//! object and the queue, so nothing has to search every endpoint (D-74).

use kernel_objects::object::{Links, ThreadId, Wait};
use kernel_objects::store::Objects;

/// Takes `thread` out of the queue it waits in and clears its record of
/// what it waited on. Returns `true` when the thread was waiting on
/// something.
///
/// The scheduler is not touched: this runs before the scheduler operation
/// that ends or suspends the thread, and a thread that is blocked is in no
/// run queue for the scheduler to correct.
pub fn cancel<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    thread: ThreadId,
) -> bool {
    let Some(wait) = objects.threads.get(thread).ok().map(|held| held.wait) else {
        return false;
    };
    match wait {
        Wait::Nothing => return false,
        Wait::Endpoint {
            endpoint, queue, ..
        } => {
            // The two pools are separate fields, so the queue reaches the
            // threads while the endpoint is borrowed.
            let Objects {
                endpoints, threads, ..
            } = objects;
            if let Ok(held) = endpoints.get_mut(endpoint) {
                if queue.is_sender() {
                    held.senders.unlink(threads, thread);
                } else {
                    held.receivers.unlink(threads, thread);
                }
            }
        }
        // The record is written exactly when the waiter is, so the thread
        // this names is this thread and there is nothing to compare.
        Wait::Notification { notification } => {
            objects
                .notifications
                .with(notification, |held| held.waiter = None);
        }
        // A thread waiting for the answer to a call is in no queue: the
        // reply object names it. The cleared record is what tells a later
        // `ipc_reply` that its caller is no longer waiting for one.
        Wait::Reply { .. } => {}
    }
    objects.threads.with(thread, |held| {
        held.wait = Wait::Nothing;
        held.wait_links = Links::UNLINKED;
    });
    true
}
