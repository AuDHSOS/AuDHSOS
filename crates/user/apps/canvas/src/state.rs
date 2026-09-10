// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The drawing state: where the pointer is, whether it is drawing, where
//! the next character goes, and what each of those puts into a surface.
//!
//! One event goes in and one [`Step`] comes out, which says what changed so
//! that the program around it knows whether to present, where to put the
//! sprite, and when to stop. Nothing here is a system call and nothing here
//! is a message; the surface is handed in.
//!
//! The pointer is a position and not a delta: the mouse of this machine
//! reports how far it moved and the screen has edges, so the position is
//! kept here and clamped to the screen at every step. A button held down
//! joins the position before an event to the position after it with a line,
//! which is why a fast movement leaves a stroke and not a row of dots.
//!
//! Invariants: the pointer is always a pixel of the screen, so every
//! position this answers with can be drawn at; the text cursor is always a
//! cell that fits, so a character is never half off the edge; what a step
//! reports as changed is inside the damage the surface collected.

use gfx::{Color, GLYPH_HEIGHT, GLYPH_WIDTH, Rect, Surface, draw_text};
use user_proto::input::{Event, KeyCode};
use user_proto::keyboard::{Keyboard, Layout};

/// The color the canvas puts down before anything is drawn on it.
pub const BACKGROUND: Color = Color::new(0x10, 0x18, 0x30);

/// The color a held button draws in.
pub const PEN: Color = Color::new(0xF0, 0x40, 0x60);

/// The color typed characters are written in. It is neither the background
/// nor the white of the pointer sprite, so a picture of the screen tells
/// the three apart without knowing where any of them is.
pub const INK: Color = Color::new(0xF0, 0xE0, 0x40);

/// Which bit of the button mask is the button that draws.
const DRAWING_BUTTON: u8 = user_proto::input::BUTTON_LEFT;

/// The key that ends the program.
///
/// It is not the escape key, which clears the canvas, and it stands for no
/// character in either layout, so ending cannot be typed by accident.
pub const ENDS: KeyCode = KeyCode::F10;

/// Where the first character of a line goes.
pub const TEXT_ORIGIN: (u32, u32) = (8, 8);

/// What one event changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing on the surface changed.
    Nothing,
    /// The pointer moved and drew nothing.
    Moved,
    /// The button was held, and the segment from one position to the other
    /// carries [`PEN`]. The two are equal for the event that pressed it,
    /// which marks a single pixel.
    Drew {
        /// Where the segment starts.
        from: (u32, u32),
        /// Where it ends, which is where the pointer now is.
        to: (u32, u32),
    },
    /// A character was written, in the cell whose top left corner this
    /// names.
    Typed {
        /// Column of the cell.
        x: u32,
        /// Row of the cell.
        y: u32,
        /// What stands in it.
        character: char,
    },
    /// The escape key put the background back and started the text over.
    Cleared,
    /// The key that ends the program came up.
    Ended,
}

/// The pointer, the pen, and the text cursor over a surface of a fixed
/// size.
#[derive(Clone, Copy, Debug)]
pub struct Canvas {
    /// Columns of the surface.
    width: u32,
    /// Rows of it.
    height: u32,
    /// Column the pointer is at.
    x: u32,
    /// Row it is at.
    y: u32,
    /// Whether the button that draws was down at the last event.
    drawing: bool,
    /// Column the next character goes in.
    text_x: u32,
    /// Row it goes in.
    text_y: u32,
    /// The layout and the modifiers that are held.
    keyboard: Keyboard,
}

impl Canvas {
    /// A canvas over a surface of this size, with the pointer in the middle
    /// of it and the text cursor at [`TEXT_ORIGIN`].
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Canvas {
            width,
            height,
            x: clamp(width.wrapping_div(2), width),
            y: clamp(height.wrapping_div(2), height),
            drawing: false,
            text_x: TEXT_ORIGIN.0,
            text_y: TEXT_ORIGIN.1,
            keyboard: Keyboard::new(Layout::Us),
        }
    }

    /// Where the pointer is.
    #[must_use]
    pub const fn cursor(&self) -> (u32, u32) {
        (self.x, self.y)
    }

    /// Where the next character goes.
    #[must_use]
    pub const fn text_cursor(&self) -> (u32, u32) {
        (self.text_x, self.text_y)
    }

    /// Whether the button that draws is down.
    #[must_use]
    pub const fn is_drawing(&self) -> bool {
        self.drawing
    }

    /// Puts the background down and starts the text over.
    ///
    /// This is what the escape key does and what the program does before it
    /// presents for the first time.
    pub fn clear(&mut self, surface: &mut Surface<'_>) {
        surface.fill(surface.bounds(), BACKGROUND);
        self.text_x = TEXT_ORIGIN.0;
        self.text_y = TEXT_ORIGIN.1;
    }

    /// Takes one event and answers with what it changed.
    pub fn feed(&mut self, event: Event, surface: &mut Surface<'_>) -> Step {
        match event {
            Event::Key(key) => {
                // The keyboard sees every key, the ones that stand for no
                // character included: it is what keeps the modifier state,
                // and a shift release that never reached it would leave the
                // next letter capital forever.
                let typed = self.keyboard.feed(key);
                if key.pressed && matches!(key.code, KeyCode::Escape) {
                    self.clear(surface);
                    return Step::Cleared;
                }
                if !key.pressed && matches!(key.code, ENDS) {
                    return Step::Ended;
                }
                match typed {
                    Some(character) => self.write(character, surface),
                    None => Step::Nothing,
                }
            }
            Event::Pointer(pointer) => {
                let from = (self.x, self.y);
                self.x = step(self.x, pointer.dx, self.width);
                self.y = step(self.y, pointer.dy, self.height);
                let to = (self.x, self.y);
                self.drawing = pointer.buttons & DRAWING_BUTTON != 0;
                if self.drawing {
                    Self::stroke(surface, from, to);
                    Step::Drew { from, to }
                } else if from == to {
                    Step::Nothing
                } else {
                    Step::Moved
                }
            }
        }
    }

    /// Draws the segment from `from` to `to` in [`PEN`] and records it as
    /// damage in one rectangle.
    fn stroke(surface: &mut Surface<'_>, from: (u32, u32), to: (u32, u32)) {
        let mut x = i64::from(from.0);
        let mut y = i64::from(from.1);
        let last_x = i64::from(to.0);
        let last_y = i64::from(to.1);
        // Bresenham with the error carried as one number: `width` is
        // positive and `height` negative, so both axes are advanced by the
        // same comparison and neither is a special case.
        let width = last_x.saturating_sub(x).saturating_abs();
        let height = last_y.saturating_sub(y).saturating_abs().saturating_neg();
        let toward_x = if x < last_x { 1_i64 } else { -1_i64 };
        let toward_y = if y < last_y { 1_i64 } else { -1_i64 };
        let mut error = width.saturating_add(height);
        loop {
            surface.set_pixel(narrow(x), narrow(y), PEN);
            if x == last_x && y == last_y {
                break;
            }
            let doubled = error.saturating_mul(2);
            if doubled >= height {
                error = error.saturating_add(height);
                x = x.saturating_add(toward_x);
            }
            if doubled <= width {
                error = error.saturating_add(width);
                y = y.saturating_add(toward_y);
            }
        }
        surface.add_damage(span(from, to));
    }

    /// Writes one character at the text cursor and moves it on.
    fn write(&mut self, character: char, surface: &mut Surface<'_>) -> Step {
        if character == '\n' {
            self.next_line();
            return Step::Nothing;
        }
        if self.text_x.saturating_add(GLYPH_WIDTH) > self.width {
            self.next_line();
        }
        let (x, y) = (self.text_x, self.text_y);
        let mut buffer = [0_u8; 4];
        let text = character.encode_utf8(&mut buffer);
        draw_text(surface, x, y, text, INK, Some(BACKGROUND));
        self.text_x = self.text_x.saturating_add(GLYPH_WIDTH);
        Step::Typed { x, y, character }
    }

    /// Moves the text cursor to the start of the next row, and to the top
    /// again when there is no next row.
    const fn next_line(&mut self) {
        self.text_x = TEXT_ORIGIN.0;
        self.text_y = self.text_y.saturating_add(GLYPH_HEIGHT);
        if self.text_y.saturating_add(GLYPH_HEIGHT) > self.height {
            self.text_y = TEXT_ORIGIN.1;
        }
    }
}

/// The smallest rectangle holding both points.
#[must_use]
pub const fn span(from: (u32, u32), to: (u32, u32)) -> Rect {
    let (left, right) = if from.0 <= to.0 {
        (from.0, to.0)
    } else {
        (to.0, from.0)
    };
    let (top, bottom) = if from.1 <= to.1 {
        (from.1, to.1)
    } else {
        (to.1, from.1)
    };
    Rect::new(
        left,
        top,
        right.saturating_sub(left).saturating_add(1),
        bottom.saturating_sub(top).saturating_add(1),
    )
}

/// `value` brought inside a screen of this many pixels along the axis.
const fn clamp(value: u32, len: u32) -> u32 {
    let last = len.saturating_sub(1);
    if value > last { last } else { value }
}

/// `position` moved by `delta` and clamped to a screen of this many pixels.
///
/// The clamping happens while the value is still wide, so a delta that
/// would carry the pointer past either edge lands on the edge rather than
/// wrapping to the other one.
fn step(position: u32, delta: i16, len: u32) -> u32 {
    let last = i64::from(len.saturating_sub(1));
    let moved = i64::from(position).saturating_add(i64::from(delta));
    let inside = if moved < 0 {
        0
    } else if moved > last {
        last
    } else {
        moved
    };
    u32::try_from(inside).unwrap_or(0)
}

/// A coordinate of the stroke as a column or a row. It cannot be negative:
/// both ends are pixels of the screen and the walk stays between them.
fn narrow(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(0)
}
