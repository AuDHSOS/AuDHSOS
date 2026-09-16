// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The shell: the line being typed, the scrollback above it, and the draw
//! commands both become.
//!
//! One window event goes in and one [`Step`] comes out, which says what the
//! program around it has to do: paint the window, run the line that was
//! entered, or let the window go. Nothing here is a system call and nothing
//! here is a message.
//!
//! What is painted is two things of different cost. A keystroke changes the
//! line being typed and nothing else, so it paints one row; anything that
//! was printed paints the whole window. [`Shell::paint`] says which of the
//! two is due, and the program clears it with [`Shell::painted`] once the
//! commands are on their way.
//!
//! Invariants: the line being typed holds at most [`MAX_INPUT`] printable
//! characters of ASCII, so the prompt, the line and the caret together
//! never exceed one row; every row of commands fits one draw message,
//! because [`Shell::render`] answers with where to continue when the list
//! is full.

use gfx::draw::{Command, List, Text};
use gfx::{Color, GLYPH_HEIGHT, GLYPH_WIDTH};
use user_proto::Bytes;
use user_proto::input::{KeyCode, KeyEvent};
use user_proto::keyboard::{Keyboard, Layout};
use user_proto::window::Event;

use crate::screen::{COLUMNS, ROWS, Row, Screen};

/// What stands before the line being typed.
pub const PROMPT: &str = "$ ";

/// What stands behind it while the window has the focus.
pub const CARET: u8 = b'_';

/// How many characters the line being typed holds: one row, less the
/// prompt and the caret.
pub const MAX_INPUT: usize = COLUMNS - 3;

/// The line being typed, and the line that was entered.
pub type Line = Bytes<MAX_INPUT>;

/// What the window is filled with.
pub const BACKGROUND: Color = Color::new(0x0C, 0x10, 0x18);

/// What the scrollback is written in.
pub const INK: Color = Color::new(0xC8, 0xE0, 0xC8);

/// What the prompt and the line being typed are written in.
pub const PROMPT_INK: Color = Color::new(0x60, 0xC0, 0xF0);

/// How wide the content of the window is: one glyph per column.
pub const WIDTH: u32 = GLYPH_WIDTH.saturating_mul(56);

/// How tall it is: one glyph per row of scrollback, and one row more for
/// the line being typed.
pub const HEIGHT: u32 = GLYPH_HEIGHT.saturating_mul(23);

const _: () = assert!(COLUMNS == 56);
const _: () = assert!(ROWS == 22);
const _: () = assert!(PROMPT.len() == 2);

/// What has to be painted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Paint {
    /// The window on the screen is what the shell holds.
    #[default]
    Nothing,
    /// The line being typed changed and nothing else did.
    Input,
    /// Everything changed.
    All,
}

/// What one event changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    /// Nothing the program has to act on.
    Nothing,
    /// The window has to be painted.
    Painted,
    /// A line was entered and waits to be run.
    Ready,
    /// The window is going away.
    Ended,
}

/// The shell over one window.
#[derive(Clone, Copy, Debug)]
pub struct Shell {
    /// The rows above the line being typed.
    screen: Screen,
    /// The line being typed.
    input: Line,
    /// The layout and the modifiers that are held.
    keyboard: Keyboard,
    /// The line that was entered and not yet taken.
    ready: Option<Line>,
    /// Whether the window has the focus, which is whether the caret shows.
    focused: bool,
    /// What has to be painted.
    paint: Paint,
    /// Whether the window is going away.
    ended: bool,
}

impl Default for Shell {
    fn default() -> Self {
        Shell::new()
    }
}

impl Shell {
    /// A shell with nothing printed and nothing typed, with everything to
    /// paint, typing on the United States layout.
    #[must_use]
    pub const fn new() -> Self {
        Shell::with_layout(Layout::Us)
    }

    /// The same, typing on `layout`.
    #[must_use]
    pub const fn with_layout(layout: Layout) -> Self {
        Shell {
            screen: Screen::new(),
            input: Line::empty(),
            keyboard: Keyboard::new(layout),
            ready: None,
            focused: false,
            paint: Paint::All,
            ended: false,
        }
    }

    /// The scrollback.
    #[must_use]
    pub const fn screen(&self) -> &Screen {
        &self.screen
    }

    /// The line being typed.
    #[must_use]
    pub fn input(&self) -> &str {
        self.input.as_str()
    }

    /// What has to be painted.
    #[must_use]
    pub const fn paint(&self) -> Paint {
        self.paint
    }

    /// Forgets what has to be painted, which the program does once the
    /// commands are on their way.
    pub const fn painted(&mut self) {
        self.paint = Paint::Nothing;
    }

    /// Whether the window is going away.
    #[must_use]
    pub const fn has_ended(&self) -> bool {
        self.ended
    }

    /// Writes `text` into the scrollback.
    pub fn print(&mut self, text: &str) {
        self.screen.print(text);
        self.paint = Paint::All;
    }

    /// Forgets the scrollback.
    pub const fn clear(&mut self) {
        self.screen.clear();
        self.paint = Paint::All;
    }

    /// Takes the line that was entered, if one waits.
    pub const fn take_line(&mut self) -> Option<Line> {
        self.ready.take()
    }

    /// Takes one event of the window.
    pub fn feed(&mut self, event: Event) -> Step {
        match event {
            Event::Key { code, pressed } => self.key(KeyEvent { code, pressed }),
            Event::Pointer { .. } => Step::Nothing,
            Event::Focus { has } => {
                self.focused = has;
                self.paint = heavier(self.paint, Paint::Input);
                Step::Painted
            }
            Event::Closed => {
                self.ended = true;
                Step::Ended
            }
        }
    }

    /// Takes one key.
    fn key(&mut self, key: KeyEvent) -> Step {
        let typed = self.keyboard.feed(key);
        if key.pressed && key.code == KeyCode::Backspace {
            return self.backspace();
        }
        match typed {
            Some('\n') => self.enter(),
            Some(character) => self.type_in(character),
            None => Step::Nothing,
        }
    }

    /// Takes the last character of the line being typed away.
    fn backspace(&mut self) -> Step {
        let Some(last) = self.input.len().checked_sub(1) else {
            return Step::Nothing;
        };
        self.input = Line::filled(self.typed(), last);
        self.paint = heavier(self.paint, Paint::Input);
        Step::Painted
    }

    /// Puts one character at the end of the line being typed.
    ///
    /// A character the font has no glyph for is not taken: this system
    /// draws the printable characters of ASCII and nothing else, so a
    /// layout that types `ö` types nothing here.
    fn type_in(&mut self, character: char) -> Step {
        let Ok(byte) = u8::try_from(character) else {
            return Step::Nothing;
        };
        if !byte.is_ascii_graphic() && byte != b' ' {
            return Step::Nothing;
        }
        let mut bytes = self.typed();
        let len = self.input.len();
        let mut written = len;
        for slot in bytes.iter_mut().skip(len).take(1) {
            *slot = byte;
            written = len.saturating_add(1);
        }
        if written == len {
            return Step::Nothing;
        }
        self.input = Line::filled(bytes, written);
        self.paint = heavier(self.paint, Paint::Input);
        Step::Painted
    }

    /// The line being typed, in an array of the width it may grow to.
    fn typed(&self) -> [u8; MAX_INPUT] {
        let mut bytes = [0_u8; MAX_INPUT];
        for (slot, byte) in bytes.iter_mut().zip(self.input.as_bytes()) {
            *slot = *byte;
        }
        bytes
    }

    /// Takes the line that was typed: it goes into the scrollback behind
    /// the prompt and waits to be run.
    fn enter(&mut self) -> Step {
        let line = self.input;
        self.input = Line::empty();
        let mut row = [0_u8; COLUMNS];
        let len = fill(&mut row, PROMPT, line.as_str(), None);
        self.screen.print(Row::filled(row, len).as_str());
        self.ready = Some(line);
        self.paint = Paint::All;
        Step::Ready
    }

    /// Writes the draw commands of the rows from `from` into `into`, and
    /// answers with where to continue when the list filled up before the
    /// last row.
    ///
    /// Row zero clears the window, so a caller that begins there paints
    /// over everything that stood in it.
    pub fn render(&self, from: usize, into: &mut List) -> Option<usize> {
        let mut row = from;
        if row == 0 && !into.push(Command::Clear { color: BACKGROUND }) {
            return Some(0);
        }
        while row < ROWS {
            let text = self.screen.row(row);
            if !text.is_empty() {
                let Some(command) = line_command(text, row, INK) else {
                    row = row.saturating_add(1);
                    continue;
                };
                if !into.push(command) {
                    return Some(row);
                }
            }
            row = row.saturating_add(1);
        }
        if !self.render_input(into) {
            return Some(ROWS);
        }
        None
    }

    /// Writes the draw command of the line being typed, and answers
    /// whether it fitted into `into`.
    pub fn render_input(&self, into: &mut List) -> bool {
        let mut row = [0_u8; COLUMNS];
        let caret = if self.focused { Some(CARET) } else { None };
        let len = fill(&mut row, PROMPT, self.input(), caret);
        let Some(command) = line_command(Row::filled(row, len).as_str(), ROWS, PROMPT_INK) else {
            return true;
        };
        into.push(command)
    }
}

/// The heavier of what is already due and what a change adds.
const fn heavier(standing: Paint, added: Paint) -> Paint {
    match (standing, added) {
        (Paint::All, _other) | (_other, Paint::All) => Paint::All,
        (Paint::Input, _other) | (_other, Paint::Input) => Paint::Input,
        (Paint::Nothing, Paint::Nothing) => Paint::Nothing,
    }
}

/// Puts the prompt, the line and the caret into `row`, and answers with how
/// many bytes they took.
fn fill(row: &mut [u8; COLUMNS], prompt: &str, line: &str, caret: Option<u8>) -> usize {
    let mut len = 0;
    for byte in prompt.bytes().chain(line.bytes()) {
        if let Some(slot) = row.get_mut(len) {
            *slot = byte;
            len = len.saturating_add(1);
        }
    }
    if let (Some(caret), Some(slot)) = (caret, row.get_mut(len)) {
        *slot = caret;
        len = len.saturating_add(1);
    }
    len
}

/// The command that writes one row at `row`, or nothing for a row the
/// command cannot carry.
fn line_command(text: &str, row: usize, color: Color) -> Option<Command> {
    let y = u32::try_from(row).unwrap_or(0).saturating_mul(GLYPH_HEIGHT);
    Some(Command::Text {
        x: 0,
        y,
        color,
        background: Some(BACKGROUND),
        text: Text::new(text)?,
    })
}
