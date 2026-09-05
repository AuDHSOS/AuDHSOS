// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod civil;
pub mod error;
pub mod instant;
#[cfg(any(test, feature = "test-strategies"))]
pub mod strategies;
pub mod unix;

pub use civil::{
    CivilTime, MAX_YEAR, MIN_YEAR, civil_from_days, days_from_civil, days_in_month, is_leap_year,
};
pub use error::TimeError;
pub use instant::{Duration, Instant};
pub use unix::UnixTime;

#[cfg(test)]
mod tests;
