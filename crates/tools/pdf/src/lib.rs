// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod document;
pub mod font;
pub mod outline;
pub mod page;
pub mod units;

pub use document::Document;
pub use font::{Font, encode};
pub use outline::Entry;
pub use page::{Link, LinkTarget, Page};
pub use units::{Color, Mils, PageSize, number, pt};

#[cfg(test)]
mod tests;
