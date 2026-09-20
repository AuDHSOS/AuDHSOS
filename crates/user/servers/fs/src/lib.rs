// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod open;
pub mod partition;
pub mod serve;
pub mod volume;

pub use error::{refusal, table_refusal};
pub use open::{Clients, MAX_CLIENTS, MAX_OPEN, ROOTS, Which};
pub use partition::Partition;
pub use serve::{NOBODY, Volumes, answer, moment};
pub use volume::{is_partitioned, mount};

#[cfg(test)]
mod tests;
