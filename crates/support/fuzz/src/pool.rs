// SPDX-License-Identifier: AGPL-3.0-only AND Apache-2.0 WITH LLVM-exception
// Copyright (C) 2026 Manuel Baesler and contributors
// Copyright (C) the LLVM Project contributors, under Apache-2.0 WITH LLVM-exception
// Ported from LLVM's libFuzzer; see NOTICE at the root of this repository.

//! The inputs a run keeps and the one it works on next.
//!
//! An input earns its place by producing a feature no kept input produces,
//! or by producing one of them in fewer bytes than the input that produced
//! it until now. The second half is what makes a corpus shrink while it
//! grows: a feature belongs to the smallest input that reaches it, and an
//! input that ends up owning no feature at all is dropped.
//!
//! The ownership table is flat and indexed by the feature itself, taken
//! modulo its size, because a run offers hundreds of features and cannot
//! afford a hash for each of them. This is libFuzzer's arrangement, and
//! its size is libFuzzer's too.
//!
//! Invariants: an index into the pool stays valid for the whole run, so a
//! dropped input leaves an empty slot rather than moving its neighbours;
//! an input with no features is never drawn; the weights are rebuilt
//! whenever the pool changes, so a draw never reads a stale one.

use std::path::PathBuf;

use crate::rng::Rng;

/// How many features the ownership table tells apart. A feature beyond
/// this many shares a slot with an older one, which costs coverage and
/// never correctness.
const FEATURE_SLOTS: usize = 1 << 21;

/// One input the run keeps.
#[derive(Clone, Debug)]
pub struct Input {
    /// The bytes, empty for a slot whose input was dropped.
    pub bytes: Vec<u8>,
    /// How many features this input is the smallest to reach.
    pub features: usize,
    /// The file it was read from or written to, if it has one.
    pub file: Option<PathBuf>,
    /// Whether the slot still holds an input.
    pub live: bool,
}

/// What offering an input to the pool amounted to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The input reached nothing that was not already reached as cheaply.
    Nothing,
    /// The input reached something no kept input reaches.
    New,
    /// The input reached only what was already reached, but in fewer
    /// bytes, and took those features over.
    Reduced,
}

/// What one slot of the ownership table holds.
#[derive(Clone, Copy, Debug, Default)]
struct Owned {
    /// The size of the smallest input that reaches the feature, or zero
    /// for a feature nothing reaches.
    smallest: u32,
    /// The input that owns the feature.
    owner: u32,
}

/// The inputs of a run.
#[derive(Debug)]
pub struct Pool {
    /// The inputs, including the slots of dropped ones.
    inputs: Vec<Input>,
    /// For each feature, the smallest input that reaches it and which
    /// input that is.
    slots: Box<[Owned]>,
    /// The running sum of the weight of each input, for drawing one.
    weights: Vec<u64>,
    /// How many features the pool covers.
    covered: usize,
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

impl Pool {
    /// An empty pool.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inputs: Vec::new(),
            slots: vec![Owned::default(); FEATURE_SLOTS].into_boxed_slice(),
            weights: Vec::new(),
            covered: 0,
        }
    }

    /// How many inputs the pool holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inputs.iter().filter(|input| input.live).count()
    }

    /// Whether the pool holds no input.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many bytes the inputs hold together.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.inputs
            .iter()
            .filter(|input| input.live)
            .fold(0, |sum, input| sum.saturating_add(input.bytes.len()))
    }

    /// How many features the pool covers.
    #[must_use]
    pub const fn covered(&self) -> usize {
        self.covered
    }

    /// The input in `index`, live or not.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Input> {
        self.inputs.get(index)
    }

    /// Every slot, live or not, oldest first.
    #[must_use]
    pub fn all(&self) -> &[Input] {
        &self.inputs
    }

    /// Whether `feature` is reached by an input no larger than `size`.
    #[must_use]
    pub fn covers(&self, feature: u32, size: usize) -> bool {
        let known = self
            .slots
            .get(slot_of(feature))
            .map_or(0, |slot| slot.smallest);
        known != 0 && usize::try_from(known).unwrap_or(usize::MAX) <= size
    }

    /// Offers `bytes` to the pool, given every feature its run produced.
    ///
    /// `shrink` decides whether an input that reaches a feature in fewer
    /// bytes than its present owner takes it over. A fuzzing run wants
    /// that; a run that is only measuring what a corpus reaches does not.
    pub fn offer(&mut self, bytes: &[u8], features: &[u32], shrink: bool) -> Verdict {
        let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        let next = self.inputs.len();
        let mut taken = Vec::new();
        let mut fresh = false;
        for feature in features {
            match self.claim(*feature, size, next, shrink) {
                Claim::Fresh => {
                    fresh = true;
                    taken.push(*feature);
                }
                Claim::TakenOver => taken.push(*feature),
                Claim::Refused => {}
            }
        }
        if taken.is_empty() {
            return Verdict::Nothing;
        }
        self.inputs.push(Input {
            bytes: bytes.to_vec(),
            features: taken.len(),
            file: None,
            live: true,
        });
        self.rebuild_weights();
        if fresh {
            Verdict::New
        } else {
            Verdict::Reduced
        }
    }

    /// Records where the input in `index` was written.
    pub fn set_file(&mut self, index: usize, file: PathBuf) {
        if let Some(input) = self.inputs.get_mut(index) {
            input.file = Some(file);
        }
    }

    /// The index the next offered input would take.
    #[must_use]
    pub const fn next_index(&self) -> usize {
        self.inputs.len()
    }

    /// Gives `feature` to the input that is about to take slot `next`, if
    /// it deserves it.
    fn claim(&mut self, feature: u32, size: u32, next: usize, shrink: bool) -> Claim {
        let index = slot_of(feature);
        let held = self.slots.get(index).copied().unwrap_or_default();
        let fresh = held.smallest == 0;
        let smaller = shrink && held.smallest > size;
        if !fresh && !smaller {
            return Claim::Refused;
        }
        if fresh {
            self.covered = self.covered.saturating_add(1);
        } else {
            self.release(usize::try_from(held.owner).unwrap_or(0));
        }
        if let Some(slot) = self.slots.get_mut(index) {
            *slot = Owned {
                smallest: size,
                owner: u32::try_from(next).unwrap_or(u32::MAX),
            };
        }
        if fresh {
            Claim::Fresh
        } else {
            Claim::TakenOver
        }
    }

    /// Takes one feature away from the input in `index`, dropping it if
    /// that was its last.
    fn release(&mut self, index: usize) {
        let Some(input) = self.inputs.get_mut(index) else {
            return;
        };
        input.features = input.features.saturating_sub(1);
        if input.features == 0 {
            input.bytes = Vec::new();
            input.live = false;
        }
    }

    /// Rebuilds the running sum the draw reads.
    ///
    /// The weight of an input is the number of features it owns times its
    /// place in the pool, so that a recent input is preferred to an old
    /// one that reaches as much: the recent one is where the run last
    /// found something. An input that owns nothing weighs nothing and is
    /// never drawn.
    fn rebuild_weights(&mut self) {
        self.weights.clear();
        self.weights.reserve(self.inputs.len());
        let mut running = 0u64;
        for (place, input) in self.inputs.iter().enumerate() {
            let weight = if input.live {
                let features = u64::try_from(input.features).unwrap_or(u64::MAX);
                let recency = u64::try_from(place).unwrap_or(u64::MAX).saturating_add(1);
                features.saturating_mul(recency)
            } else {
                0
            };
            running = running.saturating_add(weight);
            self.weights.push(running);
        }
    }

    /// Draws an input, or `None` for an empty pool.
    #[must_use]
    pub fn choose(&self, rng: &mut Rng) -> Option<usize> {
        let total = self.weights.last().copied().unwrap_or(0);
        if total == 0 {
            return None;
        }
        let drawn =
            u64::try_from(rng.below(usize::try_from(total).unwrap_or(usize::MAX))).unwrap_or(0);
        self.weights.partition_point(|sum| *sum <= drawn).into()
    }
}

/// Whether a feature went to the input that offered it.
#[derive(Clone, Copy, Debug)]
enum Claim {
    /// Nothing reached this feature before.
    Fresh,
    /// Something reached it, in more bytes.
    TakenOver,
    /// Something reaches it in no more bytes.
    Refused,
}

/// The slot of the ownership table that `feature` uses.
fn slot_of(feature: u32) -> usize {
    usize::try_from(feature).unwrap_or(0) % FEATURE_SLOTS
}
