// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::arithmetic_side_effects, reason = "small fixture arithmetic")]

use crate::{
    Fixed, Font, FontError, OutlineKind,
    metrics::{Head, Maxp, Os2, Post},
};

pub(super) fn put16(data: &mut [u8], at: usize, value: u16) {
    data[at..at + 2].copy_from_slice(&value.to_be_bytes());
}
pub(super) fn put32(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

pub(super) fn head() -> Vec<u8> {
    let mut data = vec![0; 54];
    put32(&mut data, 0, 0x0001_0000);
    put32(&mut data, 12, 0x5f0f_3cf5);
    put16(&mut data, 18, 1000);
    data
}

pub(super) fn maxp() -> Vec<u8> {
    let mut data = vec![0; 32];
    put32(&mut data, 0, 0x0001_0000);
    put16(&mut data, 4, 3);
    put16(&mut data, 14, 2);
    data
}

pub(super) fn tables() -> Vec<([u8; 4], Vec<u8>)> {
    let mut hhea = vec![0; 36];
    put32(&mut hhea, 0, 0x0001_0000);
    put16(&mut hhea, 4, 800);
    put16(&mut hhea, 6, 0xff38);
    put16(&mut hhea, 8, 100);
    put16(&mut hhea, 34, 2);
    vec![
        (*b"head", head()),
        (*b"hhea", hhea),
        (*b"hmtx", vec![1, 244, 0, 10, 2, 89, 255, 236, 0, 30]),
        (*b"maxp", maxp()),
    ]
}

pub(super) fn sfnt(mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|t| t.0);
    let mut data = vec![0; 12 + tables.len() * 16];
    put32(&mut data, 0, 0x0001_0000);
    put16(
        &mut data,
        4,
        u16::try_from(tables.len()).expect("table count"),
    );
    for (i, (tag, bytes)) in tables.into_iter().enumerate() {
        while !data.len().is_multiple_of(4) {
            data.push(0);
        }
        let at = 12 + i * 16;
        let offset = u32::try_from(data.len()).expect("offset");
        data[at..at + 4].copy_from_slice(&tag);
        put32(&mut data, at + 8, offset);
        put32(
            &mut data,
            at + 12,
            u32::try_from(bytes.len()).expect("length"),
        );
        data.extend(bytes);
    }
    data
}

#[test]
fn fixed_rounding_and_overflow() {
    for (n, d, expected) in [
        (1, 2, 0),
        (3, 2, 2),
        (5, 2, 2),
        (-1, 2, 0),
        (-3, 2, -2),
        (-5, 2, -2),
        (3, -2, -2),
        (-3, -2, 2),
    ] {
        assert_eq!(
            Fixed::from_bits(n).mul_ratio(1, d).expect("ratio").bits(),
            expected
        );
    }
    assert_eq!(
        Fixed::from_i32(3)
            .checked_div(Fixed::from_i32(2))
            .expect("division"),
        Fixed::from_bits(0x0001_8000_0000)
    );
    assert_eq!(
        Fixed::from_16_16(-0x0001_8000),
        Fixed::from_bits(-0x0001_8000_0000)
    );
    assert_eq!(Fixed::from_2_14(-8192), Fixed::from_bits(-0x8000_0000));
    assert_eq!(
        Fixed::from_bits(i64::MAX).checked_add(Fixed::from_bits(1)),
        Err(FontError::Overflow)
    );
    assert_eq!(
        Fixed::from_bits(i64::MIN).checked_neg(),
        Err(FontError::Overflow)
    );
    assert_eq!(
        Fixed::from_bits(i64::MIN).checked_sub(Fixed::ONE),
        Err(FontError::Overflow)
    );
    assert_eq!(
        Fixed::from_bits(i64::MAX).checked_mul(Fixed::from_i32(2)),
        Err(FontError::Overflow)
    );
    assert_eq!(
        Fixed::ONE.checked_div(Fixed::ZERO),
        Err(FontError::InvalidTable)
    );
    assert_eq!(
        Fixed::from_bits(i64::MIN).mul_ratio(-1, 1),
        Err(FontError::Overflow)
    );
    assert_eq!(
        Fixed::from_bits(i64::MAX)
            .mul_ratio(i64::MAX, i64::MAX)
            .expect("wide"),
        Fixed::from_bits(i64::MAX)
    );
}

#[test]
fn fractional_advances_and_repeated_metric() {
    let bytes = sfnt(tables());
    let font = Font::parse(&bytes).expect("font");
    let m = font.metrics().expect("metrics");
    assert_eq!(m.horizontal(0), Ok((500, 10)));
    assert_eq!(m.horizontal(1), Ok((601, -20)));
    assert_eq!(m.horizontal(2), Ok((601, 30)));
    assert_eq!(m.horizontal(3), Err(FontError::GlyphIndex));
    assert_eq!(
        m.advance(2, Fixed::from_i32(13)).expect("advance").bits(),
        33_556_579_484
    );
    assert_eq!(
        (m.line.ascender, m.line.descender, m.line.gap),
        (800, -200, 100)
    );
    assert_eq!(
        m.scale(-200, Fixed::from_i32(13)).expect("negative").bits(),
        -11_166_914_970
    );
    assert!(font.cmap().is_err());
}

#[test]
fn header_profile_lengths_and_constraints() {
    for end in 0..54 {
        assert!(Head::parse(&head()[..end]).is_err());
    }
    for end in 0..32 {
        assert!(Maxp::parse(&maxp()[..end], OutlineKind::TrueType).is_err());
    }
    for (offset, value) in [
        (0, 2),
        (12, 0),
        (18, 15),
        (18, 16385),
        (50, 2),
        (52, 1),
        (36, 1),
        (38, 1),
    ] {
        let mut h = head();
        put16(&mut h, offset, value);
        assert!(Head::parse(&h).is_err());
    }
    for (offset, value) in [(4, 0), (14, 0), (14, 3)] {
        let mut m = maxp();
        put16(&mut m, offset, value);
        assert!(Maxp::parse(&m, OutlineKind::TrueType).is_err());
    }
    let cff = [0, 0, 0x50, 0, 0, 3];
    assert!(Maxp::parse(&cff, OutlineKind::PostScript).is_ok());
    assert!(Maxp::parse(&cff, OutlineKind::TrueType).is_err());
}

#[test]
fn metrics_truncations_and_count_relationships() {
    for table in 0..4 {
        for end in 0..tables()[table].1.len() {
            let mut ts = tables();
            ts[table].1.truncate(end);
            let bytes = sfnt(ts);
            assert!(Font::parse(&bytes).expect("envelope").metrics().is_err());
        }
    }
    for (offset, value) in [(0, 2), (24, 1), (32, 1), (34, 0), (34, 4)] {
        let mut ts = tables();
        put16(&mut ts[1].1, offset, value);
        let bytes = sfnt(ts);
        assert!(Font::parse(&bytes).expect("envelope").metrics().is_err());
    }
}

fn os2(version: u16, len: usize) -> Vec<u8> {
    let mut d = vec![0; len];
    put16(&mut d, 0, version);
    put16(&mut d, 4, 400);
    put16(&mut d, 6, 5);
    d
}

#[test]
fn os2_versions_and_line_policy() {
    for (version, len) in [
        (0, 68),
        (0, 78),
        (1, 86),
        (2, 96),
        (3, 96),
        (4, 96),
        (5, 100),
    ] {
        let d = os2(version, len);
        assert!(Os2::parse(&d).is_ok());
        for end in 0..len {
            if version != 0 || end != 68 {
                assert!(Os2::parse(&d[..end]).is_err());
            }
        }
    }
    let mut d = os2(4, 96);
    put16(&mut d, 62, 0x80);
    put16(&mut d, 68, 900);
    put16(&mut d, 70, 0xfe70);
    put16(&mut d, 72, 0xffff);
    let mut ts = tables();
    ts.push((*b"OS/2", d.clone()));
    let bytes = sfnt(ts);
    let line = Font::parse(&bytes)
        .expect("font")
        .metrics()
        .expect("metrics")
        .line;
    assert_eq!((line.ascender, line.descender, line.gap), (900, -400, 0));
    for (offset, value) in [(0, 6), (4, 0), (4, 1001), (6, 0), (6, 10)] {
        let mut bad = d.clone();
        put16(&mut bad, offset, value);
        assert!(Os2::parse(&bad).is_err());
    }
}

#[test]
fn post_versions_names_and_bounds() {
    let mut post = vec![0; 32];
    put32(&mut post, 0, 0x0003_0000);
    put32(&mut post, 4, 0xfff4_8000);
    put16(&mut post, 8, 0xff9c);
    put16(&mut post, 10, 50);
    let p = Post::parse(&post, 3).expect("post");
    assert_eq!(p.italic_angle, Fixed::from_16_16(-753_664));
    assert_eq!(p.underline_position, -100);
    for end in 0..32 {
        assert!(Post::parse(&post[..end], 3).is_err());
    }
    put32(&mut post, 0, 0x0001_0000);
    assert!(Post::parse(&post, 258).is_ok());
    assert!(Post::parse(&post, 3).is_err());
    put32(&mut post, 0, 0x0002_0000);
    post.extend([0, 3, 0, 0, 1, 2, 1, 3, 1, b'A', 3, b'A', b'_', b'B']);
    assert!(Post::parse(&post, 3).is_ok());
    for end in 32..post.len() {
        assert!(Post::parse(&post[..end], 3).is_err());
    }
    assert!(Post::parse(&post, 3).expect("post").names_valid);
    // Name content clears the flag; bounds still fail.
    post[43] = b'-';
    assert!(!Post::parse(&post, 3).expect("post").names_valid);
    post[43] = b'_';
    post[40] = 0;
    post.remove(41);
    assert!(!Post::parse(&post, 3).expect("post").names_valid);
    for end in 32..post.len() {
        assert!(Post::parse(&post[..end], 3).is_err());
    }
    post.truncate(32);
    put32(&mut post, 0, 0x0002_5000);
    post.extend([0, 3, 1, 255, 0]);
    assert!(Post::parse(&post, 3).is_ok());
    post[34] = 255;
    assert!(Post::parse(&post, 3).is_err());
}
