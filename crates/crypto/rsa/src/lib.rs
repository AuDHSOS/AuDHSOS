// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod hash;
pub mod key;
pub mod pkcs1;
pub mod pss;
#[cfg(feature = "test-signing")]
pub mod signing;

pub(crate) mod window;

pub use error::RsaError;
pub use hash::{HashId, MAX_DIGEST};
pub use key::PublicKey;

#[cfg(test)]
mod tests;
