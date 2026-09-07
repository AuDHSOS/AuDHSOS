// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The client side of the keyboard: which modifiers are held, and which
//! character a key stands for under them.
//!
//! The server sends key codes, which name keys and not characters: the same
//! key types `y` on one layout and `z` on another, and most keys type
//! nothing at all. A client that wants text runs the events through this,
//! which keeps the modifier state and answers with a character when the key
//! stands for one.
//!
//! Two layouts are carried, `us` and `de`. They differ in the punctuation,
//! in the two letters the German layout swaps, and in nothing else; a
//! layout is a table of key, unshifted character, and shifted character,
//! and a key that is not in the table stands for no character.
//!
//! Invariants: the modifier state follows presses and releases and never
//! goes out of step, because a release of a modifier that was never pressed
//! clears a flag that is already clear; caps lock changes letters and
//! nothing else, which is what a caps lock does and what a shift does not.

use driver_i8042::keyboard::{KeyCode, KeyEvent};

/// Which layout a keyboard types in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Layout {
    /// The United States layout.
    #[default]
    Us,
    /// The German layout.
    De,
}

impl Layout {
    /// Every layout, in table order.
    pub const ALL: &[Layout] = &[Layout::Us, Layout::De];

    /// The name of the layout.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Layout::Us => "us",
            Layout::De => "de",
        }
    }

    /// The table of the layout.
    #[must_use]
    pub const fn table(self) -> &'static [(KeyCode, char, char)] {
        match self {
            Layout::Us => US,
            Layout::De => DE,
        }
    }
}

/// The United States layout: key, unshifted, shifted.
pub const US: &[(KeyCode, char, char)] = &[
    (KeyCode::A, 'a', 'A'),
    (KeyCode::B, 'b', 'B'),
    (KeyCode::C, 'c', 'C'),
    (KeyCode::D, 'd', 'D'),
    (KeyCode::E, 'e', 'E'),
    (KeyCode::F, 'f', 'F'),
    (KeyCode::G, 'g', 'G'),
    (KeyCode::H, 'h', 'H'),
    (KeyCode::I, 'i', 'I'),
    (KeyCode::J, 'j', 'J'),
    (KeyCode::K, 'k', 'K'),
    (KeyCode::L, 'l', 'L'),
    (KeyCode::M, 'm', 'M'),
    (KeyCode::N, 'n', 'N'),
    (KeyCode::O, 'o', 'O'),
    (KeyCode::P, 'p', 'P'),
    (KeyCode::Q, 'q', 'Q'),
    (KeyCode::R, 'r', 'R'),
    (KeyCode::S, 's', 'S'),
    (KeyCode::T, 't', 'T'),
    (KeyCode::U, 'u', 'U'),
    (KeyCode::V, 'v', 'V'),
    (KeyCode::W, 'w', 'W'),
    (KeyCode::X, 'x', 'X'),
    (KeyCode::Y, 'y', 'Y'),
    (KeyCode::Z, 'z', 'Z'),
    (KeyCode::Backquote, '`', '~'),
    (KeyCode::Digit1, '1', '!'),
    (KeyCode::Digit2, '2', '@'),
    (KeyCode::Digit3, '3', '#'),
    (KeyCode::Digit4, '4', '$'),
    (KeyCode::Digit5, '5', '%'),
    (KeyCode::Digit6, '6', '^'),
    (KeyCode::Digit7, '7', '&'),
    (KeyCode::Digit8, '8', '*'),
    (KeyCode::Digit9, '9', '('),
    (KeyCode::Digit0, '0', ')'),
    (KeyCode::Minus, '-', '_'),
    (KeyCode::Equal, '=', '+'),
    (KeyCode::LeftBracket, '[', '{'),
    (KeyCode::RightBracket, ']', '}'),
    (KeyCode::Backslash, '\\', '|'),
    (KeyCode::IntlBackslash, '\\', '|'),
    (KeyCode::Semicolon, ';', ':'),
    (KeyCode::Quote, '\'', '"'),
    (KeyCode::Comma, ',', '<'),
    (KeyCode::Period, '.', '>'),
    (KeyCode::Slash, '/', '?'),
    (KeyCode::Numpad0, '0', '0'),
    (KeyCode::Numpad1, '1', '1'),
    (KeyCode::Numpad2, '2', '2'),
    (KeyCode::Numpad3, '3', '3'),
    (KeyCode::Numpad4, '4', '4'),
    (KeyCode::Numpad5, '5', '5'),
    (KeyCode::Numpad6, '6', '6'),
    (KeyCode::Numpad7, '7', '7'),
    (KeyCode::Numpad8, '8', '8'),
    (KeyCode::Numpad9, '9', '9'),
    (KeyCode::NumpadDivide, '/', '/'),
    (KeyCode::NumpadMultiply, '*', '*'),
    (KeyCode::NumpadMinus, '-', '-'),
    (KeyCode::NumpadPlus, '+', '+'),
    (KeyCode::NumpadPeriod, '.', '.'),
    (KeyCode::NumpadEnter, '\n', '\n'),
    (KeyCode::Space, ' ', ' '),
    (KeyCode::Enter, '\n', '\n'),
    (KeyCode::Tab, '\t', '\t'),
];

/// The German layout: key, unshifted, shifted. It swaps the two keys
/// the United States layout calls `Y` and `Z`.
pub const DE: &[(KeyCode, char, char)] = &[
    (KeyCode::A, 'a', 'A'),
    (KeyCode::B, 'b', 'B'),
    (KeyCode::C, 'c', 'C'),
    (KeyCode::D, 'd', 'D'),
    (KeyCode::E, 'e', 'E'),
    (KeyCode::F, 'f', 'F'),
    (KeyCode::G, 'g', 'G'),
    (KeyCode::H, 'h', 'H'),
    (KeyCode::I, 'i', 'I'),
    (KeyCode::J, 'j', 'J'),
    (KeyCode::K, 'k', 'K'),
    (KeyCode::L, 'l', 'L'),
    (KeyCode::M, 'm', 'M'),
    (KeyCode::N, 'n', 'N'),
    (KeyCode::O, 'o', 'O'),
    (KeyCode::P, 'p', 'P'),
    (KeyCode::Q, 'q', 'Q'),
    (KeyCode::R, 'r', 'R'),
    (KeyCode::S, 's', 'S'),
    (KeyCode::T, 't', 'T'),
    (KeyCode::U, 'u', 'U'),
    (KeyCode::V, 'v', 'V'),
    (KeyCode::W, 'w', 'W'),
    (KeyCode::X, 'x', 'X'),
    (KeyCode::Y, 'z', 'Z'),
    (KeyCode::Z, 'y', 'Y'),
    (KeyCode::Backquote, '^', '°'),
    (KeyCode::Digit1, '1', '!'),
    (KeyCode::Digit2, '2', '"'),
    (KeyCode::Digit3, '3', '§'),
    (KeyCode::Digit4, '4', '$'),
    (KeyCode::Digit5, '5', '%'),
    (KeyCode::Digit6, '6', '&'),
    (KeyCode::Digit7, '7', '/'),
    (KeyCode::Digit8, '8', '('),
    (KeyCode::Digit9, '9', ')'),
    (KeyCode::Digit0, '0', '='),
    (KeyCode::Minus, 'ß', '?'),
    (KeyCode::Equal, '´', '`'),
    (KeyCode::LeftBracket, 'ü', 'Ü'),
    (KeyCode::RightBracket, '+', '*'),
    (KeyCode::Backslash, '#', '\''),
    (KeyCode::IntlBackslash, '<', '>'),
    (KeyCode::Semicolon, 'ö', 'Ö'),
    (KeyCode::Quote, 'ä', 'Ä'),
    (KeyCode::Comma, ',', ';'),
    (KeyCode::Period, '.', ':'),
    (KeyCode::Slash, '-', '_'),
    (KeyCode::Numpad0, '0', '0'),
    (KeyCode::Numpad1, '1', '1'),
    (KeyCode::Numpad2, '2', '2'),
    (KeyCode::Numpad3, '3', '3'),
    (KeyCode::Numpad4, '4', '4'),
    (KeyCode::Numpad5, '5', '5'),
    (KeyCode::Numpad6, '6', '6'),
    (KeyCode::Numpad7, '7', '7'),
    (KeyCode::Numpad8, '8', '8'),
    (KeyCode::Numpad9, '9', '9'),
    (KeyCode::NumpadDivide, '/', '/'),
    (KeyCode::NumpadMultiply, '*', '*'),
    (KeyCode::NumpadMinus, '-', '-'),
    (KeyCode::NumpadPlus, '+', '+'),
    (KeyCode::NumpadPeriod, '.', '.'),
    (KeyCode::NumpadEnter, '\n', '\n'),
    (KeyCode::Space, ' ', ' '),
    (KeyCode::Enter, '\n', '\n'),
    (KeyCode::Tab, '\t', '\t'),
];

/// Which modifiers are held.
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per modifier key, which is what the state of a keyboard is"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    /// Either shift key is down.
    pub shift: bool,
    /// Either control key is down.
    pub control: bool,
    /// The left alt key is down.
    pub alt: bool,
    /// The right alt key is down, which is the third level of a layout that
    /// has one.
    pub alt_graph: bool,
    /// Either meta key is down.
    pub meta: bool,
    /// Caps lock is on.
    pub caps: bool,
}

/// A keyboard as a client sees it: a layout and what is held down.
#[derive(Clone, Copy, Debug, Default)]
pub struct Keyboard {
    layout: Layout,
    modifiers: Modifiers,
}

impl Keyboard {
    /// A keyboard typing in `layout` with nothing held.
    #[must_use]
    pub const fn new(layout: Layout) -> Self {
        Keyboard {
            layout,
            modifiers: Modifiers {
                shift: false,
                control: false,
                alt: false,
                alt_graph: false,
                meta: false,
                caps: false,
            },
        }
    }

    /// Which layout it types in.
    #[must_use]
    pub const fn layout(&self) -> Layout {
        self.layout
    }

    /// Which modifiers are held.
    #[must_use]
    pub const fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Takes one event and answers with the character it typed, if it typed
    /// one.
    ///
    /// A release types nothing, and neither does a key that stands for no
    /// character. The modifier state follows both.
    pub fn feed(&mut self, event: KeyEvent) -> Option<char> {
        if self.remember(event) {
            return None;
        }
        if !event.pressed {
            return None;
        }
        self.character(event.code)
    }

    /// Which character `code` stands for under the modifiers that are held.
    #[must_use]
    pub fn character(&self, code: KeyCode) -> Option<char> {
        let (low, high) = self
            .layout
            .table()
            .iter()
            .find(|(key, _low, _high)| *key == code)
            .map(|(_key, low, high)| (*low, *high))?;
        // Caps lock is the shift of the letters and of nothing else, so a
        // key whose two characters are the same letter in two cases is the
        // only one it touches.
        let letter = low != high && low.is_alphabetic() && high.is_alphabetic();
        let shifted = if letter {
            self.modifiers.shift != self.modifiers.caps
        } else {
            self.modifiers.shift
        };
        Some(if shifted { high } else { low })
    }

    /// Notes a modifier, and says whether the event was one.
    const fn remember(&mut self, event: KeyEvent) -> bool {
        let down = event.pressed;
        match event.code {
            KeyCode::LeftShift | KeyCode::RightShift => self.modifiers.shift = down,
            KeyCode::LeftControl | KeyCode::RightControl => self.modifiers.control = down,
            KeyCode::LeftAlt => self.modifiers.alt = down,
            KeyCode::RightAlt => self.modifiers.alt_graph = down,
            KeyCode::LeftMeta | KeyCode::RightMeta => self.modifiers.meta = down,
            // The lock turns over when the key goes down and stays where it
            // is when the key comes up.
            KeyCode::CapsLock => {
                if down {
                    self.modifiers.caps = !self.modifiers.caps;
                }
            }
            _other => return false,
        }
        true
    }
}
