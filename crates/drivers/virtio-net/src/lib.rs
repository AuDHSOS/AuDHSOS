// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(any(test, feature = "test-doubles")), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod common;
#[cfg(any(test, feature = "test-doubles"))]
pub mod doubles;
pub mod error;
pub mod features;
pub mod frames;
pub mod init;
pub mod net;
pub mod queues;
pub mod registers;
pub mod rx;
pub mod tx;

pub use common::{Common, ENABLED, NO_VECTOR, RECEIVE_QUEUE, TRANSMIT_QUEUE};
pub use error::NetError;
pub use features::{WANTED, name, refused};
pub use frames::Frames;
pub use init::{ISR_CONFIG, ISR_QUEUE, NO_BUFFER, Net};
pub use net::{HEADER_LEN, MAC_LEN, mac, write_header};
pub use queues::{Configured, Queues, Rings, Side, Vectors};
pub use registers::{Registers, Structure, Width};

#[cfg(test)]
mod tests;
