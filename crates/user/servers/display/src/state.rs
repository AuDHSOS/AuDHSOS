// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the display server decides: who holds which surface, and what a
//! presentation puts on the screen.
//!
//! A client is the badge of the capability its messages arrive through. It
//! creates one surface, draws into the memory object it was given for it,
//! and says which rectangles changed; this module checks that the surface
//! is the one that client holds and copies exactly those rectangles.
//!
//! Invariants: one surface per client, and no client ever names another
//! client's; a surface is never larger than the screen, so what is copied
//! always fits; the cursor is off the screen while a presentation runs and
//! back on it afterwards, so a client never presents over the sprite and
//! never has to know about it.

use audhsos_abi::Error;
use audhsos_collections::ArrayVec;
use gfx::{Damage, PixelFormat, Rect, Surface, present};
use user_proto::display::Mode;

use crate::cursor::Cursor;

/// How many clients the server holds surfaces for.
pub const MAX_CLIENTS: usize = 8;

/// One surface and who holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    /// The badge of the client that created it.
    pub badge: u64,
    /// What that client names it.
    pub id: u32,
    /// Visible columns.
    pub width: u32,
    /// Visible rows.
    pub height: u32,
}

impl Held {
    /// How many bytes the pixels of the surface occupy.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        u64::from(self.width)
            .saturating_mul(u64::from(self.height))
            .saturating_mul(4)
    }
}

/// The screen and everyone drawing on it.
#[derive(Debug)]
pub struct Display<const N: usize> {
    /// What the machine's framebuffer is, or nothing when it has none.
    mode: Option<Mode>,
    /// The surfaces, one per client.
    surfaces: ArrayVec<Held, N>,
    /// The number the next surface gets.
    next_id: u32,
    /// Where the pointer is and what is under it.
    cursor: Cursor,
}

impl<const N: usize> Default for Display<N> {
    fn default() -> Self {
        Display::new(None)
    }
}

impl<const N: usize> Display<N> {
    /// A display over `mode`, or over no screen at all.
    #[must_use]
    pub const fn new(mode: Option<Mode>) -> Self {
        Display {
            mode,
            surfaces: ArrayVec::new(),
            next_id: 1,
            cursor: Cursor::new(),
        }
    }

    /// What the screen is.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] on a machine without a framebuffer.
    pub const fn screen(&self) -> Result<Mode, Error> {
        match self.mode {
            Some(mode) => Ok(mode),
            None => Err(Error::NotFound),
        }
    }

    /// The rectangle of the screen, empty when there is none.
    #[must_use]
    pub const fn bounds(&self) -> Rect {
        match self.mode {
            Some(mode) => Rect::new(0, 0, mode.width, mode.height),
            None => Rect::EMPTY,
        }
    }

    /// Where the pointer is and whether it is shown.
    #[must_use]
    pub const fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// How many surfaces there are.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.surfaces.len()
    }

    /// `true` when nobody holds a surface.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }

    /// The surfaces, in the order they were created.
    pub fn iter(&self) -> impl Iterator<Item = &Held> {
        self.surfaces.iter()
    }

    /// Makes a surface of `width` by `height` for `badge`.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] without a screen; [`Error::InvalidArgument`] for
    /// a surface of no pixels or one larger than the screen;
    /// [`Error::AlreadyExists`] when the client holds one already;
    /// [`Error::QuotaExceeded`] when the server holds as many as it can.
    pub fn create(&mut self, badge: u64, width: u32, height: u32) -> Result<Held, Error> {
        let mode = self.screen()?;
        if width == 0 || height == 0 || width > mode.width || height > mode.height {
            return Err(Error::InvalidArgument);
        }
        if self.of(badge).is_some() {
            return Err(Error::AlreadyExists);
        }
        let held = Held {
            badge,
            id: self.next_id,
            width,
            height,
        };
        self.surfaces.push(held).map_err(|_| Error::QuotaExceeded)?;
        self.next_id = self.next_id.saturating_add(1);
        Ok(held)
    }

    /// The surface `badge` holds, if it holds one.
    #[must_use]
    pub fn of(&self, badge: u64) -> Option<&Held> {
        self.surfaces.iter().find(|held| held.badge == badge)
    }

    /// The surface `badge` may act on under the number `id`.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for a number no surface has;
    /// [`Error::AccessDenied`] for the surface of another client.
    pub fn holding(&self, badge: u64, id: u32) -> Result<Held, Error> {
        let held = self
            .surfaces
            .iter()
            .find(|held| held.id == id)
            .ok_or(Error::NotFound)?;
        if held.badge != badge {
            return Err(Error::AccessDenied);
        }
        Ok(*held)
    }

    /// Gives up the surface `badge` holds under `id`.
    ///
    /// # Errors
    ///
    /// As [`holding`](Self::holding).
    pub fn destroy(&mut self, badge: u64, id: u32) -> Result<Held, Error> {
        let held = self.holding(badge, id)?;
        self.drop_at(|kept| kept.id == id);
        Ok(held)
    }

    /// Forgets everything a client held, which is what happens when it goes
    /// away. Answers with what it held.
    pub fn forget(&mut self, badge: u64) -> Option<Held> {
        let held = self.of(badge).copied()?;
        self.drop_at(|kept| kept.badge == badge);
        Some(held)
    }

    /// Takes out the first surface `wanted` holds for.
    fn drop_at(&mut self, wanted: impl FnMut(&Held) -> bool) {
        let index = self.surfaces.iter().position(wanted);
        if let Some(index) = index {
            self.surfaces.remove(index);
        }
    }

    /// Copies the damaged rectangles of `pixels` onto `screen` and puts the
    /// cursor back on top, and answers with how many pixels it wrote.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] or [`Error::AccessDenied`] as
    /// [`holding`](Self::holding); [`Error::InvalidArgument`] when the
    /// surface handed in is not the size the client asked for or is in
    /// another format than the screen.
    pub fn present(
        &mut self,
        badge: u64,
        id: u32,
        damage: &Damage,
        pixels: &Surface<'_>,
        screen: &mut Surface<'_>,
    ) -> Result<u64, Error> {
        let held = self.holding(badge, id)?;
        if pixels.width() != held.width || pixels.height() != held.height {
            return Err(Error::InvalidArgument);
        }
        self.cursor.erase(screen);
        // The sprite goes back on whether the copy worked or not: a screen
        // that keeps the cursor only when a client presents something the
        // server can copy would lose the pointer to a client's mistake.
        let written = present(pixels, screen, damage);
        self.cursor.draw(screen);
        written.map_err(|_| Error::InvalidArgument)
    }

    /// Moves the pointer, clamped to the screen, and draws it where it now
    /// is.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] on a machine without a screen.
    pub fn set_cursor(
        &mut self,
        x: u32,
        y: u32,
        visible: bool,
        screen: &mut Surface<'_>,
    ) -> Result<(), Error> {
        let mode = self.screen()?;
        self.cursor.erase(screen);
        self.cursor
            .place(x, y, visible, Rect::new(0, 0, mode.width, mode.height));
        self.cursor.draw(screen);
        Ok(())
    }

    /// The format a surface of this display is drawn in.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] on a machine without a screen.
    pub fn format(&self) -> Result<PixelFormat, Error> {
        Ok(PixelFormat::from_boot(self.screen()?.format))
    }
}
