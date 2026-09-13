// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

extern crate alloc;

pub mod ast;
mod bytes;
pub mod error;
pub mod eval;
pub mod fp;
pub mod func;
pub mod header;
pub mod image;
pub mod keyword;
pub mod number;
pub mod page;
pub mod parse;
pub mod record;
pub mod schema;
pub mod token;
pub mod utf8;
pub mod value;

#[cfg(test)]
mod tests;

pub use error::Error;
pub use header::{Encoding, Header, MAX_PAGE_SIZE, MIN_PAGE_SIZE};
pub use image::{Image, Row, Rows};
pub use keyword::Keyword;
pub use page::{Cell, Kind, Page, Payload};
pub use record::{Record, Serial, Value, Values};
pub use token::{Kind as TokenKind, Lexer, Token};
