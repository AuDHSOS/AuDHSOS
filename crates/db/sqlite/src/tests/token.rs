// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::token`, against SQLite's own tokenizer.
//!
//! `fixtures/tokens.corpus` is nine hundred and forty pieces of SQL, one
//! per NUL-terminated string: every rule of the tokenizer by hand, every
//! way of writing an illegal one, and eight hundred statements taken out
//! of SQLite's own test suite. `fixtures/tokens.golden` is what
//! `sqlite3GetToken` answered for each of them, one line per case, written
//! by a program that links the C library. The test is the comparison: for
//! every byte of every case, the same kind and the same length.

#![allow(clippy::arithmetic_side_effects)]

use crate::keyword::{Keyword, lookup};
use crate::token::{Kind, Lexer, Token, token};

/// The cases, split on the NUL that ends each.
fn corpus() -> Vec<&'static [u8]> {
    let bytes: &'static [u8] = include_bytes!("fixtures/tokens.corpus");
    let mut cases: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    // The last NUL ends the last case rather than starting another.
    cases.pop();
    cases
}

/// What SQLite answered for each case.
fn golden() -> Vec<&'static str> {
    let text: &'static str = include_str!("fixtures/tokens.golden");
    text.lines().collect()
}

/// The name the oracle prints for a kind. A keyword is one name, because
/// the C tokenizer answers a code per keyword and the name of the code is
/// not the name of the word.
fn name(kind: Kind) -> &'static str {
    match kind {
        Kind::Space => "Space",
        Kind::Comment => "Comment",
        Kind::Illegal => "Illegal",
        Kind::Id => "Id",
        Kind::Keyword(_) => "Keyword",
        Kind::String => "String",
        Kind::Blob => "Blob",
        Kind::Integer => "Integer",
        Kind::Float => "Float",
        Kind::QNumber => "QNumber",
        Kind::Variable => "Variable",
        Kind::Minus => "Minus",
        // The C tokenizer answers one code for both arrows and lets the
        // parser look at the length.
        Kind::Ptr | Kind::PtrPtr => "Ptr",
        Kind::Lp => "Lp",
        Kind::Rp => "Rp",
        Kind::Semi => "Semi",
        Kind::Plus => "Plus",
        Kind::Star => "Star",
        Kind::Slash => "Slash",
        Kind::Rem => "Rem",
        Kind::Eq => "Eq",
        Kind::Le => "Le",
        Kind::Ne => "Ne",
        Kind::LShift => "LShift",
        Kind::Lt => "Lt",
        Kind::Ge => "Ge",
        Kind::RShift => "RShift",
        Kind::Gt => "Gt",
        Kind::BitOr => "BitOr",
        Kind::Concat => "Concat",
        Kind::Comma => "Comma",
        Kind::BitAnd => "BitAnd",
        Kind::BitNot => "BitNot",
        Kind::Dot => "Dot",
    }
}

/// The token stream of one case, in the oracle's own notation.
fn stream(sql: &[u8]) -> String {
    let mut out = String::new();
    for token in Lexer::new(sql) {
        out.push_str(name(token.kind));
        out.push(':');
        out.push_str(&token.len.to_string());
        out.push(' ');
    }
    out.trim_end().to_owned()
}

#[test]
fn every_case_of_the_corpus_tokenizes_as_sqlite_tokenizes_it() {
    let cases = corpus();
    let golden = golden();
    assert_eq!(
        cases.len(),
        golden.len(),
        "the corpus and the golden differ"
    );
    for (case, expected) in cases.iter().zip(&golden) {
        let text = String::from_utf8_lossy(case);
        assert_eq!(
            stream(case),
            expected.trim_end(),
            "case `{text}` ({case:?})"
        );
    }
}

#[test]
fn the_tokens_of_a_case_partition_its_bytes() {
    for case in corpus() {
        let mut at = 0;
        for token in Lexer::new(case) {
            assert_eq!(token.start, at, "a gap or an overlap in {case:?}");
            assert!(token.len > 0, "a token of no bytes in {case:?}");
            at += token.len;
        }
        assert_eq!(at, case.len(), "a case that was not read to its end");
    }
}

#[test]
fn a_token_answers_the_bytes_it_was_read_from() {
    let sql = b"SELECT 1";
    let tokens: Vec<Token> = Lexer::new(sql).collect();
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0].text(sql), b"SELECT");
    assert_eq!(tokens[1].text(sql), b" ");
    assert_eq!(tokens[2].text(sql), b"1");
    assert_eq!(Lexer::new(sql).sql(), sql);
    // A token of a text it did not come from answers nothing rather than
    // a slice of the wrong bytes.
    assert_eq!(tokens[0].text(b"S"), b"");
}

#[test]
fn a_keyword_is_a_keyword_in_any_case_and_a_word_that_is_not_is_an_identifier() {
    assert_eq!(token(b"select"), Some((Kind::Keyword(Keyword::Select), 6)));
    assert_eq!(token(b"SELECT"), Some((Kind::Keyword(Keyword::Select), 6)));
    assert_eq!(token(b"SeLeCt"), Some((Kind::Keyword(Keyword::Select), 6)));
    assert_eq!(token(b"selects"), Some((Kind::Id, 7)));
    assert_eq!(token(b"selec"), Some((Kind::Id, 5)));
    assert_eq!(token(b"zzz"), Some((Kind::Id, 3)));
}

#[test]
fn every_keyword_of_the_table_is_found_by_the_lookup() {
    for keyword in [
        Keyword::Abort,
        Keyword::Select,
        Keyword::Values,
        Keyword::With,
        Keyword::Window,
    ] {
        let word = keyword.word();
        assert_eq!(lookup(word), Some(keyword), "{word:?}");
        let lower = word.to_ascii_lowercase();
        assert_eq!(lookup(&lower), Some(keyword));
    }
    assert_eq!(lookup(b"notakeyword"), None);
    assert_eq!(lookup(b""), None);
    assert_eq!(lookup(b"SELECTX"), None);
    assert_eq!(lookup(b"SELEC"), None);
}

#[test]
fn the_end_of_the_text_is_where_the_walk_stops() {
    assert_eq!(token(b""), None);
    assert_eq!(Lexer::new(b"").next(), None);
    let mut lexer = Lexer::new(b";");
    assert!(lexer.next().is_some());
    assert_eq!(lexer.next(), None);
}

#[test]
fn whitespace_and_comments_are_what_a_parser_skips() {
    assert!(Kind::Space.is_trivia());
    assert!(Kind::Comment.is_trivia());
    assert!(!Kind::Id.is_trivia());
    assert!(!Kind::Keyword(Keyword::Select).is_trivia());
}
