// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod header;
pub mod route;

pub use error::IpError;
pub use header::{Datagram, Header};
pub use route::{NextHop, Route, RoutingTable};

#[cfg(test)]
mod tests;
