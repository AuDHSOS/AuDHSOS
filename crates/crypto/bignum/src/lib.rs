// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod limbs;
pub mod modulus;

pub use error::BignumError;
pub use limbs::{add_limbs, is_less, montgomery, subtract};
pub use modulus::{MAX_BYTES, MAX_LIMBS, Modulus};

#[cfg(test)]
mod tests;
