// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::as_conversions, reason = "feedback slot indices fit in usize")]

//! Inline Caches (ICs) and Feedback Vectors.
//!
//! Feedback vectors record observed types and shapes at runtime.
//! Property lookups start as Monomorphic (1 check, direct slot load),
//! expand to Polymorphic (2-4 shapes), or degrade to Megamorphic.

use super::{bytecode::FeedbackKind, shape::ShapeId, value::StringRef};
use alloc::vec::Vec;

/// Maximum number of shapes handled inline in a polymorphic cache before degrading.
pub const POLYMORPHIC_LIMIT: usize = 4;

/// Cacheable resolution of one named property for one receiver Shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamedAccessCase {
    /// Interned property name guarded by this case.
    pub name: StringRef,
    /// Shape observed on the receiver object.
    pub receiver_shape: ShapeId,
    /// Number of `[[Prototype]]` edges from receiver to the property holder.
    pub holder_depth: u16,
    /// Shape expected on the property holder after following the cached depth.
    pub holder_shape: ShapeId,
    /// Property slot in the holder object.
    pub slot: u32,
    /// Prototype-validity epoch for inherited accesses; absent for own properties.
    pub prototype_epoch: Option<u64>,
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
        name: StringRef,
        actual_shape: ShapeId,
        prototype_epoch: Option<u64>,
    ) -> Option<NamedAccessCase> {
        match self {
            Self::Monomorphic(case) => {
                if case_matches(*case, name, actual_shape, prototype_epoch) {
                    Some(*case)
                } else {
                    None
                }
            }
            Self::Polymorphic(entries) => {
                for case in entries {
                    if case_matches(*case, name, actual_shape, prototype_epoch) {
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
                if old.name == case.name && old.receiver_shape == case.receiver_shape {
                    *old = case;
                    return;
                }
                let entries = alloc::vec![*old, case];
                *self = Self::Polymorphic(entries);
            }
            Self::Polymorphic(entries) => {
                for old in entries.iter_mut() {
                    if old.name == case.name && old.receiver_shape == case.receiver_shape {
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

fn case_matches(
    case: NamedAccessCase,
    name: StringRef,
    actual_shape: ShapeId,
    prototype_epoch: Option<u64>,
) -> bool {
    case.name == name
        && case.receiver_shape == actual_shape
        && (case.holder_depth == 0
            || matches!((case.prototype_epoch, prototype_epoch), (Some(expected), Some(actual)) if expected == actual))
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

/// Monomorphic or bounded polymorphic bytecode-function targets observed at a call site.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum CallIC {
    /// The call site has not executed yet.
    #[default]
    Uninitialized,
    /// Exactly one bytecode function has been observed.
    Monomorphic(u32),
    /// A bounded set of bytecode functions has been observed.
    Polymorphic(Vec<u32>),
    /// More targets were observed than the bounded cache retains.
    Megamorphic,
}

impl CallIC {
    /// Records one bytecode-function target.
    pub fn record(&mut self, code_id: u32) {
        match self {
            Self::Uninitialized => *self = Self::Monomorphic(code_id),
            Self::Monomorphic(current) if *current == code_id => {}
            Self::Monomorphic(current) => {
                *self = Self::Polymorphic(alloc::vec![*current, code_id]);
            }
            Self::Polymorphic(targets) if targets.contains(&code_id) => {}
            Self::Polymorphic(targets) if targets.len() < POLYMORPHIC_LIMIT => {
                targets.push(code_id);
            }
            Self::Polymorphic(_) => *self = Self::Megamorphic,
            Self::Megamorphic => {}
        }
    }
}

/// Dedicated feedback slot in a function's `FeedbackVector`.
#[derive(Clone, Debug, PartialEq)]
pub enum FeedbackSlot {
    /// Slot kind has not yet been selected by its first executing bytecode site.
    Uninitialized,
    /// Named property access IC.
    NamedAccess(NamedAccessIC),
    /// Binary operator type feedback.
    BinaryOp(BinaryOpFeedback),
    /// Call target feedback.
    Call(CallIC),
}

/// Mutable runtime feedback vector associated with a compiled function.
#[derive(Clone, Debug, Default)]
pub struct FeedbackVector {
    slots: Vec<FeedbackSlot>,
    functions: Vec<FeedbackVector>,
}

impl FeedbackVector {
    /// Allocates a new feedback vector with `count` named access slots.
    ///
    /// This constructor is retained for focused property-IC tests. Production
    /// code uses [`Self::for_code`] and the bytecode's explicit slot kinds.
    #[must_use]
    pub fn new(count: u16) -> Self {
        let slots =
            alloc::vec![FeedbackSlot::NamedAccess(NamedAccessIC::Uninitialized); count as usize];
        Self {
            slots,
            functions: Vec::new(),
        }
    }

    /// Allocates feedback for a root bytecode unit and its flat function table.
    #[must_use]
    pub fn for_code(code: &super::bytecode::BytecodeFunction) -> Self {
        let mut vector = Self::from_kinds(&code.feedback_slots);
        vector.functions = code
            .functions
            .iter()
            .map(|function| Self::from_kinds(&function.feedback_slots))
            .collect();
        vector
    }

    fn from_kinds(kinds: &[FeedbackKind]) -> Self {
        Self {
            slots: kinds
                .iter()
                .map(|kind| match kind {
                    FeedbackKind::NamedAccess => {
                        FeedbackSlot::NamedAccess(NamedAccessIC::Uninitialized)
                    }
                    FeedbackKind::BinaryOp => FeedbackSlot::BinaryOp(BinaryOpFeedback::None),
                    FeedbackKind::Call => FeedbackSlot::Call(CallIC::Uninitialized),
                })
                .collect(),
            functions: Vec::new(),
        }
    }

    /// Returns the number of allocated feedback slots.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.slots.len()
    }

    /// Returns `true` when the vector has no feedback slots.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Retrieves a mutable reference to a named access IC slot.
    pub fn get_named_ic_mut(&mut self, slot: u16) -> Option<&mut NamedAccessIC> {
        let slot = self.slots.get_mut(slot as usize)?;
        if matches!(slot, FeedbackSlot::Uninitialized) {
            *slot = FeedbackSlot::NamedAccess(NamedAccessIC::Uninitialized);
        }
        match slot {
            FeedbackSlot::NamedAccess(ic) => Some(ic),
            _ => None,
        }
    }

    /// Retrieves an immutable reference to a named access IC slot.
    #[must_use]
    pub fn get_named_ic(&self, slot: u16) -> Option<&NamedAccessIC> {
        match self.slots.get(slot as usize) {
            Some(FeedbackSlot::NamedAccess(ic)) => Some(ic),
            Some(
                FeedbackSlot::Uninitialized | FeedbackSlot::BinaryOp(_) | FeedbackSlot::Call(_),
            )
            | None => None,
        }
    }

    /// Records a bytecode-function target in a call feedback slot.
    pub fn record_call(&mut self, slot: u16, code_id: u32) -> Option<()> {
        let slot = self.slots.get_mut(slot as usize)?;
        if matches!(slot, FeedbackSlot::Uninitialized) {
            *slot = FeedbackSlot::Call(CallIC::Uninitialized);
        }
        let FeedbackSlot::Call(call) = slot else {
            return None;
        };
        call.record(code_id);
        Some(())
    }

    /// Returns immutable call feedback for one slot.
    #[must_use]
    pub fn get_call_ic(&self, slot: u16) -> Option<&CallIC> {
        match self.slots.get(slot as usize) {
            Some(FeedbackSlot::Call(call)) => Some(call),
            Some(
                FeedbackSlot::Uninitialized
                | FeedbackSlot::NamedAccess(_)
                | FeedbackSlot::BinaryOp(_),
            )
            | None => None,
        }
    }

    pub(crate) fn function_mut(&mut self, code_id: u32) -> Option<&mut Self> {
        self.functions.get_mut(code_id as usize)
    }

    pub(crate) fn matches_code(&self, code: &super::bytecode::BytecodeFunction) -> bool {
        self.matches_kinds(&code.feedback_slots)
            && self.functions.len() == code.functions.len()
            && self
                .functions
                .iter()
                .zip(&code.functions)
                .all(|(feedback, function)| {
                    feedback.matches_kinds(&function.feedback_slots)
                        && feedback.functions.is_empty()
                })
    }

    fn matches_kinds(&self, kinds: &[FeedbackKind]) -> bool {
        self.slots.len() == kinds.len()
            && self.slots.iter().zip(kinds).all(|(slot, kind)| {
                matches!(
                    (slot, kind),
                    (FeedbackSlot::NamedAccess(_), FeedbackKind::NamedAccess)
                        | (FeedbackSlot::BinaryOp(_), FeedbackKind::BinaryOp)
                        | (FeedbackSlot::Call(_), FeedbackKind::Call)
                )
            })
    }

    /// Number of feedback vectors represented by this root and its function table.
    #[must_use]
    pub const fn vector_count(&self) -> usize {
        1usize.saturating_add(self.functions.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_ic_monomorphic_to_polymorphic_transition() {
        let mut ic = NamedAccessIC::default();
        assert_eq!(ic, NamedAccessIC::Uninitialized);
        let name = StringRef::from_parts(1, 0);
        assert_eq!(ic.try_get(name, ShapeId(1), Some(0)), None);

        // First execution -> Monomorphic
        let first = NamedAccessCase {
            name,
            receiver_shape: ShapeId(1),
            holder_depth: 0,
            holder_shape: ShapeId(1),
            slot: 0,
            prototype_epoch: None,
        };
        ic.record(first);
        assert_eq!(ic, NamedAccessIC::Monomorphic(first));
        assert_eq!(ic.try_get(name, ShapeId(1), Some(0)), Some(first));
        assert_eq!(ic.try_get(name, ShapeId(1), Some(1)), Some(first));
        assert_eq!(ic.try_get(name, ShapeId(1), None), Some(first));
        assert_eq!(ic.try_get(name, ShapeId(2), Some(0)), None);
        assert_eq!(
            ic.try_get(StringRef::from_parts(2, 0), ShapeId(1), Some(0)),
            None
        );

        let refreshed = NamedAccessCase {
            prototype_epoch: None,
            ..first
        };
        ic.record(refreshed);
        assert_eq!(ic, NamedAccessIC::Monomorphic(refreshed));

        // Second shape -> Polymorphic
        let second = NamedAccessCase {
            name,
            receiver_shape: ShapeId(2),
            holder_depth: 1,
            holder_shape: ShapeId(3),
            slot: 1,
            prototype_epoch: Some(0),
        };
        ic.record(second);
        assert!(matches!(ic, NamedAccessIC::Polymorphic(_)));
        assert_eq!(ic.try_get(name, ShapeId(1), Some(1)), Some(refreshed));
        assert_eq!(ic.try_get(name, ShapeId(2), Some(0)), Some(second));
        assert_eq!(ic.try_get(name, ShapeId(3), Some(0)), None);
    }
}
