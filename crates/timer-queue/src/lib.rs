// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]
extern crate alloc;
use alloc::collections::BTreeMap;

/// Queue capacity or non-reusable token space exhausted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Maximum pending entry count reached.
    Capacity,
    /// All token values were consumed. Existing entries can still be removed.
    Tokens,
}

/// Ordered deadlines and cancellable payloads with a fixed logical entry quota.
#[derive(Debug)]
pub struct DeadlineQueue<T> {
    entries: BTreeMap<(u128, u64), T>,
    deadlines: BTreeMap<u64, u128>,
    next: u64,
    limit: usize,
}
impl<T> DeadlineQueue<T> {
    /// Creates an empty queue with the specified maximum live entry count.
    #[must_use]
    pub const fn new(limit: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            deadlines: BTreeMap::new(),
            next: 0,
            limit,
        }
    }
    /// Inserts a payload and returns its unique nonzero cancellation token.
    ///
    /// # Errors
    /// Capacity or token exhaustion; neither index is changed on failure.
    pub fn insert(&mut self, deadline: u128, value: T) -> Result<u64, Error> {
        if self.entries.len() >= self.limit {
            return Err(Error::Capacity);
        }
        let token = self.next.checked_add(1).ok_or(Error::Tokens)?;
        self.next = token;
        self.entries.insert((deadline, token), value);
        self.deadlines.insert(token, deadline);
        Ok(token)
    }
    /// Cancels a token, returning its payload if still pending.
    pub fn remove(&mut self, token: u64) -> Option<T> {
        let deadline = self.deadlines.remove(&token)?;
        self.entries.remove(&(deadline, token))
    }
    /// Removes the earliest entry when its deadline is at or before `now`.
    pub fn pop_due(&mut self, now: u128) -> Option<(u64, T)> {
        if self.next_deadline()? > now {
            return None;
        }
        let ((_, token), value) = self.entries.pop_first()?;
        self.deadlines.remove(&token);
        Some((token, value))
    }
    /// Earliest pending deadline, if any.
    #[must_use]
    pub fn next_deadline(&self) -> Option<u128> {
        self.entries
            .first_key_value()
            .map(|((deadline, _), _)| *deadline)
    }
    /// Payloads in deadline and insertion order, for inspection or tracing.
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.entries.values()
    }
    /// Number of pending entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// Whether the queue has no pending entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;
