// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The one error type of this crate; `docs/05-code-organization.md:377`.

use text_core::FontError;

/// Why an operation of this crate could not be carried out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterError {
    /// A surface of zero width or zero height.
    Empty,
    /// A stride below the bytes one row of visible pixels needs.
    Stride,
    /// A surface larger than an address of this machine.
    Overflow,
    /// A slice shorter than `height * stride` bytes.
    TooShort {
        /// What the dimensions need.
        needed: usize,
        /// What the slice holds.
        given: usize,
    },
    /// A texel the format of this surface does not carry.
    Format,
    /// A position outside the visible pixels of a surface.
    OutOfBounds,
    /// A caller slice too short for what the operation writes.
    BufferTooSmall,
    /// An argument outside the domain of a numeric function.
    Domain,
    /// A gamma value outside `(0, 16]`.
    Gamma,
    /// A paint stream whose brackets do not match.
    Stream,
    /// A run naming a face the caller did not supply.
    Face,
    /// A font table refused what this crate asked of it.
    Font(FontError),
}

impl From<FontError> for RasterError {
    fn from(error: FontError) -> Self {
        Self::Font(error)
    }
}

impl core::fmt::Display for RasterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => f.write_str("a surface of zero width or zero height"),
            Self::Stride => f.write_str("a row shorter than the width of the surface"),
            Self::Overflow => f.write_str("a surface larger than this address space"),
            Self::TooShort { needed, given } => {
                write!(f, "a surface of {needed} bytes over a slice of {given}")
            }
            Self::Format => f.write_str("a texel of another pixel format"),
            Self::OutOfBounds => f.write_str("a position outside the surface"),
            Self::BufferTooSmall => f.write_str("a caller slice too short for the operation"),
            Self::Domain => f.write_str("an argument outside the domain of the function"),
            Self::Gamma => f.write_str("a gamma value outside (0, 16]"),
            Self::Stream => f.write_str("a paint stream whose brackets do not match"),
            Self::Face => f.write_str("a run naming a face the caller did not supply"),
            Self::Font(error) => write!(f, "the font refused the operation: {error:?}"),
        }
    }
}
