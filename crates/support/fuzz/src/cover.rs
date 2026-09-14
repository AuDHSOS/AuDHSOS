// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a run has reached, as one size per feature.
//!
//! The pool keeps this beside the input that owns each feature. A worker
//! keeps it alone, which is all it needs to answer whether the input it
//! just ran reached anything: an input reaches something when it claims a
//! feature here, and an input that claims none never has to be spoken of.
//! That is what keeps feature lists off the pipe, where a run at full
//! speed would put gigabytes a second.
//!
//! Invariant: a worker's copy is never cheaper than the orchestrator's,
//! because every feature a worker claims here it also reports, and the
//! orchestrator claims it in the pool. A size here is therefore at least
//! the pool's, so a feature this refuses the pool refuses too, and
//! filtering here drops nothing the pool would have taken.

/// How many features the ownership table tells apart. A feature beyond
/// this many shares a slot with an older one, which costs coverage and
/// never correctness.
pub const FEATURE_SLOTS: usize = 1 << 21;

/// How many bytes a whole table is on the wire.
pub const COVER_BYTES: usize = FEATURE_SLOTS * 4;

/// The slot of the ownership table that `feature` uses.
#[must_use]
pub fn slot_of(feature: u32) -> usize {
    usize::try_from(feature).unwrap_or(0) % FEATURE_SLOTS
}

/// The size of the smallest input that reaches each feature, zero for a
/// feature nothing reaches.
#[derive(Clone, Debug)]
pub struct Cover {
    /// One size per slot.
    smallest: Box<[u32]>,
}

impl Default for Cover {
    fn default() -> Self {
        Self::new()
    }
}

impl Cover {
    /// A table nothing has reached.
    #[must_use]
    pub fn new() -> Self {
        Self {
            smallest: vec![0u32; FEATURE_SLOTS].into_boxed_slice(),
        }
    }

    /// Whether `feature` is reached by an input no larger than `size`.
    #[must_use]
    pub fn covers(&self, feature: u32, size: usize) -> bool {
        let known = self.smallest.get(slot_of(feature)).copied().unwrap_or(0);
        known != 0 && usize::try_from(known).unwrap_or(usize::MAX) <= size
    }

    /// Gives `feature` to an input of `size`, and answers whether it took
    /// it. `shrink` decides whether a smaller input takes a feature over,
    /// exactly as it does in the pool.
    pub fn claim(&mut self, feature: u32, size: u32, shrink: bool) -> bool {
        let Some(slot) = self.smallest.get_mut(slot_of(feature)) else {
            return false;
        };
        if *slot != 0 && !(shrink && *slot > size) {
            return false;
        }
        *slot = size;
        true
    }

    /// The table as bytes, one little-endian `u32` per slot.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(COVER_BYTES);
        for size in &self.smallest {
            out.extend_from_slice(&size.to_le_bytes());
        }
        out
    }

    /// Replaces the table with the one `bytes` holds. A short message
    /// leaves the slots past its end where they were, which can only make
    /// this table dearer than the sender's and so never drops a feature
    /// the sender would take.
    pub fn set_bytes(&mut self, bytes: &[u8]) {
        for (slot, word) in self.smallest.iter_mut().zip(bytes.as_chunks::<4>().0) {
            *slot = u32::from_le_bytes(*word);
        }
    }
}
