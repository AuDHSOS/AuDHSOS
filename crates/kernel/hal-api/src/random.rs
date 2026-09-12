// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The entropy source of the machine.
//!
//! Invariant: a seed that reaches a caller was filled word by word from the
//! hardware; a word the hardware would not give is an error and never a
//! word of the adapter's own making.

use core::fmt;

/// How many words a seed is: the thirty-two bytes a `ChaChaRng` takes.
pub const SEED_WORDS: usize = 4;

/// Why the source gave nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RandomError {
    /// A word could not be filled inside the retry bound, or the machine
    /// has no source at all.
    Unavailable,
}

impl fmt::Display for RandomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RandomError::Unavailable => f.write_str("the entropy source would not deliver"),
        }
    }
}

/// A source of seeds.
pub trait Random {
    /// Four words drawn from the hardware.
    ///
    /// # Errors
    ///
    /// [`RandomError::Unavailable`] when a word could not be filled. No
    /// partial result reaches the caller.
    fn seed(&mut self) -> Result<[u64; SEED_WORDS], RandomError>;
}
