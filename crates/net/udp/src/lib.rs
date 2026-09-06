// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod datagram;
pub mod error;
pub mod ring;
pub mod socket;

pub use datagram::{ChecksumPolicy, Datagram, HEADER_LEN, MAX_PAYLOAD_LEN};
pub use error::UdpError;
pub use ring::{DropReason, Received, Ring, record_len};
pub use socket::{Delivery, Socket, SocketId, Sockets};

#[cfg(test)]
mod tests;
