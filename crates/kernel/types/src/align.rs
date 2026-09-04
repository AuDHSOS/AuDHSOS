// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Power-of-two alignments.
//!
//! Invariant: an [`Alignment`] is always a power of two, so its mask is
//! `alignment - 1`.

use audhsos_abi::layout::PAGE_SIZE;

use crate::Error;

/// A power-of-two alignment in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Alignment(u64);

impl Alignment {
    /// Byte alignment.
    pub const BYTE: Alignment = Alignment(1);

    /// Page alignment.
    pub const PAGE: Alignment = Alignment(PAGE_SIZE);

    /// Validates that `bytes` is a power of two.
    ///
    /// # Errors
    ///
    /// `InvalidAlignment` if `bytes` is zero or not a power of two.
    pub const fn new(bytes: u64) -> Result<Self, Error> {
        if bytes.is_power_of_two() {
            Ok(Alignment(bytes))
        } else {
            Err(Error::InvalidAlignment(bytes))
        }
    }

    /// The alignment in bytes.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.0
    }

    /// `log2` of the alignment.
    #[must_use]
    pub const fn log2(self) -> u32 {
        self.0.trailing_zeros()
    }

    /// The mask selecting the bits below the alignment.
    #[must_use]
    pub const fn mask(self) -> u64 {
        self.0.wrapping_sub(1)
    }

    /// `true` if `value` is a multiple of the alignment.
    #[must_use]
    pub const fn is_aligned(self, value: u64) -> bool {
        value & self.mask() == 0
    }

    /// The largest multiple of the alignment that is not above `value`.
    #[must_use]
    pub const fn align_down(self, value: u64) -> u64 {
        value & !self.mask()
    }

    /// The smallest multiple of the alignment that is not below `value`, or
    /// `None` if it does not fit into `u64`.
    #[must_use]
    pub const fn align_up(self, value: u64) -> Option<u64> {
        match value.checked_add(self.mask()) {
            Some(bumped) => Some(self.align_down(bumped)),
            None => None,
        }
    }
}
