// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Character references, turned back into the characters they stand for.
//!
//! The table is not the whole of HTML's twenty-two hundred names. It is
//! the punctuation and the symbols documents are written with, plus every
//! numeric reference, which needs no table at all. A name that is not here
//! is left standing as the text it is, which is what a reader would rather
//! see than a character the parser guessed at.

/// The named references, with the character each stands for.
const NAMED: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", '\u{a0}'),
    ("copy", '©'),
    ("reg", '®'),
    ("trade", '™'),
    ("sect", '§'),
    ("para", '¶'),
    ("middot", '·'),
    ("bull", '•'),
    ("hellip", '…'),
    ("ndash", '–'),
    ("mdash", '—'),
    ("lsquo", '‘'),
    ("rsquo", '’'),
    ("ldquo", '“'),
    ("rdquo", '”'),
    ("laquo", '«'),
    ("raquo", '»'),
    ("dagger", '†'),
    ("Dagger", '‡'),
    ("deg", '°'),
    ("plusmn", '±'),
    ("times", '×'),
    ("divide", '÷'),
    ("frac12", '½'),
    ("frac14", '¼'),
    ("sup2", '²'),
    ("sup3", '³'),
    ("micro", 'µ'),
    ("euro", '€'),
    ("pound", '£'),
    ("yen", '¥'),
    ("cent", '¢'),
    ("larr", '←'),
    ("rarr", '→'),
    ("harr", '↔'),
    ("le", '≤'),
    ("ge", '≥'),
    ("ne", '≠'),
    ("infin", '∞'),
    ("shy", '\u{ad}'),
    ("ensp", ' '),
    ("emsp", ' '),
    ("thinsp", ' '),
];

/// The names a browser also accepts without their semicolon, because the
/// documents of the web have always been written that way.
const LEGACY: &[&str] = &["amp", "lt", "gt", "quot", "nbsp", "copy", "reg"];

/// The longest name in the table, which is how far a reference is looked
/// ahead before it is given up on.
const LONGEST: usize = 7;

/// Replaces every character reference in `text`.
pub(crate) fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    while let Some(rest) = text.get(at..) {
        let Some(offset) = rest.find('&') else {
            out.push_str(rest);
            break;
        };
        out.push_str(rest.get(..offset).unwrap_or_default());
        let start = at.saturating_add(offset);
        let after = text.get(start.saturating_add(1)..).unwrap_or_default();
        if let Some((character, length)) = reference(after) {
            out.push(character);
            at = start.saturating_add(1).saturating_add(length);
        } else {
            out.push('&');
            at = start.saturating_add(1);
        }
    }
    out
}

/// One reference, without its ampersand: the character it stands for and
/// how many bytes it took.
fn reference(text: &str) -> Option<(char, usize)> {
    if let Some(rest) = text.strip_prefix('#') {
        return numeric(rest).map(|(character, length)| (character, length.saturating_add(1)));
    }
    named(text)
}

/// A numeric reference, without its `#`.
fn numeric(text: &str) -> Option<(char, usize)> {
    let (digits, radix, prefix) = match text.strip_prefix(['x', 'X']) {
        Some(rest) => (rest, 16, 1usize),
        None => (text, 10, 0usize),
    };
    let end = digits
        .find(|character: char| !character.is_digit(radix))
        .unwrap_or(digits.len());
    let value = digits.get(..end).filter(|found| !found.is_empty())?;
    let code = u32::from_str_radix(value, radix).ok()?;
    let character = char::from_u32(code)?;
    let semicolon = usize::from(digits.get(end..).is_some_and(|rest| rest.starts_with(';')));
    Some((
        character,
        prefix.saturating_add(end).saturating_add(semicolon),
    ))
}

/// A named reference, with its semicolon or, for the legacy names,
/// without one.
fn named(text: &str) -> Option<(char, usize)> {
    // As far as a name can reach, and no further than a character can be
    // cut: a slice that ends inside one is no slice at all.
    let mut limit = text.len().min(LONGEST.saturating_add(1));
    while limit > 0 && !text.is_char_boundary(limit) {
        limit = limit.saturating_sub(1);
    }
    let head = text.get(..limit)?;
    let end = head.find(';');
    if let Some(end) = end
        && let Some(name) = head.get(..end)
        && let Some((_, character)) = NAMED.iter().find(|(known, _)| *known == name)
    {
        return Some((*character, end.saturating_add(1)));
    }
    for name in LEGACY {
        if text.starts_with(name)
            && let Some((_, character)) = NAMED.iter().find(|(known, _)| known == name)
        {
            return Some((*character, name.len()));
        }
    }
    None
}
