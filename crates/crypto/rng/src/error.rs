// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why randomness was not delivered.

use core::fmt;

/// Why an entropy source produced nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntropyError {
    /// The source is not available, or the hardware it reads reported a
    /// failure. A caller cannot recover by asking again immediately.
    Unavailable,
}

impl fmt::Display for EntropyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntropyError::Unavailable => f.write_str("the entropy source is unavailable"),
        }
    }
}

/// Why a generator produced nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RngError {
    /// The entropy source the generator reseeds from failed.
    Entropy(EntropyError),
    /// A generator that replays a fixed script has no bytes left. Only the
    /// scripted generator of the test doubles reports this; the real one
    /// never runs out.
    Exhausted,
}

impl fmt::Display for RngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RngError::Entropy(error) => write!(f, "reseeding failed: {error}"),
            RngError::Exhausted => f.write_str("the scripted generator has no bytes left"),
        }
    }
}

impl From<EntropyError> for RngError {
    fn from(error: EntropyError) -> RngError {
        RngError::Entropy(error)
    }
}
