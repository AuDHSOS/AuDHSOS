// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two sides of randomness: where bytes come from, and what asks for
//! them.

use crate::error::{EntropyError, RngError};

/// A source of unpredictable bytes.
///
/// One implementation exists per platform. On this system it will read
/// `RDSEED` through a system call; until the network stack is scheduled,
/// the only implementations are the doubles of the tests.
pub trait Entropy {
    /// Fills `out` with unpredictable bytes.
    ///
    /// # Errors
    ///
    /// [`EntropyError::Unavailable`] when the source cannot deliver.
    fn fill(&mut self, out: &mut [u8]) -> Result<(), EntropyError>;
}

/// What a protocol asks for randomness.
///
/// The trait is deliberately narrower than [`Entropy`]: a consumer should
/// not be able to tell whether it is talking to hardware or to a
/// generator, and should not be able to ask for either.
pub trait Rng {
    /// Fills `out` with random bytes.
    ///
    /// # Errors
    ///
    /// [`RngError::Entropy`] when a reseed was due and the source failed;
    /// [`RngError::Exhausted`] from a scripted generator that ran out.
    fn fill(&mut self, out: &mut [u8]) -> Result<(), RngError>;
}

/// A source that counts how often it was asked.
///
/// Only the doubles implement this. A generator exposes the count behind
/// the same feature, so that a test can assert how many reseeds happened
/// without the product surface growing a counter nobody needs.
#[cfg(any(test, feature = "test-doubles"))]
pub trait SourceCalls: Entropy {
    /// How often the source was asked.
    fn calls(&self) -> usize;
}
