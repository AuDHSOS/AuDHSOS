// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod arp;
pub mod error;
pub mod frame;
pub mod neighbor;

pub use arp::{Operation, Packet};
pub use error::EthError;
pub use frame::{Frame, HEADER_LEN, MAX_FRAME_LEN, MTU, receive};
pub use neighbor::{Event, NeighborCache, NeighborState, Resolution, Timers};

#[cfg(test)]
mod tests;
