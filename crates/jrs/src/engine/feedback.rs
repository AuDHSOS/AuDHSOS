// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::as_conversions, reason = "feedback slot indices fit in usize")]

//! Inline Caches (ICs) and Feedback Vectors.
//!
//! Feedback vectors record observed types and shapes at runtime.
//! Property lookups start as Monomorphic (1 check, direct slot load),
//! expand to Polymorphic (2-4 shapes), or degrade to Megamorphic.

use super::shape::ShapeId;
use alloc::vec::Vec;

/// Maximum number of shapes handled inline in a polymorphic cache before degrading.
pub const POLYMORPHIC_LIMIT: usize = 4;

/// Cacheable resolution of one named property for one receiver Shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamedAccessCase {
    /// Shape observed on the receiver object.
    pub receiver_shape: ShapeId,
    /// Number of `[[Prototype]]` edges from receiver to the property holder.
    pub holder_depth: u16,
    /// Property slot in the holder object.
    pub slot: u32,
    /// Prototype-validity epoch at resolution time.
    pub prototype_epoch: u64,
}

/// State machine for a named property access Inline Cache site (`o.x` / `o.x = v`).
#[derive(Clone, Debug, PartialEq, Default)]
pub enum NamedAccessIC {
    /// Not yet executed.
    #[default]
    Uninitialized,
    /// Exactly one shape observed. Property load is two instructions: check shape + read slot.
    Monomorphic(NamedAccessCase),
    /// A small sequence of shapes observed.
    Polymorphic(Vec<NamedAccessCase>),
    /// Too many shapes observed; fall back to general lookup.
    Megamorphic,
}

impl NamedAccessIC {
    /// Fast path: attempts to read the cached slot for `actual_shape`.
    #[must_use]
    pub fn try_get(
        &self,
        actual_shape: ShapeId,
        prototype_epoch: Option<u64>,
    ) -> Option<NamedAccessCase> {
        let prototype_epoch = prototype_epoch?;
        match self {
            Self::Monomorphic(case) => {
                if case.receiver_shape == actual_shape && case.prototype_epoch == prototype_epoch {
                    Some(*case)
                } else {
                    None
                }
            }
            Self::Polymorphic(entries) => {
                for case in entries {
                    if case.receiver_shape == actual_shape
                        && case.prototype_epoch == prototype_epoch
                    {
                        return Some(*case);
                    }
                }
                None
            }
            Self::Uninitialized | Self::Megamorphic => None,
        }
    }

    /// Updates the IC state after a successful slow-path property resolution.
    pub fn record(&mut self, case: NamedAccessCase) {
        match self {
            Self::Uninitialized => {
                *self = Self::Monomorphic(case);
            }
            Self::Monomorphic(old) => {
                if old.receiver_shape == case.receiver_shape {
                    *old = case;
                    return;
                }
                let entries = alloc::vec![*old, case];
                *self = Self::Polymorphic(entries);
            }
            Self::Polymorphic(entries) => {
                for old in entries.iter_mut() {
                    if old.receiver_shape == case.receiver_shape {
                        *old = case;
                        return;
                    }
                }
                if entries.len() < POLYMORPHIC_LIMIT {
                    entries.push(case);
                } else {
                    *self = Self::Megamorphic;
                }
            }
            Self::Megamorphic => {}
        }
    }
}

/// Observed types for binary operations (+, -, *, <).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BinaryOpFeedback {
    /// Uninitialized.
    #[default]
    None,
    /// Both operands were 32-bit signed Smis.
    SignedSmallInteger,
    /// Operands were IEEE-754 numbers.
    Number,
    /// Mixed types or strings.
    Generic,
}

/// Dedicated feedback slot in a function's `FeedbackVector`.
#[derive(Clone, Debug, PartialEq)]
pub enum FeedbackSlot {
    /// Named property access IC.
    NamedAccess(NamedAccessIC),
    /// Binary operator type feedback.
    BinaryOp(BinaryOpFeedback),
}

/// Mutable runtime feedback vector associated with a compiled function.
#[derive(Clone, Debug, Default)]
pub struct FeedbackVector {
    slots: Vec<FeedbackSlot>,
}

impl FeedbackVector {
    /// Allocates a new feedback vector with `count` named access slots.
    #[must_use]
    pub fn new(count: u16) -> Self {
        let mut slots = Vec::with_capacity(count as usize);
        for _ in 0..count {
            slots.push(FeedbackSlot::NamedAccess(NamedAccessIC::Uninitialized));
        }
        Self { slots }
    }

    /// Retrieves a mutable reference to a named access IC slot.
    pub fn get_named_ic_mut(&mut self, slot: u16) -> Option<&mut NamedAccessIC> {
        match self.slots.get_mut(slot as usize) {
            Some(FeedbackSlot::NamedAccess(ic)) => Some(ic),
            _ => None,
        }
    }

    /// Retrieves an immutable reference to a named access IC slot.
    #[must_use]
    pub fn get_named_ic(&self, slot: u16) -> Option<&NamedAccessIC> {
        match self.slots.get(slot as usize) {
            Some(FeedbackSlot::NamedAccess(ic)) => Some(ic),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_ic_monomorphic_to_polymorphic_transition() {
        let mut ic = NamedAccessIC::default();
        assert_eq!(ic, NamedAccessIC::Uninitialized);
        assert_eq!(ic.try_get(ShapeId(1), Some(0)), None);

        // First execution -> Monomorphic
        let first = NamedAccessCase {
            receiver_shape: ShapeId(1),
            holder_depth: 0,
            slot: 0,
            prototype_epoch: 0,
        };
        ic.record(first);
        assert_eq!(ic, NamedAccessIC::Monomorphic(first));
        assert_eq!(ic.try_get(ShapeId(1), Some(0)), Some(first));
        assert_eq!(ic.try_get(ShapeId(1), Some(1)), None);
        assert_eq!(ic.try_get(ShapeId(1), None), None);
        assert_eq!(ic.try_get(ShapeId(2), Some(0)), None);

        let refreshed = NamedAccessCase {
            prototype_epoch: 1,
            ..first
        };
        ic.record(refreshed);
        assert_eq!(ic, NamedAccessIC::Monomorphic(refreshed));

        // Second shape -> Polymorphic
        let second = NamedAccessCase {
            receiver_shape: ShapeId(2),
            holder_depth: 1,
            slot: 1,
            prototype_epoch: 0,
        };
        ic.record(second);
        assert!(matches!(ic, NamedAccessIC::Polymorphic(_)));
        assert_eq!(ic.try_get(ShapeId(1), Some(1)), Some(refreshed));
        assert_eq!(ic.try_get(ShapeId(2), Some(0)), Some(second));
        assert_eq!(ic.try_get(ShapeId(3), Some(0)), None);
    }
}
