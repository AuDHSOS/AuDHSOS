// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(any(test, feature = "test-doubles")), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

#[cfg(any(test, feature = "test-doubles"))]
pub mod doubles;
pub mod request;
pub mod service;
pub mod state;

pub use service::{AUX_FLAG, Line, Lines, pack, service, unpack};
pub use state::{Clients, Input, MAX_PRODUCED, NOBODY, Produced, Subscriber};

#[cfg(test)]
mod tests;
