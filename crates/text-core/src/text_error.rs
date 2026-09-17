// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

/// A caller-provided text workspace or numeric range is insufficient.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextError {
    /// A deterministic text work or size limit was exceeded.
    LimitExceeded,
    /// Fallible owned-buffer allocation failed.
    Allocation,
    /// The requested role has no face.
    NoFont,
    /// A font operation failed.
    Font(crate::FontError),
    /// The requested text range or paragraph constraints are invalid.
    InvalidInput,
    /// Output or scratch capacity is smaller than required.
    BufferTooSmall,
    /// A text index or coordinate exceeds its numeric range.
    Overflow,
}
impl core::fmt::Display for TextError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::LimitExceeded => "text work limit exceeded",
            Self::Allocation => "text allocation failed",
            Self::NoFont => "empty font chain",
            Self::Font(error) => return core::fmt::Display::fmt(error, f),
            Self::InvalidInput => "invalid text input",
            Self::BufferTooSmall => "text buffer too small",
            Self::Overflow => "text arithmetic overflow",
        })
    }
}
impl core::error::Error for TextError {}

impl From<crate::FontError> for TextError {
    fn from(error: crate::FontError) -> Self {
        Self::Font(error)
    }
}
