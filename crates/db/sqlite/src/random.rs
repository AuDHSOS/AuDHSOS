// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The bytes `random` and `randomblob` answer.
//!
//! SQLite draws them from `sqlite3_randomness`, which seeds itself from
//! the operating system. This crate has no operating system under it,
//! so the caller hands a seed in and the bytes follow from it: a
//! statement over a given seed answers the same bytes every time, which
//! is what lets a fixture hold them.

use alloc::vec::Vec;
use core::cell::Cell;

/// The state the bytes are drawn from.
#[derive(Clone, Debug, Default)]
pub struct Source {
    /// What the last draw left.
    state: Cell<u64>,
}

impl Source {
    /// A source that starts at `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self {
            state: Cell::new(seed),
        }
    }

    /// The next word, which is `splitmix64` over a counter: the state
    /// walks by a fixed step and the step is mixed, so a seed of nought
    /// answers bytes that are not nought.
    ///
    /// One word costs O(1).
    pub fn word(&self) -> u64 {
        let next = self.state.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
        self.state.set(next);
        let mut word = next;
        word = (word ^ (word >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        word = (word ^ (word >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        word ^ (word >> 31)
    }

    /// `bytes` of them, which costs O(n).
    pub fn bytes(&self, bytes: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes);
        while out.len() < bytes {
            out.extend_from_slice(&self.word().to_le_bytes());
        }
        out.truncate(bytes);
        out
    }
}
