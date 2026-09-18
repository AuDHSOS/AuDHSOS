// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod cache;
mod composite;
mod draw;
mod error;
mod fill;
mod flatten;
mod gamma;
mod gradient;
pub mod math;
mod mono;
mod origin;
mod outline;
mod paint;
mod pixel;
mod surface;
mod transform;

pub use cache::{CacheKey, Entry, Glyph, GlyphCache};
pub use composite::composite;
pub use draw::{Context, draw};
pub use error::RasterError;
pub use fill::{Cell, fill};
pub use flatten::{
    Edge, MAX_SEGMENTS, TOLERANCE, cubic_segments, flatten_cff, flatten_glyf, quadratic_segments,
};
pub use gamma::{Gamma, LINEAR_ONE};
pub use gradient::Gradient;
pub use mono::blend_mono;
pub use origin::{Origin, SUBPIXEL_POSITIONS, quantize};
pub use outline::{OutlineScratch, outline_edges};
pub use paint::{Bounds, PaintScratch, draw_color_glyph};
pub use pixel::{Paint, Pixel};
pub use surface::{Format, Mask, Surface, Texel};
pub use transform::{Affine, Invertible};

#[cfg(test)]
mod tests;
