// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{Error, Limits, Match, Options, Regex};

fn search(pattern: &str, text: &str) -> Result<Option<Match>, Error> {
    let regex = Regex::compile(
        &pattern.encode_utf16().collect::<Vec<_>>(),
        Options::default(),
        Limits::default(),
    )?;
    Ok(regex
        .find(
            &text.encode_utf16().collect::<Vec<_>>(),
            0,
            false,
            Limits::default(),
        )?
        .matched)
}

#[test]
fn shared_syntax_profile_is_explicit_and_does_not_change_nfa_acceptance() -> Result<(), Error> {
    use crate::syntax::{Expr, Profile, parse_with_profile};
    let limits = Limits::default();
    for pattern in [r"(a)\1", r"(?<=a)b", r"(?=ab)ab", r"(a?)*"] {
        let units: Vec<_> = pattern.encode_utf16().collect();
        assert!(parse_with_profile(&units, limits, Profile::Backtracking).is_ok());
        assert!(matches!(
            Regex::compile(&units, Options::default(), limits),
            Err(Error::Unsupported { .. })
        ));
    }
    let (tree, groups) = parse_with_profile(&[92, 49, 40, 97, 41], limits, Profile::Backtracking)?;
    assert_eq!(groups, 1);
    assert!(matches!(tree, Expr::Sequence(_)));
    assert!(crate::syntax::word(95) && !crate::syntax::word(0xd800));
    assert!(crate::syntax::newline(0x2028));
    assert!(matches!(
        parse_with_profile(&[92, 49], limits, Profile::Backtracking),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

#[test]
fn literals_alternation_quantifiers_and_search_priority() -> Result<(), Error> {
    for (pattern, text, range) in [
        ("", "abc", Some(0..0)),
        ("abc", "zzabcxx", Some(2..5)),
        ("xyz", "abc", None),
        ("a|ab", "ab", Some(0..1)),
        ("ab|a", "ab", Some(0..2)),
        ("ab|a", "ax", Some(0..1)),
        ("a*", "aaa", Some(0..3)),
        ("a*?", "aaa", Some(0..0)),
        ("a+?", "aaa", Some(0..1)),
        ("a+", "bbb", None),
        ("a?", "aa", Some(0..1)),
        ("a??", "aa", Some(0..0)),
        ("a{2,4}", "aaaaa", Some(0..4)),
        ("a{2,4}?", "aaaaa", Some(0..2)),
        ("a{2,}", "aaaa", Some(0..4)),
        ("a{2}", "baaa", Some(1..3)),
        ("a{0}", "aaa", Some(0..0)),
        ("a{0,2}", "aaa", Some(0..2)),
        ("a.*b|c", "axcxb", Some(0..5)),
        ("ab+c", "xabbcxabc", Some(1..5)),
        ("(ab|a)b", "ab", Some(0..2)),
        ("a.*?b", "a1b2b", Some(0..3)),
        ("|a", "a", Some(0..0)),
        ("a|", "a", Some(0..1)),
        ("a|", "b", Some(0..0)),
    ] {
        assert_eq!(
            search(pattern, text)?.map(|m| m.range),
            range,
            "{pattern} on {text}"
        );
    }
    Ok(())
}

#[test]
fn captures_use_last_iteration_and_clear_unmatched_subgroups() -> Result<(), Error> {
    for (pattern, text, expected) in [
        ("(a)(b)", "xab", vec![Some(1..2), Some(2..3)]),
        ("(a)?b", "b", vec![None]),
        ("()a", "a", vec![Some(0..0)]),
        ("(a)+", "aaa", vec![Some(2..3)]),
        ("((a)|(b))+", "ab", vec![Some(1..2), None, Some(1..2)]),
        ("(ab|a)b", "ab", vec![Some(0..1)]),
        ("(?:a)(b)", "ab", vec![Some(1..2)]),
        ("(a{1,2})(a)", "aaa", vec![Some(0..2), Some(2..3)]),
        ("(a|ab)*", "abab", vec![Some(0..1)]),
        ("(a|(b)){2}", "ba", vec![Some(1..2), None]),
    ] {
        assert_eq!(
            search(pattern, text)?.map(|m| m.captures),
            Some(expected),
            "{pattern}"
        );
    }
    Ok(())
}

#[test]
fn assertions_classes_escapes_and_utf16() -> Result<(), Error> {
    for (pattern, text, range) in [
        ("^a$", "a", Some(0..1)),
        ("^a$", "ba", None),
        ("a$", "a\n", None),
        ("\\bcat\\b", "a cat!", Some(2..5)),
        ("\\Bcat", "scat", Some(1..4)),
        ("[a-c]+", "xxabc", Some(2..5)),
        ("[^a]+", "aaBBa", Some(2..4)),
        ("[]", "a", None),
        ("[^]", "\n", Some(0..1)),
        ("[a-]+", "-aa", Some(0..3)),
        ("[-a]+", "-aa", Some(0..3)),
        ("[\\dA-C]+", "AB123x", Some(0..5)),
        ("[\\D]+", "12ab3", Some(2..4)),
        ("\\w+", "x_1!", Some(0..3)),
        ("\\W+", "x!?a", Some(1..3)),
        ("\\s+", "x\u{a0}\u{2028}y", Some(1..3)),
        ("\\S+", " ab ", Some(1..3)),
        ("\\x41\\u0042", "AB", Some(0..2)),
        ("\\cA", "\u{1}", Some(0..1)),
        ("[\\b]", "\u{8}", Some(0..1)),
        ("\\n\\r\\t\\v\\f\\0", "\n\r\t\u{b}\u{c}\0", Some(0..6)),
        ("\\(a\\)", "(a)", Some(0..3)),
        (".", "😀", Some(0..1)),
        ("😀", "😀", Some(0..2)),
        (".", "\n", None),
    ] {
        assert_eq!(search(pattern, text)?.map(|m| m.range), range, "{pattern}");
    }
    let regex = Regex::compile(&[0xd800], Options::default(), Limits::default())?;
    assert_eq!(
        regex
            .find(&[0xd800], 0, false, Limits::default())?
            .matched
            .map(|m| m.range),
        Some(0..1)
    );
    Ok(())
}

#[test]
fn options_sticky_offsets_and_end_of_input() -> Result<(), Error> {
    let pattern: Vec<_> = "^a$".encode_utf16().collect();
    let input: Vec<_> = "x\na\ny".encode_utf16().collect();
    let regex = Regex::compile(
        &pattern,
        Options {
            multiline: true,
            dot_all: false,
        },
        Limits::default(),
    )?;
    assert_eq!(
        regex
            .find(&input, 0, false, Limits::default())?
            .matched
            .map(|m| m.range),
        Some(2..3)
    );
    assert!(
        regex
            .find(&input, 0, true, Limits::default())?
            .matched
            .is_none()
    );
    assert_eq!(
        regex
            .find(&input, 2, true, Limits::default())?
            .matched
            .map(|m| m.range),
        Some(2..3)
    );
    assert!(
        regex
            .find(&input, 99, false, Limits::default())?
            .matched
            .is_none()
    );
    let regex = Regex::compile(
        &[46],
        Options {
            dot_all: true,
            multiline: false,
        },
        Limits::default(),
    )?;
    assert!(
        regex
            .find(&[10], 0, true, Limits::default())?
            .matched
            .is_some()
    );
    let empty = Regex::compile(&[], Options::default(), Limits::default())?;
    assert_eq!(
        empty
            .find(&[1], 1, true, Limits::default())?
            .matched
            .map(|m| m.range),
        Some(1..1)
    );
    Ok(())
}

#[test]
fn syntax_and_unsupported_constructs_are_explicit() {
    for pattern in [
        "(", ")", "[", "[z-a]", "*", "a**", "a{1", "a{2,1}", "a{1,x}", "\\", "\\xgg", "\\u00",
        "\\c1", "^*", "a]",
    ] {
        assert!(
            matches!(
                Regex::compile(
                    &pattern.encode_utf16().collect::<Vec<_>>(),
                    Options::default(),
                    Limits::default()
                ),
                Err(Error::Syntax { .. })
            ),
            "{pattern}"
        );
    }
    for pattern in [
        "(a)\\1",
        "\\0a\\01",
        "(?=ab)",
        "(?!a+)",
        "(?=.)",
        "(?=)",
        "(?=\\b)",
        "(?=(a))",
        "(?<=a)",
        "(?<x>a)",
        "(?i:a)",
        "\\k<x>",
        "\\p{L}",
        "\\u{1f600}",
        "\\q",
        "[\\d-a]",
        "(a?)*",
        "(a*)+",
    ] {
        assert!(
            matches!(
                Regex::compile(
                    &pattern.encode_utf16().collect::<Vec<_>>(),
                    Options::default(),
                    Limits::default()
                ),
                Err(Error::Unsupported { .. })
            ),
            "{pattern}"
        );
    }
}

#[test]
fn every_resource_budget_is_checked() -> Result<(), Error> {
    let defaults = Limits::default();
    for (pattern, limits) in [
        (
            "ab",
            Limits {
                pattern_units: 1,
                ..defaults
            },
        ),
        (
            "(a)",
            Limits {
                depth: 1,
                ..defaults
            },
        ),
        (
            "(a)",
            Limits {
                captures: 0,
                ..defaults
            },
        ),
        (
            "[ab]",
            Limits {
                ranges: 1,
                ..defaults
            },
        ),
        (
            "a{9}",
            Limits {
                repetition: 8,
                ..defaults
            },
        ),
        (
            "a{3}",
            Limits {
                states: 4,
                ..defaults
            },
        ),
    ] {
        assert!(
            matches!(
                Regex::compile(
                    &pattern.encode_utf16().collect::<Vec<_>>(),
                    Options::default(),
                    limits
                ),
                Err(Error::Limit { .. })
            ),
            "{pattern}"
        );
    }
    let regex = Regex::compile(&[97], Options::default(), defaults)?;
    assert_eq!(regex.capture_count(), 0);
    assert_eq!(regex.state_count(), 4);
    for limits in [
        Limits {
            input_units: 0,
            ..defaults
        },
        Limits {
            states: 1,
            ..defaults
        },
        Limits {
            capture_cells: 1,
            ..defaults
        },
        Limits {
            work: 0,
            ..defaults
        },
    ] {
        assert!(matches!(
            regex.find(&[97], 0, false, limits),
            Err(Error::Limit { .. })
        ));
    }
    let deep = format!("{}a{}", "(".repeat(1000), ")".repeat(1000));
    assert!(matches!(
        Regex::compile(
            &deep.encode_utf16().collect::<Vec<_>>(),
            Options::default(),
            defaults
        ),
        Err(Error::Limit { .. })
    ));
    Ok(())
}

#[test]
fn errors_have_messages() {
    for error in [
        Error::InvalidProgram,
        Error::Syntax {
            offset: 0,
            message: "x",
        },
        Error::Unsupported {
            offset: 0,
            feature: "x",
        },
        Error::Limit { resource: "x" },
    ] {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn adversarial_patterns_have_linear_state_visit_bounds() -> Result<(), Error> {
    for pattern in [
        "(a+)+$",
        "(a|aa)*b",
        "a*a*a*a*b",
        "(a|b|ab)+z",
        "(a+(?!b))+b",
        "a*(?=b)",
    ] {
        let regex = Regex::compile(
            &pattern.encode_utf16().collect::<Vec<_>>(),
            Options::default(),
            Limits::default(),
        )?;
        for length in [8usize, 32, 128, 512, 2048] {
            let mut input = vec![97; length];
            input.push(33);
            let report = regex.find(&input, 0, false, Limits::default())?;
            assert!(report.matched.is_none());
            let bound = regex
                .state_count()
                .saturating_mul(input.len().saturating_add(1));
            assert!(
                report.state_visits <= u64::try_from(bound).unwrap_or(u64::MAX),
                "{pattern}, {length}: {} > {bound}",
                report.state_visits
            );
        }
    }
    Ok(())
}

#[test]
fn property_literal_search_matches_utf16_windows() {
    use test_support::{generators, property};
    property::check(
        "regex-literal-search",
        &generators::pair(
            generators::vec(generators::range(97u16..=99), 0..=6),
            generators::vec(generators::range(97u16..=99), 0..=40),
        ),
        |(pattern, input)| {
            let regex = Regex::compile(pattern, Options::default(), Limits::default())
                .map_err(|e| e.to_string())?;
            let result = regex
                .find(input, 0, false, Limits::default())
                .map_err(|e| e.to_string())?
                .matched
                .map(|m| m.range);
            let expected = if pattern.is_empty() {
                Some(0..0)
            } else {
                input
                    .windows(pattern.len())
                    .position(|w| w == pattern)
                    .map(|i| i..i.saturating_add(pattern.len()))
            };
            if result == expected {
                Ok(())
            } else {
                Err(format!("{result:?} != {expected:?}"))
            }
        },
    );
}

#[test]
fn single_character_lookahead_is_zero_width_and_bounded() -> Result<(), Error> {
    for (pattern, text, expected) in [
        ("a(?!b)", "abac", Some(2..3)),
        ("a(?=b)", "ac ab", Some(3..4)),
        ("a(?![b-c])", "a", Some(0..1)),
        ("a(?=[b-c])", "a", None),
        ("(?=\\d)\\w+", "x123", Some(1..4)),
        ("(?![^])$", "a", Some(1..1)),
        ("a+(?!b)", "aaab", Some(0..2)),
        ("(?!a)", "", Some(0..0)),
        ("(?=a)", "", None),
        ("(?=a)", "a", Some(0..0)),
        ("(?=[])a", "a", None),
        ("(?![])a", "a", Some(0..1)),
    ] {
        assert_eq!(
            search(pattern, text)?.map(|m| m.range),
            expected,
            "{pattern}"
        );
    }
    let pattern =
        "([\\ud800-\\udbff]+)(?![\\udc00-\\udfff])|(^|[^\\ud800-\\udbff])([\\udc00-\\udfff]+)";
    let regex = Regex::compile(
        &pattern.encode_utf16().collect::<Vec<_>>(),
        Options::default(),
        Limits::default(),
    )?;
    for (input, expected) in [
        (
            vec![0xd800],
            Some(Match {
                range: 0..1,
                captures: vec![Some(0..1), None, None],
            }),
        ),
        (
            vec![0xdc00],
            Some(Match {
                range: 0..1,
                captures: vec![None, Some(0..0), Some(0..1)],
            }),
        ),
        (
            vec![0xd800, 0xd800, 0xdc00],
            Some(Match {
                range: 0..1,
                captures: vec![Some(0..1), None, None],
            }),
        ),
        (vec![0xd800, 0xdc00], None),
        (
            vec![97, 0xdc00],
            Some(Match {
                range: 0..2,
                captures: vec![None, Some(0..1), Some(1..2)],
            }),
        ),
    ] {
        let report = regex.find(&input, 0, false, Limits::default())?;
        assert!(
            report.state_visits
                <= u64::try_from(
                    regex
                        .state_count()
                        .saturating_mul(input.len().saturating_add(1))
                )
                .unwrap_or(u64::MAX)
        );
        assert_eq!(report.matched, expected);
    }
    Ok(())
}
