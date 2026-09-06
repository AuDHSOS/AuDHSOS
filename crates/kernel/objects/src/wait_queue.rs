// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The queue a thread waits in on an endpoint or a notification.
//!
//! Invariants: a thread is in at most one wait queue, and it is in one
//! exactly while it is blocked; the order is priority and then first in
//! first out, so the head of a queue is always a thread of the highest
//! priority that waits in it and no thread of an equal priority arrived
//! before it; `len` is the length of the chain the head begins.
//!
//! The links are the `wait_links` of [`Thread`] and not its `queue_links`,
//! which belong to the run queues of `kernel-sched` (D-74). The two are
//! separate fields because a thread is in at most one of the two kinds of
//! queue at a time and because `Scheduler::dequeue` corrects a run queue
//! after it unlinks: handed a thread whose links pointed into an endpoint
//! it would splice that queue.
//!
//! The queue lives here, beside the pool the links thread through, because
//! its operations need that pool; the rendezvous that uses it is
//! `kernel-ipc`.

use audhsos_abi::Error;

use crate::object::{Links, Thread, ThreadId};
use crate::pool::Pool;

/// The threads waiting on one side of an object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WaitQueue {
    head: Option<ThreadId>,
    tail: Option<ThreadId>,
    len: u32,
}

impl WaitQueue {
    /// A queue nobody waits in. All zeros, so an object holding one reaches
    /// the `.bss` with the rest of its pool (D-66).
    pub const EMPTY: WaitQueue = WaitQueue {
        head: None,
        tail: None,
        len: 0,
    };

    /// The thread at the front, which is the one a rendezvous serves.
    #[must_use]
    pub const fn head(&self) -> Option<ThreadId> {
        self.head
    }

    /// The thread at the back.
    #[must_use]
    pub const fn tail(&self) -> Option<ThreadId> {
        self.tail
    }

    /// How many threads wait in the queue.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.len
    }

    /// `true` if nobody waits in the queue.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    /// Puts `id` in the queue, behind the last thread whose priority is at
    /// least its own.
    ///
    /// The walk is bounded by the length of the queue, and no second
    /// structure carries the order: the position is fixed at this moment,
    /// so a later `thread_set_priority` does not move a thread that already
    /// waits.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold `id` or a
    /// thread the queue names; [`Error::InvalidState`] when `id` is already
    /// in a queue.
    pub fn enqueue<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<(), Error> {
        let priority = threads.get(id).map_err(|_| Error::InvalidHandle)?.priority;
        if !links_of(threads, id)?.is_unlinked() || self.head == Some(id) {
            return Err(Error::InvalidState);
        }
        let before = self.first_below(threads, priority);
        match before {
            Some(next) => self.link_before(threads, id, next)?,
            None => self.append(threads, id)?,
        }
        self.len = self.len.saturating_add(1);
        Ok(())
    }

    /// Takes the thread at the front out of the queue.
    pub fn dequeue_front<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
    ) -> Option<ThreadId> {
        let head = self.head?;
        if self.unlink(threads, head) {
            Some(head)
        } else {
            None
        }
    }

    /// Takes `id` out of the queue and says whether it was in it.
    pub fn unlink<const N: usize>(&mut self, threads: &mut Pool<Thread, N>, id: ThreadId) -> bool {
        let Ok(links) = threads.get(id).map(|thread| thread.wait_links) else {
            return false;
        };
        if links.is_unlinked() && self.head != Some(id) {
            return false;
        }
        if let Some(previous) = links.previous
            && let Ok(before) = threads.get_mut(previous)
        {
            before.wait_links.next = links.next;
        }
        if let Some(next) = links.next
            && let Ok(after) = threads.get_mut(next)
        {
            after.wait_links.previous = links.previous;
        }
        if self.head == Some(id) {
            self.head = links.next;
        }
        if self.tail == Some(id) {
            self.tail = links.previous;
        }
        if let Ok(thread) = threads.get_mut(id) {
            thread.wait_links = Links::UNLINKED;
        }
        self.len = self.len.saturating_sub(1);
        true
    }

    /// The first thread of the queue whose priority is below `priority`,
    /// which is where a thread of that priority goes. The walk is the
    /// iterator, so it is bounded by the length the queue records.
    fn first_below<const N: usize>(
        &self,
        threads: &Pool<Thread, N>,
        priority: u8,
    ) -> Option<ThreadId> {
        self.iter(threads).find(|id| {
            threads
                .get(*id)
                .is_ok_and(|thread| thread.priority < priority)
        })
    }

    /// Puts `id` at the back of the queue.
    fn append<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<(), Error> {
        let tail = self.tail;
        let thread = threads.get_mut(id).map_err(|_| Error::InvalidHandle)?;
        thread.wait_links = Links {
            next: None,
            previous: tail,
        };
        match tail {
            Some(previous) => {
                let before = threads
                    .get_mut(previous)
                    .map_err(|_| Error::InvalidHandle)?;
                before.wait_links.next = Some(id);
            }
            None => self.head = Some(id),
        }
        self.tail = Some(id);
        Ok(())
    }

    /// Puts `id` immediately before `next`, which is in the queue.
    fn link_before<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        next: ThreadId,
    ) -> Result<(), Error> {
        let previous = threads
            .get(next)
            .map_err(|_| Error::InvalidHandle)?
            .wait_links
            .previous;
        let thread = threads.get_mut(id).map_err(|_| Error::InvalidHandle)?;
        thread.wait_links = Links {
            next: Some(next),
            previous,
        };
        threads
            .get_mut(next)
            .map_err(|_| Error::InvalidHandle)?
            .wait_links
            .previous = Some(id);
        match previous {
            Some(before) => {
                threads
                    .get_mut(before)
                    .map_err(|_| Error::InvalidHandle)?
                    .wait_links
                    .next = Some(id);
            }
            None => self.head = Some(id),
        }
        Ok(())
    }

    /// The threads of the queue, from the front.
    #[must_use]
    pub const fn iter<'a, const N: usize>(&self, threads: &'a Pool<Thread, N>) -> Waiting<'a, N> {
        Waiting {
            threads,
            next: self.head,
            steps: self.len,
        }
    }
}

/// The links of `id`, for the checks before an insertion.
fn links_of<const N: usize>(threads: &Pool<Thread, N>, id: ThreadId) -> Result<Links, Error> {
    threads
        .get(id)
        .map(|thread| thread.wait_links)
        .map_err(|_| Error::InvalidHandle)
}

/// The threads of one wait queue, as [`WaitQueue::iter`] walks them.
#[derive(Debug)]
pub struct Waiting<'a, const N: usize> {
    threads: &'a Pool<Thread, N>,
    next: Option<ThreadId>,
    steps: u32,
}

impl<const N: usize> Iterator for Waiting<'_, N> {
    type Item = ThreadId;

    fn next(&mut self) -> Option<ThreadId> {
        if self.steps == 0 {
            return None;
        }
        let id = self.next?;
        self.steps = self.steps.saturating_sub(1);
        self.next = self.threads.get(id).ok().and_then(|t| t.wait_links.next);
        Some(id)
    }
}
