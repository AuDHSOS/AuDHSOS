// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod codec;
pub mod error;
pub mod handshake;
pub mod keys;
pub mod protection;
pub mod record;
pub mod secret;
pub mod suite;
pub mod transcript;

pub use error::TlsError;
pub use protection::RecordProtection;
pub use record::{ContentType, Header};
pub use secret::Secret;
pub use suite::CipherSuite;
pub use transcript::Transcript;

#[cfg(test)]
mod tests;
