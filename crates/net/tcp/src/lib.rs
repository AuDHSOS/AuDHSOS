// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod congestion;
pub mod connection;
pub mod error;
pub mod recv;
pub mod rto;
pub mod segment;
pub mod send;
pub mod seq;
pub mod state;
pub mod table;

pub use congestion::Congestion;
pub use connection::{Config, Connection, Endpoint};
pub use error::TcpError;
pub use recv::{MAX_HOLES, RecvBuffer};
pub use rto::Rto;
pub use segment::{Flags, HEADER_LEN, Segment, reset_for};
pub use send::SendBuffer;
pub use seq::{SeqNumber, is_acceptable};
pub use state::State;
pub use table::{ConnectionId, Connections, Delivery};

#[cfg(test)]
mod tests;
