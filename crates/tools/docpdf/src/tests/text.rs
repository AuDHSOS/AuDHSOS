// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Breaking inline content into lines.

use doc_markdown::Inline;
use doc_pdf::font::Font;
use doc_pdf::units::pt;
use test_support::generators::{pair, range, vec as gen_vec};
use test_support::property::check;

use crate::text::{Style, extent, lines};
use crate::theme;

fn body() -> Style {
    Style::new(Font::Regular, theme::BODY.size, theme::INK)
}

fn words(text: &str) -> Vec<Inline> {
    vec![Inline::Text(text.to_owned())]
}

fn rendered(content: &[Inline], measure: i64) -> Vec<String> {
    lines(content, &body(), measure)
        .into_iter()
        .map(|line| {
            line.pieces
                .into_iter()
                .map(|piece| piece.text)
                .collect::<String>()
        })
        .collect()
}

#[test]
fn a_line_that_fits_stays_one_line() {
    assert_eq!(rendered(&words("a short line"), pt(400)), ["a short line"]);
}

#[test]
fn a_line_that_does_not_fit_is_broken_at_a_space() {
    let broken = rendered(&words("one two three four five six seven eight"), pt(60));
    assert!(broken.len() > 1);
    for line in &broken {
        assert!(!line.starts_with(' '), "`{line}` starts with a space");
        assert!(!line.ends_with(' '), "`{line}` ends with a space");
    }
    assert_eq!(broken.join(" "), "one two three four five six seven eight");
}

#[test]
fn no_line_is_wider_than_the_measure_it_was_given() {
    let measure = pt(120);
    let text = "the quick brown fox jumps over the lazy dog again and again and again";
    for line in lines(&words(text), &body(), measure) {
        assert!(
            line.width <= measure,
            "a line came out {} wide, in a measure of {measure}",
            line.width
        );
    }
}

#[test]
fn a_word_wider_than_the_measure_is_cut_rather_than_left_hanging() {
    let measure = pt(40);
    let long = "crates/kernel/hal-x86_64/src/interrupts/dispatch.rs";
    let broken = lines(&words(long), &body(), measure);
    assert!(broken.len() > 1);
    for line in &broken {
        assert!(line.width <= measure);
    }
    let joined: String = broken
        .iter()
        .flat_map(|line| line.pieces.iter())
        .map(|piece| piece.text.clone())
        .collect();
    assert_eq!(joined, long);
}

#[test]
fn a_break_the_author_asked_for_is_kept() {
    let content = vec![
        Inline::Text("one".to_owned()),
        Inline::Break,
        Inline::Text("two".to_owned()),
    ];
    assert_eq!(rendered(&content, pt(400)), ["one", "two"]);
}

#[test]
fn a_code_span_is_set_in_the_fixed_pitch_face() {
    let content = vec![
        Inline::Text("call ".to_owned()),
        Inline::Code("send".to_owned()),
    ];
    let broken = lines(&content, &body(), pt(400));
    let faces: Vec<Font> = broken
        .iter()
        .flat_map(|line| line.pieces.iter())
        .map(|piece| piece.style.font)
        .collect();
    assert!(faces.contains(&Font::Mono));
    assert!(faces.contains(&Font::Regular));
}

#[test]
fn a_link_carries_its_target_down_to_the_piece() {
    let content = vec![Inline::Link {
        content: words("the docs"),
        href: "docs/README.md".to_owned(),
    }];
    let broken = lines(&content, &body(), pt(400));
    let targets: Vec<Option<String>> = broken
        .iter()
        .flat_map(|line| line.pieces.iter())
        .map(|piece| piece.style.href.clone())
        .collect();
    assert!(targets.iter().all(Option::is_some));
}

#[test]
fn strong_inside_a_link_is_both_bold_and_a_link() {
    let content = vec![Inline::Link {
        content: vec![Inline::Strong(words("here"))],
        href: "x.md".to_owned(),
    }];
    let broken = lines(&content, &body(), pt(400));
    let piece = broken
        .first()
        .and_then(|line| line.pieces.first())
        .map(|piece| (piece.style.font, piece.style.href.clone()));
    assert_eq!(piece, Some((Font::Bold, Some("x.md".to_owned()))));
}

#[test]
fn the_extent_of_a_run_is_its_unbroken_width_and_its_widest_word() {
    let content = words("aa bbbb c");
    let (natural, longest) = extent(&content, &body());
    assert!(natural > longest);
    assert_eq!(longest, body().width("bbbb"));
}

#[test]
fn the_extent_measures_a_code_span_in_the_face_it_will_be_set_in() {
    let prose = extent(&words("notification"), &body()).1;
    let code = extent(&[Inline::Code("notification".to_owned())], &body()).1;
    assert_ne!(prose, code, "a code span is not measured as prose");
}

#[test]
fn breaking_never_loses_a_character_and_never_overruns_the_measure() {
    // The two things a line breaker has to get right, over words of every
    // length against measures of every width. The measure is kept above
    // the width of a single character, below which no breaker can promise
    // anything.
    check(
        "the breaker keeps its promises",
        &pair(gen_vec(range::<u8>(1..=12), 1..=24), range::<i64>(20..=400)),
        |(lengths, points): &(Vec<u8>, i64)| {
            let text: Vec<String> = lengths
                .iter()
                .map(|length| "m".repeat(usize::from(*length)))
                .collect();
            let source = text.join(" ");
            let measure = pt(*points);
            let broken = lines(&words(&source), &body(), measure);
            for line in &broken {
                if line.width > measure {
                    return Err(format!(
                        "a line of {} is wider than the measure of {measure}",
                        line.width
                    ));
                }
            }
            let kept: String = broken
                .iter()
                .flat_map(|line| line.pieces.iter())
                .map(|piece| piece.text.replace(' ', ""))
                .collect();
            let wanted = source.replace(' ', "");
            if kept == wanted {
                Ok(())
            } else {
                Err(format!("{} characters became {}", wanted.len(), kept.len()))
            }
        },
    );
}
