// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod fragment;
pub mod header;
pub mod icmp;
pub mod ndp;
pub mod pmtu;
pub mod send;
pub mod slaac;

pub use error::Ipv6Error;
pub use fragment::{piece_of, reassemble};
pub use header::{Fragment, FragmentHeader, Header, Packet, Upper};
pub use icmp::{Message, Unreachable};
pub use ndp::{Discovery, NdpOption, Options, PrefixInformation, Rdnss};
pub use pmtu::PathMtu;
pub use send::{Outgoing, Sender};
pub use slaac::{Configuration, Configured, Dad, DadEvent, DadState, Expired, Router, Server};

#[cfg(test)]
mod tests;
