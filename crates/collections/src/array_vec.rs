// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A vector of fixed capacity.
//!
//! The storage is `[Option<T>; N]` and the invariant is that the slots
//! below the length are `Some` and the rest are `None`. That costs one
//! discriminant per slot, and it is what lets the container hold a `T`
//! that has no default and still forbid `unsafe`. The price is that there
//! is no `as_slice`: handing out a `&[T]` over storage the type system
//! believes may be empty is exactly the operation `unsafe` exists for, and
//! this crate does not have it. [`ArrayVec::iter`] is the way through.

use crate::error::CollectionError;

/// A vector that holds at most `N` values.
#[derive(Debug)]
pub struct ArrayVec<T, const N: usize> {
    /// The slots. Those below `len` are `Some`.
    slots: [Option<T>; N],
    /// How many slots are taken.
    len: usize,
}

impl<T, const N: usize> ArrayVec<T, N> {
    /// The number of values this type holds.
    pub const CAPACITY: usize = N;

    /// An empty vector.
    #[must_use]
    pub const fn new() -> ArrayVec<T, N> {
        ArrayVec {
            slots: [const { None }; N],
            len: 0,
        }
    }

    /// How many values this vector holds.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// How many values are in it.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
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

    /// Appends `value`.
    ///
    /// # Errors
    ///
    /// [`CollectionError::Full`] when the vector is at its capacity. The
    /// value is returned to nobody: a caller that needs it back keeps it.
    pub fn push(&mut self, value: T) -> Result<(), CollectionError> {
        if self.is_full() {
            return Err(CollectionError::Full);
        }
        let slot = self.slots.get_mut(self.len).ok_or(CollectionError::Full)?;
        *slot = Some(value);
        self.len = self.len.wrapping_add(1);
        Ok(())
    }

    /// Removes and returns the last value, or `None` when there is none.
    pub fn pop(&mut self) -> Option<T> {
        let last = self.len.checked_sub(1)?;
        let value = self.slots.get_mut(last)?.take();
        self.len = last;
        value
    }

    /// The value at `index`.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        self.slots.get(index)?.as_ref()
    }

    /// The value at `index`, to change.
    #[must_use]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len {
            return None;
        }
        self.slots.get_mut(index)?.as_mut()
    }

    /// The first value.
    #[must_use]
    pub fn first(&self) -> Option<&T> {
        self.get(0)
    }

    /// The last value.
    #[must_use]
    pub fn last(&self) -> Option<&T> {
        self.get(self.len.checked_sub(1)?)
    }

    /// Inserts `value` at `index`, moving everything from there on one
    /// place up. An `index` equal to the length appends.
    ///
    /// # Errors
    ///
    /// [`CollectionError::Full`] when the vector is at its capacity, and
    /// [`CollectionError::Index`] when `index` is beyond the length.
    pub fn insert(&mut self, index: usize, value: T) -> Result<(), CollectionError> {
        if index > self.len {
            return Err(CollectionError::Index(index));
        }
        if self.is_full() {
            return Err(CollectionError::Full);
        }
        let mut at = self.len;
        while at > index {
            let below = at.wrapping_sub(1);
            let moved = self.slots.get_mut(below).and_then(Option::take);
            let slot = self.slots.get_mut(at).ok_or(CollectionError::Full)?;
            *slot = moved;
            at = below;
        }
        let slot = self.slots.get_mut(index).ok_or(CollectionError::Full)?;
        *slot = Some(value);
        self.len = self.len.wrapping_add(1);
        Ok(())
    }

    /// Removes the value at `index`, moving everything above it one place
    /// down, or `None` when `index` is beyond the length.
    pub fn remove(&mut self, index: usize) -> Option<T> {
        if index >= self.len {
            return None;
        }
        let removed = self.slots.get_mut(index)?.take();
        let mut at = index;
        loop {
            let above = at.wrapping_add(1);
            if above >= self.len {
                break;
            }
            let moved = self.slots.get_mut(above).and_then(Option::take);
            let slot = self.slots.get_mut(at)?;
            *slot = moved;
            at = above;
        }
        self.len = self.len.wrapping_sub(1);
        removed
    }

    /// Removes every value.
    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            *slot = None;
        }
        self.len = 0;
    }

    /// The values in order.
    #[must_use]
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            slots: self.slots.iter(),
            left: self.len,
        }
    }

    /// The values in order, to change.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots
            .iter_mut()
            .take(self.len)
            .filter_map(Option::as_mut)
    }
}

impl<T, const N: usize> Default for ArrayVec<T, N> {
    fn default() -> ArrayVec<T, N> {
        ArrayVec::new()
    }
}

impl<'a, T, const N: usize> IntoIterator for &'a ArrayVec<T, N> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

/// The values of an [`ArrayVec`], in order.
#[derive(Debug)]
pub struct Iter<'a, T> {
    /// The slots, from the first.
    slots: core::slice::Iter<'a, Option<T>>,
    /// How many taken slots are left.
    left: usize,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        if self.left == 0 {
            return None;
        }
        self.left = self.left.wrapping_sub(1);
        self.slots.next()?.as_ref()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}
