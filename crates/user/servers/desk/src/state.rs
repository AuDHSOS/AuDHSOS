// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The desktop: which client holds which window, which window stands in
//! front, what one event of the keyboard or the pointer changes, and what
//! is painted where.
//!
//! A client is the badge of the capability its messages arrive through. It
//! opens one window, sends draw commands into it, and takes the events of
//! it one at a time; this module checks that the window is the one that
//! client holds, and the process around it carries the commands out in the
//! surface it keeps for that window.
//!
//! The keyboard reaches the window that has the focus, and the focus
//! follows the click (D-153).
//!
//! The windows stand in one order, back to front, and that order is the
//! order they are painted in. Painting a region is three steps: the
//! desktop under everything, then each window — its frame from here, its
//! content from the surface the process holds — and the bar over all of
//! them. The bar is last, so nothing is ever drawn over the clock.
//!
//! Nothing is painted before [`Desk::show`], which the key [`SHOWS`] is
//! what calls. A program that draws on this screen before the desktop does
//! would otherwise decide by a race which of the two the screen holds
//! (D-154).
//!
//! Invariants: one window per client, and no client ever names another
//! client's; the frame of every window stands whole on the screen and
//! below the bar, because a position is clamped when it is placed and when
//! it is dragged; exactly one window has the focus while any window is
//! open.

use audhsos_abi::Error;
use audhsos_collections::ArrayVec;
use audhsos_time::CivilTime;
use gfx::{Color, Rect, Surface, draw_text};
use user_proto::input::{BUTTON_LEFT, Event as Input, KeyCode, KeyEvent, PointerEvent};
use user_proto::window::{Event, Title};

use crate::bar;
use crate::window::Window;

/// How many windows the desktop holds.
pub const WINDOWS: usize = 4;

/// The badge of a capability that carries none.
///
/// A client reaches this server through a capability its parent badged,
/// and a window belongs to a badge. A request that arrives without one
/// names nobody, and two of nobody are one client to a server that keeps
/// something per client, which is why such a request gets no window.
pub const NOBODY: u64 = 0;

/// What the desktop under the windows is filled with.
pub const DESKTOP: Color = Color::new(0x18, 0x1C, 0x28);

/// The key that paints the desktop for the first time.
pub const SHOWS: KeyCode = KeyCode::F11;

/// The key that ends the desktop, on the way up.
pub const ENDS: KeyCode = KeyCode::F12;

/// Where the first window stands below the bar, and how far each further
/// window stands below and right of the one before it.
const CASCADE_X: u32 = 32;
const CASCADE_Y: u32 = 16;
const CASCADE_STEP: u32 = 28;

/// What one event changed.
#[derive(Debug)]
pub struct Outcome {
    /// What has to be painted again, empty when nothing has.
    pub repaint: Rect,
    /// The windows whose clients have an event waiting.
    pub woken: ArrayVec<u32, WINDOWS>,
    /// Whether the desktop is ending.
    pub ended: bool,
    /// Whether the pointer stands somewhere else than before.
    pub moved: bool,
}

impl Default for Outcome {
    fn default() -> Self {
        Outcome::nothing()
    }
}

impl Outcome {
    /// An outcome that changed nothing.
    #[must_use]
    pub const fn nothing() -> Self {
        Outcome {
            repaint: Rect::EMPTY,
            woken: ArrayVec::new(),
            ended: false,
            moved: false,
        }
    }

    /// Records that the client of `id` has an event waiting.
    fn wake(&mut self, id: u32) {
        if self.woken.iter().any(|woken| *woken == id) {
            return;
        }
        let _room = self.woken.push(id);
    }

    /// Records that `rect` has to be painted again.
    const fn paint(&mut self, rect: Rect) {
        self.repaint = self.repaint.union(rect);
    }
}

/// Which window is being dragged, and where it was grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Drag {
    /// The window being dragged.
    id: u32,
    /// How far right of the frame the pointer grabbed it.
    dx: u32,
    /// How far below the top of the frame it grabbed it.
    dy: u32,
}

/// The screen, the windows on it, and the bar over them.
#[derive(Debug)]
pub struct Desk {
    /// Visible columns.
    width: u32,
    /// Visible rows.
    height: u32,
    /// The windows, back to front.
    windows: ArrayVec<Window, WINDOWS>,
    /// The number the next window gets.
    next_id: u32,
    /// Where the pointer is.
    pointer: (u32, u32),
    /// Which buttons were held at the last pointer event.
    buttons: u8,
    /// The drag that is going on, if one is.
    drag: Option<Drag>,
    /// Which menu title is open, if one is.
    menu: Option<usize>,
    /// What the clock shows.
    clock: bar::Clock,
    /// Whether the desktop has been painted at all.
    shown: bool,
}

impl Desk {
    /// A desktop over a screen of this size, with the pointer in the
    /// middle of it and nothing painted yet.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Desk {
            width,
            height,
            windows: ArrayVec::new(),
            next_id: 1,
            pointer: (width.wrapping_div(2), height.wrapping_div(2)),
            buttons: 0,
            drag: None,
            menu: None,
            clock: bar::Clock::new(),
            shown: false,
        }
    }

    /// The whole screen.
    #[must_use]
    pub const fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    /// Visible columns.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Visible rows.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Where the pointer is.
    #[must_use]
    pub const fn pointer(&self) -> (u32, u32) {
        self.pointer
    }

    /// Whether the desktop has been painted at all.
    #[must_use]
    pub const fn is_shown(&self) -> bool {
        self.shown
    }

    /// Which menu title is open, if one is.
    #[must_use]
    pub const fn menu(&self) -> Option<usize> {
        self.menu
    }

    /// What the clock shows.
    #[must_use]
    pub const fn clock(&self) -> &bar::Clock {
        &self.clock
    }

    /// Paints the desktop for the first time, and answers with what has to
    /// be painted. A desktop that already stands is painted again.
    pub const fn show(&mut self) -> Rect {
        self.shown = true;
        self.bounds()
    }

    /// Puts the clock at `time` and answers with what has to be painted,
    /// which is the bar when the clock moved and nothing while the desktop
    /// is not shown.
    pub const fn tick(&mut self, time: CivilTime) -> Rect {
        let moved = self.clock.set(time);
        if moved && self.shown {
            bar::rect(self.width)
        } else {
            Rect::EMPTY
        }
    }

    /// How many windows are open.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.windows.len()
    }

    /// `true` when no window is open.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// The windows, back to front, which is the order they are painted in.
    pub fn iter(&self) -> impl Iterator<Item = &Window> {
        self.windows.iter()
    }

    /// The widest content a window of this screen may have.
    #[must_use]
    pub const fn max_width(&self) -> u32 {
        self.width
            .saturating_sub(crate::window::BORDER.saturating_mul(2))
    }

    /// The tallest content it may have.
    #[must_use]
    pub const fn max_height(&self) -> u32 {
        self.height
            .saturating_sub(bar::HEIGHT)
            .saturating_sub(crate::window::TITLE_HEIGHT)
            .saturating_sub(crate::window::BORDER)
    }

    /// Opens a window of at most `width` by `height` for `badge`, which
    /// takes the focus from whatever had it.
    ///
    /// # Errors
    ///
    /// [`Error::AccessDenied`] for a request that carries no badge;
    /// [`Error::InvalidArgument`] for a window of no pixels or a screen
    /// with no room for one; [`Error::AlreadyExists`] when the client
    /// holds a window already; [`Error::QuotaExceeded`] when the desktop
    /// holds as many as it can.
    pub fn open(
        &mut self,
        badge: u64,
        width: u32,
        height: u32,
        title: Title,
        outcome: &mut Outcome,
    ) -> Result<u32, Error> {
        if badge == NOBODY {
            return Err(Error::AccessDenied);
        }
        if width == 0 || height == 0 {
            return Err(Error::InvalidArgument);
        }
        if self.of(badge).is_some() {
            return Err(Error::AlreadyExists);
        }
        if self.max_width() == 0 || self.max_height() == 0 {
            return Err(Error::InvalidArgument);
        }
        if self.windows.is_full() {
            return Err(Error::QuotaExceeded);
        }
        let step = u32::try_from(self.windows.len())
            .unwrap_or(0)
            .saturating_mul(CASCADE_STEP);
        let id = self.next_id;
        let mut window = Window::new(
            id,
            badge,
            0,
            bar::HEIGHT,
            width.min(self.max_width()),
            height.min(self.max_height()),
            title,
        );
        window.place(
            CASCADE_X.saturating_add(step),
            bar::HEIGHT.saturating_add(CASCADE_Y).saturating_add(step),
            self.width,
            self.height,
            bar::HEIGHT,
        );
        self.windows
            .push(window)
            .map_err(|_| Error::QuotaExceeded)?;
        self.next_id = self.next_id.saturating_add(1);
        let frame = self.window(id).map_or(Rect::EMPTY, Window::frame);
        self.give_focus(id, frame, outcome);
        Ok(id)
    }

    /// The window `badge` holds, if it holds one.
    #[must_use]
    pub fn of(&self, badge: u64) -> Option<&Window> {
        self.windows.iter().find(|window| window.badge() == badge)
    }

    /// The window of this number, whoever holds it.
    #[must_use]
    pub fn window(&self, id: u32) -> Option<&Window> {
        self.windows.iter().find(|window| window.id() == id)
    }

    /// The window `badge` may act on under the number `id`.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for a number no window has;
    /// [`Error::AccessDenied`] for the window of another client.
    pub fn holding(&self, badge: u64, id: u32) -> Result<&Window, Error> {
        let window = self.window(id).ok_or(Error::NotFound)?;
        if window.badge() != badge {
            return Err(Error::AccessDenied);
        }
        Ok(window)
    }

    /// The same, to change.
    ///
    /// # Errors
    ///
    /// As [`holding`](Self::holding).
    pub fn holding_mut(&mut self, badge: u64, id: u32) -> Result<&mut Window, Error> {
        let at = self.index_of(id).ok_or(Error::NotFound)?;
        let window = self.windows.get_mut(at).ok_or(Error::NotFound)?;
        if window.badge() != badge {
            return Err(Error::AccessDenied);
        }
        Ok(window)
    }

    /// Takes the oldest event of the window `badge` holds under `id`.
    ///
    /// # Errors
    ///
    /// As [`holding`](Self::holding).
    pub fn poll(&mut self, badge: u64, id: u32) -> Result<Option<Event>, Error> {
        Ok(self.holding_mut(badge, id)?.pop())
    }

    /// Closes the window `badge` holds under `id` and answers with what
    /// has to be painted. The window that is then in front takes the
    /// focus and hears that it has, which `outcome` carries.
    ///
    /// # Errors
    ///
    /// As [`holding`](Self::holding).
    pub fn close(&mut self, badge: u64, id: u32, outcome: &mut Outcome) -> Result<Rect, Error> {
        let frame = self.holding(badge, id)?.frame();
        self.drop_window(id, outcome);
        Ok(frame)
    }

    /// Forgets the window of a client that is gone, and answers with its
    /// number and what has to be painted. The focus moves as it does on a
    /// close.
    pub fn forget(&mut self, badge: u64, outcome: &mut Outcome) -> Option<(u32, Rect)> {
        let window = self.of(badge)?;
        let (id, frame) = (window.id(), window.frame());
        self.drop_window(id, outcome);
        Some((id, frame))
    }

    /// Where `id` stands in the order.
    fn index_of(&self, id: u32) -> Option<usize> {
        self.windows.iter().position(|window| window.id() == id)
    }

    /// Takes a window out of the order and gives the focus to the window
    /// that is then in front, which hears that it has it.
    fn drop_window(&mut self, id: u32, outcome: &mut Outcome) {
        let Some(at) = self.index_of(id) else {
            return;
        };
        self.windows.remove(at);
        if self.drag.is_some_and(|drag| drag.id == id) {
            self.drag = None;
        }
        if self.menu == Some(bar::WINDOW_MENU) {
            self.menu = None;
        }
        let Some(front) = self
            .windows
            .iter()
            .last()
            .map(|window| (window.id(), window.frame()))
        else {
            return;
        };
        self.give_focus(front.0, front.1, outcome);
    }

    /// Gives the focus to `id` and takes it from every other window.
    fn focus(&mut self, id: u32) -> bool {
        let mut changed = false;
        for window in self.windows.iter_mut() {
            changed |= window.focus(window.id() == id);
        }
        changed
    }

    /// Puts `id` in front of every other window and answers whether the
    /// order changed.
    fn raise(&mut self, id: u32) -> bool {
        let Some(at) = self.index_of(id) else {
            return false;
        };
        if at.saturating_add(1) == self.windows.len() {
            return false;
        }
        let Some(window) = self.windows.remove(at) else {
            return false;
        };
        let _room = self.windows.push(window);
        true
    }

    /// The window under `x`, `y`, the one in front first.
    fn front_at(&self, x: u32, y: u32) -> Option<&Window> {
        let mut index = self.windows.len();
        while index > 0 {
            index = index.saturating_sub(1);
            let found = self
                .windows
                .get(index)
                .filter(|window| window.frame().contains(x, y));
            if found.is_some() {
                return found;
            }
        }
        None
    }

    /// The window at `entry` counted from the front, which is the order
    /// the menu of the windows lists them in.
    fn nth_from_front(&self, entry: usize) -> Option<&Window> {
        let last = self.windows.len().checked_sub(1)?;
        self.windows.get(last.checked_sub(entry)?)
    }

    /// The window that has the focus.
    #[must_use]
    pub fn focused(&self) -> Option<&Window> {
        self.windows.iter().find(|window| window.is_focused())
    }

    /// How many entries the panel of the title at `index` holds.
    #[must_use]
    pub const fn entries(&self, index: usize) -> usize {
        if index == bar::DESK_MENU {
            bar::DESK_ENTRIES.len()
        } else {
            self.windows.len()
        }
    }

    /// What stands in the entry at `entry` of the panel of `index`.
    #[must_use]
    pub fn entry_text(&self, index: usize, entry: usize) -> &str {
        if index == bar::DESK_MENU {
            return bar::DESK_ENTRIES.get(entry).copied().unwrap_or_default();
        }
        self.nth_from_front(entry)
            .map_or_default(|window| window.title().as_str())
    }

    /// Takes one event of the keyboard or the pointer.
    pub fn feed(&mut self, event: Input) -> Outcome {
        match event {
            Input::Key(key) => self.key(key),
            Input::Pointer(pointer) => self.packet(pointer),
        }
    }

    /// Takes one key.
    fn key(&mut self, key: KeyEvent) -> Outcome {
        let mut outcome = Outcome::nothing();
        // The two keys of the desktop itself never reach a client: one
        // paints the screen, the other takes it down, and a program that
        // saw either would act on a key that was not typed at it.
        if key.code == SHOWS {
            if !key.pressed {
                outcome.paint(self.show());
            }
            return outcome;
        }
        // The desktop ends whether it was ever painted or not: the run
        // that boots a machine and drives nothing has to take it down all
        // the same, because the machine ends when every program that
        // reports has reported.
        if key.code == ENDS {
            if !key.pressed {
                self.end(&mut outcome);
            }
            return outcome;
        }
        if !self.shown {
            return outcome;
        }
        let Some(id) = self.focused().map(Window::id) else {
            return outcome;
        };
        self.send(
            id,
            Event::Key {
                code: key.code,
                pressed: key.pressed,
            },
        );
        outcome.wake(id);
        outcome
    }

    /// Tells every client that its window is going away.
    fn end(&mut self, outcome: &mut Outcome) {
        outcome.ended = true;
        for window in self.windows.iter_mut() {
            window.push(Event::Closed);
            outcome.wake(window.id());
        }
    }

    /// Puts `event` in the queue of `id`.
    fn send(&mut self, id: u32, event: Event) {
        for window in self.windows.iter_mut() {
            if window.id() == id {
                window.push(event);
                return;
            }
        }
    }

    /// Takes one packet of the pointer.
    fn packet(&mut self, pointer: PointerEvent) -> Outcome {
        let mut outcome = Outcome::nothing();
        let was = self.pointer;
        self.pointer = (
            step(self.pointer.0, pointer.dx, self.width),
            step(self.pointer.1, pointer.dy, self.height),
        );
        outcome.moved = self.pointer != was;
        if !self.shown {
            self.buttons = pointer.buttons;
            return outcome;
        }
        let held = pointer.buttons & BUTTON_LEFT != 0;
        let was_held = self.buttons & BUTTON_LEFT != 0;
        self.buttons = pointer.buttons;
        let (x, y) = self.pointer;
        if held && was_held {
            self.dragging(x, y, &mut outcome);
            return outcome;
        }
        if !held {
            self.drag = None;
        }
        if held && !was_held {
            self.press(x, y, &mut outcome);
            return outcome;
        }
        self.hover(x, y, &mut outcome);
        outcome
    }

    /// Moves the window a drag is holding.
    fn dragging(&mut self, x: u32, y: u32, outcome: &mut Outcome) {
        let Some(drag) = self.drag else {
            return;
        };
        let (width, height) = (self.width, self.height);
        let Some(window) = self
            .windows
            .iter_mut()
            .find(|window| window.id() == drag.id)
        else {
            return;
        };
        let before = window.frame();
        window.place(
            x.saturating_sub(drag.dx),
            y.saturating_sub(drag.dy),
            width,
            height,
            bar::HEIGHT,
        );
        let after = window.frame();
        if before != after {
            outcome.paint(before.union(after));
        }
    }

    /// Takes the button going down.
    fn press(&mut self, x: u32, y: u32, outcome: &mut Outcome) {
        if let Some(index) = self.menu {
            self.chose(index, x, y, outcome);
            return;
        }
        if bar::rect(self.width).contains(x, y) {
            self.menu = bar::title_at(x, y);
            outcome.paint(self.menu_area());
            return;
        }
        let Some(window) = self.front_at(x, y) else {
            return;
        };
        let (id, frame, closes, inside) = (
            window.id(),
            window.frame(),
            window.close_box().contains(x, y),
            window.inside(x, y),
        );
        if closes {
            self.send(id, Event::Closed);
            outcome.wake(id);
            return;
        }
        if self.raise(id) {
            outcome.paint(frame);
        }
        self.give_focus(id, frame, outcome);
        if let Some((content_x, content_y)) = inside {
            let buttons = self.buttons;
            self.send(
                id,
                Event::Pointer {
                    x: content_x,
                    y: content_y,
                    buttons,
                },
            );
            outcome.wake(id);
        }
        self.drag = window_grab(self.front_at(x, y), x, y);
    }

    /// Gives the focus to `id`, takes it from whoever had it, and tells
    /// both.
    fn give_focus(&mut self, id: u32, frame: Rect, outcome: &mut Outcome) {
        let before = self.focused().map(|window| (window.id(), window.frame()));
        if before.map(|(had, _frame)| had) == Some(id) {
            return;
        }
        if let Some((had, had_frame)) = before {
            outcome.paint(had_frame);
            self.send(had, Event::Focus { has: false });
            outcome.wake(had);
        }
        self.focus(id);
        outcome.paint(frame);
        self.send(id, Event::Focus { has: true });
        outcome.wake(id);
    }

    /// Takes a click while a menu is open.
    fn chose(&mut self, index: usize, x: u32, y: u32, outcome: &mut Outcome) {
        let entries = self.entries(index);
        let chosen = bar::entry_at(index, entries, x, y);
        outcome.paint(self.menu_area());
        self.menu = None;
        let Some(entry) = chosen else {
            return;
        };
        if index == bar::DESK_MENU {
            match entry {
                0 => {
                    if let Some(id) = self.focused().map(Window::id) {
                        self.send(id, Event::Closed);
                        outcome.wake(id);
                    }
                }
                _quit => self.end(outcome),
            }
            return;
        }
        let Some((id, frame)) = self
            .nth_from_front(entry)
            .map(|window| (window.id(), window.frame()))
        else {
            return;
        };
        self.raise(id);
        self.give_focus(id, frame, outcome);
        outcome.paint(self.bounds());
    }

    /// Takes the pointer moving with no button held.
    fn hover(&mut self, x: u32, y: u32, outcome: &mut Outcome) {
        let Some(window) = self.front_at(x, y) else {
            return;
        };
        let id = window.id();
        let Some((content_x, content_y)) = window.inside(x, y) else {
            return;
        };
        let buttons = self.buttons;
        self.send(
            id,
            Event::Pointer {
                x: content_x,
                y: content_y,
                buttons,
            },
        );
        outcome.wake(id);
    }

    /// The bar and the panel that hangs from it, which is what a menu
    /// opening or closing changes.
    fn menu_area(&self) -> Rect {
        let bar = bar::rect(self.width);
        match self.menu {
            Some(index) => bar.union(bar::panel_rect(index, self.entries(index))),
            None => bar.union(Rect::new(0, bar::HEIGHT, self.width, panel_depth())),
        }
    }

    /// Fills `region` with the desktop, which is what stands under every
    /// window.
    pub fn paint_desktop(&self, screen: &mut Surface<'_>, region: Rect) -> Rect {
        screen.fill(region.intersect(self.bounds()), DESKTOP)
    }

    /// Paints the frame of `window`: its border, its title bar, the title
    /// in it, and the box that closes it. The content is the surface the
    /// process holds and is blitted over the middle of what this fills.
    pub fn paint_chrome(&self, screen: &mut Surface<'_>, window: &Window) -> Rect {
        let frame = window.frame();
        screen.fill(frame, window.edge());
        let (x, y) = window.title_at();
        draw_text(
            screen,
            x,
            y,
            window.title().as_str(),
            crate::window::TITLE_INK,
            None,
        );
        screen.fill(window.close_box(), crate::window::CLOSE_INK);
        frame
    }

    /// Paints the bar over everything: the titles, the panel of an open
    /// title, and the clock at the right end.
    pub fn paint_bar(&self, screen: &mut Surface<'_>) -> Rect {
        let bar = bar::rect(self.width);
        screen.fill(bar, bar::BACKGROUND);
        screen.fill(
            Rect::new(0, bar.bottom().saturating_sub(1), self.width, 1),
            bar::EDGE,
        );
        for (index, title) in bar::TITLES.iter().enumerate() {
            let cell = bar::title_rect(index);
            let open = self.menu == Some(index);
            let (ink, behind) = if open {
                (bar::HIGHLIGHT_INK, bar::HIGHLIGHT)
            } else {
                (bar::INK, bar::BACKGROUND)
            };
            if open {
                screen.fill(cell, behind);
            }
            draw_text(
                screen,
                cell.x.saturating_add(bar::TITLE_PADDING),
                bar::text_top(),
                title,
                ink,
                None,
            );
        }
        let clock = bar::clock_rect(self.width);
        let text = self.clock.text();
        let shown: user_proto::Bytes<{ bar::CLOCK_LEN }> =
            user_proto::Bytes::filled(text, bar::CLOCK_LEN);
        draw_text(
            screen,
            clock.x,
            clock.y,
            shown.as_str(),
            bar::INK,
            Some(bar::BACKGROUND),
        );
        match self.menu {
            Some(index) => bar.union(self.paint_panel(screen, index)),
            None => bar,
        }
    }

    /// Paints the panel of the open title.
    fn paint_panel(&self, screen: &mut Surface<'_>, index: usize) -> Rect {
        let entries = self.entries(index);
        let panel = bar::panel_rect(index, entries);
        if panel.is_empty() {
            return Rect::EMPTY;
        }
        screen.fill(panel, bar::BACKGROUND);
        gfx::draw::frame(screen, panel, bar::EDGE);
        for entry in 0..entries {
            let row = bar::entry_rect(index, entries, entry);
            draw_text(
                screen,
                row.x.saturating_add(bar::TITLE_PADDING),
                row.y.saturating_add(
                    bar::ENTRY_HEIGHT
                        .saturating_sub(gfx::GLYPH_HEIGHT)
                        .wrapping_div(2),
                ),
                self.entry_text(index, entry),
                bar::INK,
                None,
            );
        }
        panel
    }
}

/// How deep the deepest panel hangs, which is what closing a menu has to
/// paint over when the panel it closed is already forgotten.
fn panel_depth() -> u32 {
    u32::try_from(WINDOWS.max(bar::DESK_ENTRIES.len()))
        .unwrap_or(0)
        .saturating_mul(bar::ENTRY_HEIGHT)
}

/// The drag `window` starts when it is grabbed at `x`, `y`, which is a
/// drag only when the grab is on the title bar and not on the close box.
fn window_grab(window: Option<&Window>, x: u32, y: u32) -> Option<Drag> {
    let window = window?;
    if !window.title_bar().contains(x, y) || window.close_box().contains(x, y) {
        return None;
    }
    let frame = window.frame();
    Some(Drag {
        id: window.id(),
        dx: x.saturating_sub(frame.x),
        dy: y.saturating_sub(frame.y),
    })
}

/// `position` moved by `delta` and clamped to a screen of this many
/// pixels.
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
