// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod field;
pub mod request;
pub mod response;

pub use error::HttpError;
pub use field::{is_token, is_value, same_name, trim};
pub use request::{Method, Request};
pub use response::{Decoder, Event, Head, Headers, MAX_HEADER_LINE, MAX_STATUS_LINE, Status};

#[cfg(test)]
mod tests;
