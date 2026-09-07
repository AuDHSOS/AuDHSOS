// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The keys of a 105-key layout, how scancode set 2 spells them, and the
//! state machine that turns a stream of bytes into presses and releases.
//!
//! Set 2 is a prefix code. A plain byte is a key going down; `F0` before it
//! makes it a key coming up; `E0` before either picks the second table, the
//! one the keys that were added after the original layout stand in; `E1`
//! begins the one sequence that is neither, the eight bytes of the pause
//! key.
//!
//! A key is named here and not by its scancode, because the same byte means
//! two keys depending on the prefix — `0x71` is the point of the numeric
//! keypad and, after `E0`, the delete key — and because a layout maps names
//! to characters and never bytes.
//!
//! Invariants: the state machine holds one byte of history and a counter
//! that never leaves `0..8`, so no stream can make it grow; a byte that
//! spells no key of the table is dropped and leaves the machine in
//! [`State::Idle`], so one unknown byte costs one event and never the next.

/// How scancode set 2 spells one key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// One byte, on its own.
    Plain(u8),
    /// One byte behind the `E0` prefix.
    Extended(u8),
    /// The eight-byte sequence behind the `E1` prefix, which one key has
    /// and no other. The byte is the prefix itself.
    Sequence(u8),
}

/// The prefix that picks the second table.
pub const EXTENDED_PREFIX: u8 = 0xE0;

/// The prefix that says the key is coming up.
pub const RELEASE_PREFIX: u8 = 0xF0;

/// The prefix of the one sequence that is neither.
pub const PAUSE_PREFIX: u8 = 0xE1;

/// How many bytes follow [`PAUSE_PREFIX`]: `14 77 E1 F0 14 F0 77`.
pub const PAUSE_TAIL: u8 = 7;

/// Declares the key table once and derives the enum, the codes, the names,
/// and the encodings from it.
macro_rules! key_codes {
    ($($variant:ident = $code:literal, $kind:ident($scan:literal)),+ $(,)?) => {
        /// One key of a 105-key layout, by name and with a stable code.
        ///
        /// The codes are what an [`crate::keyboard::KeyEvent`] carries over
        /// a message, so they are part of the input protocol and never
        /// change once given out.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u16)]
        pub enum KeyCode {
            $(
                #[doc = concat!("The ", stringify!($variant), " key.")]
                $variant = $code,
            )+
        }

        impl KeyCode {
            /// Every key, in table order.
            pub const ALL: &[KeyCode] = &[$(KeyCode::$variant),+];

            /// The stable code of the key.
            #[must_use]
            #[expect(
                clippy::as_conversions,
                reason = "discriminant of a repr(u16) enum in a const fn"
            )]
            pub const fn code(self) -> u16 {
                self as u16
            }

            /// Decodes a key code.
            #[must_use]
            pub const fn from_code(code: u16) -> Option<Self> {
                match code {
                    $($code => Some(KeyCode::$variant),)+
                    _ => None,
                }
            }

            /// The name of the key.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(KeyCode::$variant => stringify!($variant),)+
                }
            }

            /// How scancode set 2 spells the key.
            #[must_use]
            pub const fn encoding(self) -> Encoding {
                match self {
                    $(KeyCode::$variant => Encoding::$kind($scan),)+
                }
            }
        }
    };
}

key_codes! {
    Escape = 1, Plain(0x76),
    F1 = 2, Plain(0x05),
    F2 = 3, Plain(0x06),
    F3 = 4, Plain(0x04),
    F4 = 5, Plain(0x0C),
    F5 = 6, Plain(0x03),
    F6 = 7, Plain(0x0B),
    F7 = 8, Plain(0x83),
    F8 = 9, Plain(0x0A),
    F9 = 10, Plain(0x01),
    F10 = 11, Plain(0x09),
    F11 = 12, Plain(0x78),
    F12 = 13, Plain(0x07),
    Backquote = 14, Plain(0x0E),
    Digit1 = 15, Plain(0x16),
    Digit2 = 16, Plain(0x1E),
    Digit3 = 17, Plain(0x26),
    Digit4 = 18, Plain(0x25),
    Digit5 = 19, Plain(0x2E),
    Digit6 = 20, Plain(0x36),
    Digit7 = 21, Plain(0x3D),
    Digit8 = 22, Plain(0x3E),
    Digit9 = 23, Plain(0x46),
    Digit0 = 24, Plain(0x45),
    Minus = 25, Plain(0x4E),
    Equal = 26, Plain(0x55),
    Backspace = 27, Plain(0x66),
    Tab = 28, Plain(0x0D),
    Q = 29, Plain(0x15),
    W = 30, Plain(0x1D),
    E = 31, Plain(0x24),
    R = 32, Plain(0x2D),
    T = 33, Plain(0x2C),
    Y = 34, Plain(0x35),
    U = 35, Plain(0x3C),
    I = 36, Plain(0x43),
    O = 37, Plain(0x44),
    P = 38, Plain(0x4D),
    LeftBracket = 39, Plain(0x54),
    RightBracket = 40, Plain(0x5B),
    Backslash = 41, Plain(0x5D),
    CapsLock = 42, Plain(0x58),
    A = 43, Plain(0x1C),
    S = 44, Plain(0x1B),
    D = 45, Plain(0x23),
    F = 46, Plain(0x2B),
    G = 47, Plain(0x34),
    H = 48, Plain(0x33),
    J = 49, Plain(0x3B),
    K = 50, Plain(0x42),
    L = 51, Plain(0x4B),
    Semicolon = 52, Plain(0x4C),
    Quote = 53, Plain(0x52),
    Enter = 54, Plain(0x5A),
    LeftShift = 55, Plain(0x12),
    IntlBackslash = 56, Plain(0x61),
    Z = 57, Plain(0x1A),
    X = 58, Plain(0x22),
    C = 59, Plain(0x21),
    V = 60, Plain(0x2A),
    B = 61, Plain(0x32),
    N = 62, Plain(0x31),
    M = 63, Plain(0x3A),
    Comma = 64, Plain(0x41),
    Period = 65, Plain(0x49),
    Slash = 66, Plain(0x4A),
    RightShift = 67, Plain(0x59),
    LeftControl = 68, Plain(0x14),
    LeftMeta = 69, Extended(0x1F),
    LeftAlt = 70, Plain(0x11),
    Space = 71, Plain(0x29),
    RightAlt = 72, Extended(0x11),
    RightMeta = 73, Extended(0x27),
    Menu = 74, Extended(0x2F),
    RightControl = 75, Extended(0x14),
    PrintScreen = 76, Extended(0x7C),
    ScrollLock = 77, Plain(0x7E),
    Pause = 78, Sequence(0xE1),
    Insert = 79, Extended(0x70),
    Home = 80, Extended(0x6C),
    PageUp = 81, Extended(0x7D),
    Delete = 82, Extended(0x71),
    End = 83, Extended(0x69),
    PageDown = 84, Extended(0x7A),
    ArrowUp = 85, Extended(0x75),
    ArrowLeft = 86, Extended(0x6B),
    ArrowDown = 87, Extended(0x72),
    ArrowRight = 88, Extended(0x74),
    NumLock = 89, Plain(0x77),
    NumpadDivide = 90, Extended(0x4A),
    NumpadMultiply = 91, Plain(0x7C),
    NumpadMinus = 92, Plain(0x7B),
    NumpadPlus = 93, Plain(0x79),
    NumpadEnter = 94, Extended(0x5A),
    NumpadPeriod = 95, Plain(0x71),
    Numpad0 = 96, Plain(0x70),
    Numpad1 = 97, Plain(0x69),
    Numpad2 = 98, Plain(0x72),
    Numpad3 = 99, Plain(0x7A),
    Numpad4 = 100, Plain(0x6B),
    Numpad5 = 101, Plain(0x73),
    Numpad6 = 102, Plain(0x74),
    Numpad7 = 103, Plain(0x6C),
    Numpad8 = 104, Plain(0x75),
    Numpad9 = 105, Plain(0x7D),
}

impl KeyCode {
    /// The key `scan` spells on its own, if any does.
    #[must_use]
    pub fn from_set2(scan: u8) -> Option<KeyCode> {
        KeyCode::ALL
            .iter()
            .copied()
            .find(|key| key.encoding() == Encoding::Plain(scan))
    }

    /// The key `scan` spells behind [`EXTENDED_PREFIX`], if any does.
    #[must_use]
    pub fn from_set2_extended(scan: u8) -> Option<KeyCode> {
        KeyCode::ALL
            .iter()
            .copied()
            .find(|key| key.encoding() == Encoding::Extended(scan))
    }
}

/// A key going down or coming up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    /// Which key.
    pub code: KeyCode,
    /// `true` for a key going down, `false` for one coming up.
    pub pressed: bool,
}

impl KeyEvent {
    /// A key going down.
    #[must_use]
    pub const fn down(code: KeyCode) -> Self {
        KeyEvent {
            code,
            pressed: true,
        }
    }

    /// A key coming up.
    #[must_use]
    pub const fn up(code: KeyCode) -> Self {
        KeyEvent {
            code,
            pressed: false,
        }
    }
}

/// Where the decoder stands between two bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum State {
    /// Nothing is pending; the next byte is a prefix or a key going down.
    #[default]
    Idle,
    /// [`EXTENDED_PREFIX`] was seen.
    Extended,
    /// [`RELEASE_PREFIX`] was seen.
    Release,
    /// Both prefixes were seen, in that order.
    ExtendedRelease,
    /// [`PAUSE_PREFIX`] was seen, and this many of the seven bytes after
    /// it.
    Pause(u8),
}

/// The state machine of scancode set 2.
#[derive(Clone, Copy, Debug, Default)]
pub struct Decoder {
    state: State,
}

impl Decoder {
    /// A decoder waiting for the first byte.
    #[must_use]
    pub const fn new() -> Self {
        Decoder { state: State::Idle }
    }

    /// Where the decoder stands.
    #[must_use]
    pub const fn state(&self) -> State {
        self.state
    }

    /// Forgets what was pending, which is what a driver does when it has
    /// lost bytes and cannot say how many.
    pub const fn reset(&mut self) {
        self.state = State::Idle;
    }

    /// Takes one byte and answers with the event it completed, if it
    /// completed one.
    pub fn feed(&mut self, byte: u8) -> Option<KeyEvent> {
        match self.state {
            State::Idle => self.idle(byte),
            State::Extended => self.extended(byte),
            State::Release => self.settle(KeyCode::from_set2(byte), false),
            State::ExtendedRelease => self.settle(KeyCode::from_set2_extended(byte), false),
            State::Pause(seen) => self.pause(seen),
        }
    }

    /// One byte with nothing pending: a prefix, or a key going down.
    fn idle(&mut self, byte: u8) -> Option<KeyEvent> {
        match byte {
            EXTENDED_PREFIX => {
                self.state = State::Extended;
                None
            }
            RELEASE_PREFIX => {
                self.state = State::Release;
                None
            }
            PAUSE_PREFIX => {
                self.state = State::Pause(0);
                None
            }
            scan => self.settle(KeyCode::from_set2(scan), true),
        }
    }

    /// One byte behind the extended prefix: the release marker, or a key
    /// of the second table going down.
    fn extended(&mut self, byte: u8) -> Option<KeyEvent> {
        if byte == RELEASE_PREFIX {
            self.state = State::ExtendedRelease;
            return None;
        }
        self.settle(KeyCode::from_set2_extended(byte), true)
    }

    /// Ends a sequence: the event if the byte spelled a key, nothing if it
    /// spelled none, and [`State::Idle`] either way.
    fn settle(&mut self, code: Option<KeyCode>, pressed: bool) -> Option<KeyEvent> {
        self.state = State::Idle;
        code.map(|code| KeyEvent { code, pressed })
    }

    /// One byte of the pause sequence, whose bytes carry nothing: the key
    /// is the sequence, so the last of them is the event.
    const fn pause(&mut self, seen: u8) -> Option<KeyEvent> {
        let next = seen.saturating_add(1);
        if next < PAUSE_TAIL {
            self.state = State::Pause(next);
            return None;
        }
        self.state = State::Idle;
        Some(KeyEvent::down(KeyCode::Pause))
    }
}
