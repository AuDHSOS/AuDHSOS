// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use std::fmt::Write;

use crate::{
    Fixed, Font,
    cff::Cff,
    glyf::Glyf,
    variation::{Axes, Instance, VariationPoint},
};

// Rebuild borrowed structures after copying every attacker-controlled byte.
fn exercise(bytes: &[u8]) -> String {
    let mut result = String::new();
    let parsed = Font::parse(bytes);
    if let Ok(font) = parsed {
        let _ = write!(
            result,
            "{:?}",
            font.metrics().map(|m| (m.head, m.maxp, m.line))
        );
        if let Ok(cmap) = font.cmap() {
            for ch in ['A', 'é', '中', '\u{ffff}', '\u{10ffff}'] {
                let _ = write!(result, "{:?}", cmap.glyph_index(ch));
            }
        }
        let mut coords = [Fixed::ZERO; 64];
        let count = font
            .table(*b"fvar")
            .and_then(|t| Axes::parse(t.data, font.table(*b"avar").map(|t| t.data)).ok())
            .map_or(0, Axes::len);
        for value in [Fixed::ZERO, Fixed::ONE, Fixed::from_i32(-1)] {
            coords.fill(value);
            let normalized = &coords[..count];
            if let Ok(instance) = Instance::new(&font, normalized) {
                let _ = write!(result, "{:?}", instance.line_metrics(Fixed::from_i32(17)));
                for id in 0..5 {
                    let _ = write!(
                        result,
                        "{:?}",
                        instance.advance(id, Fixed::from_i32(17), None)
                    );
                }
            }
            if let Ok(glyf) = Glyf::parse(&font) {
                let mut points = vec![crate::glyf::Point::default(); 512];
                let mut ends = [0; 64];
                let mut work = vec![VariationPoint::default(); 1024];
                for id in 0..5 {
                    let outline = if count == 0 {
                        glyf.outline(id, &mut points, &mut ends)
                    } else {
                        glyf.outline_instance(id, normalized, &mut points, &mut ends, &mut work)
                    };
                    let _ = write!(result, "{outline:?}");
                    if let Ok(outline) = outline {
                        let _ = write!(
                            result,
                            "{:?}{:?}",
                            &points[..outline.points],
                            &ends[..outline.contours]
                        );
                    }
                }
            }
        }
        if let Ok(cff) = Cff::parse(&font) {
            cff_paths(&cff, &mut result);
        }
    } else {
        let _ = write!(result, "{parsed:?}");
    }
    result
}
fn cff_paths(cff: &Cff<'_>, out: &mut String) {
    let mut commands = vec![crate::cff::Command::Close; 512];
    for id in 0..5 {
        for coord in [
            None,
            Some(Fixed::ONE),
            Some(Fixed::from_i32(-1)),
            Some(Fixed::from_2_14(8192)),
        ] {
            let path = match coord {
                None => cff.outline(id, &mut commands),
                Some(c) => cff.outline_instance(id, &[c], &mut commands),
            };
            let _ = write!(out, "{path:?}");
            if let Ok(n) = path {
                let _ = write!(out, "{:?}", &commands[..n]);
            }
        }
    }
}

#[test]
fn hostile_font_bytes_are_bounded_and_deterministic() {
    for original in [
        include_bytes!("fixtures/noto-cjk-subset.otf").as_slice(),
        include_bytes!("fixtures/noto-sans-variable-subset.ttf").as_slice(),
    ] {
        let mut bytes = original.to_vec();
        for at in 0..original.len() {
            for value in [0, 1, 127, 128, 255] {
                bytes[at] = value;
                let first = exercise(&bytes);
                let separate = bytes.clone();
                assert_eq!(
                    first.as_bytes(),
                    exercise(&separate).as_bytes(),
                    "byte {at}, value {value}"
                );
            }
            bytes[at] = original[at];
        }
    }
}
#[test]
fn hostile_cff2_bytes_are_bounded_and_deterministic() {
    let original = include_bytes!("fixtures/cff2-spec.bin");
    let mut bytes = original.to_vec();
    for at in 0..original.len() {
        for value in [0, 1, 63, 127, 128, 192, 255] {
            bytes[at] = value;
            let run = |data: &[u8]| {
                let mut out = String::new();
                if let Ok(cff) = Cff::parse_table(data, true, 1000) {
                    cff_paths(&cff, &mut out);
                }
                out
            };
            assert_eq!(run(&bytes), run(&bytes.clone()));
        }
        bytes[at] = original[at];
    }
}
