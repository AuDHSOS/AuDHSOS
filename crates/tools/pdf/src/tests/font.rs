// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The fonts, their metrics, and the encoding.

use test_support::generators::range;
use test_support::generators::vec as gen_vec;
use test_support::property::check;

use crate::font::{self, Font, encode, encode_char};
use crate::units::pt;

#[test]
fn every_font_has_its_own_resource_name() {
    let mut names: Vec<&str> = Font::ALL.iter().map(|font| font.resource()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), Font::ALL.len());
}

#[test]
fn the_fixed_pitch_fonts_advance_by_the_same_amount_everywhere() {
    for byte in b' '..=b'~' {
        assert_eq!(Font::Mono.advance(byte), 600);
        assert_eq!(Font::MonoBold.advance(byte), 600);
    }
}

#[test]
fn the_oblique_face_measures_like_the_upright_one() {
    for byte in b' '..=b'~' {
        assert_eq!(Font::Italic.advance(byte), Font::Regular.advance(byte));
    }
}

#[test]
fn bold_is_never_narrower_than_regular_for_a_letter() {
    for byte in b'A'..=b'Z' {
        assert!(Font::Bold.advance(byte) >= Font::Regular.advance(byte));
    }
}

#[test]
fn a_line_of_courier_is_as_wide_as_its_character_count() {
    let width = Font::Mono.width_of_str("abcdefghij", pt(10));
    assert_eq!(width, pt(60));
}

#[test]
fn ascii_encodes_to_itself() {
    assert_eq!(encode("Hello, world"), b"Hello, world".to_vec());
}

#[test]
fn the_punctuation_of_this_repository_survives_encoding() {
    assert_eq!(encode_char('\u{2014}'), 0x97);
    assert_eq!(encode_char('\u{2013}'), 0x96);
    assert_eq!(encode_char('\u{2019}'), 0x92);
    assert_eq!(encode_char('\u{201C}'), 0x93);
    assert_eq!(encode_char('\u{2026}'), 0x85);
    assert_eq!(encode_char('\u{00FC}'), 0xFC);
}

#[test]
fn every_character_the_upper_half_names_has_its_own_byte() {
    // The range between ASCII and Latin-1 is the one an encoding has to
    // be told about, one character at a time. This is that table, read
    // back: every entry, and no two on the same byte.
    let table: [(char, u8); 27] = [
        ('\u{20AC}', 0x80),
        ('\u{201A}', 0x82),
        ('\u{0192}', 0x83),
        ('\u{201E}', 0x84),
        ('\u{2026}', 0x85),
        ('\u{2020}', 0x86),
        ('\u{2021}', 0x87),
        ('\u{02C6}', 0x88),
        ('\u{2030}', 0x89),
        ('\u{0160}', 0x8A),
        ('\u{2039}', 0x8B),
        ('\u{0152}', 0x8C),
        ('\u{017D}', 0x8E),
        ('\u{2018}', 0x91),
        ('\u{2019}', 0x92),
        ('\u{201C}', 0x93),
        ('\u{201D}', 0x94),
        ('\u{2022}', 0x95),
        ('\u{2013}', 0x96),
        ('\u{2014}', 0x97),
        ('\u{02DC}', 0x98),
        ('\u{2122}', 0x99),
        ('\u{0161}', 0x9A),
        ('\u{203A}', 0x9B),
        ('\u{0153}', 0x9C),
        ('\u{017E}', 0x9E),
        ('\u{0178}', 0x9F),
    ];
    let mut bytes: Vec<u8> = Vec::new();
    for (character, byte) in table {
        assert_eq!(encode_char(character), byte, "{character:?}");
        assert!(!bytes.contains(&byte), "two characters on byte {byte:#04x}");
        bytes.push(byte);
        assert!(
            Font::Regular.advance(byte) > 0,
            "byte {byte:#04x} has no width"
        );
        assert!(
            Font::Bold.advance(byte) > 0,
            "byte {byte:#04x} has no bold width"
        );
    }
}

#[test]
fn a_character_the_encoding_has_no_place_for_is_a_question_mark_here() {
    assert_eq!(encode_char('\u{2192}'), b'?');
    assert_eq!(encode_char('\u{2264}'), b'?');
}

#[test]
fn a_no_break_space_is_a_space() {
    assert_eq!(encode_char('\u{00A0}'), 0xA0);
    assert_eq!(Font::Regular.advance(0xA0), Font::Regular.advance(b' '));
}

#[test]
fn a_control_character_has_no_width_and_no_byte_of_its_own() {
    assert_eq!(Font::Regular.advance(0), 0);
    assert_eq!(Font::Mono.advance(0x7F), 0);
    assert_eq!(encode_char('\u{7}'), b'?');
}

#[test]
fn what_the_encoding_cannot_say_is_offered_the_symbol_font_first() {
    let runs = font::runs(Font::Regular, "x \u{2264} \u{221E}");
    assert_eq!(
        runs.iter().map(|run| run.font).collect::<Vec<Font>>(),
        [Font::Regular, Font::Symbol, Font::Regular, Font::Symbol]
    );
    assert_eq!(runs.get(1).map(|run| run.bytes.clone()), Some(vec![0xA3]));
    assert_eq!(runs.get(3).map(|run| run.bytes.clone()), Some(vec![0xA5]));
}

#[test]
fn what_neither_font_has_is_written_as_what_it_is_read_as() {
    let runs = font::runs(Font::Regular, "\u{1D53D}(x) is in \u{2124}");
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(
        runs.first().map(|run| run.bytes.clone()),
        Some(b"F(x) is in Z".to_vec())
    );
}

#[test]
fn a_combining_mark_is_dropped_and_anything_else_is_a_question_mark() {
    assert_eq!(
        font::runs(Font::Regular, "s\u{0323}"),
        vec![font::Run {
            font: Font::Regular,
            bytes: b"s".to_vec(),
        }]
    );
    assert_eq!(
        font::runs(Font::Regular, "\u{AC00}"),
        vec![font::Run {
            font: Font::Regular,
            bytes: b"?".to_vec(),
        }]
    );
}

#[test]
fn a_run_of_one_font_is_one_run() {
    let runs = font::runs(Font::Bold, "\u{2264}\u{2265} ok");
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!(runs.first().map(|run| run.bytes.len()), Some(2));
}

#[test]
fn a_measured_width_counts_what_the_symbol_font_sets() {
    let with = Font::Regular.width_of_str("\u{221E}", pt(10));
    let without = Font::Regular.width_of_str("?", pt(10));
    assert!(with > without, "{with} is not wider than {without}");
    assert_eq!(with, Font::Symbol.width_of(&[0xA5], pt(10)));
}

#[test]
fn only_symbol_carries_an_encoding_of_its_own() {
    assert!(!Font::Symbol.is_winansi());
    assert!(Font::Regular.is_winansi());
    assert_eq!(Font::Symbol.bold(), Font::Symbol);
    assert!(!Font::Symbol.is_mono());
    assert_eq!(Font::Symbol.advance(0xA5), 713);
    assert_eq!(Font::Symbol.advance(0x01), 278);
}

#[test]
fn the_bold_face_of_a_face_is_the_bold_face_of_its_family() {
    assert_eq!(Font::Regular.bold(), Font::Bold);
    assert_eq!(Font::Italic.bold(), Font::Bold);
    assert_eq!(Font::Bold.bold(), Font::Bold);
    assert_eq!(Font::Mono.bold(), Font::MonoBold);
    assert_eq!(Font::MonoBold.bold(), Font::MonoBold);
}

#[test]
fn only_the_fixed_pitch_faces_say_they_are_fixed_pitch() {
    assert!(Font::Mono.is_mono());
    assert!(Font::MonoBold.is_mono());
    assert!(!Font::Regular.is_mono());
    assert!(!Font::Bold.is_mono());
    assert!(!Font::Italic.is_mono());
}

#[test]
fn every_font_names_the_standard_face_it_is() {
    let names: Vec<&str> = Font::ALL.iter().map(|font| font.base_name()).collect();
    assert_eq!(
        names,
        [
            "Helvetica",
            "Helvetica-Bold",
            "Helvetica-Oblique",
            "Courier",
            "Courier-Bold",
            "Symbol"
        ]
    );
}

#[test]
fn already_encoded_text_measures_as_the_text_it_came_from() {
    let text = "a line of prose";
    assert_eq!(
        Font::Regular.width_of(&encode(text), pt(10)),
        Font::Regular.width_of_str(text, pt(10))
    );
    assert_eq!(Font::Regular.width_of(&[], pt(10)), 0);
}

#[test]
fn a_character_the_encoding_has_no_place_for_becomes_a_question_mark() {
    assert_eq!(encode_char('\u{4E2D}'), b'?');
    assert_eq!(encode("\u{1F600}"), b"?".to_vec());
}

#[test]
fn encoding_produces_one_byte_per_character() {
    check(
        "one byte per character",
        &gen_vec(range::<u32>(1..=0x2FFF), 0..=32),
        |points: &Vec<u32>| {
            let text: String = points.iter().filter_map(|p| char::from_u32(*p)).collect();
            let encoded = encode(&text);
            if encoded.len() == text.chars().count() {
                Ok(())
            } else {
                Err(format!(
                    "{} characters became {} bytes",
                    text.chars().count(),
                    encoded.len()
                ))
            }
        },
    );
}

#[test]
fn width_grows_with_the_point_size() {
    let small = Font::Regular.width_of_str("the quick brown fox", pt(9));
    let large = Font::Regular.width_of_str("the quick brown fox", pt(18));
    assert!(large > small);
    assert_eq!(large, small.saturating_mul(2));
}
