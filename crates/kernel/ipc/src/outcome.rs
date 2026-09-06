// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What an operation of this crate leaves for the system call layer.
//!
//! Invariants: a blocked caller has no status and no return words, because
//! it has no result yet — the thread that completes the rendezvous writes
//! them into its buffer later; every thread that becomes ready here carries
//! a status with it, so a thread that wakes always learns why.

use audhsos_abi::ipc_buffer::Status;
use audhsos_abi::{Error, ThreadState};
use kernel_objects::object::{Thread, ThreadId, Wait};
use kernel_objects::pool::Pool;
use kernel_objects::wait_queue::WaitQueue;
use kernel_sched::Scheduler;
use kernel_sched::transition::Event;

/// What a thread finds in its buffer when it wakes, and which thread that
/// is. The system call layer writes it: reaching the buffer of another
/// thread needs its frame, which this crate does not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Wakeup {
    /// The thread that became ready.
    pub thread: ThreadId,
    /// The status word it finds.
    pub status: Status,
    /// The return words it finds.
    pub values: [u64; 2],
}

impl Wakeup {
    /// A thread that wakes with `error` and no return words: a destroyed
    /// object, a dropped reply object, or a cancelled wait.
    #[must_use]
    pub const fn failed(thread: ThreadId, error: Error) -> Self {
        Wakeup {
            thread,
            status: Status::failed(error),
            values: [0, 0],
        }
    }

    /// A thread that wakes with a result.
    #[must_use]
    pub const fn ok(thread: ThreadId, values: [u64; 2]) -> Self {
        Wakeup {
            thread,
            status: Status::OK,
            values,
        }
    }
}

/// What an operation leaves for the system call layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The caller blocks, so the dispatcher writes it no result at all.
    pub blocked: bool,
    /// The status of the caller, when it does not block.
    pub status: Status,
    /// The return words of the caller, when it does not block.
    pub values: [u64; 2],
    /// One thread that became ready and what belongs in its buffer.
    pub wakeup: Option<Wakeup>,
    /// The caller should switch before it returns to user mode.
    pub reschedule: bool,
}

impl Outcome {
    /// An operation that answered the caller and woke nobody.
    pub const DONE: Outcome = Outcome {
        blocked: false,
        status: Status::OK,
        values: [0, 0],
        wakeup: None,
        reschedule: false,
    };

    /// An operation that answered the caller with two return words.
    #[must_use]
    pub const fn values(first: u64, second: u64) -> Self {
        Outcome {
            values: [first, second],
            ..Outcome::DONE
        }
    }

    /// The same outcome with `status` for the caller, which is
    /// [`Status::PARTIAL`] for a message whose handles did not all fit.
    #[must_use]
    pub const fn with_status(self, status: Status) -> Self {
        Outcome { status, ..self }
    }

    /// The same outcome carrying a thread that became ready.
    #[must_use]
    pub const fn waking(self, wakeup: Wakeup) -> Self {
        Outcome {
            wakeup: Some(wakeup),
            ..self
        }
    }

    /// The same outcome, asking the caller to switch.
    #[must_use]
    pub const fn switching(self, reschedule: bool) -> Self {
        Outcome { reschedule, ..self }
    }

    /// An operation whose caller blocks: no status, no return words.
    #[must_use]
    pub const fn blocked() -> Self {
        Outcome {
            blocked: true,
            reschedule: true,
            ..Outcome::DONE
        }
    }
}

/// The threads a destroyed object left waiting.
///
/// They come out one at a time, because there can be as many of them as the
/// machine has threads and each one needs its own IPC buffer written, which
/// only the system call layer can reach. [`Waiters::wake_next`] takes one
/// out, makes it ready, clears its record of what it waited on, and says
/// what belongs in its buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Waiters {
    first: WaitQueue,
    second: WaitQueue,
    single: Option<ThreadId>,
    error: Error,
    switch: bool,
}

impl Waiters {
    /// Nobody was waiting.
    #[must_use]
    pub const fn none() -> Self {
        Waiters {
            first: WaitQueue::EMPTY,
            second: WaitQueue::EMPTY,
            single: None,
            error: Error::ObjectDestroyed,
            switch: false,
        }
    }

    /// The threads of two queues, all of them waking with `error`.
    #[must_use]
    pub const fn queues(first: WaitQueue, second: WaitQueue, error: Error) -> Self {
        Waiters {
            first,
            second,
            single: None,
            error,
            switch: false,
        }
    }

    /// One thread, waking with `error`.
    #[must_use]
    pub const fn one(thread: Option<ThreadId>, error: Error) -> Self {
        Waiters {
            first: WaitQueue::EMPTY,
            second: WaitQueue::EMPTY,
            single: thread,
            error,
            switch: false,
        }
    }

    /// `true` when nobody is left to wake.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.single.is_none() && self.first.is_empty() && self.second.is_empty()
    }

    /// Wakes the next thread and says what belongs in its buffer.
    ///
    /// A thread the pool no longer holds, and one the transition table
    /// refuses to wake, is dropped rather than reported: the object it
    /// waited on is gone either way, and there is nobody left to tell.
    pub fn wake_next<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        scheduler: &mut Scheduler,
    ) -> Option<Wakeup> {
        while let Some(id) = self.take_next(threads) {
            threads.with(id, |thread| thread.wait = Wait::Nothing);
            if let Some(reschedule) = wake(threads, scheduler, id) {
                self.switch |= reschedule;
                return Some(Wakeup::failed(id, self.error));
            }
        }
        None
    }

    /// `true` when one of the threads that woke should take the processor
    /// from the one that holds it.
    #[must_use]
    pub const fn wants_switch(&self) -> bool {
        self.switch
    }

    /// The next thread to wake, out of the single waiter first and then the
    /// two queues.
    fn take_next<const N: usize>(&mut self, threads: &mut Pool<Thread, N>) -> Option<ThreadId> {
        if let Some(id) = self.single.take() {
            return Some(id);
        }
        self.first
            .dequeue_front(threads)
            .or_else(|| self.second.dequeue_front(threads))
    }
}

/// Makes `id` ready and says whether the thread that woke should take the
/// processor. `None` says that nothing was woken: a thread that is not
/// blocked cannot be, and the transition table is what decides that.
pub(crate) fn wake<const N: usize>(
    threads: &mut Pool<Thread, N>,
    scheduler: &mut Scheduler,
    id: ThreadId,
) -> Option<bool> {
    if !state_of(threads, id).is_some_and(ThreadState::is_blocked) {
        return None;
    }
    scheduler
        .on_wake(threads, id)
        .ok()
        .map(|outcome| outcome.reschedule)
}

/// Blocks `id` in the state `event` names and clears its time slice. The
/// caller has already recorded what it waits on.
pub(crate) fn block<const N: usize>(
    threads: &mut Pool<Thread, N>,
    scheduler: &mut Scheduler,
    id: ThreadId,
    event: Event,
) -> Result<bool, Error> {
    let outcome = scheduler.on_block(threads, id, event)?;
    Ok(outcome.reschedule)
}

/// The state `id` is in, or `None` when the pool does not hold it.
pub(crate) fn state_of<const N: usize>(
    threads: &Pool<Thread, N>,
    id: ThreadId,
) -> Option<ThreadState> {
    threads.get(id).ok().map(|thread| thread.state)
}
