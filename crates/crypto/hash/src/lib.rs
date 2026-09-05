// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod hash;
pub mod hkdf;
pub mod hmac;
pub mod sha256;
pub mod sha512;

pub use error::HashError;
pub use hash::Hash;
pub use hkdf::{Prk, expand, extract};
pub use hmac::Hmac;
pub use sha256::Sha256;
pub use sha512::{Sha384, Sha512};

#[cfg(test)]
mod tests;
