// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod open;
pub mod serve;

pub use error::refusal;
pub use open::{Clients, MAX_CLIENTS, MAX_OPEN};
pub use serve::{answer, moment};

#[cfg(test)]
mod tests;
