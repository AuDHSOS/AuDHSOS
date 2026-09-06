// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod bytes;
pub mod console;
pub mod label;
pub mod memory;
pub mod name;

pub use bytes::Bytes;
pub use console::{Chunk, MAX_CHUNK};
pub use label::{Label, ProtoError, Protocol, VERSION};
pub use name::{MAX_NAME, Name};

#[cfg(test)]
mod tests;
