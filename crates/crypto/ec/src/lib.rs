// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod ed25519;
pub mod error;
pub mod fe25519;
pub mod jacobian;
pub mod montgomery;
pub mod p256;
pub mod p384;
pub mod scalar;
pub mod x25519;

pub use ed25519::Point;
pub use error::EcError;
pub use fe25519::Fe;
pub use scalar::Scalar;
pub use x25519::{PUBLIC_LEN, SCALAR_LEN, base_point, x25519};

#[cfg(test)]
mod tests;
