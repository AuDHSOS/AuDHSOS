// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The menu bar: the strip across the top of the screen, the titles in it,
//! the panel a title opens, and the digital clock at its right end.
//!
//! Everything here is geometry and colors. Which entries a panel holds is
//! the desktop's business, because one menu lists the windows that are
//! open; this module says how wide a panel of that many entries is and
//! which entry stands at a pixel.
//!
//! Invariants: the bar is [`HEIGHT`] rows at the top of the screen and
//! nothing else is drawn there; a panel hangs below the title it belongs
//! to and never reaches past the screen, because its width is fixed and
//! the titles stand at the left.

use audhsos_time::CivilTime;
use gfx::{Color, GLYPH_HEIGHT, GLYPH_WIDTH, Rect};

/// How many rows the bar occupies at the top of the screen.
pub const HEIGHT: u32 = 24;

/// How many rows one entry of a panel occupies.
pub const ENTRY_HEIGHT: u32 = 20;

/// How many columns a panel occupies.
pub const PANEL_WIDTH: u32 = 176;

/// How far from the left edge the first title stands, and how far from the
/// right edge the clock stands.
pub const MARGIN: u32 = 8;

/// How much space stands left and right of a title inside its cell.
pub const TITLE_PADDING: u32 = 8;

/// Where the text of a row sits inside a cell of [`HEIGHT`] rows.
const TEXT_TOP: u32 = 4;

/// What the bar is filled with.
pub const BACKGROUND: Color = Color::new(0xE8, 0xE8, 0xF0);

/// What the text of the bar is drawn in.
pub const INK: Color = Color::new(0x10, 0x10, 0x18);

/// What the cell of an open title, and the entry under the pointer, is
/// filled with.
pub const HIGHLIGHT: Color = Color::new(0x40, 0x80, 0xE0);

/// What the text of a highlighted cell is drawn in.
pub const HIGHLIGHT_INK: Color = Color::WHITE;

/// What the one-pixel line under the bar and around a panel is drawn in.
pub const EDGE: Color = Color::new(0x60, 0x64, 0x78);

/// How many characters the clock shows: `HH:MM:SS`.
pub const CLOCK_LEN: usize = 8;

/// The titles of the menu, in the order they stand in the bar.
pub const TITLES: [&str; 2] = ["Desk", "Windows"];

/// The title that holds what the desktop itself does.
pub const DESK_MENU: usize = 0;

/// The title that lists the windows that are open.
pub const WINDOW_MENU: usize = 1;

/// The entries of [`DESK_MENU`], in the order they stand in its panel.
pub const DESK_ENTRIES: [&str; 2] = ["Close window", "Quit"];

/// The strip the bar occupies on a screen of this width.
#[must_use]
pub const fn rect(width: u32) -> Rect {
    Rect::new(0, 0, width, HEIGHT)
}

/// How wide the cell of `title` is.
#[must_use]
pub fn cell_width(title: &str) -> u32 {
    let characters = u32::try_from(title.chars().count()).unwrap_or(0);
    characters
        .saturating_mul(GLYPH_WIDTH)
        .saturating_add(TITLE_PADDING.saturating_mul(2))
}

/// The cell of the title at `index`, empty for an index the bar has none
/// for.
#[must_use]
pub fn title_rect(index: usize) -> Rect {
    let Some(title) = TITLES.get(index) else {
        return Rect::EMPTY;
    };
    let mut x = MARGIN;
    for earlier in TITLES.iter().take(index) {
        x = x.saturating_add(cell_width(earlier));
    }
    Rect::new(x, 0, cell_width(title), HEIGHT)
}

/// Which title stands at this pixel, if the pixel is in the bar and in a
/// cell.
#[must_use]
pub fn title_at(x: u32, y: u32) -> Option<usize> {
    (0..TITLES.len()).find(|index| title_rect(*index).contains(x, y))
}

/// Where the text of a title or of the clock stands inside its cell.
#[must_use]
pub const fn text_top() -> u32 {
    TEXT_TOP
}

/// Where the clock stands on a screen of this width.
#[must_use]
pub fn clock_rect(width: u32) -> Rect {
    let text = u32::try_from(CLOCK_LEN)
        .unwrap_or(0)
        .saturating_mul(GLYPH_WIDTH);
    let x = width.saturating_sub(MARGIN).saturating_sub(text);
    Rect::new(x, TEXT_TOP, text, GLYPH_HEIGHT)
}

/// The panel the title at `index` opens, holding `entries` entries.
#[must_use]
pub fn panel_rect(index: usize, entries: usize) -> Rect {
    let cell = title_rect(index);
    if cell.is_empty() {
        return Rect::EMPTY;
    }
    let rows = u32::try_from(entries).unwrap_or(0);
    Rect::new(
        cell.x,
        HEIGHT,
        PANEL_WIDTH,
        rows.saturating_mul(ENTRY_HEIGHT),
    )
}

/// The row of the entry at `entry` in the panel of `index`.
#[must_use]
pub fn entry_rect(index: usize, entries: usize, entry: usize) -> Rect {
    let panel = panel_rect(index, entries);
    if panel.is_empty() || entry >= entries {
        return Rect::EMPTY;
    }
    let offset = u32::try_from(entry)
        .unwrap_or(0)
        .saturating_mul(ENTRY_HEIGHT);
    Rect::new(
        panel.x,
        panel.y.saturating_add(offset),
        panel.w,
        ENTRY_HEIGHT,
    )
}

/// Which entry of the panel of `index` stands at this pixel, if any.
#[must_use]
pub fn entry_at(index: usize, entries: usize, x: u32, y: u32) -> Option<usize> {
    (0..entries).find(|entry| entry_rect(index, entries, *entry).contains(x, y))
}

/// The digital clock of the bar.
///
/// It holds the hour, the minute and the second it last showed, so the
/// desktop repaints the clock when one of the three changed and leaves the
/// screen alone when the wall clock moved by less than a second.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Clock {
    /// The hour, zero to twenty-three.
    hour: u8,
    /// The minute, zero to fifty-nine.
    minute: u8,
    /// The second, zero to fifty-nine.
    second: u8,
}

impl Clock {
    /// A clock standing at midnight.
    #[must_use]
    pub const fn new() -> Self {
        Clock {
            hour: 0,
            minute: 0,
            second: 0,
        }
    }

    /// Puts the clock at `time` and answers whether what it shows changed.
    pub const fn set(&mut self, time: CivilTime) -> bool {
        let moved =
            self.hour != time.hour || self.minute != time.minute || self.second != time.second;
        self.hour = time.hour;
        self.minute = time.minute;
        self.second = time.second;
        moved
    }

    /// Puts the clock at the wall clock of `seconds` since the epoch and
    /// answers whether what it shows changed. A time this system cannot
    /// take apart leaves the clock as it stands.
    pub fn set_unix(&mut self, seconds: i64) -> bool {
        match audhsos_time::UnixTime::from_seconds(seconds).to_civil() {
            Ok(time) => self.set(time),
            Err(_error) => false,
        }
    }

    /// What the clock shows, as `HH:MM:SS`.
    #[must_use]
    pub const fn text(&self) -> [u8; CLOCK_LEN] {
        let [hour_high, hour_low] = two_digits(self.hour);
        let [minute_high, minute_low] = two_digits(self.minute);
        let [second_high, second_low] = two_digits(self.second);
        [
            hour_high,
            hour_low,
            b':',
            minute_high,
            minute_low,
            b':',
            second_high,
            second_low,
        ]
    }
}

/// The two digits of a number below a hundred, as characters. A number
/// above that shows its last two digits, which no field of a clock is.
const fn two_digits(value: u8) -> [u8; 2] {
    let value = value.wrapping_rem(100);
    [
        b'0'.saturating_add(value.wrapping_div(10)),
        b'0'.saturating_add(value.wrapping_rem(10)),
    ]
}
