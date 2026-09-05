// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod fragment;
pub mod header;
pub mod icmp;
pub mod route;
pub mod send;

pub use error::IpError;
pub use fragment::{Assembled, Fragments, Reassembler, fragment};
pub use header::{Datagram, Header, Quoted};
pub use icmp::{Message, RateLimit, TokenBucket, Unreachable};
pub use route::{NextHop, Route, RoutingTable};
pub use send::{Interface, Outgoing, Sender, Sent};

#[cfg(test)]
mod tests;
