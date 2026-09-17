// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::segment::{grapheme_boundaries, word_boundaries};
fn cases(name: &str, mut check: impl FnMut(&str, &[usize], usize)) -> usize {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/unicode/ucd/auxiliary")
        .join(name);
    let data = std::fs::read_to_string(path).expect("conformance file");
    let mut count = 0_usize;
    for (line_number, line) in data.lines().enumerate() {
        let line = line.split('#').next().expect("line").trim();
        if line.is_empty() {
            continue;
        }
        let mut text = String::new();
        let mut breaks = Vec::new();
        for token in line.split_whitespace() {
            match token {
                "÷" => breaks.push(text.len()),
                "×" => (),
                _ => text.push(
                    char::from_u32(u32::from_str_radix(token, 16).expect("code")).expect("scalar"),
                ),
            }
        }
        check(&text, &breaks, line_number.checked_add(1).expect("line"));
        count = count.checked_add(1_usize).expect("count");
    }
    count
}
#[test]
fn complete_grapheme_conformance() {
    let count = cases("GraphemeBreakTest.txt", |text, expected, line| {
        assert_eq!(
            grapheme_boundaries(text).collect::<Vec<_>>(),
            expected,
            "line {line}: {text:?}"
        );
    });
    assert_eq!(count, 853);
    eprintln!("GraphemeBreakTest: {count} cases");
}
#[test]
fn complete_word_conformance() {
    let count = cases("WordBreakTest.txt", |text, expected, line| {
        assert_eq!(
            word_boundaries(text).collect::<Vec<_>>(),
            expected,
            "line {line}: {text:?}"
        );
    });
    assert_eq!(count, 1944);
    eprintln!("WordBreakTest: {count} cases");
}

#[test]
fn complete_line_conformance() {
    use crate::segment::{Break, LineBoundary, LineUnit, line_breaks};
    let count = cases("LineBreakTest.txt", |text, expected, line| {
        let mut work = vec![LineUnit::default(); text.chars().count()];
        let mut output =
            vec![LineBoundary::default(); text.chars().count().checked_add(1).expect("length")];
        let n = line_breaks(text, &mut work, &mut output).expect("line breaks");
        let actual: Vec<_> = output[..n]
            .iter()
            .filter(|p| p.kind != Break::Prohibited)
            .map(|p| p.byte)
            .collect();
        assert_eq!(actual, expected, "line {line}: {text:?}");
    });
    assert_eq!(count, 19346);
    eprintln!("LineBreakTest: {count} cases");
}

#[test]
fn segmentation_empty_buffers_and_mandatory_breaks() {
    use crate::{
        TextError,
        segment::{Break, LineBoundary, LineUnit, line_breaks},
    };
    assert_eq!(grapheme_boundaries("").collect::<Vec<_>>(), [0]);
    assert_eq!(word_boundaries("").collect::<Vec<_>>(), [0]);
    let mut work = [LineUnit::default(); 8];
    let mut out = [LineBoundary::default(); 9];
    assert_eq!(line_breaks("", &mut [], &mut out), Ok(1));
    assert_eq!(
        out[0],
        LineBoundary {
            byte: 0,
            kind: Break::Mandatory
        }
    );
    assert_eq!(
        line_breaks("A", &mut [], &mut out),
        Err(TextError::BufferTooSmall)
    );
    assert_eq!(
        line_breaks("A", &mut work, &mut []),
        Err(TextError::BufferTooSmall)
    );
    let n = line_breaks("A\r\nB\u{2028}C", &mut work, &mut out).expect("lines");
    assert_eq!(
        out[..n]
            .iter()
            .filter(|b| b.kind == Break::Mandatory)
            .map(|b| b.byte)
            .collect::<Vec<_>>(),
        [3, 7, 8]
    );
    let mut g = grapheme_boundaries("a\u{301}");
    assert_eq!(g.next(), Some(0));
    assert_eq!(g.clone().collect::<Vec<_>>(), [3]);
    assert_eq!(g.next(), Some(3));
    assert_eq!(g.next(), None);
    assert_eq!(g.next(), None);
    let mut w = word_boundaries("a");
    assert_eq!(w.next(), Some(0));
    assert_eq!(w.clone().collect::<Vec<_>>(), [1]);
    assert_eq!(w.next(), Some(1));
    assert_eq!(w.next(), None);
    assert_eq!(w.next(), None);
    assert_eq!(
        TextError::BufferTooSmall.to_string(),
        "text buffer too small"
    );
    assert_eq!(TextError::Overflow.to_string(), "text arithmetic overflow");
}
