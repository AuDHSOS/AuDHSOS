// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::unicode::{self as u, GraphemeBreak as G, IndicConjunctBreak as I};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Emoji {
    None,
    Extended,
    Joined,
}

/// UTF-8 extended grapheme boundaries, including both ends once.
#[derive(Clone, Debug)]
pub struct GraphemeBoundaries<'a> {
    chars: core::str::CharIndices<'a>,
    end: Option<usize>,
    previous: Option<G>,
    linker: bool,
    emoji: Emoji,
    ri_odd: bool,
}
/// Iterate UAX #29 extended grapheme boundaries without allocation.
#[must_use]
pub fn grapheme_boundaries(text: &str) -> GraphemeBoundaries<'_> {
    GraphemeBoundaries {
        chars: text.char_indices(),
        end: Some(text.len()),
        previous: None,
        linker: false,
        emoji: Emoji::None,
        ri_odd: false,
    }
}
impl Iterator for GraphemeBoundaries<'_> {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        for (byte, ch) in self.chars.by_ref() {
            let code = u32::from(ch);
            let next = u::grapheme_break(code);
            let incb = u::indic_conjunct_break(code);
            let ep = u::extended_pictographic(code) == u::ExtendedPictographic::Yes;
            let boundary = self.previous.is_none_or(|prev| {
                // UAX #29 rev.49, GB3–GB999, in priority order.
                if prev == G::Cr && next == G::Lf {
                    return false;
                }
                if matches!(prev, G::Control | G::Cr | G::Lf)
                    || matches!(next, G::Control | G::Cr | G::Lf)
                {
                    return true;
                }
                if (prev == G::L && matches!(next, G::L | G::V | G::Lv | G::Lvt))
                    || (matches!(prev, G::Lv | G::V) && matches!(next, G::V | G::T))
                    || (matches!(prev, G::Lvt | G::T) && next == G::T)
                    || matches!(next, G::Extend | G::Zwj | G::SpacingMark)
                    || prev == G::Prepend
                    || (incb == I::Consonant && self.linker)
                    || (ep && self.emoji == Emoji::Joined)
                    || (prev == G::RegionalIndicator && next == G::RegionalIndicator && self.ri_odd)
                {
                    return false;
                }
                true
            });
            self.linker = incb == I::Linker || (incb == I::Extend && self.linker);
            self.emoji = if ep || (next == G::Extend && self.emoji == Emoji::Extended) {
                Emoji::Extended
            } else if next == G::Zwj && self.emoji == Emoji::Extended {
                Emoji::Joined
            } else {
                Emoji::None
            };
            self.ri_odd = next == G::RegionalIndicator
                && (self.previous != Some(G::RegionalIndicator) || !self.ri_odd);
            self.previous = Some(next);
            if boundary {
                return Some(byte);
            }
        }
        self.end.take()
    }
}
impl core::iter::FusedIterator for GraphemeBoundaries<'_> {}
