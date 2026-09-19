// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One statement written again with every literal as a `?`, which is
//! what a log of the statements a connection ran holds in place of the
//! values they carried.
//!
//! Two forms answer it, and they differ in what case the words are
//! written in:
//!
//! - [`normalized`] is `sqlite3_normalize` of
//!   `research/sqlite/ext/misc/normalize.c:556`, which writes every word
//!   in lower case.
//! - [`normalized_sql`] is `sqlite3Normalize` of
//!   `research/sqlite/src/tokenize.c:782`, which
//!   `sqlite3_normalized_sql` answers: an identifier in lower case and
//!   every other word in upper case.
//!
//! Both read the statement once, so each costs O(n) in its bytes.

use alloc::vec::Vec;

use crate::keyword::Keyword;
use crate::token::{Kind, token};

/// One statement written again in lower case with every literal as a
/// `?`, and nothing where a byte of it is one no rule accepts.
///
/// The right side of an `IN` becomes `?,?,?` whatever it held, unless it
/// holds a statement, which is what `sqlite3_normalize` writes for it.
#[must_use]
pub fn normalized(sql: &[u8]) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let mut at = 0;
    while let Some((kind, width)) = reading(sql, at) {
        let text = sql.get(at..at.saturating_add(width)).unwrap_or_default();
        at = at.saturating_add(width);
        match kind {
            Kind::Space | Kind::Comment => {}
            Kind::Illegal => return None,
            Kind::String
            | Kind::Blob
            | Kind::Integer
            | Kind::Float
            | Kind::QNumber
            | Kind::Variable => out.push(b'?'),
            _ => {
                // `NULL` is a value where nothing before it makes it a
                // word of the language, which `IS` and `NOT` do.
                if text.eq_ignore_ascii_case(b"null") && !after_is_or_not(&out) {
                    out.push(b'?');
                    continue;
                }
                separated(&mut out, text);
                out.extend(text.iter().map(u8::to_ascii_lowercase));
            }
        }
    }
    // No text the walk writes ends in a space, because a space stands
    // only where a word follows it.
    if !out.is_empty() && out.last() != Some(&b';') {
        out.push(b';');
    }
    Some(listed(out))
}

/// Whether the text written so far ends in the word `is` or the word
/// `not`, which is what makes a `NULL` after it a word and not a value.
fn after_is_or_not(out: &[u8]) -> bool {
    for word in [b"is".as_slice(), b"not".as_slice()] {
        let Some(before) = out.len().checked_sub(word.len()) else {
            continue;
        };
        if out.get(before..) != Some(word) {
            continue;
        }
        let held = before.checked_sub(1).and_then(|at| out.get(at).copied());
        if !held.is_some_and(is_name_byte) {
            return true;
        }
    }
    false
}

/// A space between two words, which stands where the byte before it and
/// the byte after it are both ones a bare name is written with.
fn separated(out: &mut Vec<u8>, text: &[u8]) {
    let last = out.last().copied();
    let first = text.first().copied();
    if last.is_some_and(is_name_byte) && first.is_some_and(is_name_byte) {
        out.push(b' ');
    }
}

/// Whether a byte is one a bare name is written with, which is
/// `IdChar` of `research/sqlite/ext/misc/normalize.c`.
const fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte >= 0x80
}

/// The same text with the right side of every `IN` written as `?,?,?`,
/// which is the second pass of `sqlite3_normalize`.
///
/// A right side that holds a statement is left as it is, and so is a
/// name that ends in the letters `in`.
fn listed(text: Vec<u8>) -> Vec<u8> {
    let mut out = text;
    let mut at = 0;
    while let Some(found) = held_in(&out, at) {
        let open = found.saturating_add(3);
        at = open;
        let before = found.checked_sub(1).and_then(|at| out.get(at).copied());
        if before.is_some_and(is_name_byte) {
            continue;
        }
        if out.get(open..).is_some_and(opens_statement) {
            continue;
        }
        let Some(close) = closing(&out, open) else {
            continue;
        };
        out.splice(open..close, b"?,?,?".iter().copied());
        at = open.saturating_add(5);
    }
    out
}

/// Where the next `in(` stands from `at` on.
fn held_in(text: &[u8], at: usize) -> Option<usize> {
    let held = text.get(at..)?;
    let found = held
        .windows(3)
        .position(|window| window == b"in(")?
        .saturating_add(at);
    Some(found)
}

/// Whether the text after an `in(` opens a statement, which the right
/// side of an `IN` is left as it is for.
fn opens_statement(text: &[u8]) -> bool {
    for word in [b"select".as_slice(), b"with".as_slice()] {
        if text.len() < word.len() || text.get(..word.len()) != Some(word) {
            continue;
        }
        if !text.get(word.len()).copied().is_some_and(is_name_byte) {
            return true;
        }
    }
    false
}

/// Where the bracket that closes the one at `open` stands.
fn closing(text: &[u8], open: usize) -> Option<usize> {
    let mut depth = 1_usize;
    for (step, byte) in text.get(open..)?.iter().enumerate() {
        if *byte == b'(' {
            depth = depth.saturating_add(1);
        }
        if *byte == b')' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(open.saturating_add(step));
            }
        }
    }
    None
}

/// Whether a statement names a file, which an `ATTACH` and a `DETACH`
/// do and which is why a word in double quotes there is a name.
fn names_a_file(sql: &[u8]) -> bool {
    let mut at = 0;
    while let Some((kind, width)) = reading(sql, at) {
        let text = sql.get(at..at.saturating_add(width)).unwrap_or_default();
        at = at.saturating_add(width);
        if matches!(kind, Kind::Space | Kind::Comment) {
            continue;
        }
        return text.eq_ignore_ascii_case(b"attach") || text.eq_ignore_ascii_case(b"detach");
    }
    false
}

/// The token that stands at `at`, and nothing where the statement ends
/// there, which is how the walks read one token after another.
fn reading(sql: &[u8], at: usize) -> Option<(Kind, usize)> {
    let held = sql.get(at..).filter(|held| !held.is_empty())?;
    token(held)
}

/// Whether a name is one bare identifier, which is what
/// `sqlite3GetToken` reading the whole of it as `TK_ID` says.
fn bare_name(name: &[u8]) -> bool {
    token(name).is_some_and(|(kind, width)| kind == Kind::Id && width == name.len())
}

/// One statement written again with every literal as a `?`, an
/// identifier in lower case and every other word in upper case, which
/// `sqlite3_normalized_sql` answers.
///
/// The right side of an `IN` becomes `?,?,?` whatever it held, unless a
/// `SELECT` opens it.
///
/// `names` are the names the statement may read, which are the tables of
/// the schema and their columns. A word in double quotes that is none of
/// them is a text and is written as a `?`, which is
/// `sqlite3VdbeUsesDoubleQuotedString` of
/// `research/sqlite/src/tokenize.c:861` saying that the resolver read it
/// as one. An `ATTACH` and a `DETACH` name a file rather than a column,
/// so a word in double quotes there is a name whatever `names` holds.
#[must_use]
pub fn normalized_sql(sql: &[u8], names: &[Vec<u8>]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut at = 0;
    let mut before = Kind::Space;
    let mut ended = Kind::Space;
    let naming = names_a_file(sql);
    // Where the right side of the `IN` being read begins, counting from
    // one, and how many brackets stood open where it did.
    let mut list = 0_usize;
    let mut depth = 0_usize;
    let mut list_depth = 0_usize;
    while let Some((kind, width)) = reading(sql, at) {
        let text = sql.get(at..at.saturating_add(width)).unwrap_or_default();
        at = at.saturating_add(width);
        ended = kind;
        if !matches!(kind, Kind::Space | Kind::Comment) {
            let held = before;
            before = kind;
            match kind {
                Kind::Keyword(Keyword::Null)
                    if matches!(held, Kind::Keyword(Keyword::Is | Keyword::Not)) =>
                {
                    out.extend_from_slice(b" NULL");
                }
                Kind::String
                | Kind::Blob
                | Kind::Integer
                | Kind::Float
                | Kind::QNumber
                | Kind::Variable
                | Kind::Keyword(Keyword::Null) => out.push(b'?'),
                Kind::Lp => {
                    depth = depth.saturating_add(1);
                    if matches!(held, Kind::Keyword(Keyword::In)) {
                        list = out.len().saturating_add(1);
                        list_depth = depth;
                    }
                    out.push(b'(');
                }
                Kind::Rp => {
                    if list > 0 && depth == list_depth {
                        out.truncate(list);
                        out.extend_from_slice(b"?,?,?");
                        list = 0;
                    }
                    depth = depth.saturating_sub(1);
                    out.push(b')');
                }
                Kind::Id => {
                    list = 0;
                    let held = crate::schema::dequote(text);
                    let text_of = text.first() == Some(&b'"')
                        && !naming
                        && !names.iter().any(|name| name.eq_ignore_ascii_case(&held));
                    if text_of {
                        out.push(b'?');
                    } else if bare_name(&held) {
                        // A name that is one bare identifier is written
                        // bare, and every other one quoted, which is
                        // `sqlite3_str_appendf(pStr, "\"%w\"", zId)`.
                        separated(&mut out, &held);
                        out.extend(held.iter().map(u8::to_ascii_lowercase));
                    } else {
                        out.push(b'"');
                        for byte in &held {
                            if *byte == b'"' {
                                out.push(b'"');
                            }
                            out.push(byte.to_ascii_lowercase());
                        }
                        out.push(b'"');
                    }
                }
                _ => {
                    if matches!(kind, Kind::Keyword(Keyword::Select)) {
                        list = 0;
                    }
                    separated(&mut out, text);
                    out.extend(text.iter().map(u8::to_ascii_uppercase));
                }
            }
        }
    }
    if ended != Kind::Semi {
        out.push(b';');
    }
    out
}
