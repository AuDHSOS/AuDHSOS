// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Fixed home queues, serialized by the machine cell.
use crate::{Event, Outcome, Scheduler};
use audhsos_abi::Error;
use kernel_objects::config::CPUS;
use kernel_objects::{Pool, Thread, ThreadId};

/// The schedulers of all processors; routing costs O(1).
#[derive(Debug)]
pub struct Processors {
    queues: [Scheduler; CPUS],
    caller: usize,
    pending: u16,
    online: u16,
    next_home: usize,
}
impl Default for Processors {
    fn default() -> Self {
        Self::new()
    }
}
impl Processors {
    /// Empty queues, initially using only the boot processor.
    #[must_use]
    #[expect(
        clippy::large_stack_arrays,
        reason = "the kernel const-initializes these queues in the static machine cell"
    )]
    pub const fn new() -> Self {
        Self {
            queues: [const { Scheduler::new() }; CPUS],
            caller: 0,
            pending: 0,
            online: 0,
            next_home: 0,
        }
    }
    /// Selects the processor holding the machine cell.
    pub fn select(&mut self, cpu: u8) {
        self.caller = usize::from(cpu).min(CPUS - 1);
    }
    /// The caller's processor number.
    #[must_use]
    pub fn caller(&self) -> u8 {
        u8::try_from(self.caller).unwrap_or(0)
    }
    /// Registers an online processor.
    pub fn online(&mut self, cpu: u8) {
        if usize::from(cpu) < CPUS {
            self.online |= 1u16 << cpu;
        }
    }
    /// Assigns a home in round robin over online processors.
    pub fn assign(&mut self) -> u8 {
        let mask = self.online | 1;
        for _ in 0..CPUS {
            let cpu = self.next_home;
            self.next_home = cpu.wrapping_add(1) % CPUS;
            if mask & (1u16 << cpu) != 0 {
                return u8::try_from(cpu).unwrap_or(0);
            }
        }
        0
    }
    /// Takes the reschedule requests for delivery after releasing the cell.
    pub fn take_pending(&mut self) -> u16 {
        core::mem::take(&mut self.pending)
    }
    fn home<const N: usize>(threads: &Pool<Thread, N>, id: ThreadId) -> Result<usize, Error> {
        let cpu = usize::from(threads.get(id).map_err(|_| Error::InvalidHandle)?.cpu);
        if cpu < CPUS {
            Ok(cpu)
        } else {
            Err(Error::InvalidArgument)
        }
    }
    const fn outcome(&mut self, cpu: usize, outcome: Outcome) -> Outcome {
        if cpu == self.caller {
            outcome
        } else {
            if outcome.reschedule {
                self.pending |= 1u16 << cpu;
            }
            Outcome::NOTHING
        }
    }
    /// The calling processor: `current`.
    #[must_use]
    pub fn current(&self) -> Option<ThreadId> {
        self.queues.get(self.caller).and_then(Scheduler::current)
    }
    /// The calling processor: `idle`.
    #[must_use]
    pub fn idle(&self) -> Option<ThreadId> {
        self.queues.get(self.caller).and_then(Scheduler::idle)
    }
    /// The calling processor: `ready_bitmap`.
    #[must_use]
    pub fn ready_bitmap(&self) -> u32 {
        self.queues
            .get(self.caller)
            .map_or(0, Scheduler::ready_bitmap)
    }
    /// The calling processor: `is_idle`.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.queues.get(self.caller).is_none_or(Scheduler::is_idle)
    }
    /// The calling processor: `highest_ready`.
    #[must_use]
    pub fn highest_ready(&self) -> Option<u8> {
        self.queues
            .get(self.caller)
            .and_then(Scheduler::highest_ready)
    }
    /// The calling processor: `ended`.
    #[must_use]
    pub fn ended(&self) -> u32 {
        self.queues.get(self.caller).map_or(0, Scheduler::ended)
    }
    /// The calling processor: `waiting_until`.
    #[must_use]
    pub fn waiting_until(&self) -> u32 {
        self.queues
            .get(self.caller)
            .map_or(0, Scheduler::waiting_until)
    }
    /// The calling processor: `next_deadline`.
    #[must_use]
    pub fn next_deadline(&self) -> Option<ThreadId> {
        self.queues
            .get(self.caller)
            .and_then(Scheduler::next_deadline)
    }
    /// Sets the calling processor’s thread.
    pub fn set_idle(&mut self, id: ThreadId) {
        if let Some(queue) = self.queues.get_mut(self.caller) {
            queue.set_idle(id);
        }
    }
    /// Sets the calling processor’s thread.
    pub fn adopt(&mut self, id: ThreadId) {
        if let Some(queue) = self.queues.get_mut(self.caller) {
            queue.adopt(id);
        }
    }
    /// Records reclamation on the calling processor.
    pub fn cleared(&mut self) {
        if let Some(queue) = self.queues.get_mut(self.caller) {
            queue.cleared();
        }
    }
    /// Runs `pick_next` on the calling processor.
    ///
    /// # Errors
    /// The queue is empty and no idle thread exists, or a thread is invalid.
    pub fn pick_next<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
    ) -> Result<ThreadId, Error> {
        self.queues
            .get_mut(self.caller)
            .ok_or(Error::InvalidArgument)?
            .pick_next(threads)
    }
    /// Runs `tick` on the calling processor.
    pub fn tick<const N: usize>(&mut self, threads: &mut Pool<Thread, N>) -> Outcome {
        self.queues
            .get_mut(self.caller)
            .map_or(Outcome::NOTHING, |queue| queue.tick(threads))
    }
    /// Finds an expired deadline in O(CPUS) and removes it in O(log n).
    pub fn expired<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        now: u64,
    ) -> Option<ThreadId> {
        self.queues
            .iter_mut()
            .find_map(|queue| queue.expired(threads, now))
    }
    /// Routes `enqueue` to the thread's home processor.
    ///
    /// # Errors
    /// The underlying scheduler's validation errors.
    pub fn enqueue<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<(), Error> {
        let cpu = Self::home(threads, id)?;
        self.queues
            .get_mut(cpu)
            .ok_or(Error::InvalidArgument)?
            .enqueue(threads, id)?;
        if cpu != self.caller {
            self.pending |= 1u16 << cpu;
        }
        Ok(())
    }
    /// Routes `dequeue` to the thread's home processor.
    ///
    /// # Errors
    /// The underlying scheduler's validation errors.
    pub fn dequeue<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<(), Error> {
        let cpu = Self::home(threads, id)?;
        self.queues
            .get_mut(cpu)
            .ok_or(Error::InvalidArgument)?
            .dequeue(threads, id)
    }
    /// Routes `start` to the thread's home processor.
    ///
    /// # Errors
    /// The underlying scheduler's validation errors.
    pub fn start<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        let cpu = Self::home(threads, id)?;
        let outcome = self
            .queues
            .get_mut(cpu)
            .ok_or(Error::InvalidArgument)?
            .start(threads, id)?;
        Ok(self.outcome(cpu, outcome))
    }
    /// Routes `on_wake` to the thread's home processor.
    ///
    /// # Errors
    /// The underlying scheduler's validation errors.
    pub fn on_wake<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        let cpu = Self::home(threads, id)?;
        let outcome = self
            .queues
            .get_mut(cpu)
            .ok_or(Error::InvalidArgument)?
            .on_wake(threads, id)?;
        Ok(self.outcome(cpu, outcome))
    }
    /// Routes `on_block` to the thread's home processor.
    ///
    /// # Errors
    /// The underlying scheduler's validation errors.
    pub fn on_block<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        event: Event,
    ) -> Result<Outcome, Error> {
        let cpu = Self::home(threads, id)?;
        let outcome = self
            .queues
            .get_mut(cpu)
            .ok_or(Error::InvalidArgument)?
            .on_block(threads, id, event)?;
        Ok(self.outcome(cpu, outcome))
    }
    /// Routes `on_block_until` to the thread's home processor.
    ///
    /// # Errors
    /// The underlying scheduler's validation errors.
    pub fn on_block_until<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        event: Event,
        deadline: u64,
    ) -> Result<Outcome, Error> {
        let cpu = Self::home(threads, id)?;
        let outcome = self
            .queues
            .get_mut(cpu)
            .ok_or(Error::InvalidArgument)?
            .on_block_until(threads, id, event, deadline)?;
        Ok(self.outcome(cpu, outcome))
    }
    /// Routes `suspend` to the thread's home processor.
    ///
    /// # Errors
    /// The underlying scheduler's validation errors.
    pub fn suspend<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        let cpu = Self::home(threads, id)?;
        let outcome = self
            .queues
            .get_mut(cpu)
            .ok_or(Error::InvalidArgument)?
            .suspend(threads, id)?;
        Ok(self.outcome(cpu, outcome))
    }
    /// Routes `resume` to the thread's home processor.
    ///
    /// # Errors
    /// The underlying scheduler's validation errors.
    pub fn resume<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        let cpu = Self::home(threads, id)?;
        let outcome = self
            .queues
            .get_mut(cpu)
            .ok_or(Error::InvalidArgument)?
            .resume(threads, id)?;
        Ok(self.outcome(cpu, outcome))
    }
    /// Routes `fault` to the thread's home processor.
    ///
    /// # Errors
    /// The underlying scheduler's validation errors.
    pub fn fault<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        let cpu = Self::home(threads, id)?;
        let outcome = self
            .queues
            .get_mut(cpu)
            .ok_or(Error::InvalidArgument)?
            .fault(threads, id)?;
        Ok(self.outcome(cpu, outcome))
    }
    /// Routes `exit` to the thread's home processor.
    ///
    /// # Errors
    /// The underlying scheduler's validation errors.
    pub fn exit<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        let cpu = Self::home(threads, id)?;
        let outcome = self
            .queues
            .get_mut(cpu)
            .ok_or(Error::InvalidArgument)?
            .exit(threads, id)?;
        Ok(self.outcome(cpu, outcome))
    }
    /// Routes `yield_now` to the thread's home processor.
    ///
    /// # Errors
    /// The underlying scheduler's validation errors.
    pub fn yield_now<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<Outcome, Error> {
        let cpu = Self::home(threads, id)?;
        let outcome = self
            .queues
            .get_mut(cpu)
            .ok_or(Error::InvalidArgument)?
            .yield_now(threads, id)?;
        Ok(self.outcome(cpu, outcome))
    }
    /// Routes `set_priority` to the thread's home processor.
    ///
    /// # Errors
    /// The underlying scheduler's validation errors.
    pub fn set_priority<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        priority: u8,
    ) -> Result<Outcome, Error> {
        let cpu = Self::home(threads, id)?;
        let outcome = self
            .queues
            .get_mut(cpu)
            .ok_or(Error::InvalidArgument)?
            .set_priority(threads, id, priority)?;
        Ok(self.outcome(cpu, outcome))
    }
}
