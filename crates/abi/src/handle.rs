// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The handle encoding.
//!
//! Invariants: a handle is never `0`; the generation is never `0`; the index
//! occupies the low [`HANDLE_INDEX_BITS`] bits and the generation the high
//! [`HANDLE_GENERATION_BITS`] bits, and the two never overlap.

use core::num::NonZeroU64;

use crate::layout::{HANDLE_GENERATION_BITS, HANDLE_INDEX_BITS};

/// A per-process name for a capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Handle(NonZeroU64);

const INDEX_MASK: u64 = (1 << HANDLE_INDEX_BITS) - 1;

impl Handle {
    /// The widest handle there is: the last index of a table at the last
    /// generation.
    ///
    /// No table hands it out — a generation counts up from one and an index
    /// is bounded by the capacity of the table — and it is here so that code
    /// which has to name a handle without an `Option` can, in a `const`
    /// where a fallible constructor cannot be used.
    pub const MAX: Handle = Handle(NonZeroU64::MAX);

    /// Builds a handle from a table index and a generation. Returns `None`
    /// for generation `0`, which is reserved so that the raw value is never
    /// `0`.
    #[must_use]
    #[expect(clippy::as_conversions, reason = "widening casts in a const fn")]
    pub const fn new(index: u32, generation: u32) -> Option<Self> {
        if generation == 0 {
            return None;
        }
        let raw = ((generation as u64) << HANDLE_INDEX_BITS) | (index as u64);
        match NonZeroU64::new(raw) {
            Some(value) => Some(Handle(value)),
            None => None,
        }
    }

    /// Decodes a raw value as it appears in the IPC buffer. Returns `None`
    /// for `0` and for a generation of `0`.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Option<Self> {
        match NonZeroU64::new(raw) {
            Some(value) if (raw >> HANDLE_INDEX_BITS) != 0 => Some(Handle(value)),
            _ => None,
        }
    }

    /// The raw value as it appears in the IPC buffer.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0.get()
    }

    /// The index into the handle table.
    #[must_use]
    #[expect(clippy::as_conversions, reason = "the mask keeps only the low 32 bits")]
    pub const fn index(self) -> u32 {
        // The mask keeps only the low 32 bits, so the narrowing cannot lose
        // information.
        (self.0.get() & INDEX_MASK) as u32
    }

    /// The generation of the table slot the handle was created for.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "the shift leaves only the high 32 bits"
    )]
    pub const fn generation(self) -> u32 {
        // The shift leaves only the high 32 bits.
        (self.0.get() >> HANDLE_INDEX_BITS) as u32
    }
}

const _: () = assert!(HANDLE_INDEX_BITS + HANDLE_GENERATION_BITS == 64);
