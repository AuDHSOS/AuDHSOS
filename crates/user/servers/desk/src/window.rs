// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One window: where it stands, what belongs to its frame, and the events
//! waiting for its client.
//!
//! A window is a content rectangle with a border around it and a title bar
//! above it. The content is what the client draws into; the rest is the
//! compositor's and no draw command reaches it. The position is that of
//! the frame, so moving a window is one assignment and every part of it
//! follows.
//!
//! Events wait here until the client asks for them. The queue holds
//! [`EVENTS`] of them and drops the oldest when it is full, counting what
//! it dropped: a client that stopped asking must not stop the desktop.
//!
//! Invariants: the content rectangle lies inside the frame; a window that
//! was placed on a screen has its whole frame on that screen, because the
//! position is clamped when it is placed and when it is moved.

use audhsos_collections::ArrayVec;
use gfx::{Color, GLYPH_HEIGHT, Rect};
use user_proto::window::{Event, Title};

/// How many pixels of border stand left of, right of, and below the
/// content.
pub const BORDER: u32 = 2;

/// How many rows the title bar occupies above the content.
pub const TITLE_HEIGHT: u32 = 20;

/// How many pixels the close box measures on a side.
pub const CLOSE_SIZE: u32 = 12;

/// How far the close box stands from the top and the right of the frame.
pub const CLOSE_MARGIN: u32 = 4;

/// How far the title stands from the left of the frame.
pub const TITLE_LEFT: u32 = 6;

/// How many events wait for a client before the oldest is dropped.
pub const EVENTS: usize = 32;

/// What the frame of the window in front is drawn in.
pub const FOCUSED: Color = Color::new(0x40, 0x80, 0xE0);

/// What the frame of every other window is drawn in.
pub const PLAIN: Color = Color::new(0x50, 0x54, 0x60);

/// What a title is written in.
pub const TITLE_INK: Color = Color::WHITE;

/// What the close box is filled with.
pub const CLOSE_INK: Color = Color::new(0xE0, 0x50, 0x50);

/// One window of the desktop.
#[derive(Debug)]
pub struct Window {
    /// What the client names it.
    id: u32,
    /// The badge of the client that opened it.
    badge: u64,
    /// Leftmost column of the frame.
    x: u32,
    /// Topmost row of it.
    y: u32,
    /// Columns of content.
    width: u32,
    /// Rows of content.
    height: u32,
    /// What stands in the title bar.
    title: Title,
    /// Whether it is the window the keyboard reaches.
    focused: bool,
    /// The events its client has not taken yet.
    events: ArrayVec<Event, EVENTS>,
    /// How many events were dropped because the queue was full.
    dropped: u32,
}

impl Window {
    /// A window of this content size at this position.
    #[must_use]
    pub const fn new(
        id: u32,
        badge: u64,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        title: Title,
    ) -> Self {
        Window {
            id,
            badge,
            x,
            y,
            width,
            height,
            title,
            focused: false,
            events: ArrayVec::new(),
            dropped: 0,
        }
    }

    /// What the client names it.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// The badge of the client that opened it.
    #[must_use]
    pub const fn badge(&self) -> u64 {
        self.badge
    }

    /// Columns of content.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Rows of content.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// What stands in the title bar.
    #[must_use]
    pub const fn title(&self) -> &Title {
        &self.title
    }

    /// Whether the keyboard reaches this window.
    #[must_use]
    pub const fn is_focused(&self) -> bool {
        self.focused
    }

    /// Says whether the keyboard reaches it and answers whether that
    /// changed.
    pub const fn focus(&mut self, focused: bool) -> bool {
        let changed = self.focused != focused;
        self.focused = focused;
        changed
    }

    /// How many bytes the pixels of the content occupy.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        u64::from(self.width)
            .saturating_mul(u64::from(self.height))
            .saturating_mul(4)
    }

    /// How wide the frame of a window of this content width is.
    #[must_use]
    pub const fn frame_width(width: u32) -> u32 {
        width.saturating_add(BORDER.saturating_mul(2))
    }

    /// How tall the frame of a window of this content height is.
    #[must_use]
    pub const fn frame_height(height: u32) -> u32 {
        height.saturating_add(TITLE_HEIGHT).saturating_add(BORDER)
    }

    /// The whole window: border, title bar, and content.
    #[must_use]
    pub const fn frame(&self) -> Rect {
        Rect::new(
            self.x,
            self.y,
            Window::frame_width(self.width),
            Window::frame_height(self.height),
        )
    }

    /// The strip the title stands in, which is also what a drag grabs.
    #[must_use]
    pub const fn title_bar(&self) -> Rect {
        Rect::new(
            self.x,
            self.y,
            Window::frame_width(self.width),
            TITLE_HEIGHT,
        )
    }

    /// The box that closes the window.
    #[must_use]
    pub const fn close_box(&self) -> Rect {
        let bar = self.title_bar();
        Rect::new(
            bar.right()
                .saturating_sub(CLOSE_SIZE)
                .saturating_sub(CLOSE_MARGIN),
            bar.y.saturating_add(CLOSE_MARGIN),
            CLOSE_SIZE,
            CLOSE_SIZE,
        )
    }

    /// Where the title is written.
    #[must_use]
    pub const fn title_at(&self) -> (u32, u32) {
        let top = TITLE_HEIGHT.saturating_sub(GLYPH_HEIGHT).wrapping_div(2);
        (
            self.x.saturating_add(TITLE_LEFT),
            self.y.saturating_add(top),
        )
    }

    /// The part of the screen the content occupies.
    #[must_use]
    pub const fn content(&self) -> Rect {
        Rect::new(
            self.x.saturating_add(BORDER),
            self.y.saturating_add(TITLE_HEIGHT),
            self.width,
            self.height,
        )
    }

    /// Where the content is, as a pixel of it: the pixel of the screen at
    /// `x`, `y` in content coordinates, or `None` outside the content.
    #[must_use]
    pub const fn inside(&self, x: u32, y: u32) -> Option<(u32, u32)> {
        let content = self.content();
        if !content.contains(x, y) {
            return None;
        }
        Some((x.saturating_sub(content.x), y.saturating_sub(content.y)))
    }

    /// Puts the frame at `x`, `y`, clamped so that the whole of it stands
    /// on a screen of `width` by `height` below a bar of `top` rows.
    ///
    /// A frame taller than the screen below the bar keeps its top edge
    /// under the bar and reaches past the bottom: the bar is what a client
    /// may never cover.
    pub const fn place(&mut self, x: u32, y: u32, width: u32, height: u32, top: u32) {
        let frame = self.frame();
        let last_x = width.saturating_sub(frame.w);
        let below = height.saturating_sub(frame.h);
        let last_y = if below < top { top } else { below };
        self.x = if x > last_x { last_x } else { x };
        self.y = if y < top {
            top
        } else if y > last_y {
            last_y
        } else {
            y
        };
    }

    /// The color the frame is drawn in.
    #[must_use]
    pub const fn edge(&self) -> Color {
        if self.focused { FOCUSED } else { PLAIN }
    }

    /// Puts `event` at the end of the queue. The oldest is dropped when the
    /// queue is full, and counted.
    pub fn push(&mut self, event: Event) {
        if self.events.is_full() {
            self.events.remove(0);
            self.dropped = self.dropped.saturating_add(1);
        }
        let _room = self.events.push(event);
    }

    /// Takes the oldest event, if one waits.
    pub fn pop(&mut self) -> Option<Event> {
        if self.events.is_empty() {
            return None;
        }
        self.events.remove(0)
    }

    /// How many events wait.
    #[must_use]
    pub const fn waiting(&self) -> usize {
        self.events.len()
    }

    /// How many events were dropped because the queue was full.
    #[must_use]
    pub const fn dropped(&self) -> u32 {
        self.dropped
    }
}
