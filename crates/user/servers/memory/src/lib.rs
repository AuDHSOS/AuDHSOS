// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(any(test, feature = "test-doubles")), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

#[cfg(any(test, feature = "test-doubles"))]
pub mod doubles;
pub mod pages;
pub mod store;

pub use pages::Pages;
pub use store::{Held, Object, Store};

#[cfg(test)]
mod tests;
