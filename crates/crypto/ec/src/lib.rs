// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod fe25519;
pub mod x25519;

pub use error::EcError;
pub use fe25519::Fe;
pub use x25519::{PUBLIC_LEN, SCALAR_LEN, base_point, x25519};

#[cfg(test)]
mod tests;
