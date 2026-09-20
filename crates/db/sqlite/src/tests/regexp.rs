// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::regexp`.
//!
//! The cases are those of `research/sqlite/test/regexp1.test` and
//! `research/sqlite/test/regexp2.test`, beside the ones that reach a
//! step the two files leave alone.

use crate::regexp::compile;

/// Whether `text` matches `pattern`, read case by case.
fn hit(pattern: &str, text: &str) -> bool {
    compile(pattern.as_bytes(), false).is_ok_and(|held| held.matches(text.as_bytes()))
}

/// What a pattern the matcher does not read is refused for.
fn why(pattern: &str) -> &'static str {
    compile(pattern.as_bytes(), false).err().unwrap_or("")
}

#[test]
fn what_the_matcher_answers_for_the_operators_of_the_grammar() {
    assert!(hit("^For ", "For since by man came death,"));
    assert!(!hit("^For ", "by man came also the resurrection"));
    assert!(hit("by|in", "For as in Adam all die,"));
    assert!(hit("a(b$|cd)", "xab"));
    assert!(!hit("a(b$|cd)", "xaby"));
    assert!(hit("a(cd|b$|e)", "xacd"));
    assert!(hit("ab*c", "ac"));
    assert!(hit("ab*c", "abbbc"));
    assert!(hit("ab+c", "abc"));
    assert!(!hit("ab+c", "ac"));
    assert!(hit("ab?c", "ac"));
    assert!(!hit("ab?c", "abbc"));
    assert!(hit("a.c", "abc"));
    assert!(hit("a.*z", "abcz"));
    assert!(hit("^([a-z]+)$", "foo"));
    assert!(hit("(^abc|def)", "abc"));
    assert!(!hit("(^abc|def)", "xabc"));
    assert!(hit("(^abc|def)", "xdef"));
}

#[test]
fn what_the_matcher_answers_for_a_counted_repeat() {
    assert!(!hit("^[a-z][a-z0-9]{0,30}$", "fooX"));
    assert!(hit("^[a-z][a-z0-9]{0,30}X$", "fooX"));
    assert!(hit("^[a-z][a-z0-9]{0,2}X$", "fooX"));
    assert!(!hit("^[a-z][a-z0-9]{0,2}X$", "foooX"));
    assert!(hit("^[a-z][a-z0-9]{0,3}X$", "foooX"));
    assert!(hit("a{1,999}bc", "abc"));
    assert!(!hit("a{999}bc", "abc"));
    assert!(hit("a{2}b", "aab"));
    assert!(!hit("[^a-z]{2}", "abc"));
    assert!(hit("[^a-z]{2}", "   "));
}

#[test]
fn what_the_matcher_answers_for_a_character_class() {
    assert!(hit("[a-z]", "foo"));
    assert!(!hit("[a-z]", "   "));
    assert!(hit("[^a-z]", "a c"));
    assert!(!hit("[^a-z]", "abc"));
    assert!(!hit("[^a]", ""));
    assert!(!hit("[1-5]", "abc-def"));
    assert!(hit("[1\\-5]", "abc-def"));
    assert!(hit("[x\\-]", "abc-def"));
    assert!(hit("-", "abc-def"));
    assert!(hit("\\-", "abc-def"));
    assert!(hit("[\\x61]", "a"));
}

#[test]
fn what_the_matcher_answers_for_the_classes_a_backslash_names() {
    assert!(!hit("\\W", "abc"));
    assert!(hit("\\W", "a c"));
    assert!(hit("\\w", "abc"));
    assert!(!hit("\\w", "   "));
    assert!(hit("\\D", "abc"));
    assert!(!hit("\\D", "5"));
    assert!(!hit("\\d", "abc"));
    assert!(hit("\\d", "a1c"));
    assert!(hit("\\s", "a c"));
    assert!(!hit("\\s", "abc"));
    assert!(hit("\\S", "abc"));
    assert!(!hit("\\S", " "));
    assert!(hit("\\bcd", "ab cd"));
    assert!(!hit("\\bcd", "abcd"));
    assert!(hit("\\u0061", "a"));
    assert!(hit("\\x62", "ab"));
    assert!(hit("\\t", "a\tb"));
    assert!(hit("\\n", "a\nb"));
    assert!(hit("a\\$", "a$"));
}

#[test]
fn what_the_matcher_answers_for_a_character_past_the_first_128() {
    assert!(hit("日本語", "日本語"));
    assert!(hit("\u{7ff}", "a\u{7ff}b"));
    assert!(hit("\u{800}", "a\u{800}b"));
    assert!(hit("\u{ffff}", "a\u{ffff}b"));
    assert!(hit("\u{10ffff}", "a\u{10ffff}b"));
    assert!(hit(".", "\u{10000}"));
}

#[test]
fn what_the_matcher_answers_for_bytes_that_are_not_utf8() {
    let pattern = compile(b".", false).unwrap();
    assert!(pattern.matches(&[0xc3]));
    assert!(pattern.matches(&[0xe6]));
    assert!(pattern.matches(&[0xe6, 0x97]));
    assert!(pattern.matches(&[0xf0]));
    assert!(pattern.matches(&[0xf0, 0x9f]));
    assert!(pattern.matches(&[0xf0, 0x9f, 0x98]));
    assert!(pattern.matches(&[0xc0, 0x80]));
    assert!(pattern.matches(&[0xe0, 0x80, 0x80]));
    assert!(pattern.matches(&[0xed, 0xa0, 0x80]));
    assert!(pattern.matches(&[0xf0, 0x80, 0x80, 0x80]));
    assert!(pattern.matches(&[0xf4, 0x90, 0x80, 0x80]));
    assert!(pattern.matches(&[0xff]));
}

#[test]
fn what_the_matcher_answers_where_a_capital_and_its_letter_are_one() {
    let pattern = compile(b"abc", true).unwrap();
    assert!(pattern.matches(b"ABC"));
    let pattern = compile(b"ABC", true).unwrap();
    assert!(pattern.matches(b"abc"));
    let pattern = compile(b"ABC.", true).unwrap();
    assert!(!pattern.matches(b"ABC"));
}

#[test]
fn what_a_pattern_the_matcher_does_not_read_is_refused_for() {
    assert_eq!(why("[x-]"), "unclosed '['");
    assert_eq!(why("[[:alpha:]]"), "POSIX character classes not supported");
    assert_eq!(why("(ab"), "unmatched '('");
    assert_eq!(why("ab)"), "unrecognized character");
    assert_eq!(why("*a"), "'*' without operand");
    assert_eq!(why("+a"), "'+' without operand");
    assert_eq!(why("?a"), "'?' without operand");
    assert_eq!(why("{2}"), "'{m,n}' without operand");
    assert_eq!(why("a{2"), "unmatched '{'");
    assert_eq!(why("a{3,2}"), "n less than m in '{m,n}'");
    assert_eq!(why("a{2,}b"), "n less than m in '{m,n}'");
    assert_eq!(why("a{0,0}"), "both m and n are zero in '{m,n}'");
    assert_eq!(why("a{1,25000}bc"), "REGEXP pattern too big");
    assert_eq!(why("a{25000}bc"), "REGEXP pattern too big");
    assert_eq!(why("a\\q"), "unknown \\ escape");
}

#[test]
fn what_a_pattern_longer_than_the_matcher_takes_is_refused_for() {
    let pattern = alloc::vec![b'a'; crate::regexp::PATTERN + 1];
    assert_eq!(
        compile(&pattern, false).err(),
        Some("REGEXP pattern too big")
    );
}

#[test]
fn what_the_matcher_answers_where_a_step_reads_past_the_last_character() {
    assert!(!hit(".", ""));
    assert!(hit("$", "abc"));
    assert!(!hit("[^a]", ""));
    assert!(!hit("\\W", ""));
    assert!(!hit("\\D", ""));
    assert!(!hit("\\S", ""));
    assert!(!hit("b\\b", "abc"));
}

#[test]
fn what_the_matcher_answers_for_the_bytes_every_match_begins_with() {
    assert!(!hit("xyz", "abc"));
    assert!(hit("abcdefghijklmnop", "abcdefghijklmnop"));
    let long = "a".repeat(40);
    assert!(hit(&long, &long));
    assert!(!hit(&long, "x"));
    assert!(!hit("a(^b)", "xab"));
    assert!(hit("a|b|c", "c"));
    assert!(!hit("a|b|c", "zzz"));
    assert!(hit("(a|b)+$", "ab"));
}

#[test]
fn what_the_matcher_answers_for_an_escape_inside_a_character_class() {
    assert!(hit("[[a]", "["));
    assert!(hit("[a-\\x7a]", "b"));
    assert!(hit("[a\\-c]", "abc-def"));
    assert!(hit("[]a]", "a"));
    assert_eq!(why("[^\\W]"), "unknown \\ escape");
}

#[test]
fn what_the_matcher_answers_for_an_escape_no_character_follows() {
    assert!(hit("a\\", "a"));
    assert!(hit("a\\(b", "a(b"));
    assert!(!hit("\\a", "a"));
    assert_eq!(why("\\u12"), "unknown \\ escape");
    assert_eq!(why("\\x1"), "unknown \\ escape");
}

#[test]
fn what_the_matcher_answers_for_a_repeat_of_a_repeat() {
    assert!(!hit("a**", "x"));
    assert!(hit("(abc){2}", "abcabcabc"));
    assert_eq!(why("(abc){12000}"), "REGEXP pattern too big");
    assert_eq!(why("a{0,12000}"), "REGEXP pattern too big");
}

#[test]
fn what_a_refusal_of_the_matcher_reads_as_a_message() {
    assert_eq!(
        crate::eval::Error::Regexp("unclosed '['").message(),
        "unclosed '['"
    );
}
