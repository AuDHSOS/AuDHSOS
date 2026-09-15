// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Heap contexts for captured lexical bindings.

use super::value::{VALUE_UNDEFINED, Value};
use alloc::vec::Vec;

/// Generation-checked reference to a heap context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextRef(u32);

const OLD_GENERATION_BIT: u32 = 1 << 31;
const YOUNG_GENERATION_SHIFT: u32 = 11;
const YOUNG_INDEX_MASK: u32 = (1 << YOUNG_GENERATION_SHIFT) - 1;
const OLD_GENERATION_SHIFT: u32 = 23;
const OLD_INDEX_MASK: u32 = (1 << OLD_GENERATION_SHIFT) - 1;

impl ContextRef {
    /// Creates a reference into the active Nursery semispace.
    #[must_use]
    pub(crate) const fn young(index: u32, generation: u32) -> Self {
        Self((index & YOUNG_INDEX_MASK) | (generation << YOUNG_GENERATION_SHIFT))
    }

    /// Creates a reference into the Old Generation.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "widening the u8 handle generation to u32 is lossless"
    )]
    pub(crate) const fn old(index: u32, generation: u8) -> Self {
        Self(
            OLD_GENERATION_BIT
                | (index & OLD_INDEX_MASK)
                | ((generation as u32) << OLD_GENERATION_SHIFT),
        )
    }

    /// Returns whether this reference addresses the Old Generation.
    #[must_use]
    pub const fn is_old(self) -> bool {
        self.0 & OLD_GENERATION_BIT != 0
    }

    /// Returns whether this reference addresses the Nursery.
    #[must_use]
    pub const fn is_young(self) -> bool {
        !self.is_old()
    }

    /// Returns the generation-local context index.
    #[must_use]
    pub const fn index(self) -> u32 {
        if self.is_old() {
            self.0 & OLD_INDEX_MASK
        } else {
            self.0 & YOUNG_INDEX_MASK
        }
    }

    /// Returns the generation used to reject stale reused references.
    #[must_use]
    pub const fn generation(self) -> u32 {
        if self.is_old() {
            (self.0 & !OLD_GENERATION_BIT) >> OLD_GENERATION_SHIFT
        } else {
            self.0 >> YOUNG_GENERATION_SHIFT
        }
    }
}

/// Captured lexical environment with an optional outer context.
#[derive(Clone, Debug)]
pub struct Context {
    /// Outer captured lexical environment.
    pub parent: Option<ContextRef>,
    /// Mutable captured binding cells.
    pub slots: Vec<Value>,
}

impl Context {
    /// Creates a context whose bindings start as `undefined`.
    #[must_use]
    pub fn new(parent: Option<ContextRef>, slot_count: usize) -> Self {
        Self {
            parent,
            slots: alloc::vec![VALUE_UNDEFINED; slot_count],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_separate_generation_space_and_context_slots() {
        let young = ContextRef::young(1023, 0xF_FFFF);
        assert!(young.is_young());
        assert_eq!(young.index(), 1023);
        assert_eq!(young.generation(), 0xF_FFFF);

        let old = ContextRef::old(0x7F_FFFF, 0xFF);
        assert!(old.is_old());
        assert_eq!(old.index(), 0x7F_FFFF);
        assert_eq!(old.generation(), 0xFF);

        let context = Context::new(Some(old), 2);
        assert_eq!(context.parent, Some(old));
        assert_eq!(context.slots, [VALUE_UNDEFINED; 2]);
    }
}
