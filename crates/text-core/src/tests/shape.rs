// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::arithmetic_side_effects,
    reason = "small literal table fixtures"
)]

use crate::{
    Fixed, FontError,
    shape::{self, Budget, Buffer, Feature, Gdef, Glyph, LayoutTable},
};

pub(super) struct Bin(pub(super) Vec<u8>);
impl Bin {
    pub(super) fn words(words: &[u16]) -> Self {
        Self(words.iter().flat_map(|v| v.to_be_bytes()).collect())
    }
    pub(super) fn set(&mut self, at: usize, value: usize) {
        self.0[at..at + 2].copy_from_slice(&u16::try_from(value).expect("offset").to_be_bytes());
    }
    pub(super) fn child(&mut self, at: usize, child: Self) {
        self.set(at, self.0.len());
        self.0.extend(child.0);
    }
}
pub(super) fn coverage(glyphs: &[u16]) -> Bin {
    let mut b = Bin::words(&[1, u16::try_from(glyphs.len()).expect("count")]);
    b.0.extend(glyphs.iter().flat_map(|v| v.to_be_bytes()));
    b
}
fn class() -> Bin {
    Bin::words(&[1, 1, 9, 1, 2, 3, 4, 5, 6, 7, 8, 9])
}
fn single(glyph: u16, target: u16) -> Bin {
    let mut b = Bin::words(&[2, 0, 1, target]);
    b.child(2, coverage(&[glyph]));
    b
}
pub(super) fn lookup(kind: u16, flags: u16, table: Bin) -> Bin {
    let mut b = Bin::words(&[kind, flags, 1, 0]);
    b.child(6, table);
    b
}
pub(super) fn layout(lookups: Vec<Bin>) -> Bin {
    let mut list = Bin::words(&[u16::try_from(lookups.len()).expect("count")]);
    list.0.resize(2 + lookups.len() * 2, 0);
    for (i, l) in lookups.into_iter().enumerate() {
        list.child(2 + i * 2, l);
    }
    let mut b = Bin::words(&[1, 0, 0, 0, 0]);
    b.child(4, Bin::words(&[0]));
    b.child(6, Bin::words(&[0]));
    b.child(8, list);
    b
}
fn execute(data: &[u8], sub: bool, ids: &[u16]) -> Result<Vec<Glyph>, FontError> {
    let mut glyphs = vec![Glyph::default(); 64];
    for (i, id) in ids.iter().enumerate() {
        glyphs[i] = Glyph::new(*id, i, i + 1);
        glyphs[i].class = if *id == 7 { 3 } else { 1 };
        glyphs[i].advance = Fixed::from_i32(100);
    }
    let mut buffer = Buffer::new(&mut glyphs, ids.len())?;
    LayoutTable::parse(data, sub)?.apply_lookup(0, Gdef::default(), &[], false, &mut buffer)?;
    if !sub {
        shape::finish(&mut buffer, false)?;
    }
    Ok(buffer.glyphs().to_vec())
}
fn ids(glyphs: &[Glyph]) -> Vec<u16> {
    glyphs.iter().map(|g| g.id).collect()
}

#[test]
fn gsub_basic_extension_reverse_and_ligatures() {
    let mut delta = Bin::words(&[1, 0, 65535]);
    delta.child(2, coverage(&[1]));
    let b = layout(vec![lookup(1, 0, delta)]);
    assert_eq!(ids(&execute(&b.0, true, &[1, 2]).unwrap()), [0, 2]);
    let b = layout(vec![lookup(1, 0, single(1, 42))]);
    assert_eq!(ids(&execute(&b.0, true, &[1, 2]).unwrap()), [42, 2]);
    for (kind, sequence, expected) in [
        (2, vec![3, 4], vec![3, 4, 2]),
        (2, vec![], vec![2]),
        (3, vec![5, 6], vec![5, 2]),
    ] {
        let mut t = Bin::words(&[1, 0, 1, 0]);
        t.child(2, coverage(&[1]));
        let mut s = Bin::words(&[u16::try_from(sequence.len()).unwrap()]);
        s.0.extend(sequence.iter().flat_map(|v: &u16| v.to_be_bytes()));
        t.child(6, s);
        let b = layout(vec![lookup(kind, 0, t)]);
        assert_eq!(ids(&execute(&b.0, true, &[1, 2]).unwrap()), expected);
    }
    let mut lig = Bin::words(&[1, 0, 1, 0]);
    lig.child(2, coverage(&[1]));
    let mut set = Bin::words(&[1, 0]);
    set.child(2, Bin::words(&[42, 2, 2]));
    lig.child(6, set);
    let b = layout(vec![lookup(4, 8, lig)]);
    let result = execute(&b.0, true, &[1, 7, 2]).unwrap();
    assert_eq!(ids(&result), [42, 7]);
    assert_eq!(
        (result[0].start, result[0].end, result[1].component),
        (0, 3, 0)
    );
    let mut ext = Bin::words(&[1, 1, 0, 8]);
    ext.0.extend(single(1, 42).0);
    let b = layout(vec![lookup(7, 0, ext)]);
    assert_eq!(ids(&execute(&b.0, true, &[1]).unwrap()), [42]);
    let mut reverse = Bin::words(&[1, 0, 1, 0, 1, 0, 1, 42]);
    reverse.child(2, coverage(&[1]));
    reverse.child(6, coverage(&[9]));
    reverse.child(10, coverage(&[3]));
    let b = layout(vec![lookup(8, 0, reverse)]);
    assert_eq!(ids(&execute(&b.0, true, &[9, 1, 3]).unwrap()), [9, 42, 3]);
}
fn context(format: u16, chain: bool, nested: u16) -> Bin {
    if format == 3 {
        let mut t = if chain {
            Bin::words(&[3, 1, 0, 2, 0, 0, 1, 0, 1, 1, nested])
        } else {
            Bin::words(&[3, 2, 1, 0, 0, 1, nested])
        };
        if chain {
            t.child(4, coverage(&[9]));
            t.child(8, coverage(&[1]));
            t.child(10, coverage(&[2]));
            t.child(14, coverage(&[3]));
        } else {
            t.child(6, coverage(&[1]));
            t.child(8, coverage(&[2]));
        }
        return t;
    }
    let mut rule = if chain {
        Bin::words(&[1, 9, 2, 2, 1, 3, 1, 1, nested])
    } else {
        Bin::words(&[2, 1, 2, 1, nested])
    };
    let mut set = Bin::words(&[1, 0]);
    set.child(2, Bin(core::mem::take(&mut rule.0)));
    let mut t = match (format, chain) {
        (1, _) => Bin::words(&[1, 0, 1, 0]),
        (2, false) => Bin::words(&[2, 0, 0, 3, 0, 0, 0]),
        _ => Bin::words(&[2, 0, 0, 0, 0, 3, 0, 0, 0]),
    };
    t.child(2, coverage(&[1]));
    if format == 1 {
        t.child(6, set);
    } else if chain {
        for at in [4, 6, 8] {
            t.child(at, class());
        }
        t.child(14, set);
    } else {
        t.child(4, class());
        t.child(10, set);
    }
    t
}
#[test]
fn every_context_format_for_substitution_and_positioning() {
    for sub in [true, false] {
        for chain in [false, true] {
            for format in 1..=3 {
                let kind = if sub {
                    if chain { 6 } else { 5 }
                } else if chain {
                    8
                } else {
                    7
                };
                let action = if sub {
                    single(2, 42)
                } else {
                    let mut b = Bin::words(&[1, 0, 4, 65526]);
                    b.child(2, coverage(&[2]));
                    b
                };
                let b = layout(vec![
                    lookup(kind, 0, context(format, chain, 1)),
                    lookup(1, 0, action),
                ]);
                let input = if chain {
                    vec![9, 1, 2, 3]
                } else {
                    vec![1, 2, 3]
                };
                let at = if chain { 2 } else { 1 };
                let result = execute(&b.0, sub, &input).unwrap();
                if sub {
                    assert_eq!(result[at].id, 42, "format {format} chain {chain}");
                } else {
                    assert_eq!(result[at].advance, Fixed::from_i32(90));
                }
            }
        }
    }
}
#[test]
fn gpos_single_pair_extension_and_device() {
    for format in [1, 2] {
        let mut t = if format == 1 {
            Bin::words(&[1, 0, 7, 3, 4, 65534])
        } else {
            Bin::words(&[2, 0, 7, 1, 3, 4, 65534])
        };
        t.child(2, coverage(&[1]));
        let b = layout(vec![lookup(1, 0, t)]);
        let r = execute(&b.0, false, &[1]).unwrap();
        assert_eq!(
            (r[0].x, r[0].y, r[0].advance),
            (Fixed::from_i32(3), Fixed::from_i32(4), Fixed::from_i32(98))
        );
    }
    for format in [1, 2] {
        let mut t = if format == 1 {
            Bin::words(&[1, 0, 4, 0, 1, 0])
        } else {
            Bin::words(&[2, 0, 4, 0, 0, 0, 2, 3, 0, 0, 0, 0, 0, 65526])
        };
        t.child(2, coverage(&[1]));
        if format == 1 {
            t.child(10, Bin::words(&[1, 2, 65526]));
        } else {
            t.child(8, class());
            t.child(10, class());
        }
        let b = layout(vec![lookup(2, 0, t)]);
        let r = execute(&b.0, false, &[1, 2]).unwrap();
        assert_eq!(r[0].advance, Fixed::from_i32(90));
        assert_eq!(r[1].advance, Fixed::from_i32(100));
    }
    let mut t = Bin::words(&[1, 0, 0x11, 5, 0]);
    t.child(2, coverage(&[1]));
    t.child(8, Bin::words(&[9, 9, 1, 0xc000]));
    let mut ext = Bin::words(&[1, 1, 0, 8]);
    ext.0.extend(t.0);
    let b = layout(vec![lookup(9, 0, ext)]);
    assert_eq!(execute(&b.0, false, &[1]).unwrap()[0].x, Fixed::from_i32(5));
}
fn anchor(x: u16, y: u16) -> Bin {
    Bin::words(&[1, x, y])
}
fn mark(kind: u16) -> Bin {
    let mut t = Bin::words(&[1, 0, 0, 1, 0, 0]);
    t.child(2, coverage(&[7]));
    t.child(4, coverage(&[1]));
    let mut marks = Bin::words(&[1, 0, 0]);
    marks.child(4, anchor(10, 20));
    t.child(8, marks);
    let mut bases = Bin::words(&[1, 0]);
    if kind == 5 {
        let mut lig = Bin::words(&[1, 0]);
        lig.child(2, anchor(40, 70));
        bases.child(2, lig);
    } else {
        bases.child(2, anchor(40, 70));
    }
    t.child(10, bases);
    t
}
#[test]
fn mark_base_ligature_mark_and_cursive_chains() {
    for kind in 4..=6 {
        let b = layout(vec![lookup(kind, 0, mark(kind))]);
        let r = execute(&b.0, false, &[1, 7]).unwrap();
        assert_eq!(
            (r[1].x, r[1].y),
            (Fixed::from_i32(-70), Fixed::from_i32(50))
        );
    }
    for flags in [0, 1] {
        let mut t = Bin::words(&[1, 0, 3, 0, 0, 0, 0, 0, 0]);
        t.child(2, coverage(&[1, 2, 3]));
        t.child(8, anchor(80, 20));
        t.child(10, anchor(10, 5));
        t.child(12, anchor(80, 30));
        t.child(14, anchor(10, 5));
        let b = layout(vec![lookup(3, flags, t)]);
        let r = execute(&b.0, false, &[1, 2, 3]).unwrap();
        let expected = if flags == 0 {
            [0, 15, 40]
        } else {
            [-40, -25, 0]
        };
        assert_eq!(
            r.iter().map(|g| g.y).collect::<Vec<_>>(),
            expected.map(Fixed::from_i32)
        );
        assert_eq!(r[0].advance, Fixed::from_i32(80));
        assert_eq!(r[1].advance, Fixed::from_i32(70));
    }
}
#[test]
fn malformed_layout_truncation_cycles_and_capacity() {
    let b = layout(vec![lookup(5, 0, context(3, false, 0))]);
    assert_eq!(execute(&b.0, true, &[1, 2]), Err(FontError::Cycle));
    let good = layout(vec![lookup(1, 0, single(1, 42))]);
    for end in 0..good.0.len() {
        assert!(execute(&good.0[..end], true, &[1]).is_err(), "prefix {end}");
    }
    for i in 0..good.0.len() {
        for value in [0, 1, 127, 255] {
            let mut copy = good.0.clone();
            copy[i] = value;
            let a = execute(&copy, true, &[1, 2]);
            let z = execute(&copy.clone(), true, &[1, 2]);
            assert_eq!(a, z);
        }
    }
    let mut data = [];
    assert!(Buffer::new(&mut data, 1).is_err());
}

fn shape_font(
    data: &[u8],
    text: &str,
    script: crate::unicode::Script,
    rtl: bool,
    language: [u8; 4],
) -> Result<Vec<Glyph>, FontError> {
    let font = crate::Font::parse(data)?;
    let metrics = font.metrics()?;
    let gdef = font
        .table(*b"GDEF")
        .map(|t| Gdef::parse(t.data, 0))
        .transpose()?
        .unwrap_or_default();
    let mut glyphs = vec![Glyph::default(); 256];
    let n = shape::prepare(text, font.cmap()?, script, rtl, &mut glyphs)?;
    let mut buffer = Buffer::new(&mut glyphs, n)?;
    if let Some(t) = font.table(*b"GSUB") {
        LayoutTable::parse(t.data, true)?.apply(
            shape::script_tag(script),
            language,
            shape::SUBSTITUTION_FEATURES,
            gdef,
            &[],
            rtl,
            &mut buffer,
            &mut Budget::default(),
        )?;
    }
    buffer.set_advances(|id| Ok(Fixed::from_i32(i32::from(metrics.horizontal(id)?.0))))?;
    if let Some(t) = font.table(*b"GPOS") {
        LayoutTable::parse(t.data, false)?.apply(
            shape::script_tag(script),
            language,
            shape::POSITION_FEATURES,
            gdef,
            &[],
            rtl,
            &mut buffer,
            &mut Budget::default(),
        )?;
    }
    shape::finish(&mut buffer, rtl)?;
    let mut result = buffer.glyphs().to_vec();
    if rtl {
        result.reverse();
    }
    Ok(result)
}
#[test]
fn real_font_shaping_oracles() {
    use crate::unicode::Script;
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tests/fixtures/DejaVu-shaping.ttf"
    ))
    .unwrap();
    for (text, script, rtl, lang) in [
        ("officeAVé", Script::Latin, false, *b"ENG "),
        ("سلام", Script::Arabic, true, *b"ARA "),
        ("بَت", Script::Arabic, true, *b"ARA "),
        ("שָׁלוֹם", Script::Hebrew, true, *b"IWR "),
        ("άЙ", Script::Greek, false, *b"ELL "),
    ] {
        let output = shape_font(&data, text, script, rtl, lang).unwrap();
        let expected: &[(u16, i32, i32, i32)] = match text {
            "officeAVé" => &[
                (10, 1253, 0, 0),
                (43, 1980, 0, 0),
                (6, 1126, 0, 0),
                (7, 1260, 0, 0),
                (4, 1270, 0, 0),
                (5, 1242, 0, 0),
                (12, 1260, 0, 0),
            ],
            "سلام" => &[(30, 1268, 0, 0), (64, 1222, 0, 0), (55, 1716, 0, 0)],
            "بَت" => &[(51, 2011, 0, 0), (31, 0, -213, -200), (49, 570, 0, 0)],
            "שָׁלוֹם" => &[
                (23, 1359, 0, 0),
                (19, 0, 0, 0),
                (21, 558, 0, 0),
                (22, 1164, 0, 0),
                (18, 0, 0, 0),
                (20, 0, 0, 0),
                (24, 1451, 0, 0),
            ],
            _ => &[(14, 1350, 0, 0), (17, 1532, 0, 0)],
        };
        assert_eq!(
            output
                .iter()
                .map(|g| (g.id, g.advance, g.x, g.y))
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|(id, a, x, y)| (
                    *id,
                    Fixed::from_i32(*a),
                    Fixed::from_i32(*x),
                    Fixed::from_i32(*y)
                ))
                .collect::<Vec<_>>(),
            "{text}"
        );
    }
}

pub(super) fn with_features(mut b: Bin, required: u16) -> Bin {
    let mut script = Bin::words(&[0, 1, 0x4a41, 0x4e20, 0]);
    script.child(0, Bin::words(&[0, required, 1, 0]));
    script.child(8, Bin::words(&[0, 65535, 1, 1]));
    let mut scripts = Bin::words(&[1, 0x4446, 0x4c54, 0]);
    scripts.child(6, script);
    b.child(4, scripts);
    let mut features = Bin::words(&[2, 0x7465, 0x7374, 0, 0x7465, 0x7374, 0]);
    features.child(6, Bin::words(&[0, 1, 0]));
    features.child(12, Bin::words(&[0, 1, 1]));
    b.child(6, features);
    b
}
#[test]
fn language_required_and_variation_feature_selection() {
    let base = layout(vec![lookup(1, 0, single(1, 2)), lookup(1, 0, single(1, 3))]);
    let b = with_features(base, 65535);
    let apply = |data: &[u8], language, features: &[Feature], coords: &[Fixed]| {
        let mut glyphs = [Glyph::new(1, 0, 1)];
        let mut buffer = Buffer::new(&mut glyphs, 1).unwrap();
        let table = LayoutTable::parse(data, true).unwrap();
        table
            .apply(
                *b"xxxx",
                language,
                features,
                Gdef::default(),
                coords,
                false,
                &mut buffer,
                &mut Budget::default(),
            )
            .unwrap();
        buffer.glyphs()[0].id
    };
    let features = [Feature {
        tag: *b"test",
        value: 1,
    }];
    assert_eq!(apply(&b.0, *b"ENG ", &features, &[]), 2);
    assert_eq!(apply(&b.0, *b"JAN ", &features, &[]), 3);
    let t = LayoutTable::parse(&b.0, true).unwrap();
    assert!(t.supports_language(*b"DFLT", *b"JAN ").unwrap());
    assert!(!t.supports_language(*b"DFLT", *b"ENG ").unwrap());
    assert!(!t.supports_language(*b"latn", *b"JAN ").unwrap());
    let required = with_features(
        layout(vec![lookup(1, 0, single(1, 2)), lookup(1, 0, single(1, 3))]),
        0,
    );
    assert_eq!(apply(&required.0, *b"ENG ", &[], &[]), 2);
    let mut variable = Bin::words(&[1, 1, 0, 0, 0, 0, 0]);
    for offset in [4, 6, 8] {
        let at = usize::from(u16::from_be_bytes([b.0[offset], b.0[offset + 1]]));
        variable.child(offset, Bin(b.0[at..].to_vec()));
    }
    let mut variations = Bin::words(&[1, 0, 0, 1, 0, 0, 0, 0]);
    let mut conditions = Bin::words(&[1, 0, 0]);
    conditions.child(4, Bin::words(&[1, 0, 8192, 16384]));
    variations.child(10, conditions);
    let mut substitute = Bin::words(&[1, 0, 1, 0, 0, 0]);
    substitute.child(10, Bin::words(&[0, 1, 1]));
    variations.child(14, substitute);
    variable.child(12, variations);
    assert_eq!(apply(&variable.0, *b"ENG ", &features, &[Fixed::ZERO]), 2);
    assert_eq!(apply(&variable.0, *b"ENG ", &features, &[Fixed::ONE]), 3);
}
#[test]
fn gdef_classes_mark_filters_carets_and_variation_devices() {
    use shape::Caret;
    let mut d = Bin::words(&[1, 3, 0, 0, 0, 0, 0, 0, 0]);
    d.child(4, Bin::words(&[2, 3, 1, 1, 1, 2, 2, 3, 3, 3, 2]));
    d.child(10, Bin::words(&[1, 2, 1, 1]));
    let mut sets = Bin::words(&[1, 1, 0, 0]);
    sets.child(6, coverage(&[2]));
    d.child(12, sets);
    let mut carets = Bin::words(&[0, 1, 0]);
    carets.child(0, coverage(&[3]));
    let mut lig = Bin::words(&[3, 0, 0, 0]);
    lig.child(2, Bin::words(&[1, 9]));
    lig.child(4, Bin::words(&[2, 4]));
    let mut varied = Bin::words(&[3, 10, 0]);
    varied.child(4, Bin::words(&[0, 0, 0x8000]));
    lig.child(6, varied);
    carets.child(4, lig);
    d.child(8, carets);
    let mut attach = Bin::words(&[0, 1, 0]);
    attach.child(0, coverage(&[1]));
    attach.child(4, Bin::words(&[2, 3, 7]));
    d.child(6, attach);
    d.child(
        16,
        Bin::words(&[1, 0, 22, 1, 0, 12, 1, 1, 1, 0, 40, 1, 1, 0, 16384, 16384]),
    );
    let gdef = Gdef::parse(&d.0, 1).unwrap();
    assert_eq!(
        (gdef.class(1), gdef.class(2), gdef.class(3), gdef.class(9)),
        (Ok(1), Ok(3), Ok(2), Ok(0))
    );
    let mut points = [0; 2];
    assert_eq!(gdef.attachment_points(1, &mut points), Ok(2));
    assert_eq!(points, [3, 7]);
    assert_eq!(gdef.attachment_points(9, &mut []), Ok(0));
    assert!(gdef.attachment_points(1, &mut []).is_err());
    assert_eq!(Gdef::default().attachment_points(1, &mut []), Ok(0));
    let mut out = [Caret::Point(0); 3];
    assert_eq!(gdef.carets(3, &[Fixed::ONE], &mut out), Ok(3));
    assert_eq!(
        out,
        [
            Caret::Coordinate(Fixed::from_i32(9)),
            Caret::Point(4),
            Caret::Coordinate(Fixed::from_i32(50))
        ]
    );
    assert!(gdef.carets(3, &[], &mut []).is_err());
    assert_eq!(gdef.carets(9, &[], &mut []), Ok(0));
    for (flags, expected) in [(0, 42), (8, 2), (0x100, 42), (0x200, 2), (16, 42)] {
        let mut l = lookup(1, flags, single(2, 42));
        if flags == 16 {
            l.0.splice(8..8, [0, 0]);
            l.set(6, 10);
        }
        let b = layout(vec![l]);
        let mut glyphs = [Glyph::new(2, 0, 1)];
        let mut buffer = Buffer::new(&mut glyphs, 1).unwrap();
        LayoutTable::parse(&b.0, true)
            .unwrap()
            .apply_lookup(0, gdef, &[Fixed::ONE], false, &mut buffer)
            .unwrap();
        assert_eq!(buffer.glyphs()[0].id, expected);
    }
    let mut pos = Bin::words(&[1, 0, 0x10, 0]);
    pos.child(2, coverage(&[1]));
    pos.child(6, Bin::words(&[0, 0, 0x8000]));
    let b = layout(vec![lookup(1, 0, pos)]);
    let mut glyphs = [Glyph::new(1, 0, 1)];
    let mut buffer = Buffer::new(&mut glyphs, 1).unwrap();
    LayoutTable::parse(&b.0, false)
        .unwrap()
        .apply_lookup(0, gdef, &[Fixed::ONE], false, &mut buffer)
        .unwrap();
    assert_eq!(buffer.glyphs()[0].x, Fixed::from_i32(40));
}

#[test]
fn hangul_kana_and_han_language_oracles() {
    use crate::unicode::Script;
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tests/fixtures/NotoCJK-shaping.otf"
    ))
    .unwrap();
    for (text, script, language, expected) in [
        ("가각", Script::Hangul, *b"KOR ", vec![(26, 920), (27, 920)]),
        (
            "\u{1100}\u{1161}\u{11a8}",
            Script::Hangul,
            *b"KOR ",
            vec![(27, 920)],
        ),
        (
            "ᄓᅢᇇ",
            Script::Hangul,
            *b"KOR ",
            vec![(32, 920), (36, 0), (42, 0)],
        ),
        ("가〮", Script::Hangul, *b"KOR ", vec![(8, 250), (26, 920)]),
        (
            "あア",
            Script::Hiragana,
            *b"JAN ",
            vec![(10, 1000), (11, 1000)],
        ),
        ("骨", Script::Han, *b"JAN ", vec![(23, 1000)]),
        ("骨", Script::Han, *b"ZHS ", vec![(24, 1000)]),
        ("骨", Script::Han, *b"ZHT ", vec![(25, 1000)]),
        ("骨", Script::Han, *b"KOR ", vec![(23, 1000)]),
    ] {
        let result = shape_font(&data, text, script, false, language).unwrap();
        assert_eq!(
            result.iter().map(|g| (g.id, g.advance)).collect::<Vec<_>>(),
            expected
                .into_iter()
                .map(|(id, a)| (id, Fixed::from_i32(a)))
                .collect::<Vec<_>>(),
            "{text}"
        );
    }
}
#[test]
fn normalization_join_controls_refusal_and_buffer_limits() {
    use crate::unicode::Script;
    let data = include_bytes!("fixtures/DejaVu-shaping.ttf");
    let font = crate::Font::parse(data).unwrap();
    let cmap = font.cmap().unwrap();
    let mut output = [Glyph::default(); 64];
    let n = shape::prepare("é", cmap, Script::Latin, false, &mut output).unwrap();
    assert_eq!(n, 1);
    let id = output[0].id;
    assert_eq!(
        shape::prepare("e\u{301}", cmap, Script::Latin, false, &mut output),
        Ok(1)
    );
    assert_eq!(output[0].id, id);
    assert_eq!((output[0].start, output[0].end), (0, 3));
    assert_eq!(
        shape::prepare("é", cmap, Script::Latin, false, &mut []),
        Err(FontError::BufferTooSmall)
    );
    assert_eq!(
        shape::prepare("é", cmap, Script::Devanagari, false, &mut output),
        Ok(1)
    );
    assert!(!shape::supported(Script::Devanagari));
    assert_eq!(shape::script_tag(Script::Devanagari), *b"DFLT");
    for (text, forms) in [
        ("بت", vec![2, 8]),
        ("ب\u{200c}ت", vec![1, 1, 1]),
        ("ب\u{200d}ت", vec![2, 4, 8]),
    ] {
        let n = shape::prepare(text, cmap, Script::Arabic, true, &mut output).unwrap();
        assert_eq!(
            output[..n].iter().map(|g| g.form).collect::<Vec<_>>(),
            forms
        );
    }
    assert_eq!(
        shape::prepare("(", cmap, Script::Common, true, &mut output),
        Ok(1)
    );
    assert_eq!(output[0].id, cmap.glyph_index(')'));
    let mut storage = [Glyph::new(1, 0, 1)];
    let mut buffer = Buffer::new(&mut storage, 1).unwrap();
    let mut multiple = Bin::words(&[1, 0, 1, 0]);
    multiple.child(2, coverage(&[1]));
    multiple.child(6, Bin::words(&[2, 1, 2]));
    let b = layout(vec![lookup(2, 0, multiple)]);
    assert_eq!(
        LayoutTable::parse(&b.0, true).unwrap().apply_lookup(
            0,
            Gdef::default(),
            &[],
            false,
            &mut buffer
        ),
        Err(FontError::BufferTooSmall)
    );
}

#[test]
fn contextual_depth_expansion_and_work_limits() {
    for depth in [15, 16] {
        let mut lookups = Vec::new();
        for i in 0..depth {
            let mut nested = context(3, false, u16::try_from(i + 1).unwrap());
            nested.set(10, 0);
            lookups.push(lookup(5, 0, nested));
        }
        lookups.push(lookup(1, 0, single(1, 42)));
        let b = layout(lookups);
        if depth == 15 {
            assert_eq!(ids(&execute(&b.0, true, &[1, 2]).unwrap()), [42, 2]);
        } else {
            assert_eq!(execute(&b.0, true, &[1, 2]), Err(FontError::LimitExceeded));
        }
    }
    let mut context = Bin::words(&[3, 2, 2, 0, 0, 0, 1, 1, 2]);
    context.child(6, coverage(&[1]));
    context.child(8, coverage(&[2]));
    let mut multiple = Bin::words(&[1, 0, 1, 0]);
    multiple.child(2, coverage(&[1]));
    multiple.child(6, Bin::words(&[2, 3, 4]));
    let b = layout(vec![
        lookup(5, 0, context),
        lookup(2, 0, multiple),
        lookup(1, 0, single(2, 42)),
    ]);
    assert_eq!(ids(&execute(&b.0, true, &[1, 2, 2]).unwrap()), [3, 4, 2, 2]);
}

/// A table whose default language system selects one feature over `refs`.
pub(super) fn featured(lookups: Vec<Bin>, tag: [u8; 4], refs: &[u16]) -> Bin {
    let mut b = layout(lookups);
    let mut script = Bin::words(&[0, 0]);
    script.child(0, Bin::words(&[0, 65535, 1, 0]));
    let mut scripts = Bin::words(&[1, 0x4446, 0x4c54, 0]);
    scripts.child(6, script);
    b.child(4, scripts);
    let mut feature = Bin::words(&[0, u16::try_from(refs.len()).expect("count")]);
    feature.0.extend(refs.iter().flat_map(|v| v.to_be_bytes()));
    let mut features = Bin::words(&[
        1,
        u16::from_be_bytes([tag[0], tag[1]]),
        u16::from_be_bytes([tag[2], tag[3]]),
        0,
    ]);
    features.child(6, feature);
    b.child(6, features);
    b
}

/// Apply `lookups` to `len` copies of glyph 1.
fn long_line(len: usize, lookups: Vec<Bin>, budget: &mut Budget) -> Result<(), FontError> {
    let refs: Vec<u16> = (0..u16::try_from(lookups.len()).expect("count")).collect();
    let b = featured(lookups, *b"test", &refs);
    let mut glyphs = vec![Glyph::new(1, 0, 1); len];
    let mut buffer = Buffer::new(&mut glyphs, len)?;
    LayoutTable::parse(&b.0, true)?.apply(
        *b"DFLT",
        *b"dflt",
        &[Feature {
            tag: *b"test",
            value: 1,
        }],
        Gdef::default(),
        &[],
        false,
        &mut buffer,
        budget,
    )
}

/// `n` single substitutions that cover glyph 1.
fn covering(n: usize) -> Vec<Bin> {
    (0..n).map(|_| lookup(1, 0, single(1, 1))).collect()
}

#[test]
fn a_line_of_65536_glyphs_runs_sixteen_lookups() {
    let mut budget = Budget::default();
    assert_eq!(long_line(65_536, covering(16), &mut budget), Ok(()));
    assert!(budget.spent() >= 32 * 65_536);
}

#[test]
fn lookups_without_subtables_spend_one_operation_per_glyph() {
    let empty = |n| (0..n).map(|_| Bin::words(&[1, 0, 0])).collect();
    let mut budget = Budget::default();
    assert_eq!(long_line(16_384, empty(64), &mut budget), Ok(()));
    assert!(budget.spent() >= 64 * 16_384);
    assert_eq!(
        long_line(16_384, empty(128), &mut Budget::default()),
        Err(FontError::LimitExceeded)
    );
}

#[test]
fn applications_sharing_a_budget_stop_at_its_limit() {
    let mut budget = Budget::default();
    long_line(65_536, covering(8), &mut budget).unwrap();
    let one = budget.spent();
    long_line(65_536, covering(8), &mut budget).unwrap();
    assert_eq!(budget.spent(), 2 * one);
    let mut budget = Budget {
        spent: Budget::LIMIT - one + 1,
    };
    assert_eq!(
        long_line(65_536, covering(8), &mut budget),
        Err(FontError::LimitExceeded)
    );
    assert_eq!(budget.spent(), Budget::LIMIT + 1);
}

#[test]
fn selected_lookups_run_in_list_order_across_bitset_words() {
    let mut lookups: Vec<Bin> = (0..130).map(|_| lookup(1, 0, single(999, 999))).collect();
    for (index, glyph) in [(0, 1), (63, 2), (64, 3), (129, 4)] {
        lookups[index] = lookup(1, 0, single(glyph, glyph + 1));
    }
    let b = featured(lookups, *b"test", &[129, 64, 0, 63, 129]);
    let mut glyphs = [Glyph::new(1, 0, 1)];
    let mut buffer = Buffer::new(&mut glyphs, 1).unwrap();
    LayoutTable::parse(&b.0, true)
        .unwrap()
        .apply(
            *b"DFLT",
            *b"dflt",
            &[Feature {
                tag: *b"test",
                value: 1,
            }],
            Gdef::default(),
            &[],
            false,
            &mut buffer,
            &mut Budget::default(),
        )
        .unwrap();
    assert_eq!(ids(buffer.glyphs()), [5]);
}

#[test]
fn contextual_matches_cost_their_span_not_the_line() {
    let mut context = Bin::words(&[3, 1, 1, 0, 0, 1]);
    context.child(6, coverage(&[1]));
    let b = layout(vec![lookup(5, 0, context), lookup(1, 0, single(1, 2))]);
    let mut glyphs = vec![Glyph::new(1, 0, 1); 4096];
    let mut buffer = Buffer::new(&mut glyphs, 4096).unwrap();
    LayoutTable::parse(&b.0, true)
        .unwrap()
        .apply_lookup(0, Gdef::default(), &[], false, &mut buffer)
        .unwrap();
    assert!(buffer.glyphs().iter().all(|g| g.id == 2));
}

#[test]
fn context_actions_reach_marked_glyphs_moved_by_insertions() {
    let mut context = Bin::words(&[3, 2, 2, 0, 0, 0, 1, 3, 2]);
    context.child(6, coverage(&[1]));
    context.child(8, coverage(&[2]));
    let mut multiple = Bin::words(&[1, 0, 1, 0]);
    multiple.child(2, coverage(&[1]));
    multiple.child(6, Bin::words(&[3, 3, 4, 5]));
    let b = layout(vec![
        lookup(5, 0, context),
        lookup(2, 0, multiple),
        lookup(1, 0, single(2, 42)),
    ]);
    assert_eq!(
        ids(&execute(&b.0, true, &[1, 2, 2]).unwrap()),
        [3, 4, 5, 42, 2]
    );
}

#[test]
fn layout_font_mutations_are_bounded_and_deterministic() {
    use crate::unicode::Script;
    let original = include_bytes!("fixtures/DejaVu-shaping.ttf");
    let font = crate::Font::parse(original).unwrap();
    let ranges = [*b"GSUB", *b"GPOS", *b"GDEF"].map(|tag| {
        let t = font.table(tag).unwrap();
        let start = usize::try_from(t.offset).unwrap();
        start..start + t.data.len()
    });
    for range in ranges {
        for index in range {
            for value in [0, 255] {
                let mut bytes = original.to_vec();
                bytes[index] = value;
                for (text, script, rtl, lang) in [
                    ("officeAVé", Script::Latin, false, *b"ENG "),
                    ("بَتسلام", Script::Arabic, true, *b"ARA "),
                ] {
                    let first = shape_font(&bytes, text, script, rtl, lang);
                    let second = shape_font(&bytes.clone(), text, script, rtl, lang);
                    assert_eq!(first, second, "byte {index} value {value}");
                }
            }
        }
    }
}

#[test]
fn coverage_ranges_alternate_values_and_anchor_formats() {
    let mut single = Bin::words(&[2, 0, 2, 42, 43]);
    single.child(2, Bin::words(&[2, 1, 1, 2, 0]));
    let b = layout(vec![lookup(1, 0, single)]);
    assert_eq!(ids(&execute(&b.0, true, &[1, 2, 3]).unwrap()), [42, 43, 3]);
    let mut alternate = Bin::words(&[1, 0, 1, 0]);
    alternate.child(2, coverage(&[1]));
    alternate.child(6, Bin::words(&[2, 42, 43]));
    let b = with_features(layout(vec![lookup(3, 0, alternate)]), 65535);
    for (value, expected) in [(0, 1), (1, 42), (2, 43), (3, 1)] {
        let mut glyphs = [Glyph::new(1, 0, 1)];
        let mut buffer = Buffer::new(&mut glyphs, 1).unwrap();
        LayoutTable::parse(&b.0, true)
            .unwrap()
            .apply(
                *b"DFLT",
                *b"ENG ",
                &[Feature {
                    tag: *b"test",
                    value,
                }],
                Gdef::default(),
                &[],
                false,
                &mut buffer,
                &mut Budget::default(),
            )
            .unwrap();
        assert_eq!(buffer.glyphs()[0].id, expected);
    }
    for format in [2, 3] {
        let mut t = Bin::words(&[1, 0, 0, 1, 0, 0]);
        t.child(2, coverage(&[7]));
        t.child(4, coverage(&[1]));
        let mut marks = Bin::words(&[1, 0, 0]);
        marks.child(
            4,
            if format == 2 {
                Bin::words(&[2, 10, 20, 3])
            } else {
                Bin::words(&[3, 10, 20, 0, 0])
            },
        );
        t.child(8, marks);
        let mut bases = Bin::words(&[1, 0]);
        bases.child(2, anchor(40, 70));
        t.child(10, bases);
        let b = layout(vec![lookup(4, 0, t)]);
        assert_eq!(
            execute(&b.0, false, &[1, 7]).unwrap()[1].y,
            Fixed::from_i32(50)
        );
    }
}

#[test]
fn contextual_actions_index_the_modified_sequence() {
    for (replacement, index, target, expected) in [
        (vec![3, 4], 1, 4, vec![3, 42, 7, 2, 2]),
        (vec![3, 4], 2, 2, vec![3, 4, 7, 42, 2]),
        (vec![], 0, 2, vec![7, 42, 2]),
    ] {
        let mut context = Bin::words(&[3, 2, 2, 0, 0, 0, 1, index, 2]);
        context.child(6, coverage(&[1]));
        context.child(8, coverage(&[2]));
        let mut multiple = Bin::words(&[1, 0, 1, 0]);
        multiple.child(2, coverage(&[1]));
        let mut sequence = Bin::words(&[u16::try_from(replacement.len()).unwrap()]);
        sequence
            .0
            .extend(replacement.iter().flat_map(|v: &u16| v.to_be_bytes()));
        multiple.child(6, sequence);
        let b = layout(vec![
            lookup(5, 8, context),
            lookup(2, 0, multiple),
            lookup(1, 0, single(target, 42)),
        ]);
        assert_eq!(ids(&execute(&b.0, true, &[1, 7, 2, 2]).unwrap()), expected);
    }
    let mut context = Bin::words(&[3, 4, 2, 0, 0, 0, 0, 1, 1, 2, 2]);
    for (i, glyph) in [1, 2, 3, 4].into_iter().enumerate() {
        context.child(6 + i * 2, coverage(&[glyph]));
    }
    let mut ligature = Bin::words(&[1, 0, 1, 0]);
    ligature.child(2, coverage(&[2]));
    let mut set = Bin::words(&[1, 0]);
    set.child(2, Bin::words(&[9, 2, 3]));
    ligature.child(6, set);
    let b = layout(vec![
        lookup(5, 0, context),
        lookup(4, 0, ligature),
        lookup(1, 0, single(4, 42)),
    ]);
    assert_eq!(
        ids(&execute(&b.0, true, &[1, 2, 3, 4]).unwrap()),
        [1, 9, 42]
    );
}

#[test]
fn substituted_classes_control_mark_advances() {
    let mut d = Bin::words(&[1, 0, 0, 0, 0, 0]);
    d.child(4, Bin::words(&[1, 1, 2, 1, 3]));
    let gdef = Gdef::parse(&d.0, 0).unwrap();
    for (from, to, expected) in [(1, 2, Fixed::ZERO), (2, 1, Fixed::ONE)] {
        let b = layout(vec![lookup(1, 0, single(from, to))]);
        let mut glyphs = [Glyph::new(from, 0, 1)];
        let mut buffer = Buffer::new(&mut glyphs, 1).unwrap();
        LayoutTable::parse(&b.0, true)
            .unwrap()
            .apply_lookup(0, gdef, &[], false, &mut buffer)
            .unwrap();
        buffer.set_advances(|_| Ok(Fixed::ONE)).unwrap();
        assert_eq!(buffer.glyphs()[0].advance, expected);
    }
    let mut ligature = Bin::words(&[1, 0, 1, 0]);
    ligature.child(2, coverage(&[7]));
    let mut set = Bin::words(&[1, 0]);
    set.child(2, Bin::words(&[9, 2, 7]));
    ligature.child(6, set);
    let b = layout(vec![lookup(4, 0, ligature)]);
    assert_eq!(execute(&b.0, true, &[7, 7]).unwrap()[0].class, 3);
}

#[test]
fn repeated_feature_references_spend_the_work_budget() {
    let mut b = layout(vec![lookup(1, 0, single(1, 2))]);
    let mut lang = Bin::words(&[0, 65535, 1024]);
    lang.0.resize(6 + 2048, 0);
    let mut script = Bin::words(&[0, 0]);
    script.child(0, lang);
    let mut scripts = Bin::words(&[1, 0x4446, 0x4c54, 0]);
    scripts.child(6, script);
    b.child(4, scripts);
    let mut feature = Bin::words(&[0, 1024]);
    feature.0.resize(4 + 2048, 0);
    let mut features = Bin::words(&[1, 0x7465, 0x7374, 0]);
    features.child(6, feature);
    b.child(6, features);
    let mut storage = [];
    let mut buffer = Buffer::new(&mut storage, 0).unwrap();
    assert_eq!(
        LayoutTable::parse(&b.0, true).unwrap().apply(
            *b"DFLT",
            *b"dflt",
            &[Feature {
                tag: *b"test",
                value: 1
            }],
            Gdef::default(),
            &[],
            false,
            &mut buffer,
            &mut Budget::default(),
        ),
        Err(FontError::LimitExceeded)
    );
}

/// Apply lookup 0 to `ids` in `capacity` slots; return the glyphs and gap moves.
fn gap_moves(
    data: &[u8],
    ids: &[u16],
    capacity: usize,
) -> (Result<(), FontError>, Vec<Glyph>, usize) {
    let mut glyphs = vec![Glyph::default(); capacity];
    for (i, id) in ids.iter().enumerate() {
        glyphs[i] = Glyph::new(*id, i, i + 1);
    }
    let mut buffer = Buffer::new(&mut glyphs, ids.len()).unwrap();
    let result = LayoutTable::parse(data, true).unwrap().apply_lookup(
        0,
        Gdef::default(),
        &[],
        false,
        &mut buffer,
    );
    (result, buffer.glyphs().to_vec(), buffer.moved)
}
fn sequence(glyphs: &[u16]) -> Bin {
    let mut t = Bin::words(&[1, 0, 1, 0]);
    t.child(2, coverage(&[1]));
    let mut s = Bin::words(&[u16::try_from(glyphs.len()).unwrap()]);
    s.0.extend(glyphs.iter().flat_map(|v| v.to_be_bytes()));
    t.child(6, s);
    t
}

#[test]
fn ligatures_move_linear_glyphs_per_pass() {
    let mut lig = Bin::words(&[1, 0, 1, 0]);
    lig.child(2, coverage(&[1]));
    let mut set = Bin::words(&[1, 0]);
    set.child(2, Bin::words(&[42, 2, 2]));
    lig.child(6, set);
    let b = layout(vec![lookup(4, 0, lig)]);
    let n = 16_384;
    let input: Vec<u16> = (0..n).map(|i| if i % 2 == 0 { 1 } else { 2 }).collect();
    let (result, glyphs, moved) = gap_moves(&b.0, &input, n);
    result.unwrap();
    assert_eq!(glyphs.len(), n / 2);
    for (k, g) in glyphs.iter().enumerate() {
        assert_eq!((g.id, g.start, g.end), (42, 2 * k, 2 * k + 2));
    }
    assert!(moved <= 2 * n, "moved {moved}");
}

#[test]
fn multiple_substitution_moves_linear_glyphs_per_pass() {
    let n = 8_192;
    let b = layout(vec![lookup(2, 0, sequence(&[3, 4]))]);
    let (result, glyphs, moved) = gap_moves(&b.0, &vec![1; n], 2 * n);
    result.unwrap();
    assert_eq!(ids(&glyphs), [3, 4].repeat(n));
    assert!(glyphs.iter().enumerate().all(|(i, g)| g.start == i / 2));
    assert!(moved <= 4 * n, "moved {moved}");
    let b = layout(vec![lookup(2, 0, sequence(&[]))]);
    let input: Vec<u16> = (0..n).map(|i| if i % 2 == 0 { 1 } else { 2 }).collect();
    let (result, glyphs, moved) = gap_moves(&b.0, &input, n);
    result.unwrap();
    assert_eq!(ids(&glyphs), vec![2; n / 2]);
    assert!(moved <= 2 * n, "moved {moved}");
}

#[test]
fn failed_pass_leaves_contiguous_glyphs() {
    let mut t = Bin::words(&[1, 0, 1, 0]);
    t.child(2, coverage(&[1, 9]));
    t.child(6, Bin::words(&[2, 3, 4]));
    let b = layout(vec![lookup(2, 0, t)]);
    let (result, glyphs, _) = gap_moves(&b.0, &[1, 5, 9], 8);
    assert_eq!(result, Err(FontError::InvalidTable));
    assert_eq!(ids(&glyphs), [3, 4, 5, 9]);
}

#[test]
fn long_ligatures_skip_marks_and_move_linear_glyphs() {
    let mut lig = Bin::words(&[1, 0, 1, 0]);
    lig.child(2, coverage(&[1]));
    let mut set = Bin::words(&[1, 0]);
    set.child(2, Bin::words(&[42, 4, 2, 3, 4]));
    lig.child(6, set);
    let b = layout(vec![lookup(4, 8, lig)]);
    let result = execute(&b.0, true, &[1, 7, 2, 7, 3, 7, 4, 5]).unwrap();
    assert_eq!(ids(&result), [42, 7, 7, 7, 5]);
    let components: Vec<_> = result.iter().map(|g| g.component).collect();
    assert_eq!(components[1..4], [0, 1, 2]);
    let n = 4_000;
    let input: Vec<u16> = (0..n).map(|i| [1, 2, 3, 4][i % 4]).collect();
    let (result, glyphs, moved) = gap_moves(&b.0, &input, n);
    result.unwrap();
    assert_eq!(ids(&glyphs), vec![42; n / 4]);
    assert!(moved <= 2 * n, "moved {moved}");
}

#[test]
fn context_actions_out_of_order_move_linear_glyphs() {
    let mut context = Bin::words(&[3, 2, 2, 0, 0, 1, 2, 0, 1]);
    context.child(6, coverage(&[1]));
    context.child(8, coverage(&[2]));
    let mut second = Bin::words(&[1, 0, 1, 0]);
    second.child(2, coverage(&[2]));
    second.child(6, Bin::words(&[2, 5, 6]));
    let b = layout(vec![
        lookup(5, 0, context),
        lookup(2, 0, sequence(&[3, 4])),
        lookup(2, 0, second),
    ]);
    let n = 8_192;
    let input: Vec<u16> = (0..n).map(|i| if i % 2 == 0 { 1 } else { 2 }).collect();
    let (result, glyphs, moved) = gap_moves(&b.0, &input, 2 * n);
    result.unwrap();
    assert_eq!(ids(&glyphs), [3, 4, 5, 6].repeat(n / 2));
    assert!(moved <= 4 * n, "moved {moved}");
}
