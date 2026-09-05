// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod chacha;
#[cfg(any(test, feature = "test-doubles"))]
pub mod doubles;
pub mod error;
pub mod source;

pub use chacha::{ChaChaRng, RESEED_BYTES};
pub use error::{EntropyError, RngError};
pub use source::{Entropy, Rng};

#[cfg(test)]
mod tests;
