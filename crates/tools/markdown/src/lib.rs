// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![forbid(unsafe_code)]
#![expect(
    clippy::doc_markdown,
    reason = "the prose of this crate names CommonMark, which is a standard and not an identifier"
)]
#![doc = include_str!("../README.md")]

pub mod block;
pub mod inline;
pub mod parse;

pub use block::{Align, Block};
pub use inline::{Inline, plain};
pub use parse::document as parse;

#[cfg(test)]
mod tests;
