// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod cursor;
pub mod state;

pub use cursor::{CURSOR_HEIGHT, CURSOR_SHAPE, CURSOR_WIDTH, Cursor};
pub use state::{Display, Held, MAX_CLIENTS};

#[cfg(test)]
mod tests;
