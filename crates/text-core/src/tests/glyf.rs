// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::arithmetic_side_effects,
    reason = "bounded fixture construction"
)]

use super::metrics::{put16, put32, sfnt, tables};
use crate::{
    Fixed, Font, FontError,
    glyf::{Glyf, MAX_DEPTH, Point},
    variation::VariationPoint,
};

pub(super) fn simple() -> Vec<u8> {
    let mut data = vec![0; 14];
    put16(&mut data, 0, 1);
    put16(&mut data, 10, 2);
    data.extend([0x37, 0x33, 0x35, 10, 20, 20, 20]);
    data
}

fn component(id: u16, flags: u16, args: &[u8]) -> Vec<u8> {
    let mut data = vec![0; 14];
    put16(&mut data, 0, 0xffff);
    put16(&mut data, 10, flags);
    put16(&mut data, 12, id);
    data.extend(args);
    data
}

pub(super) fn font(glyphs: &[Vec<u8>], long: bool) -> Vec<u8> {
    let mut ts = tables();
    let n = u16::try_from(glyphs.len()).expect("glyph count");
    put16(&mut ts[0].1, 50, u16::from(long));
    put16(&mut ts[1].1, 34, 1);
    put16(&mut ts[3].1, 4, n);
    ts[2].1 = vec![0; usize::from(n) * 2 + 2];
    put16(&mut ts[2].1, 0, 500);
    let mut data = Vec::new();
    let mut loca = Vec::new();
    for g in glyphs.iter().chain(std::iter::once(&Vec::new())) {
        if long {
            loca.extend(u32::try_from(data.len()).expect("offset").to_be_bytes());
        } else {
            loca.extend(
                u16::try_from(data.len() / 2)
                    .expect("short offset")
                    .to_be_bytes(),
            );
        }
        data.extend(g);
        if !data.len().is_multiple_of(2) {
            data.push(0);
        }
    }
    ts.push((*b"glyf", data));
    ts.push((*b"loca", loca));
    sfnt(ts)
}

fn decode(bytes: &[u8], id: u16) -> Result<(Vec<Point>, Vec<usize>), FontError> {
    let f = Font::parse(bytes)?;
    let glyf = Glyf::parse(&f)?;
    let mut ps = vec![Point::default(); 128];
    let mut cs = vec![0; 128];
    let outline = glyf.outline(id, &mut ps, &mut cs)?;
    ps.truncate(outline.points);
    cs.truncate(outline.contours);
    Ok((ps, cs))
}

fn xy(points: &[Point]) -> Vec<(i64, i64)> {
    points.iter().map(|p| (p.x.bits(), p.y.bits())).collect()
}

#[test]
fn simple_short_and_long_loca_and_empty() {
    for long in [false, true] {
        let bytes = font(&[Vec::new(), simple()], long);
        assert_eq!(decode(&bytes, 0), Ok((vec![], vec![])));
        let (ps, cs) = decode(&bytes, 1).expect("outline");
        assert_eq!(cs, [2]);
        assert_eq!(
            xy(&ps),
            [
                (10 << 32, 20 << 32),
                (30 << 32, 20 << 32),
                (30 << 32, 40 << 32)
            ]
        );
        assert!(ps.iter().all(|p| p.on_curve));
        assert_eq!(decode(&bytes, 2), Err(FontError::GlyphIndex));
    }
}

#[test]
fn repeated_flags_and_signed_long_deltas() {
    let mut s = vec![0; 14];
    put16(&mut s, 0, 1);
    put16(&mut s, 10, 2);
    s.extend([0x09, 2, 0, 10, 0xff, 0xec, 0, 20, 0, 20, 0xff, 0xec, 0, 20]);
    let (p, _) = decode(&font(&[s.clone()], false), 0).expect("repeats");
    assert_eq!(
        xy(&p),
        [(10 << 32, 20 << 32), (-10 << 32, 0), (10 << 32, 20 << 32)]
    );
    s[15] = 3;
    assert_eq!(decode(&font(&[s], false), 0), Err(FontError::InvalidTable));
}

#[test]
fn transforms_offsets_and_fractional_points() {
    let uniform = component(0, 0x000a, &[5, 255, 0x20, 0]);
    let scaled = component(0, 0x080a, &[5, 255, 0x20, 0]);
    let separate = component(0, 0x0043, &[0, 5, 0, 10, 0x20, 0, 0x60, 0]);
    let matrix = component(0, 0x0082, &[0, 0, 0, 0, 0x40, 0, 0xc0, 0, 0, 0]);
    let bytes = font(&[simple(), uniform, scaled, separate, matrix], false);
    assert_eq!(
        xy(&decode(&bytes, 1).expect("uniform").0)[0],
        (10 << 32, 9 << 32)
    );
    assert_eq!(
        xy(&decode(&bytes, 2).expect("scaled").0)[0],
        (15 << 31, 19 << 31)
    );
    assert_eq!(
        xy(&decode(&bytes, 3).expect("separate").0)[0],
        (10 << 32, 40 << 32)
    );
    assert_eq!(
        xy(&decode(&bytes, 4).expect("matrix").0)[0],
        (-20 << 32, 10 << 32)
    );
}

#[test]
fn point_matching_and_use_my_metrics() {
    let mut c = component(0, 0x22, &[5, 10]);
    c.extend([0x02, 0x00, 0, 0, 1, 0]);
    let bytes = font(&[simple(), c], false);
    let (ps, cs) = decode(&bytes, 1).expect("point match");
    assert_eq!(cs, [2, 5]);
    assert_eq!(ps[1], ps[3]);
    let face = Font::parse(&bytes).expect("font");
    let glyf = Glyf::parse(&face).expect("glyf");
    let mut points = [Point::default(); 10];
    let mut contours = [0; 10];
    let out = glyf
        .outline(1, &mut points, &mut contours)
        .expect("outline");
    assert_eq!(
        out.phantoms[1]
            .x
            .checked_sub(out.phantoms[0].x)
            .expect("advance"),
        Fixed::from_i32(500)
    );
    assert_eq!(
        glyf.outline(1, &mut [], &mut []),
        Err(FontError::BufferTooSmall)
    );
}

#[test]
fn cycles_depth_and_component_bounds() {
    assert_eq!(
        decode(&font(&[component(0, 2, &[0, 0])], false), 0),
        Err(FontError::Cycle)
    );
    assert_eq!(
        decode(
            &font(&[component(1, 2, &[0, 0]), component(0, 2, &[0, 0])], false),
            0
        ),
        Err(FontError::Cycle)
    );
    assert_eq!(
        decode(&font(&[component(7, 2, &[0, 0])], false), 0),
        Err(FontError::GlyphIndex)
    );
    let mut chain = vec![Vec::new()];
    for i in 0..MAX_DEPTH {
        chain.push(component(u16::try_from(i).expect("depth"), 2, &[0, 0]));
    }
    assert!(
        decode(
            &font(&chain, false),
            u16::try_from(MAX_DEPTH - 1).expect("depth")
        )
        .is_ok()
    );
    assert_eq!(
        decode(
            &font(&chain, false),
            u16::try_from(MAX_DEPTH).expect("depth")
        ),
        Err(FontError::LimitExceeded)
    );
    for flags in [0, 0x1802, 0x42 | 8, 0x12, 0x8002] {
        assert!(
            decode(
                &font(&[simple(), component(0, flags, &[0, 0, 0, 0, 0, 0])], false),
                1
            )
            .is_err()
        );
    }
}

#[test]
fn malformed_loca_and_truncated_glyphs() {
    for g in [simple(), component(0, 0x103, &[0, 0, 0, 0, 0, 1, 0x7f])] {
        for end in 1..g.len() {
            let mut ts = tables();
            put16(&mut ts[0].1, 50, 1);
            put16(&mut ts[3].1, 4, 2);
            let mut loca = vec![0; 12];
            put32(&mut loca, 8, u32::try_from(end).expect("length"));
            ts.push((*b"glyf", g[..end].to_vec()));
            ts.push((*b"loca", loca));
            assert!(decode(&sfnt(ts), 1).is_err(), "end {end}");
        }
    }
    let bytes = font(&[simple()], false);
    let face = Font::parse(&bytes).expect("font");
    let at = usize::try_from(face.table(*b"loca").expect("loca").offset).expect("offset");
    for (offset, value) in [(at, 100), (at + 2, 0xffff)] {
        let mut bad = bytes.clone();
        put16(&mut bad, offset, value);
        assert!(Glyf::parse(&Font::parse(&bad).expect("font")).is_err());
    }
}

#[test]
fn outline_separate_buffers_are_identical() {
    let a = font(&[simple(), component(0, 0x0a, &[5, 0, 0x20, 0])], false);
    let b = a.clone();
    let serialize = |data: &[u8]| {
        let (ps, cs) = decode(data, 1).expect("outline");
        let mut out = Vec::new();
        for p in ps {
            out.extend(p.x.bits().to_be_bytes());
            out.extend(p.y.bits().to_be_bytes());
            out.push(u8::from(p.on_curve));
        }
        for c in cs {
            out.extend(u32::try_from(c).expect("contour").to_be_bytes());
        }
        out
    };
    assert_eq!(serialize(&a), serialize(&b));
}

#[test]
fn static_face_instances_an_empty_coordinate_slice() {
    let bytes = font(&[Vec::new(), simple()], false);
    let face = Font::parse(&bytes).expect("font");
    assert!(face.table(*b"fvar").is_none());
    let glyf = Glyf::parse(&face).expect("glyf");
    let mut ps = [Point::default(); 64];
    let mut cs = [0; 64];
    let plain = glyf.outline(1, &mut ps, &mut cs).expect("outline");
    let mut qs = [Point::default(); 64];
    let mut ds = [0; 64];
    let mut work = [VariationPoint::default(); 64];
    let instance = glyf
        .outline_instance(1, &[], &mut qs, &mut ds, &mut work)
        .expect("static instance");
    assert_eq!(instance, plain);
    assert_eq!(xy(&qs[..instance.points]), xy(&ps[..plain.points]));
    assert_eq!(ds[..instance.contours], cs[..plain.contours]);
    for coords in [&[Fixed::ZERO][..], &[Fixed::ONE, Fixed::ZERO][..]] {
        assert_eq!(
            glyf.outline_instance(1, coords, &mut qs, &mut ds, &mut work),
            Err(FontError::InvalidTable)
        );
    }
}
