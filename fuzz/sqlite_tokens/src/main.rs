// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The SQL tokenizer against arbitrary bytes: no input may panic, and
//! whatever the bytes are, the tokens of them partition them.
//!
//! A tokenizer that can stall is a parser that can hang, so the property
//! that matters most here is the one that is easiest to lose: every token
//! is at least one byte, and the tokens together are the input, in order,
//! with no gap and no overlap.

use db_sqlite::keyword::lookup;
use db_sqlite::token::{Kind, Lexer, token};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let mut at = 0;
    for found in Lexer::new(bytes) {
        assert_eq!(found.start, at, "a gap or an overlap");
        assert!(found.len > 0, "a token of no bytes");
        let text = found.text(bytes);
        assert_eq!(text.len(), found.len, "a token that is not its own text");
        at = at.saturating_add(found.len);
        assert!(at <= bytes.len(), "a token past the end of the input");
        if let Kind::Keyword(keyword) = found.kind {
            assert_eq!(lookup(text), Some(keyword), "a keyword that is not one");
            assert!(!text.is_empty());
        }
    }
    assert_eq!(at, bytes.len(), "an input that was not read to its end");

    // The first token of the whole input is the first token of the walk.
    match (token(bytes), Lexer::new(bytes).next()) {
        (None, None) => assert!(bytes.is_empty()),
        (Some((kind, len)), Some(first)) => {
            assert_eq!(kind, first.kind);
            assert_eq!(len, first.len);
        }
        _ => panic!("the walk and the reader disagree about the first token"),
    }
});
