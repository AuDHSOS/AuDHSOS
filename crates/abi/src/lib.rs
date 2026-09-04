// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod handle;
pub mod layout;
pub mod object;
pub mod rights;

pub use error::Error;
pub use handle::Handle;
pub use object::ObjectType;
pub use rights::Rights;

#[cfg(test)]
mod tests;
