// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod drive;
pub mod error;
pub mod handle;
pub mod queue;
pub mod receive;
pub mod resolve;
pub mod select;
pub mod stack;

pub use drive::{all_nodes, solicited_node};
pub use error::StackError;
pub use handle::{Handle, Slots};
pub use queue::Queue;
pub use resolve::Connecting;
pub use select::{common_prefix, order_destinations, policy, scope, source_for};
pub use stack::{
    ADDRESSES, ADVERTISED, CANDIDATES, Config, FRAME_LEN, HEADER_LEN, HELD, NEIGHBORS, PATHS,
    PAYLOAD_LEN, PREFIXES, REASSEMBLY, ROUTES, SLOTS, Stack,
};

#[cfg(test)]
mod tests;
