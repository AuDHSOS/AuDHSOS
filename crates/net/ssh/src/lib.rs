// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod cipher;
pub mod error;
pub mod exchange;
pub mod ident;
pub mod kex;
pub mod keys;
pub mod msg;
pub mod packet;
pub mod wire;

pub use cipher::ChaChaPoly;
pub use error::SshError;
pub use exchange::{Ephemeral, HashInput, Method, Reply, exchange_hash};
pub use ident::Greeting;
pub use kex::{Choice, KexInit, Proposal, negotiate};
pub use keys::{Key, derive};
pub use packet::{Decoded, Decoder, Encoder, SequenceNumber};
pub use wire::{Mpint, NameList, Names, Reader, Writer};

#[cfg(test)]
mod tests;
