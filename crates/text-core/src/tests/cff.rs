// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::arithmetic_side_effects,
    reason = "bounded font fixture construction"
)]

use crate::{
    Fixed, Font, FontError,
    cff::{Cff, Command, Index, MACHINE_BYTES, Position},
};

fn index(objects: &[Vec<u8>], cff2: bool) -> Vec<u8> {
    let mut bytes = if cff2 {
        u32::try_from(objects.len())
            .expect("count")
            .to_be_bytes()
            .to_vec()
    } else {
        u16::try_from(objects.len())
            .expect("count")
            .to_be_bytes()
            .to_vec()
    };
    if objects.is_empty() {
        return bytes;
    }
    bytes.push(4);
    let mut offset = 1_u32;
    for object in objects.iter().chain(std::iter::once(&Vec::new())) {
        bytes.extend(offset.to_be_bytes());
        offset += u32::try_from(object.len()).expect("length");
    }
    for object in objects {
        bytes.extend(object);
    }
    bytes
}

fn number(bytes: &mut Vec<u8>, value: usize) {
    bytes.push(29);
    bytes.extend(i32::try_from(value).expect("number").to_be_bytes());
}

fn font(programs: &[Vec<u8>], local: &[Vec<u8>], global: &[Vec<u8>]) -> Vec<u8> {
    font_extra(programs, local, global, &[])
}
fn font_extra(
    programs: &[Vec<u8>],
    local: &[Vec<u8>],
    global: &[Vec<u8>],
    extra: &[u8],
) -> Vec<u8> {
    let mut charset = vec![0];
    for id in 1..programs.len() {
        charset.extend(
            u16::try_from(if id == 1 {
                34
            } else if id == 2 {
                125
            } else {
                id
            })
            .expect("SID")
            .to_be_bytes(),
        );
    }
    font_charset(programs, local, global, extra, &charset)
}
fn font_charset(
    programs: &[Vec<u8>],
    local: &[Vec<u8>],
    global: &[Vec<u8>],
    extra: &[u8],
    charset_bytes: &[u8],
) -> Vec<u8> {
    let names = index(&[b"Test".to_vec()], false);
    let globals = index(global, false);
    let chars = index(programs, false);
    let prefix =
        4 + names.len() + index(&[vec![0; 23 + extra.len()]], false).len() + 2 + globals.len();
    let charset = prefix + chars.len();
    let private = charset + charset_bytes.len();
    let mut top = Vec::new();
    number(&mut top, prefix);
    top.push(17);
    number(&mut top, 6);
    number(&mut top, private);
    top.push(18);
    number(&mut top, charset);
    top.push(15);
    top.extend(extra);
    let mut bytes = vec![1, 0, 4, 4];
    bytes.extend(names);
    bytes.extend(index(&[top], false));
    bytes.extend([0, 0]);
    bytes.extend(globals);
    bytes.extend(chars);
    bytes.extend(charset_bytes);
    number(&mut bytes, 6);
    bytes.push(19);
    bytes.extend(index(local, false));
    bytes
}

fn decode(program: &[u8]) -> Result<Vec<Command>, FontError> {
    decode_font(&font(&[program.to_vec()], &[], &[]), 0)
}
fn decode_font(bytes: &[u8], glyph: u16) -> Result<Vec<Command>, FontError> {
    let c = Cff::parse_table(bytes, false, 1000)?;
    let mut out = vec![Command::Close; 256];
    let n = c.outline(glyph, &mut out)?;
    out.truncate(n);
    Ok(out)
}
fn p(x: i32, y: i32) -> Position {
    Position {
        x: Fixed::from_i32(x),
        y: Fixed::from_i32(y),
    }
}

#[test]
fn cff_index_widths_empty_entries_and_truncation() {
    for cff2 in [false, true] {
        let bytes = index(&[vec![1, 2], vec![], vec![3]], cff2);
        let (i, n) = Index::parse(&bytes, cff2).expect("index");
        assert_eq!(n, bytes.len());
        assert_eq!(i.len(), 3);
        assert_eq!(i.get(1), Ok(&[][..]));
        assert!(i.get(3).is_err());
        for end in 0..bytes.len() {
            assert!(Index::parse(&bytes[..end], cff2).is_err());
        }
        for value in [0, 5, 255] {
            let mut bad = bytes.clone();
            bad[if cff2 { 4 } else { 2 }] = value;
            assert!(Index::parse(&bad, cff2).is_err());
        }
    }
    for width in 1..=4 {
        let mut b = vec![0, 1, width];
        for v in [1_u32, 2] {
            b.extend(&v.to_be_bytes()[4 - usize::from(width)..]);
        }
        b.push(0xaa);
        assert_eq!(
            Index::parse(&b, false).expect("width").0.get(0),
            Ok(&[0xaa][..])
        );
    }
    assert!(Index::parse(&[0, 1, 1, 0, 1], false).is_err());
    assert!(Index::parse(&[0, 1, 1, 2, 1], false).is_err());
    assert!(Index::parse(&[0, 1, 0, 1], true).is_err());
}

#[test]
fn cff_lines_curves_widths_and_closure() {
    let program = [
        248, 136, 149, 159, 21, 169, 139, 139, 179, 5, 149, 139, 149, 149, 139, 149, 8, 14,
    ];
    assert_eq!(
        decode(&program).expect("path"),
        [
            Command::Move(p(10, 20)),
            Command::Line(p(40, 20)),
            Command::Line(p(40, 60)),
            Command::Curve(p(50, 60), p(60, 70), p(60, 80)),
            Command::Close
        ]
    );
    assert_eq!(
        decode(&[139, 139, 21, 149, 159, 6, 169, 179, 7, 14]).expect("hv lines"),
        [
            Command::Move(p(0, 0)),
            Command::Line(p(10, 0)),
            Command::Line(p(10, 20)),
            Command::Line(p(10, 50)),
            Command::Line(p(50, 50)),
            Command::Close
        ]
    );
    assert_eq!(decode(&[14]).expect("empty"), []);
}

#[test]
fn cff_subroutines_masks_and_recursion() {
    let local = vec![vec![149, 159, 21, 32, 29, 11]];
    let global = vec![vec![169, 139, 5, 11]];
    let bytes = font(&[vec![149, 159, 1, 19, 0x80, 32, 10, 14]], &local, &global);
    assert_eq!(
        decode_font(&bytes, 0).expect("subrs"),
        [
            Command::Move(p(10, 20)),
            Command::Line(p(40, 20)),
            Command::Close
        ]
    );
    let recursive = font(&[vec![32, 10, 14]], &[vec![32, 10, 11]], &[]);
    assert_eq!(decode_font(&recursive, 0), Err(FontError::Cycle));
    let mut chain = Vec::new();
    for i in 0..11 {
        chain.push(vec![33 + i, 10, 11]);
    }
    chain.push(vec![11]);
    assert_eq!(
        decode_font(&font(&[vec![32, 10, 14]], &chain, &[]), 0),
        Err(FontError::LimitExceeded)
    );
    for program in [
        vec![32, 10, 14],
        vec![11],
        vec![149, 159, 1, 19],
        vec![139, 139, 21],
        vec![0],
        vec![255, 0, 0],
    ] {
        assert!(decode(&program).is_err());
    }
}

#[test]
fn cff_arithmetic_and_storage_are_deterministic() {
    // sqrt(9) * 2, then transient storage and readback, as x of moveto.
    let program = [
        148, 12, 26, 141, 12, 24, 139, 12, 20, 139, 12, 21, 139, 21, 14,
    ];
    assert_eq!(
        decode(&program).expect("arithmetic"),
        [Command::Move(p(6, 0)), Command::Close]
    );
    let random = [12, 23, 139, 21, 14];
    assert_eq!(decode(&random), decode(&random));
    for program in [
        vec![139, 12, 21, 14],
        vec![138, 12, 26, 14],
        vec![140, 139, 12, 12, 14],
        vec![12, 10, 14],
        vec![139; 49],
    ] {
        assert!(decode(&program).is_err());
    }
}

#[test]
fn cff_curve_families_and_flex() {
    for (op, values, last) in [
        (26, vec![1, 2, 3, 4], p(2, 8)),
        (27, vec![1, 2, 3, 4], p(7, 3)),
        (30, vec![1, 2, 3, 4, 5], p(6, 9)),
        (31, vec![1, 2, 3, 4, 5], p(8, 7)),
        (24, vec![1, 2, 3, 4, 5, 6, 7, 8], p(16, 20)),
        (25, vec![1, 2, 3, 4, 5, 6, 7, 8], p(16, 20)),
    ] {
        let mut program = vec![139, 139, 21];
        program.extend(values.into_iter().map(|v| v + 139));
        program.extend([op, 14]);
        let out = decode(&program).expect("curve");
        let (Command::Line(point) | Command::Curve(_, _, point)) = out[out.len() - 2] else {
            panic!("path")
        };
        assert_eq!(point, last, "op {op}");
    }
    for (op, n) in [(34, 7), (35, 13), (36, 9), (37, 11)] {
        let mut program = vec![139, 139, 21];
        program.extend(vec![140; n]);
        program.extend([12, op, 14]);
        let out = decode(&program).expect("flex");
        assert_eq!(out.len(), 4);
        assert!(matches!(out[1], Command::Curve(..)));
        assert!(matches!(out[2], Command::Curve(..)));
    }
}

#[test]
fn cff_deprecated_composites_use_standard_encoding() {
    let bytes = font(
        &[
            vec![149, 159, 204, 247, 86, 14],
            vec![140, 141, 21, 14],
            vec![142, 143, 21, 14],
        ],
        &[],
        &[],
    );
    assert_eq!(
        decode_font(&bytes, 0).expect("seac"),
        [
            Command::Move(p(1, 2)),
            Command::Close,
            Command::Move(p(13, 24)),
            Command::Close
        ]
    );
    let recursive = font(
        &[vec![14], vec![139, 139, 204, 247, 86, 14], vec![14]],
        &[],
        &[],
    );
    assert!(decode_font(&recursive, 1).is_err());
}

#[test]
fn cff2_specification_example_blends_at_default() {
    let bytes = include_bytes!("fixtures/cff2-spec.bin");
    let c = Cff::parse_table(bytes, true, 1000).expect("CFF2 example");
    let mut out = [Command::Close; 16];
    let n = c.outline(0, &mut out).expect("default blend");
    assert_eq!(
        &out[..n],
        &[
            Command::Move(p(50, 0)),
            Command::Line(p(550, 0)),
            Command::Line(p(550, 500)),
            Command::Line(p(50, 500)),
            Command::Close
        ]
    );
    assert_eq!(c.outline(1, &mut out), Ok(n));
    for end in 0..bytes.len() {
        assert!(
            Cff::parse_table(&bytes[..end], true, 1000)
                .and_then(|c| c.outline(0, &mut out))
                .is_err(),
            "prefix {end}"
        );
    }
}

#[test]
fn cff_malformed_dictionaries_indices_and_charstrings() {
    let good = font(&[vec![139, 139, 21, 149, 159, 5, 14]], &[], &[]);
    for end in 0..good.len() {
        assert!(
            Cff::parse_table(&good[..end], false, 1000).is_err(),
            "prefix {end}"
        );
    }
    for i in 0..good.len() {
        for byte in [0, 1, 0x7f, 0xff] {
            let mut bad = good.clone();
            bad[i] = byte;
            if let Ok(c) = Cff::parse_table(&bad, false, 1000) {
                let _ = c.outline(0, &mut [Command::Close; 64]);
            }
        }
    }
    let c = Cff::parse_table(&good, false, 1000).expect("font");
    assert_eq!(c.outline(0, &mut []), Err(FontError::BufferTooSmall));
}

#[test]
fn cff_noto_cjk_host_font_matches_literal_coordinates() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tests/fixtures/noto-cjk-subset.otf"
    ))
    .expect("Noto subset");
    let font = Font::parse(&bytes).expect("sfnt");
    let cmap = font.cmap().expect("cmap");
    let c = Cff::parse(&font).expect("CFF");
    let mut out = [Command::Close; 128];
    for (ch, first, len) in [
        ('A', p(4, 0), 16),
        ('中', p(458, 840), 27),
        ('日', p(253, 352), 19),
    ] {
        let n = c.outline(cmap.glyph_index(ch), &mut out).expect("glyph");
        assert_eq!(n, len);
        assert_eq!(out[0], Command::Move(first));
    }
}

#[test]
fn cff_arithmetic_operators_have_literal_results() {
    for (args, op, expected) in [
        (vec![0, 1], 3, 0),
        (vec![1, 1], 3, 1),
        (vec![1, 0], 3, 0),
        (vec![0, 0], 4, 0),
        (vec![0, 1], 4, 1),
        (vec![1, 0], 4, 1),
        (vec![0], 5, 1),
        (vec![7], 5, 0),
        (vec![-7], 9, 7),
        (vec![7], 9, 7),
        (vec![2, 3], 10, 5),
        (vec![2, 3], 11, -1),
        (vec![9, 3], 12, 3),
        (vec![7], 14, -7),
        (vec![2, 2], 15, 1),
        (vec![2, 3], 15, 0),
        (vec![7, 8, 1, 2], 22, 7),
        (vec![7, 8, 2, 1], 22, 8),
        (vec![0], 26, 0),
    ] {
        let mut program: Vec<u8> = args
            .into_iter()
            .map(|n| u8::try_from(n + 139).expect("operand"))
            .collect();
        program.extend([12, op, 139, 21, 14]);
        assert_eq!(
            decode(&program),
            Ok(vec![Command::Move(p(expected, 0)), Command::Close]),
            "op {op}"
        );
    }
    for (stack, operators, point) in [
        (vec![7], vec![12, 27], p(7, 7)),
        (vec![7, 8], vec![12, 28], p(8, 7)),
        (vec![7, 0], vec![12, 29], p(7, 7)),
        (vec![7, -1], vec![12, 29], p(7, 7)),
        (vec![7, 8, 2, 1], vec![12, 30], p(8, 7)),
        (vec![7, 8, 0, 1], vec![12, 30], p(7, 8)),
        (vec![7, 8, 9], vec![12, 18], p(7, 8)),
    ] {
        let mut program: Vec<u8> = stack
            .into_iter()
            .map(|n| u8::try_from(n + 139).expect("operand"))
            .collect();
        program.extend(operators);
        program.extend([21, 14]);
        assert_eq!(
            decode(&program),
            Ok(vec![Command::Move(point), Command::Close])
        );
    }
    assert_eq!(decode(&[12, 0, 14]), Ok(vec![]));
    assert!(decode(&[140, 12, 0, 14]).is_err());
}

#[test]
fn cff_decimal_exponents_and_affine_matrices() {
    for (real, factor) in [
        (vec![30, 0x0a, 0x00, 0x2f], 2),
        (vec![30, 0x2c, 0x3f], 2),
        (vec![30, 0xe0, 0xa0, 0x02, 0xff], -2),
        (vec![30, 0x2b, 0x1f], 20000),
    ] {
        let mut matrix = real.clone();
        matrix.extend([139, 139]);
        matrix.extend(real);
        matrix.extend([140, 141, 12, 7]);
        let bytes = font_extra(&[vec![149, 159, 21, 14]], &[], &[], &matrix);
        let out = decode_font(&bytes, 0).expect("matrix");
        // Decimal conversion retains Q32.32 fractions; compare within one design-unit ulp per multiplication.
        let Command::Move(point) = out[0] else {
            panic!("move")
        };
        assert!(
            (point.x.bits() - Fixed::from_i32(1000 + 10 * factor).bits()).unsigned_abs() <= 10000
        );
        assert!(
            (point.y.bits() - Fixed::from_i32(2000 + 20 * factor).bits()).unsigned_abs() <= 20000
        );
    }
    for bad in [
        vec![30, 0xff],
        vec![30, 0x1b, 0xff],
        vec![30, 0xae, 0x1f],
        vec![30, 0x1d],
        vec![30, 0x1a, 0xaf],
        vec![30, 0x1c, 0x99, 0xff],
        vec![255],
        vec![139; 49],
    ] {
        assert!(Cff::parse_table(&font_extra(&[vec![14]], &[], &[], &bad), false, 1000).is_err());
    }
}

fn fd_font(select: &[u8]) -> Vec<u8> {
    fd_font_glyphs(2, select)
}
fn fd_font_glyphs(glyphs: usize, select: &[u8]) -> Vec<u8> {
    cff2_font(
        &vec![vec![140, 141, 21]; glyphs],
        &[vec![], vec![141, 139, 139, 142, 140, 141, 12, 7]],
        select,
    )
}
fn cff2_font(programs: &[Vec<u8>], fds: &[Vec<u8>], select: &[u8]) -> Vec<u8> {
    let chars = index(programs, true);
    let fds = index(fds, true);
    let mut top = Vec::new();
    number(&mut top, 29);
    top.push(17);
    number(&mut top, 29 + chars.len());
    top.extend([12, 36]);
    number(&mut top, 29 + chars.len() + fds.len());
    top.extend([12, 37]);
    assert_eq!(top.len(), 20);
    let mut bytes = vec![2, 0, 5, 0, 20];
    bytes.extend(top);
    bytes.extend([0; 4]);
    bytes.extend(chars);
    bytes.extend(fds);
    bytes.extend(select);
    bytes
}
#[test]
fn cff2_font_dictionary_selection_formats() {
    for select in [
        vec![0, 0, 1],
        vec![3, 0, 2, 0, 0, 0, 0, 1, 1, 0, 2],
        vec![
            4, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 2,
        ],
    ] {
        let bytes = fd_font(&select);
        let cff = Cff::parse_table(&bytes, true, 1000).expect("FDSelect");
        let mut out = [Command::Close; 8];
        assert_eq!(cff.outline(0, &mut out), Ok(2));
        assert_eq!(out[0], Command::Move(p(1, 2)));
        assert_eq!(cff.outline(1, &mut out), Ok(2));
        assert_eq!(out[0], Command::Move(p(3, 8)));
        for end in 0..select.len() {
            assert!(Cff::parse_table(&fd_font(&select[..end]), true, 1000).is_err());
        }
        for at in 0..select.len() {
            for value in [0, 1, 2, 3, 4, 255] {
                let mut bad = select.clone();
                bad[at] = value;
                let bytes = fd_font(&bad);
                if let Ok(c) = Cff::parse_table(&bytes, true, 1000) {
                    assert!(c.outline(0, &mut out).is_ok());
                    assert!(c.outline(1, &mut out).is_ok());
                }
            }
        }
    }
}
#[test]
fn cff_predefined_charsets_resolve_composite_space() {
    for charset in 0..=2 {
        let bytes = font_extra(
            &[vec![149, 159, 171, 171, 14], vec![140, 141, 21, 14]],
            &[],
            &[],
            &[139 + charset, 15],
        );
        assert_eq!(
            decode_font(&bytes, 0),
            Ok(vec![
                Command::Move(p(1, 2)),
                Command::Close,
                Command::Move(p(11, 22)),
                Command::Close
            ])
        );
    }
}

/// Issue #483: `FDSelect` ranges resolve by binary search at every boundary.
#[test]
fn cff2_font_dictionary_selection_many_ranges() {
    // Range lengths 1, 2, 3, ... alternate between FD 0 and FD 1.
    let mut ranges = Vec::new();
    let mut first = 0;
    while first < 1000 {
        ranges.push((first, ranges.len() % 2));
        first += ranges.len();
    }
    let glyphs = first;
    let mut narrow = vec![3];
    narrow.extend(u16::try_from(ranges.len()).expect("count").to_be_bytes());
    let mut wide = vec![4];
    wide.extend(u32::try_from(ranges.len()).expect("count").to_be_bytes());
    for &(first, fd) in &ranges {
        narrow.extend(u16::try_from(first).expect("first").to_be_bytes());
        narrow.push(u8::try_from(fd).expect("fd"));
        wide.extend(u32::try_from(first).expect("first").to_be_bytes());
        wide.extend(u16::try_from(fd).expect("fd").to_be_bytes());
    }
    narrow.extend(u16::try_from(glyphs).expect("sentinel").to_be_bytes());
    wide.extend(u32::try_from(glyphs).expect("sentinel").to_be_bytes());
    for select in [narrow, wide] {
        let bytes = fd_font_glyphs(glyphs, &select);
        let cff = Cff::parse_table(&bytes, true, 1000).expect("FDSelect");
        let mut out = [Command::Close; 8];
        let mut range = 0;
        for glyph in 0..glyphs {
            if ranges.get(range + 1).is_some_and(|r| r.0 == glyph) {
                range += 1;
            }
            let expected = if ranges[range].1 == 0 {
                p(1, 2)
            } else {
                p(3, 8)
            };
            let id = u16::try_from(glyph).expect("glyph");
            assert_eq!(cff.outline(id, &mut out), Ok(2), "glyph {glyph}");
            assert_eq!(out[0], Command::Move(expected), "glyph {glyph}");
        }
    }
}

/// Issue #482: endchar components resolve through format 1 and 2 charset
/// ranges; the lowest glyph wins a duplicate SID.
#[test]
fn cff_composite_components_resolve_through_charset_ranges() {
    // (first SID, glyphs after the first); `A` is SID 34, `grave` SID 125.
    let mut ranges = vec![(200, 99), (30, 9)];
    ranges.extend((0..50).map(|k| (400 + k, 0)));
    ranges.extend([(120, 10), (34, 0)]);
    let glyphs = 1 + ranges.iter().map(|r| r.1 + 1).sum::<usize>();
    assert_eq!(glyphs, 173);
    let mut programs = vec![vec![14]; glyphs];
    programs[0] = vec![149, 159, 204, 247, 86, 14];
    programs[105] = vec![140, 141, 21, 14];
    programs[166] = vec![142, 143, 21, 14];
    programs[172] = vec![144, 145, 21, 14];
    for format in [1u8, 2] {
        let mut charset = vec![format];
        for &(first, n) in &ranges {
            charset.extend(u16::try_from(first).expect("SID").to_be_bytes());
            if format == 1 {
                charset.push(u8::try_from(n).expect("n"));
            } else {
                charset.extend(u16::try_from(n).expect("n").to_be_bytes());
            }
        }
        let bytes = font_charset(&programs, &[], &[], &[], &charset);
        assert_eq!(
            decode_font(&bytes, 0),
            Ok(vec![
                Command::Move(p(1, 2)),
                Command::Close,
                Command::Move(p(13, 24)),
                Command::Close
            ])
        );
        // `a` (SID 66) is absent.
        let mut missing = programs.clone();
        missing[0] = vec![149, 159, 204, 236, 14];
        let bytes = font_charset(&missing, &[], &[], &[], &charset);
        assert_eq!(decode_font(&bytes, 0), Err(FontError::GlyphIndex));
    }
    // Format 0: SID 34 at glyphs 1 and 3; the lowest glyph wins.
    let programs = [
        vec![149, 159, 204, 247, 86, 14],
        vec![140, 141, 21, 14],
        vec![142, 143, 21, 14],
        vec![144, 145, 21, 14],
    ];
    let charset = [0, 0, 34, 0, 125, 0, 34];
    assert_eq!(
        decode_font(&font_charset(&programs, &[], &[], &[], &charset), 0),
        Ok(vec![
            Command::Move(p(1, 2)),
            Command::Close,
            Command::Move(p(13, 24)),
            Command::Close
        ])
    );
}

/// Issue #482: 65,534 one-glyph ranges; the old O(G × R) lookup does not
/// finish in test time.
#[test]
fn cff_composite_components_resolve_in_one_charset_pass() {
    let glyphs = usize::from(u16::MAX);
    let mut programs = vec![vec![14]; glyphs];
    programs[0] = vec![149, 159, 204, 247, 86, 14];
    // Glyph `g` has SID `65535 - g`: `A` (34) and `grave` (125) sit last.
    programs[glyphs - 34] = vec![140, 141, 21, 14];
    programs[glyphs - 125] = vec![142, 143, 21, 14];
    let mut charset = vec![1];
    for glyph in 1..glyphs {
        charset.extend(u16::try_from(glyphs - glyph).expect("SID").to_be_bytes());
        charset.push(0);
    }
    let bytes = font_charset(&programs, &[], &[], &[], &charset);
    assert_eq!(
        decode_font(&bytes, 0),
        Ok(vec![
            Command::Move(p(1, 2)),
            Command::Close,
            Command::Move(p(13, 24)),
            Command::Close
        ])
    );
}

/// Issue #484: a CFF decoder holds 48 operands, a CFF2 decoder 513.
#[test]
fn cff_decoder_state_is_sized_by_format() {
    assert!(
        MACHINE_BYTES[0] <= 1024,
        "CFF state {} bytes",
        MACHINE_BYTES[0]
    );
    assert_eq!(
        MACHINE_BYTES[1] - MACHINE_BYTES[0],
        (513 - 48) * core::mem::size_of::<Fixed>()
    );
}

/// Issue #484: operand limits at 48 for CFF and 513 for CFF2, in
/// charstrings and in dictionaries.
#[test]
fn cff_operand_limits_follow_the_format() {
    // rmoveto 0 0, `n` zero operands, rlineto, endchar.
    let lines = |n: usize| {
        let mut program = vec![139, 139, 21];
        program.extend(vec![139; n]);
        program.extend([5, 14]);
        program
    };
    assert_eq!(decode(&lines(48)).map(|c| c.len()), Ok(26));
    assert_eq!(decode(&lines(49)), Err(FontError::LimitExceeded));
    // Operator 0 ignores its operands in a CFF DICT.
    let mut top = vec![139; 48];
    top.push(0);
    assert!(Cff::parse_table(&font_extra(&[vec![14]], &[], &[], &top), false, 1000).is_ok());
    top.insert(0, 139);
    assert_eq!(
        Cff::parse_table(&font_extra(&[vec![14]], &[], &[], &top), false, 1000).err(),
        Some(FontError::LimitExceeded)
    );
    // CFF2 clears the stack at an unrecognized operator (2) and at DICT
    // operator 0.
    let select = [0, 0];
    for (n, ok) in [(513, true), (514, false)] {
        let mut program = vec![139; n];
        program.push(2);
        let bytes = cff2_font(&[program], &[vec![]], &select);
        let cff = Cff::parse_table(&bytes, true, 1000).expect("CFF2");
        let mut out = [Command::Close; 4];
        let result = cff.outline(0, &mut out);
        assert_eq!(
            result,
            if ok {
                Ok(0)
            } else {
                Err(FontError::LimitExceeded)
            }
        );
        let mut fd = vec![139; n];
        fd.push(0);
        let bytes = cff2_font(&[vec![]], &[fd], &select);
        assert_eq!(
            Cff::parse_table(&bytes, true, 1000).err(),
            if ok {
                None
            } else {
                Some(FontError::LimitExceeded)
            }
        );
    }
}
