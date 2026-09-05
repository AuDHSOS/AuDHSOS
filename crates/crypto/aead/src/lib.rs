// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod aead;
pub mod chacha20;
pub mod chachapoly;
pub mod error;
pub mod poly1305;

pub use aead::{Aead, TAG_LEN, Tag};
pub use chacha20::ChaCha20;
pub use chachapoly::ChaCha20Poly1305;
pub use error::AeadError;
pub use poly1305::Poly1305;

#[cfg(test)]
mod tests;
