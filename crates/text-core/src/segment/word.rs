// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::unicode::{self as u, WordBreak as W};

/// UTF-8 default word boundaries, including punctuation and whitespace segments.
#[derive(Clone, Debug)]
pub struct WordBoundaries<'a> {
    chars: core::str::CharIndices<'a>,
    end: Option<usize>,
    immediate: Option<W>,
    left: Option<W>,
    before: Option<W>,
    ri_odd: bool,
}
/// Iterate UAX #29 default word boundaries without allocation.
#[must_use]
pub fn word_boundaries(text: &str) -> WordBoundaries<'_> {
    WordBoundaries {
        chars: text.char_indices(),
        end: Some(text.len()),
        immediate: None,
        left: None,
        before: None,
        ri_odd: false,
    }
}
const fn ignored(value: W) -> bool {
    matches!(value, W::Extend | W::Format | W::Zwj)
}
const fn newline(value: W) -> bool {
    matches!(value, W::Cr | W::Lf | W::Newline)
}
const fn letter(value: W) -> bool {
    matches!(value, W::ALetter | W::HebrewLetter)
}
const fn mid_letter(value: W) -> bool {
    matches!(value, W::MidLetter | W::MidNumLet | W::SingleQuote)
}
const fn mid_number(value: W) -> bool {
    matches!(value, W::MidNum | W::MidNumLet | W::SingleQuote)
}
impl WordBoundaries<'_> {
    fn boundary(&self, code: u32, right: W) -> bool {
        let Some(immediate) = self.immediate else {
            return true;
        };
        // UAX #29 rev.49, WB3–WB4 precede ignore-rule replacement.
        if immediate == W::Cr && right == W::Lf {
            return false;
        }
        if newline(immediate) || newline(right) {
            return true;
        }
        if (immediate == W::Zwj && u::extended_pictographic(code) == u::ExtendedPictographic::Yes)
            || (immediate == W::WSegSpace && right == W::WSegSpace)
            || ignored(right)
        {
            return false;
        }
        let Some(left) = self.left else {
            return true;
        };
        let after = self
            .chars
            .clone()
            .map(|(_, c)| u::word_break(u32::from(c)))
            .find(|v| !ignored(*v));
        let before = self.before;
        // WB5–WB13b.
        if (letter(left) && letter(right))
            || (letter(left) && mid_letter(right) && after.is_some_and(letter))
            || (before.is_some_and(letter) && mid_letter(left) && letter(right))
            || (left == W::HebrewLetter && right == W::SingleQuote)
            || (left == W::HebrewLetter
                && right == W::DoubleQuote
                && after == Some(W::HebrewLetter))
            || (before == Some(W::HebrewLetter)
                && left == W::DoubleQuote
                && right == W::HebrewLetter)
            || (left == W::Numeric && right == W::Numeric)
            || (letter(left) && right == W::Numeric)
            || (left == W::Numeric && letter(right))
            || (before == Some(W::Numeric) && mid_number(left) && right == W::Numeric)
            || (left == W::Numeric && mid_number(right) && after == Some(W::Numeric))
            || (left == W::Katakana && right == W::Katakana)
            || ((letter(left) || matches!(left, W::Numeric | W::Katakana | W::ExtendNumLet))
                && right == W::ExtendNumLet)
            || (left == W::ExtendNumLet
                && (letter(right) || matches!(right, W::Numeric | W::Katakana)))
        {
            return false;
        }
        // WB15/WB16.
        !(left == W::RegionalIndicator && right == W::RegionalIndicator && self.ri_odd)
    }
}
impl Iterator for WordBoundaries<'_> {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        while let Some((byte, ch)) = self.chars.next() {
            let code = u32::from(ch);
            let right = u::word_break(code);
            let boundary = self.boundary(code, right);
            if !ignored(right) {
                self.ri_odd = right == W::RegionalIndicator
                    && (self.left != Some(W::RegionalIndicator) || !self.ri_odd);
                self.before = self.left;
                self.left = Some(right);
            }
            self.immediate = Some(right);
            if boundary {
                return Some(byte);
            }
        }
        self.end.take()
    }
}
impl core::iter::FusedIterator for WordBoundaries<'_> {}
