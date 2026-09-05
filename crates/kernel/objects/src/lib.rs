// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod pool;
pub mod quota;
#[cfg(any(test, feature = "test-strategies"))]
pub mod strategies;

pub use pool::{ObjectId, Pool, PoolError};
pub use quota::{Quota, QuotaExceeded};

#[cfg(test)]
mod tests;
