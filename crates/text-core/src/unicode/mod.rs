// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unicode 18.0.0 properties generated from the retained UCD sources.

#[rustfmt::skip]
mod generated;
pub use generated::{
    BidiClass, DefaultIgnorable, EastAsianWidth, ExtendedPictographic, GeneralCategory,
    GraphemeBreak, IndicConjunctBreak, JoiningType, LineBreak, Script, VERSION, VERSION_MAJOR,
    WordBreak, bidi_class, combining_class, default_ignorable, east_asian_width,
    extended_pictographic, general_category, grapheme_break, indic_conjunct_break, joining_type,
    line_break, script, word_break,
};

struct Range<T> {
    lo: u32,
    hi: u32,
    value: T,
}
fn lookup<T: Copy>(ranges: &[Range<T>], code: u32, default: T) -> T {
    ranges
        .partition_point(|range| range.lo <= code)
        .checked_sub(1)
        .and_then(|i| ranges.get(i))
        .filter(|range| code <= range.hi)
        .map_or(default, |range| range.value)
}

/// Explicit `Script_Extensions`; an empty slice means the singleton Script value.
#[must_use]
pub fn script_extensions(code: u32) -> &'static [Script] {
    lookup(generated::EXTENSIONS, code, &[])
}

/// Paired-bracket direction for UAX #9 rule N0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BracketKind {
    /// Opening bracket.
    Open,
    /// Closing bracket.
    Close,
}
/// `Bidi_Paired_Bracket` and `Bidi_Paired_Bracket_Type`, when defined.
#[must_use]
pub fn paired_bracket(code: u32) -> Option<(u32, BracketKind)> {
    generated::BRACKETS
        .binary_search_by_key(&code, |row| row.0)
        .ok()
        .and_then(|i| generated::BRACKETS.get(i))
        .map(|(_, pair, open)| {
            (
                *pair,
                if *open {
                    BracketKind::Open
                } else {
                    BracketKind::Close
                },
            )
        })
}
/// `Bidi_Mirroring_Glyph`, or the input when no mirror glyph is defined.
#[must_use]
pub fn mirror(code: u32) -> u32 {
    generated::MIRRORS
        .binary_search_by_key(&code, |row| row.0)
        .ok()
        .and_then(|i| generated::MIRRORS.get(i))
        .map_or(code, |row| row.1)
}
/// Direct canonical decomposition; Hangul decomposition is algorithmic.
#[must_use]
pub fn canonical_decomposition(code: u32) -> &'static [u32] {
    generated::DECOMPOSITIONS
        .binary_search_by_key(&code, |row| row.0)
        .ok()
        .and_then(|i| generated::DECOMPOSITIONS.get(i))
        .map_or(&[], |row| row.1)
}

pub(crate) fn composition(
    first: u32,
    second: u32,
    mut covered: impl FnMut(u32) -> bool,
) -> Option<u32> {
    generated::DECOMPOSITIONS.iter().find_map(|(code, parts)| {
        if *parts == [first, second] && covered(*code) {
            Some(*code)
        } else {
            None
        }
    })
}
