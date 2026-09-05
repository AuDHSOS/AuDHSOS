// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod image;
pub mod sections;
#[cfg(any(test, feature = "test-strategies"))]
pub mod strategies;

pub use error::ElfError;
pub use image::{Constraints, Image, MAX_SEGMENTS, Segment, parse};
pub use sections::{SHDR_LEN, Section, Sections, sections};

#[cfg(test)]
mod tests;
