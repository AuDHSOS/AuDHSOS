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

/// State machine for a named property access Inline Cache site (`o.x` / `o.x = v`).
#[derive(Clone, Debug, PartialEq, Default)]
pub enum NamedAccessIC {
    /// Not yet executed.
    #[default]
    Uninitialized,
    /// Exactly one shape observed. Property load is two instructions: check shape + read slot.
    Monomorphic {
        /// Cached `ShapeId`.
        shape: ShapeId,
        /// Cached slot offset.
        slot: u32,
    },
    /// A small sequence of shapes observed.
    Polymorphic(Vec<(ShapeId, u32)>),
    /// Too many shapes observed; fall back to general lookup.
    Megamorphic,
}

impl NamedAccessIC {
    /// Fast path: attempts to read the cached slot for `actual_shape`.
    #[must_use]
    pub fn try_get_slot(&self, actual_shape: ShapeId) -> Option<u32> {
        match self {
            Self::Monomorphic { shape, slot } => {
                if *shape == actual_shape {
                    Some(*slot)
                } else {
                    None
                }
            }
            Self::Polymorphic(entries) => {
                for &(shape, slot) in entries {
                    if shape == actual_shape {
                        return Some(slot);
                    }
                }
                None
            }
            Self::Uninitialized | Self::Megamorphic => None,
        }
    }

    /// Updates the IC state after a successful slow-path property resolution.
    pub fn record_shape(&mut self, shape: ShapeId, slot: u32) {
        match self {
            Self::Uninitialized => {
                *self = Self::Monomorphic { shape, slot };
            }
            Self::Monomorphic {
                shape: old_shape,
                slot: old_slot,
            } => {
                if *old_shape == shape {
                    return;
                }
                let entries = alloc::vec![(*old_shape, *old_slot), (shape, slot)];
                *self = Self::Polymorphic(entries);
            }
            Self::Polymorphic(entries) => {
                for &(s, _) in entries.iter() {
                    if s == shape {
                        return;
                    }
                }
                if entries.len() < POLYMORPHIC_LIMIT {
                    entries.push((shape, slot));
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
        assert_eq!(ic.try_get_slot(ShapeId(1)), None);

        // First execution -> Monomorphic
        ic.record_shape(ShapeId(1), 0);
        assert!(matches!(
            ic,
            NamedAccessIC::Monomorphic {
                shape: ShapeId(1),
                slot: 0
            }
        ));
        assert_eq!(ic.try_get_slot(ShapeId(1)), Some(0));
        assert_eq!(ic.try_get_slot(ShapeId(2)), None);

        // Second shape -> Polymorphic
        ic.record_shape(ShapeId(2), 1);
        assert!(matches!(ic, NamedAccessIC::Polymorphic(_)));
        assert_eq!(ic.try_get_slot(ShapeId(1)), Some(0));
        assert_eq!(ic.try_get_slot(ShapeId(2)), Some(1));
        assert_eq!(ic.try_get_slot(ShapeId(3)), None);
    }
}
