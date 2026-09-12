// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The pointer sprite, and what stood on the screen under it.
//!
//! The sprite is drawn straight onto the screen after every presentation
//! and taken off again before the next one, so what a client presents never
//! has to know about it. Taking it off means putting back the pixels that
//! were saved when it was drawn, which is why the cursor carries a small
//! block of them.
//!
//! A shape is one bitmap. Which of its pixels are the white body and
//! which the black edge follows from the shape itself: a pixel whose four
//! neighbours all belong to the shape is body, and every other pixel of it
//! is edge. So there is one table per shape to keep right instead of two
//! that have to agree.
//!
//! There are two shapes: the arrow, and the double arrow shown while a
//! window is being resized. Both are drawn from the position the pointer
//! is at, so swapping the shape changes what is drawn and nothing else:
//! no hot spot moves and no saved block changes size.
//!
//! Invariant: the saved block belongs to the position the sprite was last
//! drawn at, so restoring it puts the pixels back where they came from.

use gfx::{Color, Rect, Surface};
use user_proto::display::CursorShape;

/// Width of the sprite in pixels.
pub const CURSOR_WIDTH: u32 = 16;

/// Height of the sprite in pixels.
pub const CURSOR_HEIGHT: u32 = 16;

/// How many pixels the saved block holds.
const SAVED_PIXELS: usize = 256;

/// The arrow, one bit per pixel, the leftmost pixel in the highest bit. It
/// is the arrow of this project and comes from nowhere else.
pub const ARROW_SHAPE: [u16; 16] = [
    0b1000_0000_0000_0000,
    0b1100_0000_0000_0000,
    0b1110_0000_0000_0000,
    0b1111_0000_0000_0000,
    0b1111_1000_0000_0000,
    0b1111_1100_0000_0000,
    0b1111_1110_0000_0000,
    0b1111_1111_0000_0000,
    0b1111_1111_1000_0000,
    0b1111_1111_1100_0000,
    0b1111_1110_0000_0000,
    0b1110_0111_0000_0000,
    0b1100_0111_0000_0000,
    0b0000_0011_1000_0000,
    0b0000_0011_1000_0000,
    0b0000_0001_0000_0000,
];

/// The double arrow of a resize, drawn the same way. It runs from the top
/// left to the bottom right, the diagonal a corner is dragged on, and is
/// the same under a turn of half a circle, so neither end is the tip.
pub const RESIZE_SHAPE: [u16; 16] = [
    0b1111_0000_0000_0000,
    0b1111_1000_0000_0000,
    0b1111_1100_0000_0000,
    0b1111_1100_0000_0000,
    0b0111_1100_0000_0000,
    0b0011_1110_0000_0000,
    0b0000_0111_0000_0000,
    0b0000_0011_1000_0000,
    0b0000_0001_1100_0000,
    0b0000_0000_1110_0000,
    0b0000_0000_0111_1100,
    0b0000_0000_0011_1110,
    0b0000_0000_0011_1111,
    0b0000_0000_0011_1111,
    0b0000_0000_0001_1111,
    0b0000_0000_0000_1111,
];

/// The rows of `shape`.
const fn rows_of(shape: CursorShape) -> &'static [u16; 16] {
    match shape {
        CursorShape::Arrow => &ARROW_SHAPE,
        CursorShape::Resize => &RESIZE_SHAPE,
    }
}

/// The color of the body of the sprite.
const BODY: Color = Color::WHITE;

/// The color of its edge.
const EDGE: Color = Color::BLACK;

/// The row above `row` of `rows`, or nothing above the first.
fn above(rows: &[u16; 16], row: usize) -> u16 {
    row.checked_sub(1)
        .and_then(|index| rows.get(index))
        .copied()
        .unwrap_or(0)
}

/// The row below `row` of `rows`, or nothing below the last.
fn below(rows: &[u16; 16], row: usize) -> u16 {
    rows.get(row.saturating_add(1)).copied().unwrap_or(0)
}

/// The pixels of `row` that lie inside the shape on every side.
fn body_of(rows: &[u16; 16], row: usize) -> u16 {
    let bits = rows.get(row).copied().unwrap_or(0);
    bits & (bits << 1) & (bits >> 1) & above(rows, row) & below(rows, row)
}

/// The color of the pixel at `column` of `row` of `shape`, or `None` where
/// the sprite shows what is behind it.
///
/// It is public because the end-to-end run checks a picture of the screen
/// against the sprite it should hold, and asking this is the one way to do
/// that without a second copy of the rule that decides body from edge.
#[must_use]
pub fn pixel_of(shape: CursorShape, row: usize, column: u32) -> Option<Color> {
    // Not saturating: a column beyond the last would come back as the
    // last, and a shape whose last column is set would answer for a pixel
    // it does not have.
    let bit = 1_u16.checked_shl(CURSOR_WIDTH.checked_sub(1)?.checked_sub(column)?)?;
    let rows = rows_of(shape);
    let bits = rows.get(row).copied().unwrap_or(0);
    if bits & bit == 0 {
        return None;
    }
    if body_of(rows, row) & bit == 0 {
        Some(EDGE)
    } else {
        Some(BODY)
    }
}

/// Where the pointer is, and what the screen held under it.
#[derive(Clone, Copy, Debug)]
pub struct Cursor {
    /// Column of the hot spot.
    x: u32,
    /// Row of the hot spot.
    y: u32,
    /// Whether the sprite is drawn at all.
    visible: bool,
    /// Which sprite it is.
    shape: CursorShape,
    /// Where the sprite stands on the screen, when it stands there.
    drawn: Option<(u32, u32)>,
    /// The pixels that were there before, row by row.
    saved: [Color; SAVED_PIXELS],
}

impl Default for Cursor {
    fn default() -> Self {
        Cursor::new()
    }
}

impl Cursor {
    /// A cursor at the origin, not yet drawn.
    #[must_use]
    pub const fn new() -> Self {
        Cursor {
            x: 0,
            y: 0,
            visible: true,
            shape: CursorShape::Arrow,
            drawn: None,
            saved: [Color::BLACK; SAVED_PIXELS],
        }
    }

    /// Where the hot spot is.
    #[must_use]
    pub const fn position(&self) -> (u32, u32) {
        (self.x, self.y)
    }

    /// Whether the sprite is shown.
    #[must_use]
    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    /// Which sprite it shows.
    #[must_use]
    pub const fn shape(&self) -> CursorShape {
        self.shape
    }

    /// Whether the sprite stands on the screen at the moment.
    #[must_use]
    pub const fn is_drawn(&self) -> bool {
        self.drawn.is_some()
    }

    /// The rectangle the sprite covers at its position.
    #[must_use]
    pub const fn bounds(&self) -> Rect {
        Rect::new(self.x, self.y, CURSOR_WIDTH, CURSOR_HEIGHT)
    }

    /// Moves the hot spot, clamped to `screen`, and says which sprite
    /// stands there and whether it is shown.
    pub const fn place(&mut self, x: u32, y: u32, visible: bool, shape: CursorShape, screen: Rect) {
        let last_column = screen.w.saturating_sub(1);
        let last_row = screen.h.saturating_sub(1);
        self.x = if x > last_column { last_column } else { x };
        self.y = if y > last_row { last_row } else { y };
        self.visible = visible;
        self.shape = shape;
    }

    /// Puts back what stood under the sprite, if it stands anywhere.
    pub fn erase(&mut self, screen: &mut Surface<'_>) {
        let Some((x, y)) = self.drawn.take() else {
            return;
        };
        for row in 0..CURSOR_HEIGHT {
            for column in 0..CURSOR_WIDTH {
                let index =
                    usize::try_from(row.saturating_mul(CURSOR_WIDTH).saturating_add(column))
                        .unwrap_or(0);
                let color = self.saved.get(index).copied().unwrap_or(Color::BLACK);
                screen.set_pixel(x.saturating_add(column), y.saturating_add(row), color);
            }
        }
        screen.add_damage(Rect::new(x, y, CURSOR_WIDTH, CURSOR_HEIGHT));
    }

    /// Saves what is under the sprite and draws it, if it is shown.
    pub fn draw(&mut self, screen: &mut Surface<'_>) {
        if !self.visible {
            return;
        }
        let (x, y) = (self.x, self.y);
        for row in 0..CURSOR_HEIGHT {
            for column in 0..CURSOR_WIDTH {
                let index =
                    usize::try_from(row.saturating_mul(CURSOR_WIDTH).saturating_add(column))
                        .unwrap_or(0);
                let at = (x.saturating_add(column), y.saturating_add(row));
                let under = screen.pixel(at.0, at.1).unwrap_or(Color::BLACK);
                if let Some(slot) = self.saved.get_mut(index) {
                    *slot = under;
                }
                let row_index = usize::try_from(row).unwrap_or(0);
                if let Some(color) = pixel_of(self.shape, row_index, column) {
                    screen.set_pixel(at.0, at.1, color);
                }
            }
        }
        self.drawn = Some((x, y));
        screen.add_damage(Rect::new(x, y, CURSOR_WIDTH, CURSOR_HEIGHT));
    }
}
