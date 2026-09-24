// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#[path = "../../generator/mod.rs"]
mod generator;
use crate::unicode as u;

#[test]
fn unicode_generation_is_byte_identical_and_properties_match_all_code_points() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let model = generator::load(&root.join("../../docs/unicode/ucd")).expect("UCD");
    assert_eq!(
        generator::generate(&model).expect("generate"),
        include_str!("../unicode/generated.rs")
    );
    for property in &model.properties {
        for (code, expected) in property.values.iter().enumerate() {
            let code = u32::try_from(code).expect("code point");
            let actual = property_id(property.name, code);
            let expected = if property.name == "CombiningClass" {
                property.names[usize::from(*expected)].parse().expect("CCC")
            } else {
                *expected
            };
            assert_eq!(actual, expected, "{} U+{code:04X}", property.name);
        }
    }
    let scripts = model
        .properties
        .iter()
        .find(|p| p.name == "Script")
        .expect("scripts");
    let mut previous = 0;
    for (start, end, names) in &model.extensions {
        for code in previous..*start {
            assert!(u::script_extensions(code).is_empty());
        }
        let expected: Vec<_> = names
            .iter()
            .map(|n| {
                u16::try_from(
                    scripts
                        .names
                        .iter()
                        .position(|v| v == n)
                        .expect("script alias"),
                )
                .expect("ID")
            })
            .collect();
        for code in *start..=*end {
            assert_eq!(
                u::script_extensions(code)
                    .iter()
                    .copied()
                    .map(script_id)
                    .collect::<Vec<_>>(),
                expected,
                "U+{code:04X}"
            );
        }
        previous = end.checked_add(1).expect("Unicode end");
    }
    for code in previous..0x11_0000 {
        assert!(u::script_extensions(code).is_empty());
    }
    for (code, mirror) in model.mirrors {
        assert_eq!(u::mirror(code), mirror);
    }
    for (code, pair, kind) in model.brackets {
        assert_eq!(
            u::paired_bracket(code),
            Some((
                pair,
                if kind == "o" {
                    u::BracketKind::Open
                } else {
                    u::BracketKind::Close
                }
            ))
        );
    }
    for (code, sequence) in model.decompositions {
        assert_eq!(u::canonical_decomposition(code), sequence);
    }
    assert_eq!(model.compositions.len(), 961);
    for (first, second, code) in model.compositions {
        assert_eq!(u::composition(first, second, |_| true), Some(code));
    }
    assert_eq!(u::VERSION, "18.0.0");
}
#[expect(
    clippy::as_conversions,
    reason = "generated repr(u16) enums have dense discriminants"
)]
fn property_id(name: &str, code: u32) -> u16 {
    match name {
        "GeneralCategory" => u::general_category(code) as u16,
        "CombiningClass" => u16::from(u::combining_class(code)),
        "GraphemeBreak" => u::grapheme_break(code) as u16,
        "WordBreak" => u::word_break(code) as u16,
        "LineBreak" => u::line_break(code) as u16,
        "EastAsianWidth" => u::east_asian_width(code) as u16,
        "Script" => u::script(code) as u16,
        "BidiClass" => u::bidi_class(code) as u16,
        "IndicConjunctBreak" => u::indic_conjunct_break(code) as u16,
        "DefaultIgnorable" => u::default_ignorable(code) as u16,
        "ExtendedPictographic" => u::extended_pictographic(code) as u16,
        "JoiningType" => u::joining_type(code) as u16,
        _ => panic!("unknown property"),
    }
}
#[test]
fn unicode_defaults_ranges_and_auxiliary_mappings() {
    assert_eq!(u::general_category(0x4e01), u::GeneralCategory::Lo);
    assert_eq!(u::general_category(0xd800), u::GeneralCategory::Cs);
    assert_eq!(u::general_category(0x10_ffff), u::GeneralCategory::Cn);
    assert_eq!(u::general_category(u32::MAX), u::GeneralCategory::Cn);
    assert_eq!(u::bidi_class(0x0590), u::BidiClass::R);
    assert_eq!(u::bidi_class(0x061d), u::BidiClass::Al);
    assert_eq!(u::joining_type(0x0301), u::JoiningType::T);
    assert_eq!(u::joining_type(0x200c), u::JoiningType::U);
    assert_eq!(u::joining_type(0x200d), u::JoiningType::C);
    assert_eq!(
        u::indic_conjunct_break(0x094d),
        u::IndicConjunctBreak::Linker
    );
    assert_eq!(u::canonical_decomposition(0xe9), &[0x65, 0x301]);
    assert!(u::canonical_decomposition(0x41).is_empty());
    assert_eq!(u::mirror(0x28), 0x29);
    assert_eq!(u::mirror(0x41), 0x41);
    assert_eq!(u::paired_bracket(0x28), Some((0x29, u::BracketKind::Open)));
    assert_eq!(u::paired_bracket(0x41), None);
    assert!(u::script_extensions(0x30fc).contains(&u::Script::Hiragana));
    assert!(u::script_extensions(0x30fc).contains(&u::Script::Katakana));
    assert!(u::script_extensions(0x41).is_empty());
}

#[test]
fn composition_honors_uax15_exclusions() {
    assert_eq!(u::composition(0x65, 0x301, |_| true), Some(0xe9));
    assert_eq!(u::composition(0x65, 0x301, |_| false), None);
    assert_eq!(u::composition(0xe9, 0x301, |_| true), None);
    // Listed: U+FB2A, U+0958. Singleton: U+212B. Non-starter: U+0344, U+0F73.
    assert_eq!(u::canonical_decomposition(0xfb2a), &[0x5e9, 0x5c1]);
    assert_eq!(u::composition(0x5e9, 0x5c1, |_| true), None);
    assert_eq!(u::canonical_decomposition(0x958), &[0x915, 0x93c]);
    assert_eq!(u::composition(0x915, 0x93c, |_| true), None);
    assert_eq!(u::canonical_decomposition(0x212b), &[0xc5]);
    assert_eq!(u::composition(0x41, 0x30a, |_| true), Some(0xc5));
    assert_eq!(u::canonical_decomposition(0x344), &[0x308, 0x301]);
    assert_eq!(u::composition(0x308, 0x301, |_| true), None);
    assert_eq!(u::canonical_decomposition(0xf73), &[0xf71, 0xf72]);
    assert_eq!(u::composition(0xf71, 0xf72, |_| true), None);
}

#[expect(clippy::as_conversions, reason = "Script has repr(u16)")]
fn script_id(script: u::Script) -> u16 {
    script as u16
}
