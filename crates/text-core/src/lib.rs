// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod layout;
#[cfg(feature = "alloc")]
pub use layout::{Layout, layout, measure};
pub use layout::{layout_into, measure_into};
pub mod bidi;
pub mod cff;
pub mod cmap;
pub mod colr;
mod error;
mod fixed;
pub mod glyf;
pub mod metrics;
mod read;
pub mod resolve;
pub mod segment;
pub use resolve::{FontSet, Language, Role, TextStyle};
mod sfnt;
pub mod shape;
mod text_error;
pub use text_error::TextError;
pub mod unicode;
pub mod variation;

pub use error::FontError;
pub use fixed::Fixed;
pub use sfnt::{Font, FontCollection, MAX_FACES, MAX_TABLES, MAX_TOTAL_TABLES, OutlineKind, Table};

#[cfg(test)]
mod tests;
