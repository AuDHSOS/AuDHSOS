// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::arithmetic_side_effects,
    reason = "bounded synthetic font records"
)]
use super::metrics::{put16, put32, sfnt, tables};
use crate::{
    Fixed, Font, FontSet, Language, Role, TextError, TextStyle, bidi,
    resolve::{self, Run},
    shape::Glyph,
    unicode::Script,
};

pub(super) fn font_for(chars: &[char], upem: u16) -> Vec<u8> {
    let mut tables = tables();
    for (tag, table) in &mut tables {
        if tag == b"head" {
            put16(table, 18, upem);
        }
    }
    let mut cmap = vec![0; 28 + chars.len() * 12];
    put16(&mut cmap, 2, 1);
    put16(&mut cmap, 4, 3);
    put16(&mut cmap, 6, 10);
    put32(&mut cmap, 8, 12);
    put16(&mut cmap, 12, 12);
    put32(&mut cmap, 16, u32::try_from(16 + chars.len() * 12).unwrap());
    put32(&mut cmap, 24, u32::try_from(chars.len()).unwrap());
    let mut codes = chars.to_vec();
    codes.sort_unstable();
    for (i, c) in codes.into_iter().enumerate() {
        put32(&mut cmap, 28 + i * 12, u32::from(c));
        put32(&mut cmap, 32 + i * 12, u32::from(c));
        put32(&mut cmap, 36 + i * 12, 1);
    }
    tables.push((*b"cmap", cmap));
    sfnt(tables)
}
fn runs(set: &FontSet<'_, '_>, style: &TextStyle, text: &str) -> Result<Vec<Run>, TextError> {
    let n = text.chars().count();
    let mut units = vec![bidi::Unit::default(); n];
    let mut seq = vec![0; n];
    bidi::resolve(text, bidi::Direction::Auto, &mut units, &mut seq)?;
    let mut scratch = vec![Glyph::default(); 64];
    let mut output = vec![Run::default(); n];
    let count = resolve::resolve_into(set, style, text, &units, &mut scratch, &mut output)?;
    output.truncate(count);
    Ok(output)
}
#[test]
fn ordered_role_fallback_whole_clusters_and_scales() {
    let first = font_for(&['e', 'A'], 1000);
    let second = font_for(&['é', '中'], 2000);
    let ui = [Font::parse(&first).unwrap(), Font::parse(&second).unwrap()];
    let mono = [ui[1], ui[0]];
    let set = FontSet {
        ui: &ui,
        mono: &mono,
        generation: 42,
    };
    let style = TextStyle::default();
    let result = runs(&set, &style, "Ae\u{301}中").unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(
        result
            .iter()
            .map(|r| (r.start, r.end, r.face, r.script, r.missing))
            .collect::<Vec<_>>(),
        [
            (0, 1, 0, Script::Latin, false),
            (1, 4, 1, Script::Latin, false),
            (4, 7, 1, Script::Han, false)
        ]
    );
    assert_eq!(result[0].scale, style.size.mul_ratio(1, 1000).unwrap());
    assert_eq!(result[1].scale, style.size.mul_ratio(1, 2000).unwrap());
    assert!(
        result
            .iter()
            .all(|r| r.baseline == Fixed::ZERO && r.generation == 42)
    );
    let mono_style = TextStyle {
        role: Role::Mono,
        ..style
    };
    let result = runs(&set, &mono_style, "A中").unwrap();
    assert_eq!((result[0].face, result[1].face), (1, 0));
    let missing = runs(&set, &style, "😀").unwrap();
    assert_eq!((missing[0].face, missing[0].missing), (0, true));
}
#[test]
fn language_han_suitability_and_weight_coordinates() {
    let nominal = font_for(&['骨'], 1000);
    let cjk = include_bytes!("fixtures/NotoCJK-shaping.otf");
    let fonts = [Font::parse(&nominal).unwrap(), Font::parse(cjk).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 7,
    };
    for (lang, tag) in [
        ("ja", *b"JAN "),
        ("ko", *b"KOR "),
        ("zh-Hans", *b"ZHS "),
        ("zh-Hant", *b"ZHT "),
    ] {
        let style = TextStyle {
            lang: Language::parse(lang).unwrap(),
            ..TextStyle::default()
        };
        let output = runs(&set, &style, "骨").unwrap();
        assert_eq!(output[0].face, usize::from(lang.starts_with("zh")));
        assert_eq!(output[0].language.tag(), tag);
    }
    let output = runs(&set, &TextStyle::default(), "骨").unwrap();
    assert_eq!(output[0].face, 0);
    let only = [fonts[0]];
    let set = FontSet {
        ui: &only,
        mono: &only,
        generation: 8,
    };
    let style = TextStyle {
        lang: Language::parse("ja").unwrap(),
        ..TextStyle::default()
    };
    assert_eq!(runs(&set, &style, "骨").unwrap()[0].face, 0);
    let variable = [Font::parse(include_bytes!("fixtures/noto-sans-variable-subset.ttf")).unwrap()];
    let set = FontSet {
        ui: &variable,
        mono: &variable,
        generation: 9,
    };
    let style = TextStyle {
        weight: 900,
        ..TextStyle::default()
    };
    let result = runs(&set, &style, "A").unwrap();
    assert_eq!(result[0].coordinates(), [Fixed::ONE, Fixed::ZERO]);
}
#[test]
fn script_inheritance_bidi_and_language_validation() {
    let font = font_for(&['A', 'א', 'あ', 'ー', ' '], 1000);
    let fonts = [Font::parse(&font).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 0,
    };
    let style = TextStyle::default();
    let result = runs(&set, &style, "  A א あー").unwrap();
    assert_eq!(result[0].script, Script::Latin);
    assert!(
        result
            .iter()
            .any(|r| r.script == Script::Hebrew && r.level & 1 != 0)
    );
    assert_eq!(result.last().unwrap().script, Script::Hiragana);
    let refused = runs(&set, &style, "क").unwrap();
    assert!(refused[0].simple);
    assert_eq!(Language::parse("zh-tw").unwrap().tag(), *b"ZHT ");
    assert_eq!(Language::parse("zh-hans-TW").unwrap().tag(), *b"ZHS ");
    assert_eq!(Language::parse("xx-XX").unwrap(), Language::UND);
    for bad in ["", "a", "en-", "en_Us", "éé", "en-123456789"] {
        assert!(Language::parse(bad).is_err());
    }
    assert!(Language::from_opentype([0; 4]).is_err());
    assert_eq!(
        Language::from_opentype(*b"JAN ").unwrap(),
        Language::parse("ja").unwrap()
    );
}
#[test]
fn resolution_bad_inputs_and_capacities() {
    let data = font_for(&['A'], 1000);
    let fonts = [Font::parse(&data).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &[],
        generation: 0,
    };
    let style = TextStyle::default();
    assert_eq!(
        runs(
            &set,
            &TextStyle {
                role: Role::Mono,
                ..style
            },
            "A"
        ),
        Err(TextError::NoFont)
    );
    assert_eq!(
        runs(
            &set,
            &TextStyle {
                size: Fixed::ZERO,
                ..style
            },
            "A"
        ),
        Err(TextError::InvalidInput)
    );
    assert!(runs(&set, &TextStyle { weight: 0, ..style }, "A").is_err());
    let mut units = [bidi::Unit::default(); 1];
    bidi::resolve("A", bidi::Direction::Auto, &mut units, &mut [0]).unwrap();
    assert_eq!(
        resolve::resolve_into(&set, &style, "A", &[], &mut [], &mut []),
        Err(TextError::InvalidInput)
    );
    assert_eq!(
        resolve::resolve_into(
            &set,
            &style,
            "A",
            &units,
            &mut [Glyph::default(); 1],
            &mut []
        ),
        Err(TextError::BufferTooSmall)
    );
    assert!(
        resolve::resolve_into(&set, &style, "A", &units, &mut [], &mut [Run::default(); 1])
            .is_err()
    );
    assert_eq!(runs(&set, &style, "").unwrap(), []);
    assert_eq!(TextError::NoFont.to_string(), "empty font chain");
    assert_eq!(
        TextError::Font(crate::FontError::Truncated).to_string(),
        crate::FontError::Truncated.to_string()
    );
}

#[test]
fn variation_sequence_coverage_precedes_plain_base_fallback() {
    let plain = font_for(&['A'], 1000);
    let mut cmap = vec![0; 78];
    put16(&mut cmap, 2, 2);
    put16(&mut cmap, 6, 5);
    put32(&mut cmap, 8, 48);
    put16(&mut cmap, 12, 3);
    put16(&mut cmap, 14, 10);
    put32(&mut cmap, 16, 20);
    put16(&mut cmap, 20, 12);
    put32(&mut cmap, 24, 28);
    put32(&mut cmap, 32, 1);
    put32(&mut cmap, 36, 65);
    put32(&mut cmap, 40, 65);
    put32(&mut cmap, 44, 1);
    put16(&mut cmap, 48, 14);
    put32(&mut cmap, 50, 30);
    put32(&mut cmap, 54, 1);
    cmap[58..61].copy_from_slice(&[0, 254, 15]);
    put32(&mut cmap, 65, 21);
    put32(&mut cmap, 69, 1);
    cmap[73..76].copy_from_slice(&[0, 0, 65]);
    put16(&mut cmap, 76, 2);
    let mut tables = tables();
    tables.push((*b"cmap", cmap));
    let varied = sfnt(tables);
    let fonts = [Font::parse(&plain).unwrap(), Font::parse(&varied).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 0,
    };
    let output = runs(&set, &TextStyle::default(), "A\u{fe0f}").unwrap();
    assert_eq!((output[0].face, output[0].missing), (1, false));
    let only = [fonts[0]];
    let set = FontSet {
        ui: &only,
        mono: &only,
        generation: 0,
    };
    assert!(runs(&set, &TextStyle::default(), "A\u{fe0f}").unwrap()[0].missing);
    assert_eq!(Language::parse("zh-HK").unwrap().tag(), *b"ZHH ");
    assert_eq!(Language::parse("zh-Hans-HK").unwrap().tag(), *b"ZHS ");
}

#[test]
fn language_extensions_do_not_select_han_regions() {
    for tag in ["zh-x-HK-Hant", "zh-u-Hant", "zh-a-HK-x-Hant"] {
        assert_eq!(Language::parse(tag).unwrap().tag(), *b"ZHS ");
    }
    for tag in ["x-Hant", "i-klingon", "en-GB-oed", "zh-yue"] {
        assert_eq!(Language::parse(tag).unwrap(), Language::UND);
    }
    for tag in ["zh-Hant-TW-x-Hans", "zh-Hant-u-ca-chinese"] {
        assert_eq!(Language::parse(tag).unwrap().tag(), *b"ZHT ");
    }
    for tag in [
        "x",
        "zh-x",
        "zh-u",
        "zh-Hant-Hans",
        "en-US-GB",
        "en-a-aa-A-bb",
        "en-1901-1901",
        "en-abcde-ABCDE",
        "en-12",
        "en-1234-ab",
    ] {
        assert_eq!(Language::parse(tag), Err(TextError::InvalidInput), "{tag}");
    }
    assert!(Language::parse(&"a".repeat(256)).is_err());
    assert_eq!(
        Language::parse("en-Latn-001-1901-a-foo-x-HK")
            .unwrap()
            .tag(),
        *b"ENG "
    );
}
