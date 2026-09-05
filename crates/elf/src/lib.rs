// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod image;
#[cfg(any(test, feature = "test-strategies"))]
pub mod strategies;

pub use error::ElfError;
pub use image::{Constraints, Image, MAX_SEGMENTS, Segment, parse};

#[cfg(test)]
mod tests;
