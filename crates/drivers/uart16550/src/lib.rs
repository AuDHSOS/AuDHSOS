// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(any(test, feature = "test-doubles")), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

#[cfg(any(test, feature = "test-doubles"))]
pub mod doubles;
pub mod uart;

pub use uart::{POLL_LIMIT, Register, Registers, Uart16550, UartError};

#[cfg(test)]
mod tests;
