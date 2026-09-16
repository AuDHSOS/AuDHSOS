// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Draw commands: what a program asks for, and what carries it out.
//!
//! A program that draws into a surface it does not hold says what it wants
//! drawn instead of writing pixels. One command is one operation of this
//! crate — a fill, an outline, a segment, a run of text — and a list of at
//! most [`LIST_CAPACITY`] of them is what travels in one message. The
//! executor draws every command of a list into a surface and records what
//! it changed in the damage set of that surface.
//!
//! The text of a command is ASCII, at most [`TEXT_CAPACITY`] bytes, because
//! the font of this crate draws the printable characters of ASCII and a
//! message carries the text by value.
//!
//! Invariants: a command never writes outside the surface it is drawn
//! into, because every operation clips first; the rectangle a command
//! answers with is what it wrote and is inside the surface.

use crate::font::{GLYPH_HEIGHT, GLYPH_WIDTH, draw_text};
use crate::format::Color;
use crate::rect::Rect;
use crate::surface::Surface;

/// How many bytes the text of one command holds.
pub const TEXT_CAPACITY: usize = 56;

/// How many commands one list holds.
pub const LIST_CAPACITY: usize = 16;

/// The text of a [`Command::Text`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Text {
    /// The bytes, of which the first `len` count.
    bytes: [u8; TEXT_CAPACITY],
    /// How many there are.
    len: usize,
}

impl Default for Text {
    fn default() -> Self {
        Text::empty()
    }
}

impl Text {
    /// Text of no characters.
    #[must_use]
    pub const fn empty() -> Self {
        Text {
            bytes: [0; TEXT_CAPACITY],
            len: 0,
        }
    }

    /// The text over `source`, or `None` for more than [`TEXT_CAPACITY`]
    /// bytes and for a byte outside ASCII, which the font has no glyph
    /// for.
    #[must_use]
    pub fn new(source: &str) -> Option<Self> {
        Text::from_bytes(source.as_bytes())
    }

    /// The same over bytes, which is what a message carries.
    #[must_use]
    pub fn from_bytes(source: &[u8]) -> Option<Self> {
        if !source.is_ascii() {
            return None;
        }
        let mut text = Text::empty();
        let slot = text.bytes.get_mut(..source.len())?;
        slot.copy_from_slice(source);
        text.len = source.len();
        Some(text)
    }

    /// How many bytes the text has.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// `true` when the text has no byte.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bytes of the text.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or_default()
    }

    /// The text as a string, which it is: every byte of it is ASCII.
    #[must_use]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap_or_default()
    }

    /// How wide the text is when it is drawn.
    #[must_use]
    pub fn width(&self) -> u32 {
        u32::try_from(self.len)
            .unwrap_or(u32::MAX)
            .saturating_mul(GLYPH_WIDTH)
    }
}

/// One operation on a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    /// Every pixel of the surface takes this color.
    Clear {
        /// What it takes.
        color: Color,
    },
    /// The rectangle is filled.
    Fill {
        /// Where.
        rect: Rect,
        /// With what.
        color: Color,
    },
    /// The edge of the rectangle is drawn, one pixel wide.
    Frame {
        /// Which rectangle.
        rect: Rect,
        /// In what color.
        color: Color,
    },
    /// The segment between two pixels is drawn.
    Line {
        /// Where it starts.
        from: (u32, u32),
        /// Where it ends.
        to: (u32, u32),
        /// In what color.
        color: Color,
    },
    /// A run of text is written, one row of cells.
    Text {
        /// Column of the first cell.
        x: u32,
        /// Row of it.
        y: u32,
        /// What the glyphs are drawn in.
        color: Color,
        /// What the cells around them are filled with, if anything.
        background: Option<Color>,
        /// What is written.
        text: Text,
    },
}

/// A list of at most [`LIST_CAPACITY`] commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct List {
    /// The commands, of which the first `len` count.
    commands: [Command; LIST_CAPACITY],
    /// How many there are.
    len: usize,
}

impl Default for List {
    fn default() -> Self {
        List::new()
    }
}

impl List {
    /// A list of no commands.
    #[must_use]
    pub const fn new() -> Self {
        List {
            commands: [const {
                Command::Clear {
                    color: Color::BLACK,
                }
            }; LIST_CAPACITY],
            len: 0,
        }
    }

    /// How many commands the list holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// `true` when the list holds none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `true` when no further command fits.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.len >= LIST_CAPACITY
    }

    /// Forgets every command.
    pub const fn clear(&mut self) {
        self.len = 0;
    }

    /// Appends `command` and reports whether it fitted.
    pub fn push(&mut self, command: Command) -> bool {
        let Some(slot) = self.commands.get_mut(self.len) else {
            return false;
        };
        *slot = command;
        self.len = self.len.saturating_add(1);
        true
    }

    /// The commands, in the order they were appended.
    pub fn iter(&self) -> impl Iterator<Item = &Command> {
        self.commands.iter().take(self.len)
    }
}

/// Draws the segment from `from` to `to` and answers with the rectangle it
/// wrote in.
///
/// The walk is Bresenham with the error carried as one number: `width` is
/// positive and `height` negative, so both axes advance by the same
/// comparison and neither is a special case.
///
/// A segment longer than the surface is refused and draws nothing. The
/// walk is one step per pixel of its longer axis whether that pixel is
/// inside the surface or not, so a segment that spans the coordinate
/// space would cost four thousand million steps for a picture of at most
/// a few million pixels — and the commands of `draw_all` come from a
/// client that a server may not let decide how long it runs. A caller
/// that holds such a segment clips it to the surface first.
pub fn line(surface: &mut Surface<'_>, from: (u32, u32), to: (u32, u32), color: Color) -> Rect {
    if steps_of(from, to) > reach_of(surface) {
        return Rect::EMPTY;
    }
    let mut x = i64::from(from.0);
    let mut y = i64::from(from.1);
    let last_x = i64::from(to.0);
    let last_y = i64::from(to.1);
    let width = last_x.saturating_sub(x).saturating_abs();
    let height = last_y.saturating_sub(y).saturating_abs().saturating_neg();
    let toward_x = if x < last_x { 1_i64 } else { -1_i64 };
    let toward_y = if y < last_y { 1_i64 } else { -1_i64 };
    let mut error = width.saturating_add(height);
    loop {
        surface.set_pixel(narrow(x), narrow(y), color);
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
    let written = Rect::span(from, to).clip_to(surface.width(), surface.height());
    surface.add_damage(written);
    written
}

/// Draws the edge of `rect`, one pixel wide, and answers with the part of
/// the surface it wrote.
pub fn frame(surface: &mut Surface<'_>, rect: Rect, color: Color) -> Rect {
    let area = rect.clip_to(surface.width(), surface.height());
    if area.is_empty() {
        return Rect::EMPTY;
    }
    let bottom = area.bottom().saturating_sub(1);
    let right = area.right().saturating_sub(1);
    surface.fill(Rect::new(area.x, area.y, area.w, 1), color);
    surface.fill(Rect::new(area.x, bottom, area.w, 1), color);
    surface.fill(Rect::new(area.x, area.y, 1, area.h), color);
    surface.fill(Rect::new(right, area.y, 1, area.h), color);
    area
}

/// Carries out one command and answers with the part of the surface it
/// wrote.
pub fn draw(surface: &mut Surface<'_>, command: &Command) -> Rect {
    match *command {
        Command::Clear { color } => surface.fill(surface.bounds(), color),
        Command::Fill { rect, color } => surface.fill(rect, color),
        Command::Frame { rect, color } => frame(surface, rect, color),
        Command::Line { from, to, color } => line(surface, from, to, color),
        Command::Text {
            x,
            y,
            color,
            background,
            text,
        } => draw_text(surface, x, y, text.as_str(), color, background),
    }
}

/// Carries out every command of `commands`, in order, and answers with the
/// rectangle that encloses what they wrote.
pub fn draw_all(surface: &mut Surface<'_>, commands: &List) -> Rect {
    let mut written = Rect::EMPTY;
    for command in commands.iter() {
        written = written.union(draw(surface, command));
    }
    written
}

/// The rectangle one row of `text` occupies when it is drawn at `x`, `y`.
#[must_use]
pub fn text_bounds(x: u32, y: u32, text: &Text) -> Rect {
    Rect::new(x, y, text.width(), GLYPH_HEIGHT)
}

/// How many steps the walk of the segment from `from` to `to` takes,
/// which is the longer of its two axes.
fn steps_of(from: (u32, u32), to: (u32, u32)) -> u64 {
    let across = u64::from(from.0.abs_diff(to.0));
    let down = u64::from(from.1.abs_diff(to.1));
    across.max(down)
}

/// How many steps a segment of this surface may take: one per column and
/// one per row, which is longer than any segment with both ends inside.
fn reach_of(surface: &Surface<'_>) -> u64 {
    u64::from(surface.width()).saturating_add(u64::from(surface.height()))
}

/// A coordinate of a segment as a column or a row. It cannot be negative:
/// both ends are pixels and the walk stays between them.
fn narrow(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(0)
}
