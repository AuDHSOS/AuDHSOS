// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod addr;
pub mod checksum;
pub mod cursor;
pub mod error;
pub mod protocol;

pub use addr::{Ipv4Addr, Ipv4Cidr, MacAddr, Port};
pub use checksum::{Checksum, checksum, is_valid, transport_v4};
pub use cursor::{Reader, Writer};
pub use error::WireError;
pub use protocol::{EtherType, Protocol};

#[cfg(test)]
mod tests;
