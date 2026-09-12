// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod cursor;
pub mod state;

pub use cursor::{ARROW_SHAPE, CURSOR_HEIGHT, CURSOR_WIDTH, Cursor, RESIZE_SHAPE, pixel_of};
pub use state::{Display, Held, MAX_CLIENTS, NOBODY};
// The shape of the sprite is a field of the protocol, and the end-to-end
// run asks this crate which pixels a shape has. Re-exported so that both
// name one type.
pub use user_proto::display::CursorShape;

#[cfg(test)]
mod tests;
