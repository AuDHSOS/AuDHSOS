// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Bounded test fixture arithmetic"
)]

use crate::{FontError, cmap::Cmap};

fn p16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_be_bytes());
}
fn p24(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 3].copy_from_slice(&v.to_be_bytes()[1..]);
}
fn p32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_be_bytes());
}
fn num(n: usize) -> u32 {
    u32::try_from(n).expect("fixture")
}
fn n16(n: usize) -> u16 {
    u16::try_from(n).expect("fixture")
}

fn wrap(tables: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut b = vec![0; 4 + 8 * tables.len()];
    p16(&mut b, 2, n16(tables.len()));
    for (i, (platform, encoding, table)) in tables.iter().enumerate() {
        let at = 4 + i * 8;
        let start = num(b.len());
        p16(&mut b, at, *platform);
        p16(&mut b, at + 2, *encoding);
        p32(&mut b, at + 4, start);
        b.extend_from_slice(table);
    }
    b
}

fn zero() -> Vec<u8> {
    let mut b = vec![0; 262];
    p16(&mut b, 2, 262);
    b[6 + 65] = 7;
    b
}
fn six() -> Vec<u8> {
    let mut b = vec![0; 16];
    for (at, v) in [(0, 6), (2, 16), (6, 65), (8, 3), (10, 7), (12, 0), (14, 9)] {
        p16(&mut b, at, v);
    }
    b
}
fn four() -> Vec<u8> {
    // Literal worked example, cmap.html, Format 4.
    let mut b = vec![0; 48];
    p16(&mut b, 0, 4);
    p16(&mut b, 2, 48);
    p16(&mut b, 6, 8);
    for (i, (start, end, delta)) in [
        (10, 20, 65527),
        (30, 90, 65518),
        (153, 480, 65456),
        (65535, 65535, 1),
    ]
    .into_iter()
    .enumerate()
    {
        p16(&mut b, 14 + i * 2, end);
        p16(&mut b, 24 + i * 2, start);
        p16(&mut b, 32 + i * 2, delta);
    }
    b
}
fn four_array() -> Vec<u8> {
    let mut b = vec![0; 38];
    for (at, v) in [
        (0, 4),
        (2, 38),
        (6, 4),
        (14, 67),
        (16, 65535),
        (20, 65),
        (22, 65535),
        (24, 65535),
        (26, 1),
        (28, 4),
        (32, 8),
        (34, 0),
        (36, 10),
    ] {
        p16(&mut b, at, v);
    }
    b
}
fn groups(format: u16) -> Vec<u8> {
    let mut b = vec![0; 40];
    p16(&mut b, 0, format);
    p32(&mut b, 4, 40);
    p32(&mut b, 12, 2);
    for (i, (start, end, id)) in [(65, 67, 7), (0x1f600, 0x1f601, 20)]
        .into_iter()
        .enumerate()
    {
        p32(&mut b, 16 + 12 * i, start);
        p32(&mut b, 20 + 12 * i, end);
        p32(&mut b, 24 + 12 * i, id);
    }
    b
}
fn variations() -> Vec<u8> {
    let mut b = vec![0; 38];
    p16(&mut b, 0, 14);
    p32(&mut b, 2, 38);
    p32(&mut b, 6, 1);
    p24(&mut b, 10, 0xfe0f);
    p32(&mut b, 13, 21);
    p32(&mut b, 17, 29);
    p32(&mut b, 21, 1);
    p24(&mut b, 25, 65);
    b[28] = 1;
    p32(&mut b, 29, 1);
    p24(&mut b, 33, 67);
    p16(&mut b, 36, 42);
    b
}
fn parse(table: Vec<u8>, glyphs: u16) -> Result<Vec<u16>, FontError> {
    let bytes = wrap(&[(0, 4, table)]);
    let cmap = Cmap::parse(&bytes, glyphs)?;
    Ok([
        '@',
        'A',
        'B',
        'C',
        'D',
        '\u{1f600}',
        '\u{1f601}',
        '\u{10ffff}',
    ]
    .map(|c| cmap.glyph_index(c))
    .to_vec())
}

#[test]
fn cmap_all_primary_formats_match_literal_vectors() -> Result<(), FontError> {
    assert_eq!(parse(zero(), 100)?, [0, 7, 0, 0, 0, 0, 0, 0]);
    assert_eq!(parse(six(), 100)?, [0, 7, 0, 9, 0, 0, 0, 0]);
    assert_eq!(parse(four_array(), 100)?, [0, 7, 0, 9, 0, 0, 0, 0]);
    assert_eq!(parse(groups(12), 100)?, [0, 7, 8, 9, 0, 20, 21, 0]);
    assert_eq!(parse(groups(13), 100)?, [0, 7, 7, 7, 0, 20, 20, 0]);
    let bytes = wrap(&[(3, 1, four())]);
    let cmap = Cmap::parse(&bytes, 401)?;
    for (c, id) in [
        (9, 0),
        (10, 1),
        (20, 11),
        (21, 0),
        (30, 12),
        (90, 72),
        (153, 73),
        (480, 400),
        (481, 0),
        (65535, 0),
    ] {
        assert_eq!(cmap.glyph_index(char::from_u32(c).expect("scalar")), id);
    }
    assert_eq!(cmap.format(), 4);
    Ok(())
}

#[test]
fn cmap_selection_prefers_full_repertoire_and_unicode_platform() -> Result<(), FontError> {
    let mut windows = groups(12);
    p32(&mut windows, 24, 50);
    let bytes = wrap(&[(0, 3, four()), (0, 4, groups(12)), (3, 10, windows)]);
    let cmap = Cmap::parse(&bytes, 500)?;
    assert_eq!(cmap.format(), 12);
    assert_eq!(cmap.glyph_index('A'), 7);
    // An unselected subtable's payload does not participate in mappings.
    let bytes = wrap(&[(0, 3, four()), (0, 4, groups(12))]);
    assert_eq!(Cmap::parse(&bytes, 100)?.glyph_index('A'), 7);
    Ok(())
}

#[test]
fn cmap_variations_distinguish_default_nondefault_and_unsupported() -> Result<(), FontError> {
    let bytes = wrap(&[(0, 4, groups(12)), (0, 5, variations())]);
    let cmap = Cmap::parse(&bytes, 100)?;
    assert_eq!(cmap.variation_glyph('A', '\u{fe0f}'), Some(7));
    assert_eq!(cmap.variation_glyph('B', '\u{fe0f}'), Some(8));
    assert_eq!(cmap.variation_glyph('C', '\u{fe0f}'), Some(42));
    assert_eq!(cmap.variation_glyph('D', '\u{fe0f}'), None);
    assert_eq!(cmap.variation_glyph('A', '\u{fe0e}'), None);
    assert_eq!(cmap.variation_glyph('A', '\u{e0100}'), None);
    let bytes = wrap(&[(0, 4, groups(12))]);
    assert_eq!(
        Cmap::parse(&bytes, 100)?.variation_glyph('A', '\u{fe0f}'),
        None
    );
    let mut uvs = variations();
    p16(&mut uvs, 36, 0);
    p24(&mut uvs, 33, 0x1f999);
    let bytes = wrap(&[(0, 4, groups(12)), (0, 5, uvs)]);
    assert_eq!(
        Cmap::parse(&bytes, 100)?.variation_glyph('\u{1f999}', '\u{fe0f}'),
        Some(0)
    );
    Ok(())
}

#[test]
fn cmap_truncated_prefixes_of_every_format_are_rejected() {
    let mut samples = vec![zero(), four(), four_array(), six(), groups(12), groups(13)]
        .into_iter()
        .map(|t| wrap(&[(0, 4, t)]))
        .collect::<Vec<_>>();
    samples.push(wrap(&[(0, 4, groups(12)), (0, 5, variations())]));
    for sample in samples {
        for n in 0..sample.len() {
            assert!(Cmap::parse(&sample[..n], 500).is_err(), "prefix {n}");
        }
    }
}

#[test]
fn cmap_bad_glyphs_are_rejected_before_lookup() {
    for table in [zero(), four(), four_array(), six(), groups(12), groups(13)] {
        assert_eq!(parse(table, 7).err(), Some(FontError::GlyphIndex));
    }
    let bytes = wrap(&[(0, 4, groups(12)), (0, 5, variations())]);
    assert_eq!(Cmap::parse(&bytes, 42).err(), Some(FontError::GlyphIndex));
    assert_eq!(Cmap::parse(&bytes, 0).err(), Some(FontError::InvalidTable));
}

#[test]
fn cmap_header_aliases_order_and_unsupported_encodings_are_rejected() {
    let mut b = wrap(&[(0, 4, groups(12))]);
    p16(&mut b, 0, 1);
    assert!(Cmap::parse(&b, 100).is_err());
    p16(&mut b, 0, 0);
    p16(&mut b, 2, 65);
    assert_eq!(Cmap::parse(&b, 100).err(), Some(FontError::LimitExceeded));
    for start in [0, 4, 8, u32::MAX] {
        let mut b = wrap(&[(0, 4, groups(12))]);
        p32(&mut b, 8, start);
        assert!(Cmap::parse(&b, 100).is_err());
    }
    for tables in [
        vec![(3, 1, four()), (0, 4, groups(12))],
        vec![(0, 4, groups(12)), (0, 4, groups(12))],
        vec![(0, 5, groups(12))],
        vec![(3, 10, variations())],
        vec![(1, 0, zero())],
        vec![(3, 0, four())],
        vec![],
    ] {
        assert!(Cmap::parse(&wrap(&tables), 500).is_err());
    }
}

#[test]
fn cmap_four_malformed_segments_offsets_and_sentinel_are_rejected() {
    for (at, v) in [
        (6, 0),
        (6, 3),
        (6, 0xfffe),
        (14, 64),
        (16, 67),
        (20, 68),
        (22, 0xfffe),
        (28, 1),
        (28, 2),
        (28, 0xfffe),
        (18, 1),
    ] {
        let mut b = four_array();
        p16(&mut b, at, v);
        assert!(parse(b, 100).is_err(), "field {at}={v}");
    }
}

#[test]
fn cmap_groups_validate_order_ranges_counts_and_overflow() {
    for format in [12, 13] {
        for (at, v) in [
            (4, 39),
            (4, u32::MAX),
            (8, 1),
            (12, u32::MAX),
            (16, 68),
            (20, 0x0011_0000),
            (28, 67),
            (32, 0x0011_0000),
            (36, 100),
        ] {
            let mut b = groups(format);
            p32(&mut b, at, v);
            assert!(parse(b, 100).is_err(), "format {format} field {at}={v}");
        }
    }
    let mut b = groups(12);
    p32(&mut b, 24, u32::MAX);
    assert_eq!(parse(b, 100).err(), Some(FontError::Overflow));
    let mut b = six();
    p16(&mut b, 6, 65535);
    assert!(parse(b, 100).is_err());
    let mut b = six();
    p16(&mut b, 8, 65535);
    assert!(parse(b, 100).is_err());
    let mut b = zero();
    p16(&mut b, 2, 261);
    assert!(parse(b, 100).is_err());
}

#[test]
fn cmap_variation_bad_offsets_scalars_selectors_and_partition_are_rejected() {
    for (at, v) in [
        (2, 37),
        (2, u32::MAX),
        (6, 261),
        (13, 10),
        (13, u32::MAX),
        (17, 10),
        (17, u32::MAX),
        (21, u32::MAX),
        (29, u32::MAX),
    ] {
        let mut vrs = variations();
        p32(&mut vrs, at, v);
        assert!(
            Cmap::parse(&wrap(&[(0, 4, groups(12)), (0, 5, vrs)]), 100).is_err(),
            "field {at}={v}"
        );
    }
    for (at, v) in [
        (10, 65),
        (10, 0x0011_0000),
        (25, 0x0010_ffff),
        (25, 0xd800),
        (33, 0xd800),
        (33, 0x0011_0000),
        (33, 65),
    ] {
        let mut vrs = variations();
        p24(&mut vrs, at, v);
        assert!(
            Cmap::parse(&wrap(&[(0, 4, groups(12)), (0, 5, vrs)]), 100).is_err(),
            "field {at}={v}"
        );
    }
}

#[test]
fn cmap_mutations_preserve_validated_glyph_bounds() {
    for table in [zero(), four_array(), groups(12), variations()] {
        let bytes = if table == variations() {
            wrap(&[(0, 4, groups(12)), (0, 5, table)])
        } else {
            wrap(&[(0, 4, table)])
        };
        for i in 0..bytes.len() {
            for value in [0, 1, 127, 255] {
                let mut changed = bytes.clone();
                changed[i] = value;
                if let Ok(cmap) = Cmap::parse(&changed, 100) {
                    for c in ['\0', 'A', 'B', 'C', '\u{ffff}', '\u{1f600}', '\u{10ffff}'] {
                        assert!(cmap.glyph_index(c) < 100);
                        assert!(
                            cmap.variation_glyph(c, '\u{fe0f}')
                                .is_none_or(|id| id < 100)
                        );
                    }
                }
            }
        }
    }
}
