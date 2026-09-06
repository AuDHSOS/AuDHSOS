// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod client;
pub mod error;
pub mod message;
pub mod option;

pub use client::{
    CLIENT_PORT, Client, Config, Lease, MAX_DNS_SERVERS, Outgoing, SERVER_PORT, State,
};
pub use error::DhcpError;
pub use message::{
    FIXED_LEN, HARDWARE_ETHERNET, MAGIC_COOKIE, MAX_MESSAGE_LEN, MIN_MESSAGE_LEN, Message,
    MessageType, Op,
};
pub use option::{OptionCode, Options, write_option};

#[cfg(test)]
mod tests;
