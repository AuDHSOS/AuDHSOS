// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Rectangles and the set of them that says what changed.
//!
//! A rectangle is a position and a size in pixels, never a pair of corners,
//! so no rectangle can be turned around. The right and the bottom edge
//! saturate rather than wrap: a rectangle that would reach past the end of
//! the coordinate space ends there, and every operation that follows sees a
//! rectangle inside the space.
//!
//! Invariants: a rectangle of zero width or zero height is empty and covers
//! nothing; the damage set holds at most [`DAMAGE_CAPACITY`] rectangles and
//! collapses to the one that encloses them all rather than dropping any.

/// How many rectangles the damage set holds before it collapses.
pub const DAMAGE_CAPACITY: usize = 16;

/// A rectangle in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    /// Leftmost column.
    pub x: u32,
    /// Topmost row.
    pub y: u32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
}

impl Rect {
    /// The rectangle that covers nothing.
    pub const EMPTY: Rect = Rect::new(0, 0, 0, 0);

    /// The rectangle at `x`, `y` of this size.
    #[must_use]
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Rect { x, y, w, h }
    }

    /// `true` when the rectangle covers no pixel.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// The column past the last one, saturated.
    #[must_use]
    pub const fn right(self) -> u32 {
        self.x.saturating_add(self.w)
    }

    /// The row past the last one, saturated.
    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.y.saturating_add(self.h)
    }

    /// `true` when the pixel is inside.
    #[must_use]
    pub const fn contains(self, x: u32, y: u32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The rectangle both cover, empty when they share no pixel.
    #[must_use]
    pub const fn intersect(self, other: Rect) -> Rect {
        if self.is_empty() || other.is_empty() {
            return Rect::EMPTY;
        }
        let x = if self.x > other.x { self.x } else { other.x };
        let y = if self.y > other.y { self.y } else { other.y };
        let right = if self.right() < other.right() {
            self.right()
        } else {
            other.right()
        };
        let bottom = if self.bottom() < other.bottom() {
            self.bottom()
        } else {
            other.bottom()
        };
        if right <= x || bottom <= y {
            return Rect::EMPTY;
        }
        Rect::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
    }

    /// The smallest rectangle that covers both. An empty operand leaves the
    /// other one as it is.
    #[must_use]
    pub const fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x = if self.x < other.x { self.x } else { other.x };
        let y = if self.y < other.y { self.y } else { other.y };
        let right = if self.right() > other.right() {
            self.right()
        } else {
            other.right()
        };
        let bottom = if self.bottom() > other.bottom() {
            self.bottom()
        } else {
            other.bottom()
        };
        Rect::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
    }

    /// `true` when the two share at least one pixel.
    #[must_use]
    pub const fn overlaps(self, other: Rect) -> bool {
        !self.intersect(other).is_empty()
    }

    /// The part of the rectangle inside a surface of this size.
    #[must_use]
    pub const fn clip_to(self, width: u32, height: u32) -> Rect {
        self.intersect(Rect::new(0, 0, width, height))
    }
}

/// The rectangles that changed since the last presentation.
///
/// A rectangle that overlaps one already in the set is merged into it, so
/// a surface that is filled twice in the same place reports one rectangle.
/// A set that is full collapses to the rectangle that encloses everything
/// it holds: presenting too much is slower, presenting too little is wrong.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    /// The rectangles, of which the first `len` count.
    rects: [Rect; DAMAGE_CAPACITY],
    /// How many of them there are.
    len: usize,
}

impl Damage {
    /// A set of no rectangles.
    #[must_use]
    pub const fn new() -> Self {
        Damage {
            rects: [Rect::EMPTY; DAMAGE_CAPACITY],
            len: 0,
        }
    }

    /// How many rectangles the set holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// `true` when nothing changed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `true` when the next rectangle collapses the set.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.len >= DAMAGE_CAPACITY
    }

    /// Forgets everything, which is what presenting does.
    pub const fn clear(&mut self) {
        self.len = 0;
    }

    /// The rectangles, in the order they were added.
    pub fn iter(&self) -> impl Iterator<Item = Rect> + '_ {
        self.rects.iter().take(self.len).copied()
    }

    /// The rectangle that encloses every rectangle of the set.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.iter().fold(Rect::EMPTY, Rect::union)
    }

    /// Records that `rect` changed. An empty rectangle changes nothing.
    pub fn push(&mut self, rect: Rect) {
        if rect.is_empty() {
            return;
        }
        for held in self.rects.iter_mut().take(self.len) {
            if held.overlaps(rect) {
                *held = held.union(rect);
                return;
            }
        }
        if let Some(slot) = self.rects.get_mut(self.len) {
            *slot = rect;
            self.len = self.len.saturating_add(1);
            return;
        }
        let bounds = self.bounds().union(rect);
        self.clear();
        self.push(bounds);
    }
}
