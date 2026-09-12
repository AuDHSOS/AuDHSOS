// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod packet;
pub mod wire;

pub use error::SshError;
pub use packet::{Decoded, Decoder, Encoder, SequenceNumber};
pub use wire::{Mpint, NameList, Names, Reader, Writer};

#[cfg(test)]
mod tests;
