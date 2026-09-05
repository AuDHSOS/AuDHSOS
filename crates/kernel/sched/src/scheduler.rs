// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Which thread runs next.
//!
//! Invariants: a thread is in at most one run queue, and it is in one
//! exactly when its state is [`ThreadState::Ready`]; the bitmap names a
//! priority exactly when that priority's queue holds someone; the running
//! thread is in no queue; the idle thread is never in a queue and is picked
//! only when every queue is empty.

use audhsos_abi::layout::{DEFAULT_TIME_SLICE_TICKS, PRIORITY_COUNT};
use audhsos_abi::{Error, ThreadState};
use kernel_objects::object::{Links, Thread, ThreadId};
use kernel_objects::pool::Pool;

use crate::transition::{Event, next};

/// Number of priorities, as a `usize` for indexing the queues.
#[expect(
    clippy::as_conversions,
    reason = "widening the priority count of the layout to an index width, in a constant"
)]
pub const PRIORITIES: usize = PRIORITY_COUNT as usize;

const _: () = assert!(PRIORITIES == 32, "the ready bitmap is a u32");

/// One run queue: the ends of a list threaded through the thread entries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Queue {
    head: Option<ThreadId>,
    tail: Option<ThreadId>,
}

impl Queue {
    /// An empty queue.
    const EMPTY: Queue = Queue {
        head: None,
        tail: None,
    };

    const fn is_empty(self) -> bool {
        self.head.is_none()
    }
}

/// What a call to the scheduler asks the caller to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Outcome {
    /// The caller should switch threads before it returns to user mode.
    pub reschedule: bool,
}

impl Outcome {
    /// Nothing to do.
    pub const NOTHING: Outcome = Outcome { reschedule: false };

    /// Switch before returning.
    pub const RESCHEDULE: Outcome = Outcome { reschedule: true };
}

/// The run queues of one processor.
#[derive(Debug)]
pub struct Scheduler {
    queues: [Queue; PRIORITIES],
    ready: u32,
    current: Option<ThreadId>,
    idle: Option<ThreadId>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    /// A scheduler with empty queues and no idle thread yet. The value is
    /// all zeros and the constructor is `const`, as everything the kernel
    /// keeps in a `static` has to be (D-66).
    #[must_use]
    pub const fn new() -> Self {
        Scheduler {
            queues: [Queue::EMPTY; PRIORITIES],
            ready: 0,
            current: None,
            idle: None,
        }
    }

    /// Names the thread that runs when nothing else can. It is never
    /// queued and never blocks.
    pub const fn set_idle(&mut self, idle: ThreadId) {
        self.idle = Some(idle);
    }

    /// The idle thread, if one was named.
    #[must_use]
    pub const fn idle(&self) -> Option<ThreadId> {
        self.idle
    }

    /// The thread on the processor.
    #[must_use]
    pub const fn current(&self) -> Option<ThreadId> {
        self.current
    }

    /// The priorities that hold a ready thread, as a bit per priority.
    #[must_use]
    pub const fn ready_bitmap(&self) -> u32 {
        self.ready
    }

    /// `true` if no queue holds anyone.
    #[must_use]
    pub const fn is_idle(&self) -> bool {
        self.ready == 0
    }

    /// The highest priority that holds a ready thread.
    #[must_use]
    pub const fn highest_ready(&self) -> Option<u8> {
        if self.ready == 0 {
            return None;
        }
        let leading = self.ready.leading_zeros();
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            reason = "31 minus a count of at most 31 fits a byte, and the function is const"
        )]
        let priority = (31_u32.wrapping_sub(leading)) as u8;
        Some(priority)
    }

    fn queue(&self, priority: u8) -> Option<&Queue> {
        self.queues.get(usize::from(priority))
    }

    fn queue_mut(&mut self, priority: u8) -> Option<&mut Queue> {
        self.queues.get_mut(usize::from(priority))
    }

    fn mark_ready(&mut self, priority: u8, ready: bool) {
        if usize::from(priority) >= PRIORITIES {
            return;
        }
        let bit = 1_u32.wrapping_shl(u32::from(priority));
        if ready {
            self.ready |= bit;
        } else {
            self.ready &= !bit;
        }
    }

    /// Puts `id` at the end of the queue of its priority. A thread that is
    /// already queued, the idle thread, and a priority outside the table
    /// are refused.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidArgument`] for the idle thread or a priority the
    /// table has no queue for; [`Error::InvalidState`] when the thread is
    /// already in a queue.
    pub fn enqueue<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<(), Error> {
        if self.idle == Some(id) {
            return Err(Error::InvalidArgument);
        }
        let thread = threads.get_mut(id).map_err(|_| Error::InvalidHandle)?;
        let priority = thread.priority;
        if usize::from(priority) >= PRIORITIES {
            return Err(Error::InvalidArgument);
        }
        if !thread.links.is_unlinked() {
            return Err(Error::InvalidState);
        }
        let tail = self.queue(priority).and_then(|queue| queue.tail);
        if self
            .queue(priority)
            .is_some_and(|queue| queue.head == Some(id))
        {
            return Err(Error::InvalidState);
        }
        thread.links = Links {
            next: None,
            previous: tail,
        };
        match tail {
            Some(previous) => {
                let before = threads
                    .get_mut(previous)
                    .map_err(|_| Error::InvalidHandle)?;
                before.links.next = Some(id);
            }
            None => {
                if let Some(queue) = self.queue_mut(priority) {
                    queue.head = Some(id);
                }
            }
        }
        if let Some(queue) = self.queue_mut(priority) {
            queue.tail = Some(id);
        }
        self.mark_ready(priority, true);
        Ok(())
    }

    /// Takes `id` out of the queue it is in. A thread that is in none is
    /// left alone.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold the thread.
    pub fn dequeue<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<(), Error> {
        let thread = threads.get(id).map_err(|_| Error::InvalidHandle)?;
        let priority = thread.priority;
        let links = thread.links;
        let head = self.queue(priority).and_then(|queue| queue.head);
        if links.is_unlinked() && head != Some(id) {
            return Ok(());
        }
        if let Some(previous) = links.previous {
            let before = threads
                .get_mut(previous)
                .map_err(|_| Error::InvalidHandle)?;
            before.links.next = links.next;
        }
        if let Some(next) = links.next {
            let after = threads.get_mut(next).map_err(|_| Error::InvalidHandle)?;
            after.links.previous = links.previous;
        }
        if let Some(queue) = self.queue_mut(priority) {
            if queue.head == Some(id) {
                queue.head = links.next;
            }
            if queue.tail == Some(id) {
                queue.tail = links.previous;
            }
            let empty = queue.is_empty();
            self.mark_ready(priority, !empty);
        }
        let thread = threads.get_mut(id).map_err(|_| Error::InvalidHandle)?;
        thread.links = Links::UNLINKED;
        Ok(())
    }

    /// The thread that should run now: the first thread of the highest
    /// priority that holds one, or the idle thread when every queue is
    /// empty. The picked thread leaves its queue, its state becomes
    /// [`ThreadState::Running`], and it receives a fresh time slice; the
    /// thread that was running goes back into a queue when it is still
    /// ready.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold a thread the
    /// queues name; [`Error::NotRunnable`] when no queue holds anyone and
    /// no idle thread was named.
    pub fn pick_next<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
    ) -> Result<ThreadId, Error> {
        if let Some(current) = self.current
            && let Ok(thread) = threads.get(current)
            && thread.state == ThreadState::Running
            && Some(current) != self.idle
        {
            let outgoing = Self::apply(threads, current, Event::Preempt);
            if outgoing.is_ok() {
                self.enqueue(threads, current)?;
            }
        }
        let Some(priority) = self.highest_ready() else {
            let idle = self.idle.ok_or(Error::NotRunnable)?;
            self.current = Some(idle);
            return Ok(idle);
        };
        let id = self
            .queue(priority)
            .and_then(|queue| queue.head)
            .ok_or(Error::InvalidState)?;
        self.dequeue(threads, id)?;
        let thread = threads.get_mut(id).map_err(|_| Error::InvalidHandle)?;
        thread.state = next(thread.state, Event::Schedule)?;
        thread.time_slice = DEFAULT_TIME_SLICE_TICKS;
        self.current = Some(id);
        Ok(id)
    }

    /// Applies `event` to the state of `id` without touching the queues.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidState`] when the transition table does not allow the
    /// event in the thread's state.
    fn apply<const N: usize>(
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        event: Event,
    ) -> Result<ThreadState, Error> {
        let thread = threads.get_mut(id).map_err(|_| Error::InvalidHandle)?;
        let state = next(thread.state, event)?;
        thread.state = state;
        Ok(state)
    }

    /// Starts `id`: it becomes ready and enters the queue of its priority.
    ///
    /// # Errors
    ///
    /// As the transition table and [`Scheduler::enqueue`].
    pub fn start<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        Self::apply(threads, id, Event::Start)?;
        self.enqueue(threads, id)?;
        Ok(self.preempts_current(threads, id))
    }

    /// Wakes `id` from a blocked state. Waking a thread that is already
    /// ready changes nothing and is no error, so that two sources of the
    /// same wake-up do not fight.
    ///
    /// # Errors
    ///
    /// As the transition table and [`Scheduler::enqueue`].
    pub fn on_wake<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        let state = threads.get(id).map_err(|_| Error::InvalidHandle)?.state;
        if state == ThreadState::Ready {
            return Ok(Outcome::NOTHING);
        }
        Self::apply(threads, id, Event::Wake)?;
        self.enqueue(threads, id)?;
        Ok(self.preempts_current(threads, id))
    }

    /// Blocks the running thread on what `event` names.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] when `event` blocks nothing;
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidState`] when the thread is not running.
    pub fn on_block<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        event: Event,
    ) -> Result<Outcome, Error> {
        if event.blocked_state().is_none() {
            return Err(Error::InvalidArgument);
        }
        self.dequeue(threads, id)?;
        Self::apply(threads, id, event)?;
        Self::spend_slice(threads, id);
        Ok(self.left_the_processor(id))
    }

    /// Suspends `id`, wherever it was.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidState`] when the transition table does not allow the
    /// event in the thread's state.
    pub fn suspend<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        self.dequeue(threads, id)?;
        Self::apply(threads, id, Event::Suspend)?;
        Self::spend_slice(threads, id);
        Ok(self.left_the_processor(id))
    }

    /// Resumes a suspended or faulted thread.
    ///
    /// # Errors
    ///
    /// As the transition table and [`Scheduler::enqueue`].
    pub fn resume<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        Self::apply(threads, id, Event::Resume)?;
        self.enqueue(threads, id)?;
        Ok(self.preempts_current(threads, id))
    }

    /// Stops `id` on a fault it has no handler for.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidState`] when the transition table does not allow the
    /// event in the thread's state.
    pub fn fault<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        self.dequeue(threads, id)?;
        Self::apply(threads, id, Event::Fault)?;
        Self::spend_slice(threads, id);
        Ok(self.left_the_processor(id))
    }

    /// Ends `id`, wherever it was. A thread that has already exited is
    /// left alone, so that killing a dead thread is not an error.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidState`] when the transition table does not allow the
    /// event in the thread's state.
    pub fn exit<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        let state = threads.get(id).map_err(|_| Error::InvalidHandle)?.state;
        if state == ThreadState::Exited {
            return Ok(Outcome::NOTHING);
        }
        self.dequeue(threads, id)?;
        Self::apply(threads, id, Event::Exit)?;
        Self::spend_slice(threads, id);
        Ok(self.left_the_processor(id))
    }

    /// The running thread gives up the rest of its time slice; it goes to
    /// the end of the queue of its priority.
    ///
    /// # Errors
    ///
    /// As the transition table and [`Scheduler::enqueue`].
    pub fn yield_now<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        Self::apply(threads, id, Event::Yield)?;
        let thread = threads.get_mut(id).map_err(|_| Error::InvalidHandle)?;
        thread.time_slice = 0;
        self.enqueue(threads, id)?;
        Ok(Outcome::RESCHEDULE)
    }

    /// Changes the priority of `id` within what its creator allowed. A
    /// ready thread moves to the queue of its new priority.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidArgument`] when the priority is above the thread's
    /// maximum or outside the priorities of this system.
    pub fn set_priority<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        priority: u8,
    ) -> Result<Outcome, Error> {
        let thread = threads.get(id).map_err(|_| Error::InvalidHandle)?;
        if usize::from(priority) >= PRIORITIES || priority > thread.max_priority {
            return Err(Error::InvalidArgument);
        }
        let queued = thread.state == ThreadState::Ready;
        if queued {
            self.dequeue(threads, id)?;
        }
        let thread = threads.get_mut(id).map_err(|_| Error::InvalidHandle)?;
        thread.priority = priority;
        if queued {
            self.enqueue(threads, id)?;
            return Ok(self.preempts_current(threads, id));
        }
        if self.current == Some(id) {
            // The running thread lowered itself below someone who waits.
            return Ok(self.higher_waits(priority));
        }
        Ok(Outcome::NOTHING)
    }

    /// Counts one tick against the time slice of the running thread and
    /// asks for a switch when it runs out. The idle thread has no slice.
    pub fn tick<const N: usize>(&mut self, threads: &mut Pool<Thread, N>) -> Outcome {
        let Some(current) = self.current else {
            return self.wake_from_idle();
        };
        if Some(current) == self.idle {
            return self.wake_from_idle();
        }
        let Ok(thread) = threads.get_mut(current) else {
            return Outcome::RESCHEDULE;
        };
        thread.time_slice = thread.time_slice.saturating_sub(1);
        if thread.time_slice == 0 {
            return Outcome::RESCHEDULE;
        }
        Outcome::NOTHING
    }

    /// Ends the time slice of `id`: a thread that leaves the processor
    /// before its slice ran out keeps no credit for its next run.
    fn spend_slice<const N: usize>(threads: &mut Pool<Thread, N>, id: ThreadId) {
        if let Ok(thread) = threads.get_mut(id) {
            thread.time_slice = 0;
        }
    }

    /// The idle thread yields to anyone who became ready.
    const fn wake_from_idle(&self) -> Outcome {
        if self.ready == 0 {
            Outcome::NOTHING
        } else {
            Outcome::RESCHEDULE
        }
    }

    /// Whether `id`, now ready, should take the processor from the thread
    /// that holds it.
    fn preempts_current<const N: usize>(&self, threads: &Pool<Thread, N>, id: ThreadId) -> Outcome {
        let Ok(thread) = threads.get(id) else {
            return Outcome::NOTHING;
        };
        let Some(current) = self.current else {
            return Outcome::RESCHEDULE;
        };
        if Some(current) == self.idle {
            return Outcome::RESCHEDULE;
        }
        let Ok(running) = threads.get(current) else {
            return Outcome::RESCHEDULE;
        };
        if thread.priority > running.priority {
            Outcome::RESCHEDULE
        } else {
            Outcome::NOTHING
        }
    }

    /// Whether anyone above `priority` waits for the processor.
    const fn higher_waits(&self, priority: u8) -> Outcome {
        match self.highest_ready() {
            Some(highest) if highest > priority => Outcome::RESCHEDULE,
            _ => Outcome::NOTHING,
        }
    }

    /// The thread that just left the processor asks for a switch only when
    /// it was the one running.
    fn left_the_processor(&mut self, id: ThreadId) -> Outcome {
        // The caller has already set the state; the slice is spent either
        // way, so nothing is carried over into the next run.

        if self.current == Some(id) {
            self.current = None;
            Outcome::RESCHEDULE
        } else {
            Outcome::NOTHING
        }
    }
}
