// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The tokenizer: SQL text in, tokens out.
//!
//! This is `src/tokenize.c` of the SQLite source as logic: the same
//! character classes, the same rules, the same answers, including the ones
//! that are surprising — a comment that is never closed is a comment, an
//! identifier may be quoted four ways, a number with a letter stuck to it
//! is one illegal token rather than two, and a blob literal with an odd
//! number of digits is illegal but is still consumed to its closing quote.
//!
//! It borrows: a token is a kind and a span, and the text stays where it
//! was. It allocates nothing and can refuse nothing — every byte belongs
//! to exactly one token, and a byte that belongs to no rule is a token of
//! its own, [`Kind::Illegal`], which is how the parser above it reports a
//! syntax error with a position rather than a shrug.
//!
//! One difference from the C, and it is the input and not the rule: that
//! tokenizer reads a string that ends at a NUL byte, so a NUL is where it
//! stops. This one reads a slice that knows its own end, so a NUL inside
//! it is one illegal byte and the walk goes on.

use crate::keyword::{Keyword, lookup};

/// What a token is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Whitespace, and the byte-order mark.
    Space,
    /// `-- to the end of the line`, or `/* ... */`.
    Comment,
    /// A byte, or a run of them, that no rule accepts.
    Illegal,
    /// An identifier: bare, `"quoted"`, `[bracketed]` or `` `quoted` ``.
    Id,
    /// A word that is one of SQL's own.
    Keyword(Keyword),
    /// `'text'`.
    String,
    /// `x'0a0b'`.
    Blob,
    /// A whole number, in decimal or in hexadecimal.
    Integer,
    /// A number with a fraction or an exponent.
    Float,
    /// A number written with `_` between its digits, which the parser
    /// takes apart rather than the tokenizer.
    QNumber,
    /// `?`, `?1`, `:name`, `@name`, `$name`.
    Variable,
    /// `-`.
    Minus,
    /// `->`.
    Ptr,
    /// `->>`.
    PtrPtr,
    /// `(`.
    Lp,
    /// `)`.
    Rp,
    /// `;`.
    Semi,
    /// `+`.
    Plus,
    /// `*`.
    Star,
    /// `/`.
    Slash,
    /// `%`.
    Rem,
    /// `=` or `==`.
    Eq,
    /// `<=`.
    Le,
    /// `<>` or `!=`.
    Ne,
    /// `<<`.
    LShift,
    /// `<`.
    Lt,
    /// `>=`.
    Ge,
    /// `>>`.
    RShift,
    /// `>`.
    Gt,
    /// `|`.
    BitOr,
    /// `||`.
    Concat,
    /// `,`.
    Comma,
    /// `&`.
    BitAnd,
    /// `~`.
    BitNot,
    /// `.`.
    Dot,
}

impl Kind {
    /// Whether the token carries no meaning for the grammar, which is
    /// what a parser skips.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(self, Kind::Space | Kind::Comment)
    }
}

/// One token: what it is and where it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Token {
    /// What it is.
    pub kind: Kind,
    /// Where it begins, as an offset into the text.
    pub start: usize,
    /// How many bytes it is.
    pub len: usize,
}

impl Token {
    /// The bytes of the token, out of the text it was read from.
    #[must_use]
    pub fn text<'a>(&self, sql: &'a [u8]) -> &'a [u8] {
        sql.get(self.start..self.start.saturating_add(self.len))
            .unwrap_or_default()
    }
}

/// The tokens of a statement, in order.
#[derive(Clone, Debug)]
pub struct Lexer<'a> {
    /// The whole text.
    sql: &'a [u8],
    /// How far the walk has come.
    at: usize,
}

impl<'a> Lexer<'a> {
    /// A walk over `sql` that has not begun.
    #[must_use]
    pub const fn new(sql: &'a [u8]) -> Self {
        Lexer { sql, at: 0 }
    }

    /// The text being walked.
    #[must_use]
    pub const fn sql(&self) -> &'a [u8] {
        self.sql
    }
}

impl Iterator for Lexer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        // The walk never passes the end, so what is left is a slice and
        // the end of the tokens is the end of that slice.
        let rest = self.sql.get(self.at..).unwrap_or_default();
        let (kind, len) = token(rest)?;
        let start = self.at;
        self.at = self.at.saturating_add(len);
        Some(Token { kind, start, len })
    }
}

/// The first token of `sql`, and how many bytes it is. The length is never
/// zero, so a walk always moves on.
#[must_use]
pub fn token(sql: &[u8]) -> Option<(Kind, usize)> {
    let first = *sql.first()?;
    Some(match class(first) {
        CC_SPACE => (Kind::Space, run(sql, 1, is_space)),
        CC_MINUS => minus(sql),
        CC_LP => (Kind::Lp, 1),
        CC_RP => (Kind::Rp, 1),
        CC_SEMI => (Kind::Semi, 1),
        CC_PLUS => (Kind::Plus, 1),
        CC_STAR => (Kind::Star, 1),
        CC_SLASH => slash(sql),
        CC_PERCENT => (Kind::Rem, 1),
        CC_EQ => (Kind::Eq, if at(sql, 1) == Some(b'=') { 2 } else { 1 }),
        CC_LT => match at(sql, 1) {
            Some(b'=') => (Kind::Le, 2),
            Some(b'>') => (Kind::Ne, 2),
            Some(b'<') => (Kind::LShift, 2),
            _ => (Kind::Lt, 1),
        },
        CC_GT => match at(sql, 1) {
            Some(b'=') => (Kind::Ge, 2),
            Some(b'>') => (Kind::RShift, 2),
            _ => (Kind::Gt, 1),
        },
        CC_BANG => {
            if at(sql, 1) == Some(b'=') {
                (Kind::Ne, 2)
            } else {
                (Kind::Illegal, 1)
            }
        }
        CC_PIPE => {
            if at(sql, 1) == Some(b'|') {
                (Kind::Concat, 2)
            } else {
                (Kind::BitOr, 1)
            }
        }
        CC_COMMA => (Kind::Comma, 1),
        CC_AND => (Kind::BitAnd, 1),
        CC_TILDA => (Kind::BitNot, 1),
        CC_QUOTE => quoted(sql, first),
        CC_DOT => {
            if at(sql, 1).is_some_and(is_digit) {
                number(sql)
            } else {
                (Kind::Dot, 1)
            }
        }
        CC_DIGIT => number(sql),
        CC_QUOTE2 => bracketed(sql),
        CC_VARNUM => (Kind::Variable, run(sql, 1, is_digit)),
        CC_DOLLAR | CC_VARALPHA => variable(sql),
        CC_KYWD0 => word(sql),
        CC_X => blob(sql),
        CC_BOM => {
            if at(sql, 1) == Some(0xbb) && at(sql, 2) == Some(0xbf) {
                (Kind::Space, 3)
            } else {
                (Kind::Id, run(sql, 1, is_id))
            }
        }
        CC_KYWD | CC_ID => (Kind::Id, run(sql, 1, is_id)),
        // A NUL, and every byte no rule claims.
        _ => (Kind::Illegal, 1),
    })
}

/// `-`, `->`, `->>`, or a comment to the end of the line.
fn minus(sql: &[u8]) -> (Kind, usize) {
    match at(sql, 1) {
        Some(b'-') => (Kind::Comment, run(sql, 2, |byte| byte != b'\n')),
        Some(b'>') => {
            if at(sql, 2) == Some(b'>') {
                (Kind::PtrPtr, 3)
            } else {
                (Kind::Ptr, 2)
            }
        }
        _ => (Kind::Minus, 1),
    }
}

/// `/`, or a comment that runs to `*/` or to the end of the text.
fn slash(sql: &[u8]) -> (Kind, usize) {
    if at(sql, 1) != Some(b'*') || at(sql, 2).is_none() {
        return (Kind::Slash, 1);
    }
    let mut i = 3;
    let mut previous = at(sql, 2).unwrap_or(0);
    loop {
        if previous == b'*' && at(sql, i) == Some(b'/') {
            // The comment is closed, and the slash belongs to it.
            return (Kind::Comment, i.saturating_add(1));
        }
        match at(sql, i) {
            Some(byte) => {
                previous = byte;
                i = i.saturating_add(1);
            }
            // A comment that is never closed runs to the end.
            None => return (Kind::Comment, i),
        }
    }
}

/// `'text'`, `"name"` or `` `name` ``, where the delimiter is doubled to
/// stand for itself.
fn quoted(sql: &[u8], delimiter: u8) -> (Kind, usize) {
    let mut i = 1;
    loop {
        let Some(byte) = at(sql, i) else {
            // No closing delimiter: the run is illegal, and every byte of
            // it belongs to that one token.
            return (Kind::Illegal, i);
        };
        if byte == delimiter {
            if at(sql, i.saturating_add(1)) == Some(delimiter) {
                i = i.saturating_add(1);
            } else {
                let kind = if delimiter == b'\'' {
                    Kind::String
                } else {
                    Kind::Id
                };
                return (kind, i.saturating_add(1));
            }
        }
        i = i.saturating_add(1);
    }
}

/// `[name]`, which ends at the first `]` and has no escape.
fn bracketed(sql: &[u8]) -> (Kind, usize) {
    let mut i = 1;
    loop {
        match at(sql, i) {
            Some(b']') => return (Kind::Id, i.saturating_add(1)),
            Some(_) => i = i.saturating_add(1),
            None => return (Kind::Illegal, i),
        }
    }
}

/// `?`, `?12`, `:name`, `@name`, `$name`, and the two shapes Tcl brings
/// with it: `$name(text)` and `$part::part`.
fn variable(sql: &[u8]) -> (Kind, usize) {
    let mut kind = Kind::Variable;
    let mut named = 0usize;
    let mut i = 1;
    while let Some(byte) = at(sql, i) {
        if is_id(byte) {
            named = named.saturating_add(1);
        } else if byte == b'(' && named > 0 {
            // A Tcl array index: everything to the closing bracket.
            loop {
                i = i.saturating_add(1);
                match at(sql, i) {
                    Some(b')') => {
                        i = i.saturating_add(1);
                        break;
                    }
                    Some(byte) if !is_space(byte) => {}
                    _ => {
                        kind = Kind::Illegal;
                        break;
                    }
                }
            }
            break;
        } else if byte == b':' && at(sql, i.saturating_add(1)) == Some(b':') {
            i = i.saturating_add(1);
        } else {
            break;
        }
        i = i.saturating_add(1);
    }
    if named == 0 {
        kind = Kind::Illegal;
    }
    (kind, i)
}

/// A word that could be a keyword: the letters are scanned, and what is
/// scanned is looked up.
fn word(sql: &[u8]) -> (Kind, usize) {
    let letters = |byte: u8| matches!(class(byte), CC_X | CC_KYWD0 | CC_KYWD);
    let mut i = 1;
    while at(sql, i).is_some_and(letters) {
        i = i.saturating_add(1);
    }
    if at(sql, i).is_some_and(is_id) {
        // A letter this word cannot be a keyword by: a digit, a dollar, or
        // a byte of a wider character.
        return (Kind::Id, run(sql, i.saturating_add(1), is_id));
    }
    let text = sql.get(..i).unwrap_or_default();
    (lookup(text).map_or(Kind::Id, Kind::Keyword), i)
}

/// `x'0a'`, or an identifier that happens to begin with an `x`.
fn blob(sql: &[u8]) -> (Kind, usize) {
    if at(sql, 1) != Some(b'\'') {
        return word(sql);
    }
    let mut i = 2;
    while at(sql, i).is_some_and(is_hex) {
        i = i.saturating_add(1);
    }
    // An odd number of digits is as wrong as a missing quote, and both
    // take the rest of the literal with them.
    if at(sql, i) != Some(b'\'') || i % 2 != 0 {
        while at(sql, i).is_some_and(|byte| byte != b'\'') {
            i = i.saturating_add(1);
        }
        if at(sql, i).is_some() {
            i = i.saturating_add(1);
        }
        return (Kind::Illegal, i);
    }
    (Kind::Blob, i.saturating_add(1))
}

/// A number: decimal or hexadecimal, with a fraction, an exponent, and the
/// digit separators that make it one the parser has to put together.
fn number(sql: &[u8]) -> (Kind, usize) {
    let mut kind = Kind::Integer;
    let mut i = 0;
    if at(sql, 0) == Some(b'0')
        && matches!(at(sql, 1), Some(b'x' | b'X'))
        && at(sql, 2).is_some_and(is_hex)
    {
        i = 3;
        i = digits(sql, i, &mut kind, is_hex);
    } else {
        i = digits(sql, i, &mut kind, is_digit);
        if at(sql, i) == Some(b'.') {
            if kind == Kind::Integer {
                kind = Kind::Float;
            }
            i = digits(sql, i.saturating_add(1), &mut kind, is_digit);
        }
        if matches!(at(sql, i), Some(b'e' | b'E')) && exponent(sql, i) {
            if kind == Kind::Integer {
                kind = Kind::Float;
            }
            i = digits(sql, i.saturating_add(2), &mut kind, is_digit);
        }
    }
    // A letter stuck to a number makes the whole of it one illegal token,
    // which is why `1x` is not `1` and `x`.
    if at(sql, i).is_some_and(is_id) {
        return (Kind::Illegal, run(sql, i, is_id));
    }
    (kind, i)
}

/// Whether an `e` at `i` begins an exponent, which needs a digit after it
/// or a sign and then a digit.
fn exponent(sql: &[u8], i: usize) -> bool {
    let after = at(sql, i.saturating_add(1));
    after.is_some_and(is_digit)
        || (matches!(after, Some(b'+' | b'-'))
            && at(sql, i.saturating_add(2)).is_some_and(is_digit))
}

/// Digits of one kind from `i`, with `_` between them allowed and noted.
fn digits(sql: &[u8], from: usize, kind: &mut Kind, digit: fn(u8) -> bool) -> usize {
    let mut i = from;
    loop {
        match at(sql, i) {
            Some(byte) if digit(byte) => {}
            Some(b'_') => *kind = Kind::QNumber,
            _ => return i,
        }
        i = i.saturating_add(1);
    }
}

/// The byte at `offset`.
fn at(sql: &[u8], offset: usize) -> Option<u8> {
    sql.get(offset).copied()
}

/// How far a run of bytes that `keep` accepts reaches from `from`.
fn run(sql: &[u8], from: usize, keep: fn(u8) -> bool) -> usize {
    let mut i = from;
    while at(sql, i).is_some_and(keep) {
        i = i.saturating_add(1);
    }
    i
}

/// Whether the byte may appear inside an identifier: a letter, a digit,
/// `_`, `$`, or a byte of a character wider than ASCII.
#[must_use]
pub const fn is_id(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte >= 0x80
}

/// Whether the byte separates tokens.
const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// Whether the byte is a decimal digit.
const fn is_digit(byte: u8) -> bool {
    byte.is_ascii_digit()
}

/// Whether the byte is a hexadecimal digit.
const fn is_hex(byte: u8) -> bool {
    byte.is_ascii_hexdigit()
}

/// The class of a byte, which is what decides the rule it is read under.
/// The table is the one `tokenize.c` carries, value for value, and it has
/// an entry for every byte there is, so the fallback below is for the
/// compiler rather than for any input.
fn class(byte: u8) -> u8 {
    CLASS.get(usize::from(byte)).copied().unwrap_or(CC_ILLEGAL)
}

/// A byte no rule claims.
const CC_ILLEGAL: u8 = 28;

/// The letter `x`, which may begin a blob literal.
const CC_X: u8 = 0;
/// The first letter of a keyword.
const CC_KYWD0: u8 = 1;
/// A letter a keyword may carry.
const CC_KYWD: u8 = 2;
/// A digit.
const CC_DIGIT: u8 = 3;
/// `$`.
const CC_DOLLAR: u8 = 4;
/// `@`, `#` and `:`, which name a variable.
const CC_VARALPHA: u8 = 5;
/// `?`, which numbers one.
const CC_VARNUM: u8 = 6;
/// Whitespace.
const CC_SPACE: u8 = 7;
/// `"`, `'` and `` ` ``.
const CC_QUOTE: u8 = 8;
/// `[`.
const CC_QUOTE2: u8 = 9;
/// `|`.
const CC_PIPE: u8 = 10;
/// `-`.
const CC_MINUS: u8 = 11;
/// `<`.
const CC_LT: u8 = 12;
/// `>`.
const CC_GT: u8 = 13;
/// `=`.
const CC_EQ: u8 = 14;
/// `!`.
const CC_BANG: u8 = 15;
/// `/`.
const CC_SLASH: u8 = 16;
/// `(`.
const CC_LP: u8 = 17;
/// `)`.
const CC_RP: u8 = 18;
/// `;`.
const CC_SEMI: u8 = 19;
/// `+`.
const CC_PLUS: u8 = 20;
/// `*`.
const CC_STAR: u8 = 21;
/// `%`.
const CC_PERCENT: u8 = 22;
/// `,`.
const CC_COMMA: u8 = 23;
/// `&`.
const CC_AND: u8 = 24;
/// `~`.
const CC_TILDA: u8 = 25;
/// `.`.
const CC_DOT: u8 = 26;
/// A byte of a character wider than ASCII.
const CC_ID: u8 = 27;
/// The first byte of the UTF-8 byte-order mark.
const CC_BOM: u8 = 30;

/// The class of every byte.
const CLASS: [u8; 256] = [
    29, 28, 28, 28, 28, 28, 28, 28, 28, 7, 7, 28, 7, 7, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    28, 28, 28, 28, 28, 28, 28, 7, 15, 8, 5, 4, 22, 24, 8, 17, 18, 21, 20, 23, 11, 26, 16, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 5, 19, 12, 14, 13, 6, 5, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 0, 2, 2, 9, 28, 28, 28, 2, 8, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 0, 2, 2, 28, 10, 28, 25, 28, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27,
    27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27,
    27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27,
    27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27,
    27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27,
    27, 27, 27, 27, 27, 30, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27,
];
