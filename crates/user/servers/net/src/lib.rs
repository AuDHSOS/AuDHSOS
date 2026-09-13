// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod memory;
pub mod server;
pub mod sockets;

pub use memory::{CONNECTIONS, DATAGRAMS, OUTGOING, Pool, REGION_BYTES, SOCKETS, WINDOW};
pub use server::{Rings, Server, refusal};
pub use sockets::{Entry, Kind, MAX_SOCKETS, Sockets};

#[cfg(test)]
mod tests;
