// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(any(test, feature = "test-doubles")), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod blk;
pub mod common;
pub mod config;
#[cfg(any(test, feature = "test-doubles"))]
pub mod doubles;
pub mod error;
pub mod features;
pub mod registers;
pub mod request;

pub use blk::{Blk, Chain, ISR_CONFIG, ISR_QUEUE, Rings, Segment, Vectors};
pub use common::{Common, ENABLED, NO_VECTOR, REQUEST_QUEUE};
pub use config::SECTOR_LEN;
pub use error::BlkError;
pub use features::{WANTED, name, refused};
pub use registers::{Registers, Structure, Width};
pub use request::{HEADER_LEN, Kind, Request, S_IOERR, S_OK, S_UNSUPP, STATUS_LEN, status};

#[cfg(test)]
mod tests;
