// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod memory;
pub mod protocols;
pub mod status;
pub mod tables;
pub mod types;
pub mod utf16;

pub use status::Status;
pub use types::{Guid, Handle, TableHeader};

#[cfg(test)]
mod tests;
