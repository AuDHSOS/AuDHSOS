// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod reader;
pub mod tag;
pub mod time;

pub use audhsos_time::CivilTime;
pub use error::DerError;
pub use reader::{BitString, MAX_DEPTH, Oid, Reader};
pub use tag::Tag;
pub use time::{from_generalized_time, from_utc_time};

#[cfg(test)]
mod tests;
