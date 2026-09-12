// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The entropy source of this architecture: `RDSEED`.
//!
//! Invariant: every word of a seed that reaches a caller came out of one
//! successful `RDSEED`; a word the instruction refused through the whole
//! retry bound is an error and never a word this module invented.

use kernel_hal_api::random::{Random, RandomError, SEED_WORDS};

use crate::instructions::read_seed;

/// How many times one word is drawn again before the source is called
/// unavailable. The instruction fails while the hardware pool refills, and
/// the count is per word: a word that took two attempts does not shorten
/// what the next one gets.
pub const MAX_RETRIES: u32 = 64;

/// The hardware source of this processor.
///
/// A value of this type says that the processor reports `RDSEED` in
/// `CPUID`; [`HardwareRandom::of_this_processor`] is what checks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HardwareRandom {
    _private: (),
}

impl HardwareRandom {
    /// The source of this processor, or `None` when it has no `RDSEED`.
    #[must_use]
    pub fn of_this_processor() -> Option<Self> {
        crate::instructions::has_rdseed().then_some(HardwareRandom { _private: () })
    }
}

impl Random for HardwareRandom {
    fn seed(&mut self) -> Result<[u64; SEED_WORDS], RandomError> {
        let mut seed = [0_u64; SEED_WORDS];
        for word in &mut seed {
            *word = draw().ok_or(RandomError::Unavailable)?;
        }
        Ok(seed)
    }
}

/// One word, drawn again while the source says it has nothing.
fn draw() -> Option<u64> {
    for _ in 0..MAX_RETRIES {
        // SAFETY: a `HardwareRandom` exists only for a processor whose
        // `CPUID` reported the instruction.
        if let Some(word) = unsafe { read_seed() } {
            return Some(word);
        }
    }
    None
}
