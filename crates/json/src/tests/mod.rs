// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::*;
#[test]
fn strict_grammar_and_arena() {
    for text in [
        "null",
        "true",
        "false",
        "-0",
        "1.25e+3",
        "1E-3",
        "1e400",
        "[]",
        "{}",
        "[1,{},[true]]",
        "{\"a\":1,\"a\":2}",
        " \t\r\n1\n",
    ] {
        assert!(
            parse(
                &text.encode_utf16().collect::<Vec<_>>(),
                Limits::default(),
                &mut 1_000_000
            )
            .is_ok(),
            "{text}"
        );
    }
    for text in [
        "",
        "undefined",
        "NaN",
        "Infinity",
        "+1",
        "01",
        "-01",
        ".1",
        "1.",
        "1e",
        "1e+",
        "[1,]",
        "{\"x\":1,}",
        "{x:1}",
        "[,,]",
        "/*x*/1",
        "1 2",
        "\u{a0}1",
        "\"\n\"",
        "\"\\x41\"",
        "\"\\uX000\"",
        "\"\\u00\"",
        "\"\\q\"",
        "[",
        "{",
        "tru",
        "\"",
    ] {
        assert!(
            matches!(
                parse(
                    &text.encode_utf16().collect::<Vec<_>>(),
                    Limits::default(),
                    &mut 1_000_000
                ),
                Err(Error::Syntax(_))
            ),
            "{text}"
        );
    }
    let d = parse(
        &" {\"a\":-0} ".encode_utf16().collect::<Vec<_>>(),
        Limits::default(),
        &mut 100,
    )
    .unwrap();
    assert_eq!(d.root, 1);
    assert_eq!(d.nodes[0].source, 6..8);
    assert!(matches!(d.nodes[0].kind,Kind::Number(n) if n.is_sign_negative()));
}
#[test]
fn quoting_roundtrips_every_code_unit() {
    for unit in 0..=u16::MAX {
        let mut out = Vec::new();
        quote(&mut out, &[unit], 16, &mut 100).unwrap();
        let d = parse(&out, Limits::default(), &mut 100).unwrap();
        assert!(matches!(&d.nodes[d.root].kind,Kind::String(s) if s.as_ref()==[unit]));
    }
    let mut out = Vec::new();
    quote(&mut out, &[0xd83d, 0xde00], 8, &mut 100).unwrap();
    assert_eq!(out, [34, 0xd83d, 0xde00, 34]);
    let d = parse(
        &"\"\\b\\f\\n\\r\\t\\/\\\\\\\"\\u004A\""
            .encode_utf16()
            .collect::<Vec<_>>(),
        Limits::default(),
        &mut 100,
    )
    .unwrap();
    assert!(
        matches!(&d.nodes[d.root].kind,Kind::String(s) if s.as_ref()==[8,12,10,13,9,47,92,34,74])
    );
}
#[test]
fn quotas() {
    for limits in [
        Limits {
            input: 0,
            ..Limits::default()
        },
        Limits {
            nodes: 0,
            ..Limits::default()
        },
        Limits {
            nodes: 1,
            ..Limits::default()
        },
        Limits {
            depth: 0,
            ..Limits::default()
        },
        Limits {
            strings: 0,
            ..Limits::default()
        },
    ] {
        assert!(matches!(
            parse(
                &"[\"x\"]".encode_utf16().collect::<Vec<_>>(),
                limits,
                &mut 100
            ),
            Err(Error::Limit)
        ));
    }
    assert!(matches!(
        parse(&[49], Limits::default(), &mut 0),
        Err(Error::Limit)
    ));
    let deep = format!("{}0{}", "[".repeat(100), "]".repeat(100));
    assert!(matches!(
        parse(
            &deep.encode_utf16().collect::<Vec<_>>(),
            Limits::default(),
            &mut 1000
        ),
        Err(Error::Limit)
    ));
    assert_eq!(quote(&mut Vec::new(), &[1], 1, &mut 100), Err(Error::Limit));
    assert_eq!(quote(&mut Vec::new(), &[1], 100, &mut 0), Err(Error::Limit));
}
