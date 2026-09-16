// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The text of a `CREATE TABLE` with one constraint taken out of it.
//!
//! `alterDropConstraintFunc` is the document: `ALTER TABLE ... DROP
//! CONSTRAINT` and `ALTER TABLE ... ALTER COLUMN ... DROP NOT NULL`
//! read the tokens of the statement rather than the tree it was parsed
//! into, and cut the text of the constraint out of it.

use alloc::vec::Vec;

use crate::keyword::Keyword;
use crate::token::{Kind, token};

/// Which constraint a statement loses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dropped<'a> {
    /// The constraint of that name, wherever it stands.
    Named(&'a [u8]),
    /// The `NOT NULL` of the column at that place, counting from nought.
    NotNull(usize),
}

/// Why a constraint could not be dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The statement holds no constraint of that name.
    NoSuch(Vec<u8>),
    /// The constraint of that name is neither a `CHECK` nor a `NOT
    /// NULL`, which are the two `alterDropConstraintFunc` cuts.
    Kept(Vec<u8>),
}

/// The next token of `sql` that carries meaning, with a bracketed run
/// answered whole, which is `getConstraintToken`.
///
/// Reading `n` bytes costs O(n).
fn next(sql: &[u8]) -> (Kind, usize) {
    let mut at = 0_usize;
    let kind = loop {
        let Some((held, len)) = token(sql.get(at..).unwrap_or_default()) else {
            return (Kind::Illegal, at);
        };
        at = at.saturating_add(len);
        if !held.is_trivia() {
            break held;
        }
    };
    if kind != Kind::Lp {
        return (kind, at);
    }
    // A bracketed run is one token, which is what lets the `(a != b)` of
    // a `CHECK` hold a comma.
    let mut depth = 1_usize;
    while depth > 0 {
        let Some((held, len)) = token(sql.get(at..).unwrap_or_default()) else {
            return (Kind::Illegal, at);
        };
        at = at.saturating_add(len);
        match held {
            Kind::Lp => depth = depth.saturating_add(1),
            Kind::Rp => depth = depth.saturating_sub(1),
            Kind::Illegal => return (Kind::Illegal, at),
            _ => {}
        }
    }
    (Kind::Lp, at)
}

/// How many bytes of `sql` carry no meaning, which is `getWhitespace`.
fn whitespace(sql: &[u8]) -> usize {
    let mut at = 0_usize;
    while let Some((kind, len)) = token(sql.get(at..).unwrap_or_default()) {
        if !kind.is_trivia() {
            break;
        }
        at = at.saturating_add(len);
    }
    at
}

/// Whether a token ends the constraint that runs before it, which is
/// the list `getConstraint` reads.
const fn ends(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Rp
            | Kind::Comma
            | Kind::Illegal
            | Kind::Keyword(
                Keyword::Constraint
                    | Keyword::Primary
                    | Keyword::Not
                    | Keyword::Unique
                    | Keyword::Check
                    | Keyword::Default
                    | Keyword::Collate
                    | Keyword::References
                    | Keyword::Foreign
                    | Keyword::As
                    | Keyword::Generated
            )
    )
}

/// How many bytes of `sql` the constraint that begins there runs for,
/// which is `getConstraint`.
///
/// Reading `n` bytes costs O(n).
fn constraint(sql: &[u8]) -> usize {
    let mut at = 0_usize;
    loop {
        let (kind, len) = next(sql.get(at..).unwrap_or_default());
        if ends(kind) {
            return at;
        }
        at = at.saturating_add(len);
    }
}

/// Whether the name a token carries is the one being dropped, with the
/// quotes taken off both, which is `quotedCompare`.
fn names(sql: &[u8], token: (usize, usize), wanted: &[u8]) -> bool {
    let (start, end) = token;
    let text = sql.get(start..end).unwrap_or_default();
    crate::schema::dequote(text).eq_ignore_ascii_case(wanted)
}

/// Where the columns of a `CREATE TABLE` begin, which is
/// `skipCreateTable`: the byte after the bracket that opens them.
///
/// Reading `n` bytes costs O(n).
fn columns_at(sql: &[u8]) -> Option<usize> {
    let mut at = 0_usize;
    loop {
        let (kind, len) = token(sql.get(at..).unwrap_or_default())?;
        at = at.saturating_add(len);
        match kind {
            Kind::Lp => return Some(at),
            Kind::Illegal => return None,
            _ => {}
        }
    }
}

/// Whether a token that follows the name of a constraint says the
/// constraint is written out, which is the set
/// `alterDropConstraintFunc` reads as a `CHECK` of no body.
const fn bodyless(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Comma
            | Kind::Rp
            | Kind::Keyword(
                Keyword::Constraint
                    | Keyword::Default
                    | Keyword::Collate
                    | Keyword::Generated
                    | Keyword::As
            )
    )
}

/// Where the constraint `dropped` stands in `sql`, or nothing where the
/// statement holds none, which is the walk of
/// `alterDropConstraintFunc`.
///
/// Reading the tokens costs O(n) in the bytes of the statement.
///
/// # Errors
///
/// [`Error::Kept`] where the constraint of that name is neither a
/// `CHECK` nor a `NOT NULL`.
fn cut(sql: &[u8], dropped: Dropped<'_>) -> Result<Option<(usize, usize)>, Error> {
    let Some(mut at) = columns_at(sql) else {
        return Ok(None);
    };
    let mut column = 0_usize;
    loop {
        let start = at;
        let rest = sql.get(at..).unwrap_or_default();
        let (kind, len) = next(rest);
        at = at.saturating_add(len);
        match kind {
            Kind::Keyword(Keyword::Constraint) => {
                if let Some(run) = self_named(sql, &mut at, (start, column), dropped)? {
                    return Ok(Some(run));
                }
            }
            // A `NOT NULL` the statement wrote under no name.
            Kind::Keyword(Keyword::Not) if dropped == Dropped::NotNull(column) => {
                let end = at.saturating_add(constraint(sql.get(at..).unwrap_or_default()));
                return Ok(Some((start, end)));
            }
            Kind::Rp | Kind::Illegal => return Ok(None),
            Kind::Comma => column = column.saturating_add(1),
            _ => {}
        }
    }
}

/// Where a constraint the statement named stands, where it is the one
/// being dropped, with `at` left after it.
///
/// # Errors
///
/// [`Error::Kept`] where the constraint is neither a `CHECK` nor a
/// `NOT NULL`.
fn self_named(
    sql: &[u8],
    at: &mut usize,
    held: (usize, usize),
    dropped: Dropped<'_>,
) -> Result<Option<(usize, usize)>, Error> {
    let (start, column) = held;
    *at = at.saturating_add(whitespace(sql.get(*at..).unwrap_or_default()));
    let named = *at;
    let (_, len) = next(sql.get(*at..).unwrap_or_default());
    *at = at.saturating_add(len);
    let same = match dropped {
        Dropped::Named(name) => names(sql, (named, *at), name),
        Dropped::NotNull(_) => false,
    };
    // The token after the name says what the constraint is; one of the
    // words that opens another constraint says this one is written out.
    let (kind, len) = next(sql.get(*at..).unwrap_or_default());
    if bodyless(kind) {
        if same {
            return Ok(Some((start, *at)));
        }
        return Ok(None);
    }
    *at = at.saturating_add(len);
    *at = at.saturating_add(constraint(sql.get(*at..).unwrap_or_default()));
    let null = kind == Kind::Keyword(Keyword::Not);
    if !same && (!null || dropped != Dropped::NotNull(column)) {
        return Ok(None);
    }
    // A `DROP NOT NULL` reaches here only where the constraint is the
    // `NOT NULL` it names, so only a name says a constraint stands.
    if let Dropped::Named(name) = dropped
        && !null
        && kind != Kind::Keyword(Keyword::Check)
    {
        return Err(Error::Kept(name.to_vec()));
    }
    Ok(Some((start, *at)))
}

/// `sql` with the constraint `dropped` cut out of it, or the statement
/// as it stands where a `DROP NOT NULL` names a column that holds none,
/// which is `alterDropConstraintFunc`.
///
/// Reading the tokens costs O(n) in the bytes of the statement.
///
/// # Errors
///
/// [`Error::NoSuch`] where the statement holds no constraint of that
/// name, and [`Error::Kept`] where the constraint of that name is
/// neither a `CHECK` nor a `NOT NULL`.
pub fn without(sql: &[u8], dropped: Dropped<'_>) -> Result<Vec<u8>, Error> {
    let Some((start, end)) = cut(sql, dropped)? else {
        return match dropped {
            Dropped::Named(name) => Err(Error::NoSuch(name.to_vec())),
            // `DROP NOT NULL` on a column that holds none is no error,
            // which SQLite takes from postgres.
            Dropped::NotNull(_) => Ok(sql.to_vec()),
        };
    };
    Ok(written(sql, start, end))
}

/// The bytes the statement holds once the run from `start` to `end` is
/// gone, with the space and the comma beside it read as
/// `alterDropConstraintFunc` reads them.
fn written(sql: &[u8], start: usize, end: usize) -> Vec<u8> {
    let mut end = end.saturating_add(whitespace(sql.get(end..).unwrap_or_default()));
    let mut start = start;
    let mut space: &[u8] = b" ";
    let after = token(sql.get(end..).unwrap_or_default()).map(|(kind, _)| kind);
    if matches!(after, Some(Kind::Rp | Kind::Comma)) {
        space = b"";
        if start.checked_sub(1).and_then(|before| sql.get(before)) == Some(&b',') {
            start = start.saturating_sub(1);
        }
    }
    // A constraint at the end of the text carries nothing after it.
    end = end.min(sql.len());
    let mut out = sql.get(..start).unwrap_or_default().to_vec();
    out.extend_from_slice(space);
    out.extend_from_slice(sql.get(end..).unwrap_or_default());
    out
}

/// Whether the statement already holds a constraint of that name,
/// which `sqlite3AlterAddConstraint` refuses a second one under.
///
/// Reading the tokens costs O(n) in the bytes of the statement.
#[must_use]
pub fn holds(sql: &[u8], name: &[u8]) -> bool {
    !matches!(cut(sql, Dropped::Named(name)), Ok(None))
}

/// `sql` with `text` written into it as a constraint of the column at
/// `at`, or as a constraint of the table where `at` is nothing, which
/// is `alterAddConstraintFunc`.
///
/// The text is written as the statement wrote it, so the spacing and
/// the case of the words it holds stand.
///
/// Reading the tokens costs O(n) in the bytes of the statement.
#[must_use]
pub fn with(sql: &[u8], at: Option<usize>, text: &[u8]) -> Vec<u8> {
    let Some(mut into) = columns_at(sql) else {
        return sql.to_vec();
    };
    let mut column = 0_usize;
    let mut kind;
    loop {
        // The first token of the column is read whatever it is, so a
        // column named `constraint` is one column and not a clause.
        let (_, len) = next(sql.get(into..).unwrap_or_default());
        into = into.saturating_add(len);
        loop {
            let (held, len) = next(sql.get(into..).unwrap_or_default());
            kind = held;
            if matches!(held, Kind::Comma | Kind::Rp | Kind::Illegal) {
                break;
            }
            into = into.saturating_add(len);
        }
        column = column.saturating_add(1);
        let more = match at {
            Some(wanted) => column <= wanted,
            // A constraint of the table's own is written before the
            // bracket that closes the columns.
            None => kind != Kind::Rp,
        };
        if !more || kind == Kind::Illegal {
            break;
        }
    }
    into = into.saturating_add(whitespace(sql.get(into..).unwrap_or_default()));
    let mut out = sql.get(..into).unwrap_or_default().to_vec();
    out.extend_from_slice(if at.is_some() { b" " } else { b", " });
    out.extend_from_slice(text);
    out.extend_from_slice(sql.get(into..).unwrap_or_default());
    out
}
