// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod exchange;

pub use error::HttpsError;
pub use exchange::{Buffers, Handler, MAX_FIELDS, Refused, Transport, exchange};

#[cfg(test)]
mod tests;
