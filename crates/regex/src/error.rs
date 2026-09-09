// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Invalid, unsupported and exhausted inputs are distinct typed errors.

use core::fmt;

/// A compile or matching failure, never an implicit fallback to another engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Invalid pattern syntax at a UTF-16 code-unit offset.
    Syntax {
        /// Offset of the offending token.
        offset: usize,
        /// The grammar constraint that failed.
        message: &'static str,
    },
    /// Recognized syntax outside the implemented finite-automaton subset.
    Unsupported {
        /// Offset in the pattern.
        offset: usize,
        /// Unsupported feature; no fallback is performed.
        feature: &'static str,
    },
    /// A configured resource was exhausted.
    Limit {
        /// The resource that ran out.
        resource: &'static str,
    },
    /// Internal invariant failure. Compiled instructions are not public inputs.
    InvalidProgram,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { offset, message } => {
                write!(f, "regex syntax at unit {offset}: {message}")
            }
            Self::Unsupported { offset, feature } => {
                write!(f, "unsupported regex at unit {offset}: {feature}")
            }
            Self::Limit { resource } => write!(f, "regex limit: {resource}"),
            Self::InvalidProgram => f.write_str("invalid regex program"),
        }
    }
}
impl core::error::Error for Error {}
