// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A first-in, first-out buffer of fixed capacity.
//!
//! A full buffer refuses the write. It does not overwrite the oldest
//! value, which is what a ring that carries received frames must not do
//! silently: the caller is the one that decides whether losing the head or
//! refusing the tail is right, and it can only decide if it is told.

use crate::error::CollectionError;

/// A queue that holds at most `N` values.
#[derive(Debug)]
pub struct RingBuffer<T, const N: usize> {
    /// The slots. `len` of them are taken, starting at `head` and wrapping.
    slots: [Option<T>; N],
    /// Where the oldest value is.
    head: usize,
    /// How many values are in the buffer.
    len: usize,
}

impl<T, const N: usize> RingBuffer<T, N> {
    /// The number of values this type holds.
    pub const CAPACITY: usize = N;

    /// An empty buffer.
    #[must_use]
    pub const fn new() -> RingBuffer<T, N> {
        RingBuffer {
            slots: [const { None }; N],
            head: 0,
            len: 0,
        }
    }

    /// How many values this buffer holds.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// How many values are in it.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// How many more values would fit. Exactly that many writes succeed
    /// before one is refused.
    #[must_use]
    pub const fn free(&self) -> usize {
        N.wrapping_sub(self.len)
    }

    /// Whether it holds nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether one more value would not fit.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.len >= N
    }

    /// Appends `value` behind the newest one.
    ///
    /// # Errors
    ///
    /// [`CollectionError::Full`] when the buffer is at its capacity.
    pub fn push(&mut self, value: T) -> Result<(), CollectionError> {
        if self.is_full() {
            return Err(CollectionError::Full);
        }
        let at = Self::wrap(self.head.wrapping_add(self.len));
        let slot = self.slots.get_mut(at).ok_or(CollectionError::Full)?;
        *slot = Some(value);
        self.len = self.len.wrapping_add(1);
        Ok(())
    }

    /// Removes and returns the oldest value, or `None` when there is none.
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let value = self.slots.get_mut(self.head)?.take();
        self.head = Self::wrap(self.head.wrapping_add(1));
        self.len = self.len.wrapping_sub(1);
        value
    }

    /// The oldest value, without removing it.
    #[must_use]
    pub fn peek(&self) -> Option<&T> {
        self.get(0)
    }

    /// The value `index` places behind the oldest one.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        let at = Self::wrap(self.head.wrapping_add(index));
        self.slots.get(at)?.as_ref()
    }

    /// Removes every value.
    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            *slot = None;
        }
        self.head = 0;
        self.len = 0;
    }

    /// The values from the oldest to the newest.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        (0..self.len).filter_map(|index| self.get(index))
    }

    /// The position `at` names once the buffer has been wrapped around.
    /// A capacity of zero has no positions and answers zero, which no
    /// accessor reaches because the length is zero as well.
    const fn wrap(at: usize) -> usize {
        match at.checked_rem(N) {
            Some(position) => position,
            None => 0,
        }
    }
}

impl<T, const N: usize> Default for RingBuffer<T, N> {
    fn default() -> RingBuffer<T, N> {
        RingBuffer::new()
    }
}
