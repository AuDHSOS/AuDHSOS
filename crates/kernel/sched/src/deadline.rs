// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Fixed-capacity deadline min-heap; insert, pop, and cancellation cost O(log n).

use audhsos_abi::Error;
use kernel_objects::{Pool, Thread, ThreadId};

#[derive(Clone, Copy, Debug)]
struct Entry {
    deadline: u64,
    order: u64,
    id: ThreadId,
}

impl Entry {
    const EMPTY: Self = Self {
        deadline: 0,
        order: 0,
        id: ThreadId::new(0, 0),
    };
}

/// Occupied entries precede `len`; each live thread records its entry's index.
#[derive(Debug)]
pub(crate) struct Deadlines<const CAPACITY: usize> {
    entries: [Entry; CAPACITY],
    len: usize,
    order: u64,
    #[cfg(test)]
    pub(crate) comparisons: core::cell::Cell<usize>,
}

#[expect(
    clippy::indexing_slicing,
    reason = "heap indices are below len, which never exceeds CAPACITY"
)]
impl<const CAPACITY: usize> Deadlines<CAPACITY> {
    pub(crate) const fn new() -> Self {
        Self {
            entries: [Entry::EMPTY; CAPACITY],
            len: 0,
            order: 0,
            #[cfg(test)]
            comparisons: core::cell::Cell::new(0),
        }
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    pub(crate) const fn first(&self) -> Option<ThreadId> {
        if self.len == 0 {
            None
        } else {
            Some(self.entries[0].id)
        }
    }

    /// Checks admission before the scheduler changes the thread's state.
    pub(crate) fn can_insert<const N: usize>(
        &self,
        threads: &Pool<Thread, N>,
        id: ThreadId,
    ) -> Result<(), Error> {
        let thread = threads.get(id).map_err(|_| Error::InvalidHandle)?;
        if thread.deadline_index.is_some() || thread.deadline.is_some() {
            return Err(Error::InvalidState);
        }
        if self.len == CAPACITY {
            return Err(Error::PoolExhausted);
        }
        if self.order == u64::MAX {
            return Err(Error::QuotaExceeded);
        }
        Ok(())
    }

    pub(crate) fn insert<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        id: ThreadId,
        deadline: u64,
    ) -> Result<(), Error> {
        self.can_insert(threads, id)?;
        let index = self.len;
        self.entries[index] = Entry {
            deadline,
            order: self.order,
            id,
        };
        self.order = self.order.wrapping_add(1);
        self.len = self.len.wrapping_add(1);
        threads.with(id, |thread| {
            thread.deadline = Some(deadline);
            thread.deadline_index = Some(index);
        });
        self.sift_up(threads, index);
        Ok(())
    }

    pub(crate) fn expired<const N: usize>(
        &mut self,
        threads: &mut Pool<Thread, N>,
        now: u64,
    ) -> Option<ThreadId> {
        while self.len != 0 {
            let entry = self.entries[0];
            if !threads
                .get(entry.id)
                .is_ok_and(|thread| thread.deadline == Some(entry.deadline))
            {
                self.remove_at(threads, 0);
                continue;
            }
            if entry.deadline > now {
                return None;
            }
            self.remove_at(threads, 0);
            return Some(entry.id);
        }
        None
    }

    pub(crate) fn remove<const N: usize>(&mut self, threads: &mut Pool<Thread, N>, id: ThreadId) {
        let Ok(thread) = threads.get(id) else {
            return;
        };
        if let Some(index) = thread.deadline_index
            && index < self.len
            && self.entries[index].id == id
        {
            self.remove_at(threads, index);
        }
    }

    fn remove_at<const N: usize>(&mut self, threads: &mut Pool<Thread, N>, index: usize) {
        let removed = self.entries[index];
        threads.with(removed.id, |thread| {
            thread.deadline = None;
            thread.deadline_index = None;
        });
        self.len = self.len.wrapping_sub(1);
        if index != self.len {
            self.entries[index] = self.entries[self.len];
            self.record_index(threads, index);
            let at = self.sift_up(threads, index);
            self.sift_down(threads, at);
        }
        self.entries[self.len] = Entry::EMPTY;
        if self.len == 0 {
            self.order = 0;
        }
    }

    fn before(&self, left: usize, right: usize) -> bool {
        #[cfg(test)]
        {
            self.comparisons.set(self.comparisons.get().wrapping_add(1));
        }
        let left = self.entries[left];
        let right = self.entries[right];
        (left.deadline, left.order) < (right.deadline, right.order)
    }

    fn record_index<const N: usize>(&self, threads: &mut Pool<Thread, N>, index: usize) {
        threads.with(self.entries[index].id, |thread| {
            thread.deadline_index = Some(index);
        });
    }

    fn swap<const N: usize>(&mut self, threads: &mut Pool<Thread, N>, left: usize, right: usize) {
        self.entries.swap(left, right);
        self.record_index(threads, left);
        self.record_index(threads, right);
    }

    fn sift_up<const N: usize>(&mut self, threads: &mut Pool<Thread, N>, mut at: usize) -> usize {
        while at != 0 {
            let parent = at.wrapping_sub(1) / 2;
            if !self.before(at, parent) {
                break;
            }
            self.swap(threads, at, parent);
            at = parent;
        }
        at
    }

    fn sift_down<const N: usize>(&mut self, threads: &mut Pool<Thread, N>, mut at: usize) {
        // Only the first len / 2 entries have children.
        while at < self.len / 2 {
            let left = at.wrapping_mul(2).wrapping_add(1);
            let right = left.wrapping_add(1);
            let child = if right < self.len && self.before(right, left) {
                right
            } else {
                left
            };
            if !self.before(child, at) {
                break;
            }
            self.swap(threads, at, child);
            at = child;
        }
    }

    #[cfg(test)]
    pub(crate) const fn set_order(&mut self, order: u64) {
        self.order = order;
    }
}
