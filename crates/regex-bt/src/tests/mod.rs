// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use crate::{Error, Limits, Match, Options, Regex};
fn find(pattern: &str, text: &str) -> Result<Option<Match>, Error> {
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
fn strings(pattern: &str, text: &str) -> Result<Option<Vec<Option<String>>>, Error> {
    let text: Vec<_> = text.encode_utf16().collect();
    let regex = Regex::compile(
        &pattern.encode_utf16().collect::<Vec<_>>(),
        Options::default(),
        Limits::default(),
    )?;
    Ok(regex
        .find(&text, 0, false, Limits::default())?
        .matched
        .map(|m| {
            std::iter::once(Some(m.range))
                .chain(m.captures)
                .map(|range| range.map(|r| String::from_utf16(&text[r]).unwrap()))
                .collect()
        }))
}
#[test]
fn regular_subset_matches_thompson_captures_and_priority() -> Result<(), Error> {
    for pattern in [
        "",
        "a",
        "a|ab",
        "ab|a",
        "(ab|a)b",
        "a+",
        "a+?",
        "(a|b)*",
        "(a|aa)*b",
        "(a+)+$",
        "a{2,4}",
        "a{2,4}?",
        "(a|(b)){2}",
        "((a)|(b))+",
        "^a$",
        "\\b(a+)\\B",
        "[a-c]+",
        "[^x]",
        "[]",
        "[^]",
        "(a)(?!b)",
        "(a)(?=b)",
        "a{0}",
        "(?:a|b){0,2}",
    ] {
        let pattern: Vec<_> = pattern.encode_utf16().collect();
        for options in [
            Options::default(),
            Options {
                multiline: true,
                dot_all: true,
            },
        ] {
            let bt = Regex::compile(&pattern, options, Limits::default())?;
            let nfa = audhsos_regex::Regex::compile(&pattern, options, Limits::default().core)?;
            assert_eq!(bt.capture_count(), nfa.capture_count());
            for text in [
                "", "a", "ab", "aa", "aab", "abab", "abaabb", "b", "abc", "x\na\nb", "\r\n",
            ] {
                let input: Vec<_> = text.encode_utf16().collect();
                for from in 0..=input.len() + 1 {
                    for sticky in [false, true] {
                        let report = bt.find(&input, from, sticky, Limits::default())?;
                        assert_eq!(
                            report.matched,
                            nfa.find(&input, from, sticky, Limits::default().core)?
                                .matched,
                            "{pattern:?} {text:?} {from} {sticky}"
                        );
                        assert!(report.work >= report.state_visits);
                    }
                }
            }
        }
    }
    Ok(())
}
#[test]
fn references_nullable_loops_and_atomic_lookaround() -> Result<(), Error> {
    for (pattern, text, expected) in [
        (r"(a+)\1", "aaaa", vec![Some("aaaa"), Some("aa")]),
        (r"^(a|b)+\1$", "abb", vec![Some("abb"), Some("b")]),
        (r"\1(a)", "a", vec![Some("a"), Some("a")]),
        (r"(a\1)", "a", vec![Some("a"), Some("a")]),
        (r"(a)?b\1", "b", vec![Some("b"), None]),
        (r"(?=(a+))a*b\1", "baabac", vec![Some("aba"), Some("a")]),
        (
            r"(.*?)a(?!(a+)b\2c)\2(.*)",
            "baaabaac",
            vec![Some("baaabaac"), Some("ba"), None, Some("abaac")],
        ),
        (r"(a*)*", "b", vec![Some(""), None]),
        (r"(a*)+", "b", vec![Some(""), Some("")]),
        (r"(){2,4}", "", vec![Some(""), Some("")]),
        (r"(a?)*", "aa", vec![Some("aa"), Some("a")]),
        (r"(a?)*?a", "a", vec![Some("a"), None]),
        (r"(?:|a)*b", "ab", vec![Some("ab")]),
        (r"((a)?b)*", "abb", vec![Some("abb"), Some("b"), None]),
        (
            r"(?<=(a+)(b+))c",
            "aabbc",
            vec![Some("c"), Some("aa"), Some("bb")],
        ),
        (
            r"(?<=([ab]+)([bc]+))$",
            "abc",
            vec![Some(""), Some("a"), Some("bc")],
        ),
        (r"(?<=(\w+)\1)c", "ababc", vec![Some("c"), Some("abab")]),
        (r"(?<=\1(\w+))c", "ababc", vec![Some("c"), Some("ab")]),
        (r"(?<!a)b", "cb", vec![Some("b")]),
        (r"(?=(a|ab))\1b", "abb", vec![Some("ab"), Some("a")]),
        (r"(?=(?!(b))(a))a", "a", vec![Some("a"), None, Some("a")]),
        (r"(?<=(a))\1", "aa", vec![Some("a"), Some("a")]),
        (
            r"(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)\10",
            "abcdefghijj",
            vec![
                Some("abcdefghijj"),
                Some("a"),
                Some("b"),
                Some("c"),
                Some("d"),
                Some("e"),
                Some("f"),
                Some("g"),
                Some("h"),
                Some("i"),
                Some("j"),
            ],
        ),
    ] {
        assert_eq!(
            strings(pattern, text)?,
            Some(expected.into_iter().map(|v| v.map(str::to_owned)).collect()),
            "{pattern} on {text}"
        );
    }
    for (pattern, text) in [
        (r"^(a+)\1$", "aaa"),
        (r"^(?=(a+))a*b\1$", "aabac"),
        (r"(?<!a)b", "ab"),
        (r"(?<=a)b", "b"),
        (r"(?=(a|ab))\1c", "abc"),
        (r"(?<!(a))\1b", "ab"),
    ] {
        assert_eq!(find(pattern, text)?, None, "{pattern} on {text}");
    }
    Ok(())
}
#[test]
fn utf16_is_lossless_and_backtracking_really_restores_state() -> Result<(), Error> {
    let regex = Regex::compile(
        &[40, 0xd800, 41, 92, 49],
        Options::default(),
        Limits::default(),
    )?;
    let r = regex.find(&[0xd800, 0xd800], 0, false, Limits::default())?;
    assert_eq!(
        r.matched,
        Some(Match {
            range: 0..2,
            captures: vec![Some(0..1)]
        })
    );
    let regex = Regex::compile(
        &"^(a|aa)*b$".encode_utf16().collect::<Vec<_>>(),
        Options::default(),
        Limits::default(),
    )?;
    let report = regex.find(
        &"aaaaac".encode_utf16().collect::<Vec<_>>(),
        0,
        true,
        Limits::default(),
    )?;
    assert!(report.matched.is_none() && report.backtracks > 10 && report.peak_backtrack_frames > 0);
    let regex = Regex::compile(
        &"(?=(?=a)a)a".encode_utf16().collect::<Vec<_>>(),
        Options::default(),
        Limits::default(),
    )?;
    assert_eq!(
        regex
            .find(&[97], 0, false, Limits::default())?
            .peak_assertion_frames,
        2
    );
    Ok(())
}
#[test]
fn compile_quotas_are_errors_not_silent_nonmatches() {
    let base = Limits::default();
    for (pattern, limits) in [
        (
            "a",
            Limits {
                core: audhsos_regex::Limits {
                    pattern_units: 0,
                    ..base.core
                },
                ..base
            },
        ),
        (
            "a",
            Limits {
                core: audhsos_regex::Limits {
                    states: 0,
                    ..base.core
                },
                ..base
            },
        ),
        (
            "(a)",
            Limits {
                core: audhsos_regex::Limits {
                    captures: 0,
                    ..base.core
                },
                ..base
            },
        ),
        (
            "(a)",
            Limits {
                core: audhsos_regex::Limits {
                    depth: 1,
                    ..base.core
                },
                ..base
            },
        ),
        (
            "[ab]",
            Limits {
                core: audhsos_regex::Limits {
                    ranges: 1,
                    ..base.core
                },
                ..base
            },
        ),
        (
            "a{2}",
            Limits {
                core: audhsos_regex::Limits {
                    repetition: 1,
                    ..base.core
                },
                ..base
            },
        ),
        (
            "a",
            Limits {
                core: audhsos_regex::Limits {
                    capture_cells: 1,
                    ..base.core
                },
                ..base
            },
        ),
        (
            "a*",
            Limits {
                core: audhsos_regex::Limits {
                    capture_cells: 2,
                    ..base.core
                },
                ..base
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
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "explicit independent resource-limit cases remain readable as a table"
)]
fn match_quotas_are_errors_not_silent_nonmatches() -> Result<(), Error> {
    let base = Limits::default();
    for (pattern, text, limits) in [
        (
            "a",
            "a",
            Limits {
                core: audhsos_regex::Limits {
                    input_units: 0,
                    ..base.core
                },
                ..base
            },
        ),
        (
            "a",
            "a",
            Limits {
                core: audhsos_regex::Limits {
                    states: 0,
                    ..base.core
                },
                ..base
            },
        ),
        (
            "a",
            "a",
            Limits {
                core: audhsos_regex::Limits {
                    capture_cells: 0,
                    ..base.core
                },
                ..base
            },
        ),
        (
            "a",
            "a",
            Limits {
                core: audhsos_regex::Limits {
                    work: 0,
                    ..base.core
                },
                ..base
            },
        ),
        (
            "(a|b)",
            "a",
            Limits {
                core: audhsos_regex::Limits {
                    capture_cells: 4,
                    ..base.core
                },
                ..base
            },
        ),
        (
            "a*",
            "a",
            Limits {
                backtrack_frames: 0,
                ..base
            },
        ),
        (
            "(?=a)a",
            "a",
            Limits {
                assertion_frames: 0,
                ..base
            },
        ),
        (
            "(?=(?=a))a",
            "a",
            Limits {
                assertion_frames: 1,
                ..base
            },
        ),
        (
            "(a+)+$",
            "aaaaaaaaaaaaaaaaaaaab",
            Limits {
                core: audhsos_regex::Limits {
                    work: 2000,
                    ..base.core
                },
                ..base
            },
        ),
    ] {
        let r = Regex::compile(
            &pattern.encode_utf16().collect::<Vec<_>>(),
            Options::default(),
            base,
        )?;
        assert!(
            matches!(
                r.find(&text.encode_utf16().collect::<Vec<_>>(), 0, false, limits),
                Err(Error::Limit { .. })
            ),
            "{pattern}"
        );
    }
    Ok(())
}

#[test]
fn exact_work_boundary_and_out_of_bounds_start() -> Result<(), Error> {
    let base = Limits::default();
    let r = Regex::compile(&[97], Options::default(), base)?;
    let report = r.find(&[97], 0, false, base)?;
    assert_eq!(
        r.find(
            &[97],
            0,
            false,
            Limits {
                core: audhsos_regex::Limits {
                    work: report.work,
                    ..base.core
                },
                ..base
            }
        )?
        .matched,
        report.matched
    );
    assert!(
        r.find(
            &[97],
            0,
            false,
            Limits {
                core: audhsos_regex::Limits {
                    work: report.work - 1,
                    ..base.core
                },
                ..base
            }
        )
        .is_err()
    );
    assert!(r.find(&[97], 2, false, base)?.matched.is_none());
    Ok(())
}
#[test]
fn syntax_restrictions_and_automaton_isolation() {
    for p in ["(", "[", "a{3,2}", "a**", "\\", "(?=a", "(?<=a", "(?=a)*"] {
        assert!(
            matches!(
                Regex::compile(
                    &p.encode_utf16().collect::<Vec<_>>(),
                    Options::default(),
                    Limits::default()
                ),
                Err(Error::Syntax { .. })
            ),
            "{p}"
        );
    }
    for p in ["(?<name>a)", "\\9", "\\0x\\1", "\\k<x>", "\\p{L}", "(?i)a"] {
        assert!(
            matches!(
                Regex::compile(
                    &p.encode_utf16().collect::<Vec<_>>(),
                    Options::default(),
                    Limits::default()
                ),
                Err(Error::Unsupported { .. })
            ),
            "{p}"
        );
    }
    for p in ["(a)\\1", "(?=ab)ab", "(?<=a)b", "(a?)*"] {
        assert!(
            Regex::compile(
                &p.encode_utf16().collect::<Vec<_>>(),
                Options::default(),
                Limits::default()
            )
            .is_ok()
        );
        assert!(
            matches!(
                audhsos_regex::Regex::compile(
                    &p.encode_utf16().collect::<Vec<_>>(),
                    Options::default(),
                    Limits::default().core
                ),
                Err(Error::Unsupported { .. })
            ),
            "{p}"
        );
    }
}

#[test]
fn reverse_atomicity_and_nested_empty_capture_regressions() -> Result<(), Error> {
    // Expected results independently checked against Node; null captures are None.
    for (p, t, expected) in [
        (r"(?<=(a|ab))c", "abc", vec![Some("c"), Some("ab")]),
        (
            r"(?<=((a)|(b))+)c",
            "abc",
            vec![Some("c"), Some("a"), Some("a"), None],
        ),
        (r"(?<=(a?)+)b", "b", vec![Some("b"), Some("")]),
        (
            r"(?=(a?)*)(a*)",
            "aa",
            vec![Some("aa"), Some("a"), Some("aa")],
        ),
        (r"(?!(a))b\1", "b", vec![Some("b"), None]),
        (r"(?!(?=(a)))b", "b", vec![Some("b"), None]),
        (r"(?<!(a+))b", "b", vec![Some("b"), None]),
        (r"(?<=a(?=b))b", "ab", vec![Some("b")]),
        (r"(?<=(a+))a", "aaa", vec![Some("a"), Some("a")]),
        (r"(a*){2,3}", "aa", vec![Some("aa"), Some("")]),
        (r"(a?){2,4}?", "a", vec![Some("a"), Some("")]),
        (r"((?:)|a)+b", "ab", vec![Some("ab"), Some("a")]),
        (r"(?:()|a)*", "a", vec![Some("a"), None]),
        (r"(?=(a+))\1", "aaa", vec![Some("aaa"), Some("aaa")]),
        (r"(?!a|bc)b", "bd", vec![Some("b")]),
        (
            r"(?<=(a|aa)*)(b)",
            "aab",
            vec![Some("b"), Some("a"), Some("b")],
        ),
        (r"(?<=(.)\1)a", "bba", vec![Some("a"), Some("b")]),
        (r"(?<=\1(.))a", "bba", vec![Some("a"), Some("b")]),
        (
            r"(a|(bc))*\1",
            "abcbc",
            vec![Some("abcbc"), Some("bc"), Some("bc")],
        ),
        (r"(?=(a))(?!b)\1", "a", vec![Some("a"), Some("a")]),
        (r"(a?){0}", "a", vec![Some(""), None]),
        (r"(a?){2}", "", vec![Some(""), Some("")]),
        (r"((a)?)*", "a", vec![Some("a"), Some("a"), Some("a")]),
        (r"(?<=a{1,3}?)(b)", "aab", vec![Some("b"), Some("b")]),
    ] {
        assert_eq!(
            strings(p, t)?,
            Some(expected.into_iter().map(|v| v.map(str::to_owned)).collect()),
            "{p} {t}"
        );
    }
    assert_eq!(find("(?!a|bc)b", "bc")?, None);
    Ok(())
}

#[test]
fn compile_and_runtime_shared_work_prevents_pathological_empty_expansion() -> Result<(), Error> {
    let limits = Limits {
        core: audhsos_regex::Limits {
            work: 1000,
            ..Limits::default().core
        },
        ..Limits::default()
    };
    for p in ["(?:){9999}", "(?:(?:){9999}){9999}"] {
        assert!(
            matches!(
                Regex::compile(
                    &p.encode_utf16().collect::<Vec<_>>(),
                    Options::default(),
                    limits
                ),
                Err(Error::Limit {
                    resource: "compilation work"
                })
            ),
            "{p}"
        );
    }
    let r = Regex::compile(
        &"a*b".encode_utf16().collect::<Vec<_>>(),
        Options::default(),
        Limits::default(),
    )?;
    let input = vec![97; 1000];
    assert!(matches!(
        r.find(&input, 0, false, limits),
        Err(Error::Limit { .. })
    ));
    let text = "x".repeat(60);
    let deep = format!("{}a{}", "(?=".repeat(60), ")".repeat(60));
    assert!(matches!(
        Regex::compile(
            &deep.encode_utf16().collect::<Vec<_>>(),
            Options::default(),
            Limits::default()
        ),
        Err(Error::Limit { .. })
    ));
    assert!(find("(?<=a)b", &text)?.is_none());
    for p in [r"\999999999999999999999999999999999999999", r"\k<a>"] {
        assert!(
            Regex::compile(
                &p.encode_utf16().collect::<Vec<_>>(),
                Options::default(),
                Limits::default()
            )
            .is_err()
        );
    }
    Ok(())
}
