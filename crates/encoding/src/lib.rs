// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod base64;
pub mod error;
pub mod hex;
pub mod pem;
#[cfg(any(test, feature = "test-strategies"))]
pub mod strategies;

pub use error::EncodingError;
pub use pem::Block;

#[cfg(test)]
mod tests;
