// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::arithmetic_side_effects, reason = "bounded fixture arithmetic")]

use super::metrics::{put16, put32};
use crate::{
    Fixed, FontError,
    cff::{Cff, Command, Position},
    glyf::Point,
    variation::{Axes, AxisValue, DeltaMap, Gvar, Hvar, ItemStore, Mvar, VariationPoint},
};

pub(super) fn fvar() -> Vec<u8> {
    let mut d = vec![0; 36];
    put32(&mut d, 0, 0x0001_0000);
    put16(&mut d, 4, 16);
    put16(&mut d, 6, 2);
    put16(&mut d, 8, 1);
    put16(&mut d, 10, 20);
    put16(&mut d, 14, 8);
    d[16..20].copy_from_slice(b"wght");
    put32(&mut d, 20, 100 << 16);
    put32(&mut d, 24, 400 << 16);
    put32(&mut d, 28, 900 << 16);
    put16(&mut d, 34, 256);
    d
}
fn avar() -> Vec<u8> {
    let mut d = vec![0; 10];
    put32(&mut d, 0, 0x0001_0000);
    put16(&mut d, 6, 1);
    put16(&mut d, 8, 4);
    for (from, to) in [
        (-16384_i16, -16384_i16),
        (0, 0),
        (8192, 4096),
        (16384, 16384),
    ] {
        d.extend(from.to_be_bytes());
        d.extend(to.to_be_bytes());
    }
    d
}

#[test]
fn variation_axes_normalize_clamp_and_remap() {
    let f = fvar();
    let a = avar();
    let axes = Axes::parse(&f, Some(&a)).expect("axes");
    let mut out = [Fixed::ZERO];
    for (value, bits) in [
        (100, -0x1_0000_0000_i64),
        (400, 0),
        (650, 0x4000_0000),
        (900, 0x1_0000_0000),
        (1000, 0x1_0000_0000),
    ] {
        axes.normalize(
            &[AxisValue {
                tag: *b"wght",
                value: Fixed::from_i32(value),
            }],
            &mut out,
        )
        .expect("normalize");
        assert_eq!(out[0].bits(), bits);
    }
    axes.normalize(&[], &mut out).expect("default");
    assert_eq!(out, [Fixed::ZERO]);
    assert!(axes.normalize(&[], &mut []).is_err());
    let v = AxisValue {
        tag: *b"wght",
        value: Fixed::ONE,
    };
    assert!(axes.normalize(&[v, v], &mut out).is_err());
    assert!(
        axes.normalize(
            &[AxisValue {
                tag: *b"xxxx",
                value: Fixed::ONE
            }],
            &mut out
        )
        .is_err()
    );
    for end in 0..f.len() {
        assert!(Axes::parse(&f[..end], None).is_err());
    }
    for end in 0..a.len() {
        assert!(Axes::parse(&f, Some(&a[..end])).is_err());
    }
    for (offset, value) in [(8, 65), (10, 19), (14, 7), (32, 2)] {
        let mut bad = f.clone();
        put16(&mut bad, offset, value);
        assert!(Axes::parse(&bad, None).is_err());
    }
    let mut bad = a.clone();
    put16(&mut bad, 18, 0);
    assert!(Axes::parse(&f, Some(&bad)).is_err());
}

fn store() -> Vec<u8> {
    let mut d = vec![0; 12];
    put16(&mut d, 0, 1);
    put32(&mut d, 2, 12);
    put16(&mut d, 6, 1);
    put32(&mut d, 8, 28);
    d.extend([0, 1, 0, 2]);
    for v in [0_i16, 16384, 16384, -16384, -16384, 0] {
        d.extend(v.to_be_bytes());
    }
    d.extend([0, 2, 0, 1, 0, 2, 0, 0, 0, 1, 0, 100, 236, 255, 56, 40]);
    d
}

#[test]
fn item_store_hvar_mvar_and_packed_maps() {
    let d = store();
    let s = ItemStore::parse(&d, 1).expect("store");
    for (coord, a, b) in [
        (8192, 50, -100),
        (-8192, -10, 20),
        (0, 0, 0),
        (16384, 100, -200),
    ] {
        let coords = [Fixed::from_2_14(coord)];
        assert_eq!(s.delta(0, 0, &coords), Ok(Fixed::from_i32(a)));
        assert_eq!(s.delta(0, 1, &coords), Ok(Fixed::from_i32(b)));
    }
    let mut h = vec![0; 20];
    put32(&mut h, 0, 0x0001_0000);
    put32(&mut h, 4, 20);
    h.extend(&d);
    let h = Hvar::parse(&h, 1).expect("HVAR");
    assert_eq!(h.advance(0, &[Fixed::ONE]), Ok(Fixed::from_i32(100)));
    assert_eq!(h.side_bearing(0, true, &[Fixed::ONE]), Ok(None));
    let mut m = vec![0; 20];
    put32(&mut m, 0, 0x0001_0000);
    put16(&mut m, 6, 8);
    put16(&mut m, 8, 1);
    put16(&mut m, 10, 20);
    m[12..16].copy_from_slice(b"hasc");
    put16(&mut m, 18, 1);
    m.extend(&d);
    let m = Mvar::parse(&m, 1).expect("MVAR");
    assert_eq!(m.delta(*b"hasc", &[Fixed::ONE]), Ok(Fixed::from_i32(-200)));
    assert_eq!(m.delta(*b"zzzz", &[Fixed::ONE]), Ok(Fixed::ZERO));
    let map = DeltaMap::parse(&[0, 0x11, 0, 2, 0, 1, 0, 6]).expect("map");
    assert_eq!(map.get(0), Ok((0, 1)));
    assert_eq!(map.get(99), Ok((1, 2)));
    for end in 0..d.len() {
        assert!(ItemStore::parse(&d[..end], 1).is_err());
    }
    assert!(s.delta(1, 0, &[Fixed::ZERO]).is_err());
    assert!(s.delta(0, 2, &[Fixed::ZERO]).is_err());
    assert!(s.delta(0, 0, &[]).is_err());
    assert!(s.delta(0, 0, &[Fixed::from_i32(2)]).is_err());
    for (at, value) in [(0, 2), (12, 2), (30, 3), (36, 3)] {
        let mut bad = d.clone();
        put16(&mut bad, at, value);
        assert!(ItemStore::parse(&bad, 1).is_err());
    }
}

fn gvar(payload: &[u8], peak: i16, intermediate: bool, shared: bool) -> Vec<u8> {
    let mut glyph = vec![0; 10];
    put16(&mut glyph, 0, if shared { 0x8001 } else { 1 });
    put16(&mut glyph, 2, if intermediate { 14 } else { 10 });
    put16(
        &mut glyph,
        4,
        u16::try_from(payload.len() - if shared { 4 } else { 0 }).expect("payload"),
    );
    put16(
        &mut glyph,
        6,
        0x8000 | if shared { 0 } else { 0x2000 } | if intermediate { 0x4000 } else { 0 },
    );
    put16(&mut glyph, 8, u16::from_be_bytes(peak.to_be_bytes()));
    if intermediate {
        glyph.extend([0, 0, 0x40, 0]);
    }
    glyph.extend(payload);
    let mut d = vec![0; 28];
    put32(&mut d, 0, 0x0001_0000);
    put16(&mut d, 4, 1);
    put32(&mut d, 8, 28);
    put16(&mut d, 12, 1);
    put16(&mut d, 14, 1);
    put32(&mut d, 16, 28);
    put32(&mut d, 24, u32::try_from(glyph.len()).expect("length"));
    d.extend(glyph);
    d
}
fn points() -> [Point; 7] {
    [
        Point::new(Fixed::ZERO, Fixed::ZERO, true),
        Point::new(Fixed::from_i32(50), Fixed::ZERO, true),
        Point::new(Fixed::from_i32(100), Fixed::ZERO, true),
        Point::default(),
        Point::new(Fixed::from_i32(500), Fixed::ZERO, true),
        Point::default(),
        Point::default(),
    ]
}

#[test]
fn gvar_sparse_iup_shared_and_intermediate_tuples() {
    for (intermediate, shared, peak, coord) in [
        (false, false, 16384, 8192),
        (false, true, 16384, 8192),
        (true, false, 8192, 12288),
    ] {
        let d = gvar(&[2, 1, 0, 2, 1, 0, 20, 0x81], peak, intermediate, shared);
        let g = Gvar::parse(&d, 1, 1).expect("gvar");
        let mut points = points();
        let mut work = [VariationPoint::default(); 7];
        g.apply(0, &[Fixed::from_2_14(coord)], &mut points, &[2], &mut work)
            .expect("IUP");
        assert_eq!(points[1].x, Fixed::from_i32(55));
        assert_eq!(points[2].x, Fixed::from_i32(110));
        assert_eq!(points[4].x, Fixed::from_i32(500));
    }
    let d = gvar(&[3, 2, 0, 0, 2, 2, 5, 5, 20, 0x82], 16384, false, false);
    let g = Gvar::parse(&d, 1, 1).expect("duplicates");
    let mut points = points();
    g.apply(
        0,
        &[Fixed::ONE],
        &mut points,
        &[2],
        &mut [VariationPoint::default(); 7],
    )
    .expect("cumulative repeated points");
    assert_eq!(points[0].x, Fixed::from_i32(10));
    assert_eq!(points[1].x, Fixed::from_i32(65));
}

#[test]
fn gvar_malformed_packed_runs_and_offsets() {
    let d = gvar(&[2, 1, 0, 2, 1, 0, 20, 0x81], 16384, false, false);
    for end in 0..d.len() {
        assert!(Gvar::parse(&d[..end], 1, 1).is_err());
    }
    for payload in [
        &[2, 2, 0, 1, 1, 0, 20, 0x81][..],
        &[2, 1, 0, 9, 1, 0, 20, 0x81],
        &[2, 1, 0, 2, 2, 0, 20, 0, 0x81],
        &[2, 1, 0, 2, 1, 0, 20],
    ] {
        let bad = gvar(payload, 16384, false, false);
        let g = Gvar::parse(&bad, 1, 1).expect("header");
        assert!(
            g.apply(
                0,
                &[Fixed::ONE],
                &mut points(),
                &[2],
                &mut [VariationPoint::default(); 7]
            )
            .is_err()
        );
    }
    for at in [4, 12, 14, 24] {
        let mut bad = d.clone();
        put16(&mut bad, at, 0xffff);
        assert!(Gvar::parse(&bad, 1, 1).is_err());
    }
    let g = Gvar::parse(&d, 1, 1).expect("header");
    assert_eq!(
        g.apply(0, &[Fixed::ONE], &mut points(), &[2], &mut []),
        Err(FontError::BufferTooSmall)
    );
}

#[test]
fn cff2_blends_match_literal_nondefault_coordinates() {
    let c = Cff::parse_table(include_bytes!("fixtures/cff2-spec.bin"), true, 1000).expect("CFF2");
    let mut out = [Command::Close; 16];
    let n = c
        .outline_instance(0, &[Fixed::from_2_14(-8192)], &mut out)
        .expect("instance");
    assert_eq!(n, 5);
    assert_eq!(
        out[0],
        Command::Move(Position {
            x: Fixed::from_i32(100),
            y: Fixed::ZERO
        })
    );
    assert_eq!(
        out[1],
        Command::Line(Position {
            x: Fixed::from_i32(500),
            y: Fixed::ZERO
        })
    );
    assert!(c.outline_instance(0, &[], &mut out).is_err());
    assert!(
        c.outline_instance(0, &[Fixed::from_i32(2)], &mut out)
            .is_err()
    );
}

#[test]
fn variable_host_font_outlines_and_advances_apply_deltas_once() {
    use crate::{Font, glyf::Glyf, variation::Instance};
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tests/fixtures/noto-sans-variable-subset.ttf"
    ))
    .expect("Noto variable subset");
    let font = Font::parse(&bytes).expect("font");
    let coords = [Fixed::ONE, Fixed::from_i32(-1)];
    let instance = Instance::new(&font, &coords).expect("instance");
    let glyf = Glyf::parse(&font).expect("glyf");
    let cmap = font.cmap().expect("cmap");
    let mut points = [Point::default(); 512];
    let mut contours = [0; 64];
    let mut scratch = vec![VariationPoint::default(); 1024];
    for (ch, width) in [('A', 574), ('g', 524), ('é', 482)] {
        let id = cmap.glyph_index(ch);
        let outline = glyf
            .outline_instance(id, &coords, &mut points, &mut contours, &mut scratch)
            .expect("varied outline");
        assert_eq!(
            instance.advance(id, Fixed::from_i32(1000), Some(&outline)),
            Ok(Fixed::from_i32(width))
        );
        if ch == 'A' {
            assert_eq!(
                (points[0].x, points[0].y),
                (Fixed::from_i32(392), Fixed::ZERO)
            );
        }
        let mut wrong = outline;
        wrong.phantoms[1].x = Fixed::from_i32(12345);
        assert_eq!(
            instance.advance(id, Fixed::from_i32(1000), Some(&wrong)),
            Ok(Fixed::from_i32(width))
        );
    }
    let without_hvar = super::metrics::sfnt(
        font.tables()
            .filter(|t| t.tag != *b"HVAR")
            .map(|t| (t.tag, t.data.to_vec()))
            .collect(),
    );
    let font = Font::parse(&without_hvar).expect("font without HVAR");
    let instance = Instance::new(&font, &coords).expect("instance");
    let id = cmap.glyph_index('A');
    let outline = Glyf::parse(&font)
        .expect("glyf")
        .outline_instance(id, &coords, &mut points, &mut contours, &mut scratch)
        .expect("phantom metrics");
    assert_eq!(
        instance.advance(id, Fixed::from_i32(1000), Some(&outline)),
        Ok(Fixed::from_i32(574))
    );
    assert_eq!(
        instance.advance(id, Fixed::ONE, None),
        Err(FontError::MissingOutline)
    );
}
