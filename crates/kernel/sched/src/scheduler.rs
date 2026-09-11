// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Which thread runs next.
//!
//! Invariants: a thread is in at most one run queue, and it is in one
//! exactly when its state is [`ThreadState::Ready`]; the bitmap names a
//! priority exactly when that priority's queue holds someone; the running
//! thread is in no queue; the idle thread is never in a queue and is picked
//! only when every queue is empty.
//!
//! The links are the `queue_links` of a thread; the `wait_links` beside
//! them belong to the wait queues of `kernel-ipc`, and a thread is in at
//! most one of the two kinds of queue at a time (D-74). They are separate
//! fields because [`Scheduler::dequeue`] corrects the head, the tail, and
//! the ready bitmap of a run queue after it unlinks: handed a thread whose
//! links pointed into an endpoint queue it would splice that queue.
//!
//! The first of those is why every operation that takes a thread off the
//! processor — suspend, exit, fault, block — goes through one place that
//! asks the transition table before it touches a queue. An operation the
//! table refuses changes nothing at all.

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
    ended: u32,
    deadlines: Queue,
    waiting: u32,
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
            ended: 0,
            deadlines: Queue::EMPTY,
            waiting: 0,
        }
    }

    /// How many threads have ended and have not been cleared away.
    ///
    /// This is what lets the kernel's sweep answer without looking: it
    /// runs after every system call and after every switch, and on a
    /// machine where nothing ended it has nothing to find. The count is
    /// raised here, where a thread enters [`ThreadState::Exited`], and
    /// lowered by [`Scheduler::cleared`]; those two are its only writers.
    #[must_use]
    pub const fn ended(&self) -> u32 {
        self.ended
    }

    /// Says that one ended thread has been cleared away.
    pub const fn cleared(&mut self) {
        self.ended = self.ended.saturating_sub(1);
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

    /// Says that the processor is already running `thread`, which is what
    /// the kernel tells the scheduler about itself during the bring-up:
    /// the code that will switch away is the idle thread, and the first
    /// switch needs somewhere to write its context.
    pub const fn adopt(&mut self, thread: ThreadId) {
        self.current = Some(thread);
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
        if !thread.queue_links.is_unlinked() {
            return Err(Error::InvalidState);
        }
        let tail = self.queue(priority).and_then(|queue| queue.tail);
        if self
            .queue(priority)
            .is_some_and(|queue| queue.head == Some(id))
        {
            return Err(Error::InvalidState);
        }
        thread.queue_links = Links {
            next: None,
            previous: tail,
        };
        match tail {
            Some(previous) => {
                let before = threads
                    .get_mut(previous)
                    .map_err(|_| Error::InvalidHandle)?;
                before.queue_links.next = Some(id);
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
        let links = thread.queue_links;
        let head = self.queue(priority).and_then(|queue| queue.head);
        if links.is_unlinked() && head != Some(id) {
            return Ok(());
        }
        if let Some(previous) = links.previous {
            let before = threads
                .get_mut(previous)
                .map_err(|_| Error::InvalidHandle)?;
            before.queue_links.next = links.next;
        }
        if let Some(next) = links.next {
            let after = threads.get_mut(next).map_err(|_| Error::InvalidHandle)?;
            after.queue_links.previous = links.previous;
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
        thread.queue_links = Links::UNLINKED;
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
            let outgoing = self.apply(threads, current, Event::Preempt);
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

    /// How many threads wait with a deadline.
    #[must_use]
    pub const fn waiting_until(&self) -> u32 {
        self.waiting
    }

    /// The earliest deadline anyone waits for.
    #[must_use]
    pub const fn next_deadline(&self) -> Option<ThreadId> {
        self.deadlines.head
    }

    /// Blocks `id` on `event` until `deadline`, which is microseconds since
    /// boot. The thread enters the deadline list at its place, and
    /// [`Scheduler::expired`] hands it back once the tick count passes it.
    ///
    /// # Errors
    ///
    /// As [`Scheduler::on_block`]. A refused operation leaves the thread in
    /// no deadline list.
    pub fn on_block_until<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        event: Event,
        deadline: u64,
    ) -> Result<Outcome, Error> {
        let outcome = self.on_block(threads, id, event)?;
        self.insert_deadline(threads, id, deadline)?;
        Ok(outcome)
    }

    /// The first thread whose deadline is at or before `now`, taken out of
    /// the list. `None` stops the walk: the list is ordered, so the first
    /// entry that has not passed is the end of what this tick wakes.
    ///
    /// The thread is out of the list and its deadline is cleared; making it
    /// ready is the caller's, which is what lets the layer above write into
    /// its buffer what it wakes with.
    pub fn expired<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        now: u64,
    ) -> Option<ThreadId> {
        loop {
            let head = self.deadlines.head?;
            let Ok(thread) = threads.get_mut(head) else {
                // The pool no longer holds the head, so nothing can follow
                // its links to what stands behind it. The list is let go of
                // whole rather than left with a head that answers `None`
                // for ever: `None` is how the caller learns that nothing is
                // due, so an entry that cannot be read would stop every
                // deadline behind it and every one made after it. The
                // threads keep the signals they wait for; only the
                // deadlines are lost.
                self.deadlines = Queue::EMPTY;
                self.waiting = 0;
                return None;
            };
            let Some(deadline) = thread.deadline else {
                // An entry that has lost the deadline that put it here
                // cannot be woken by one, and `remove_deadline` reads that
                // deadline to decide whether the thread is in a list at
                // all, so it would leave the entry where it is. It comes
                // out here, and the walk goes on to the next.
                let next = thread.deadline_links.next;
                thread.deadline_links = Links::UNLINKED;
                self.detach_head(threads, next);
                continue;
            };
            if deadline > now {
                return None;
            }
            self.remove_deadline(threads, head);
            return Some(head);
        }
    }

    /// Points the list past a head that has just been taken out of it.
    fn detach_head<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        next: Option<ThreadId>,
    ) {
        self.deadlines.head = next;
        match next {
            Some(next) => {
                if let Ok(entry) = threads.get_mut(next) {
                    entry.deadline_links.previous = None;
                }
            }
            None => self.deadlines.tail = None,
        }
        self.waiting = self.waiting.saturating_sub(1);
    }

    /// Puts `id` into the deadline list at its place: behind everyone whose
    /// deadline is at or before its own, so that two threads that named the
    /// same microsecond come out in the order they went in.
    ///
    /// The walk starts at the tail, which is where a deadline later than
    /// every other belongs and which is the common case; only a deadline
    /// that falls inside the list costs a walk, and the list is bounded by
    /// the number of threads.
    fn insert_deadline<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        deadline: u64,
    ) -> Result<(), Error> {
        // The place comes first and the thread is marked second. Writing
        // the deadline before the splice was known would leave, on a
        // refused insert, a thread the list does not hold but that says it
        // is in one — and the next removal of it reads its empty links as
        // both ends of the list and clears the head and the tail, every
        // other waiter with them.
        let after = self.place_for(threads, deadline);
        let before = match after {
            Some(after) => {
                threads
                    .get(after)
                    .map_err(|_| Error::InvalidHandle)?
                    .deadline_links
                    .next
            }
            None => self.deadlines.head,
        };
        let thread = threads.get_mut(id).map_err(|_| Error::InvalidHandle)?;
        thread.deadline = Some(deadline);
        thread.deadline_links = Links {
            next: before,
            previous: after,
        };
        match after {
            Some(after) => {
                threads
                    .get_mut(after)
                    .map_err(|_| Error::InvalidHandle)?
                    .deadline_links
                    .next = Some(id);
            }
            None => self.deadlines.head = Some(id),
        }
        match before {
            Some(before) => {
                threads
                    .get_mut(before)
                    .map_err(|_| Error::InvalidHandle)?
                    .deadline_links
                    .previous = Some(id);
            }
            None => self.deadlines.tail = Some(id),
        }
        self.waiting = self.waiting.saturating_add(1);
        Ok(())
    }

    /// The last thread whose deadline is at or before `deadline`, or `None`
    /// when the new entry belongs at the front.
    fn place_for<const N: usize>(
        &self,
        threads: &Pool<Thread, N>,
        deadline: u64,
    ) -> Option<ThreadId> {
        let mut at = self.deadlines.tail;
        // The walk is bounded by the length the list says it has, so links
        // that were torn cannot make it run forever.
        for _ in 0..self.waiting {
            let id = at?;
            let thread = threads.get(id).ok()?;
            if thread.deadline.is_some_and(|held| held <= deadline) {
                return Some(id);
            }
            at = thread.deadline_links.previous;
        }
        None
    }

    /// Takes `id` out of the deadline list and clears its deadline. A
    /// thread that waits without one is left alone.
    fn remove_deadline<const N: usize>(&mut self, threads: &mut Pool<Thread, N>, id: ThreadId) {
        let Ok(thread) = threads.get_mut(id) else {
            return;
        };
        if thread.deadline.take().is_none() {
            return;
        }
        let links = thread.deadline_links;
        thread.deadline_links = Links::UNLINKED;
        match links.previous {
            Some(previous) => {
                if let Ok(before) = threads.get_mut(previous) {
                    before.deadline_links.next = links.next;
                }
            }
            None => self.deadlines.head = links.next,
        }
        match links.next {
            Some(next) => {
                if let Ok(after) = threads.get_mut(next) {
                    after.deadline_links.previous = links.previous;
                }
            }
            None => self.deadlines.tail = links.previous,
        }
        self.waiting = self.waiting.saturating_sub(1);
    }

    /// Applies `event` to the state of `id` without touching the queues.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidState`] when the transition table does not allow the
    /// event in the thread's state.
    fn apply<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        event: Event,
    ) -> Result<ThreadState, Error> {
        let thread = threads.get_mut(id).map_err(|_| Error::InvalidHandle)?;
        let state = next(thread.state, event)?;
        let was = thread.state;
        thread.state = state;
        if state == ThreadState::Exited && was != ThreadState::Exited {
            self.ended = self.ended.saturating_add(1);
        }
        if state != ThreadState::BlockedNotification {
            // A thread signalled, suspended, or killed before its deadline
            // leaves the list in the call that changes its state, so an
            // entry never outlives the wait it belongs to.
            self.remove_deadline(threads, id);
        }
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
        self.apply(threads, id, Event::Start)?;
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
        self.apply(threads, id, Event::Wake)?;
        self.enqueue(threads, id)?;
        Ok(self.preempts_current(threads, id))
    }

    /// Takes `id` off the processor on `event`: the table first, the queue
    /// afterwards.
    ///
    /// That order is the whole of it. A thread is in a run queue exactly
    /// while its state is [`ThreadState::Ready`], so a thread that leaves
    /// its queue must be one whose state is about to stop being `Ready` —
    /// and only the table knows whether it may. Taking it out first and
    /// asking afterwards leaves a thread that the table refused ready and
    /// in no queue, which is a thread [`Scheduler::pick_next`] can never
    /// find again and nobody is told about: the caller has an error in its
    /// hand and every reason to believe that nothing happened.
    ///
    /// The other way round nothing can be left half done. `dequeue` fails
    /// only for a thread the pool does not hold, and [`Scheduler::apply`]
    /// has just held it; the state it wrote does not reach anything
    /// `dequeue` reads.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidState`] when the transition table does not allow the
    /// event in the thread's state. The thread is untouched either way.
    fn leaves_the_processor<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        event: Event,
    ) -> Result<Outcome, Error> {
        self.apply(threads, id, event)?;
        self.dequeue(threads, id)?;
        Self::spend_slice(threads, id);
        Ok(self.left_the_processor(id))
    }

    /// Blocks the running thread on what `event` names.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] when `event` blocks nothing;
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidState`] when the transition table does not allow the
    /// event in the thread's state. A refused operation changes nothing:
    /// not the state, not a queue, not the time slice.
    pub fn on_block<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        event: Event,
    ) -> Result<Outcome, Error> {
        if event.blocked_state().is_none() {
            return Err(Error::InvalidArgument);
        }
        self.leaves_the_processor(threads, id, event)
    }

    /// Suspends `id`, wherever it was.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidState`] when the transition table does not allow the
    /// event in the thread's state. A refused operation changes nothing:
    /// not the state, not a queue, not the time slice.
    pub fn suspend<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        self.leaves_the_processor(threads, id, Event::Suspend)
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
        self.apply(threads, id, Event::Resume)?;
        self.enqueue(threads, id)?;
        Ok(self.preempts_current(threads, id))
    }

    /// Stops `id` on a fault it has no handler for. Only the thread on the
    /// processor can fault, so a thread in any other state is refused and
    /// stays exactly where it was, its queue included.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidState`] when the transition table does not allow the
    /// event in the thread's state. A refused operation changes nothing:
    /// not the state, not a queue, not the time slice.
    pub fn fault<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        self.leaves_the_processor(threads, id, Event::Fault)
    }

    /// Ends `id`, wherever it was. A thread that has already exited is
    /// left alone, so that killing a dead thread is not an error.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the pool does not hold the thread;
    /// [`Error::InvalidState`] when the transition table does not allow the
    /// event in the thread's state. A refused operation changes nothing:
    /// not the state, not a queue, not the time slice.
    pub fn exit<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        let state = threads.get(id).map_err(|_| Error::InvalidHandle)?.state;
        if state == ThreadState::Exited {
            return Ok(Outcome::NOTHING);
        }
        self.leaves_the_processor(threads, id, Event::Exit)
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
        self.apply(threads, id, Event::Yield)?;
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
