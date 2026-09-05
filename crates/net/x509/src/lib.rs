// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod algorithm;
#[cfg(any(test, feature = "test-certificates"))]
pub mod builder;
pub mod certificate;
pub mod error;
pub mod oid;

pub use algorithm::{SignatureAlgorithm, SubjectPublicKey};
pub use certificate::{BasicConstraints, Certificate, KeyUsage, Validity};
pub use error::X509Error;

#[cfg(test)]
mod tests;
