// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod font;
pub mod format;
pub mod present;
pub mod rect;
#[cfg(feature = "test-strategies")]
pub mod strategies;
pub mod surface;

pub use font::{GLYPH_HEIGHT, GLYPH_WIDTH, GLYPHS, draw_text, glyph};
pub use format::{BYTES_PER_PIXEL, Color, PixelFormat};
pub use present::{PixelSink, PresentError, present};
pub use rect::{DAMAGE_CAPACITY, Damage, Rect};
pub use surface::{Surface, SurfaceError};

#[cfg(test)]
mod tests;
