// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Generators for the types of this crate, behind the feature
//! `test-strategies`.

use test_support::generators::{BoxGen, Generator, one_of, pair, range};

use crate::format::{Color, PixelFormat};
use crate::rect::Rect;

/// Both pixel formats.
const FORMATS: [PixelFormat; 2] = [PixelFormat::Rgbx8888, PixelFormat::Bgrx8888];

/// A pixel format.
#[must_use]
pub fn any_pixel_format() -> BoxGen<PixelFormat> {
    one_of(FORMATS.to_vec()).boxed()
}

/// A color.
#[must_use]
pub fn any_color() -> BoxGen<Color> {
    pair(range(0u8..=255), pair(range(0u8..=255), range(0u8..=255)))
        .map(|(r, (g, b))| Color::new(r, g, b))
        .boxed()
}

/// A rectangle whose position and size lie below `bound`, empty ones and
/// ones that reach past a surface of that size included.
#[must_use]
pub fn any_rect(bound: u32) -> BoxGen<Rect> {
    pair(
        pair(range(0u32..=bound), range(0u32..=bound)),
        pair(range(0u32..=bound), range(0u32..=bound)),
    )
    .map(|((x, y), (w, h))| Rect::new(x, y, w, h))
    .boxed()
}
