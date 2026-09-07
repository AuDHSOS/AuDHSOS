// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Lengths, page sizes, and colours.
//!
//! Every length in this crate is a whole number of thousandths of a point.
//! A PDF point is a seventy-second of an inch, and the width tables of a
//! Type 1 font are already given in thousandths of the point size, so the
//! unit is the one the format itself counts in: a line is laid out, broken,
//! and written without a single floating-point number, and two runs of the
//! same document lay out identically on any machine.

/// A length in thousandths of a point.
pub type Mils = i64;

/// The length of `points` whole points.
#[must_use]
pub const fn pt(points: i64) -> Mils {
    points.saturating_mul(1000)
}

/// Writes a length as a PDF number, with no trailing zeros and no
/// exponent: `72500` becomes `72.5`, `72000` becomes `72`.
#[must_use]
pub fn number(mils: Mils) -> String {
    let magnitude = mils.unsigned_abs();
    let whole = magnitude.wrapping_div(1000);
    let fraction = magnitude.wrapping_rem(1000);
    let sign = if mils < 0 { "-" } else { "" };
    if fraction == 0 {
        return format!("{sign}{whole}");
    }
    let digits = format!("{fraction:03}");
    let trimmed = digits.trim_end_matches('0');
    format!("{sign}{whole}.{trimmed}")
}

/// The size of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageSize {
    /// The width.
    pub width: Mils,
    /// The height.
    pub height: Mils,
}

impl PageSize {
    /// ISO A4, 595 by 842 points.
    pub const A4: Self = Self {
        width: pt(595),
        height: pt(842),
    };

    /// US Letter, 612 by 792 points.
    pub const LETTER: Self = Self {
        width: pt(612),
        height: pt(792),
    };
}

/// A colour in the device RGB space, one byte per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    /// The red channel.
    pub red: u8,
    /// The green channel.
    pub green: u8,
    /// The blue channel.
    pub blue: u8,
}

impl Color {
    /// Black.
    pub const BLACK: Self = Self::gray(0);
    /// White.
    pub const WHITE: Self = Self::gray(255);

    /// A colour from its three channels.
    #[must_use]
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    /// A shade of gray, `0` black and `255` white.
    #[must_use]
    pub const fn gray(level: u8) -> Self {
        Self {
            red: level,
            green: level,
            blue: level,
        }
    }

    /// Whether the three channels are one number: a grey is written with
    /// the operator that takes one, which is two thirds shorter than the
    /// one that takes three and says the same thing.
    #[must_use]
    pub const fn is_gray(self) -> bool {
        self.red == self.green && self.green == self.blue
    }

    /// The one channel of a grey, as a PDF number in the range zero to
    /// one.
    #[must_use]
    pub fn level(self) -> String {
        number(channel(self.red))
    }

    /// The three channels as PDF numbers in the range zero to one.
    #[must_use]
    pub fn components(self) -> [String; 3] {
        [
            number(channel(self.red)),
            number(channel(self.green)),
            number(channel(self.blue)),
        ]
    }
}

/// A byte channel as thousandths of full intensity.
fn channel(value: u8) -> Mils {
    Mils::from(value).saturating_mul(1000).wrapping_div(255)
}
