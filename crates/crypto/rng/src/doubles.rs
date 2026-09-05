// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Sources and generators for tests: a script, a pattern, and a failure.

use crate::error::{EntropyError, RngError};
use crate::source::{Entropy, Rng, SourceCalls};

/// A generator that hands out a fixed script and then refuses.
///
/// The trace tests of the protocol need a handshake whose every value is
/// known in advance; this is what makes the client's private key and its
/// random the ones a document wrote down.
pub struct ScriptedRng<'a> {
    /// The bytes still to hand out.
    remaining: &'a [u8],
}

impl<'a> ScriptedRng<'a> {
    /// A generator over `script`.
    #[must_use]
    pub const fn new(script: &'a [u8]) -> ScriptedRng<'a> {
        ScriptedRng { remaining: script }
    }

    /// Bytes the script still holds.
    #[must_use]
    pub const fn left(&self) -> usize {
        self.remaining.len()
    }
}

impl Rng for ScriptedRng<'_> {
    fn fill(&mut self, out: &mut [u8]) -> Result<(), RngError> {
        if out.len() > self.remaining.len() {
            return Err(RngError::Exhausted);
        }
        let (head, tail) = self.remaining.split_at(out.len());
        for (slot, byte) in out.iter_mut().zip(head) {
            *slot = *byte;
        }
        self.remaining = tail;
        Ok(())
    }
}

/// A source that counts up, so that a test can tell what it delivered.
pub struct CountingEntropy {
    /// The next byte to hand out.
    next: u8,
    /// How many calls the source has served.
    calls: usize,
}

impl CountingEntropy {
    /// A source starting at `first`.
    #[must_use]
    pub const fn new(first: u8) -> CountingEntropy {
        CountingEntropy {
            next: first,
            calls: 0,
        }
    }

    /// How often the source was asked.
    #[must_use]
    pub const fn calls(&self) -> usize {
        self.calls
    }
}

impl SourceCalls for CountingEntropy {
    fn calls(&self) -> usize {
        self.calls
    }
}

impl Entropy for CountingEntropy {
    fn fill(&mut self, out: &mut [u8]) -> Result<(), EntropyError> {
        self.calls = self.calls.wrapping_add(1);
        for slot in out.iter_mut() {
            *slot = self.next;
            self.next = self.next.wrapping_add(1);
        }
        Ok(())
    }
}

/// A source that never delivers.
pub struct FailingEntropy;

impl SourceCalls for FailingEntropy {
    fn calls(&self) -> usize {
        0
    }
}

impl Entropy for FailingEntropy {
    fn fill(&mut self, _out: &mut [u8]) -> Result<(), EntropyError> {
        Err(EntropyError::Unavailable)
    }
}
