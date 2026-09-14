// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

extern crate alloc;

pub mod agg;
pub mod ast;
mod bytes;
pub mod change;
pub mod db;
pub mod error;
pub mod eval;
pub mod format;
pub mod fp;
pub mod func;
pub mod header;
pub mod image;
pub mod journal;
pub mod keyword;
pub mod number;
pub mod page;
pub mod parse;
pub mod pragma;
pub mod random;
pub mod record;
pub mod schema;
pub mod token;
pub mod tree;
pub mod utf8;
pub mod value;
pub mod wal;

#[cfg(test)]
mod tests;

pub use error::Error;
pub use header::{Encoding, Header, MAX_PAGE_SIZE, MIN_PAGE_SIZE};
pub use image::{Image, Row, Rows};
pub use keyword::Keyword;
pub use page::{Cell, Kind, Page, Payload};
pub use record::{Record, Serial, Value, Values};
pub use token::{Kind as TokenKind, Lexer, Token};
