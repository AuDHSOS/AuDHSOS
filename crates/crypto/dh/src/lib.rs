// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod group;
pub mod modp;

pub use error::DhError;
pub use group::{
    GROUP14_GENERATOR, GROUP14_PRIME, GROUP14_PRIME_BYTES, GROUP14_SECRET_BYTES, group14,
};
pub use modp::ModpGroup;

#[cfg(test)]
mod tests;
