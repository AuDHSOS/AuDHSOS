// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::arithmetic_side_effects, reason = "bounded fixture arithmetic")]

//! R3: exact-area coverage under the non-zero winding rule.

use text_core::Fixed;

use crate::{Cell, Edge, Format, RasterError, Surface, Texel, fill};

/// A rectangle as four edges, wound clockwise in device space.
fn rectangle(left: Fixed, top: Fixed, right: Fixed, bottom: Fixed) -> [Edge; 4] {
    [
        Edge {
            x0: left,
            y0: top,
            x1: right,
            y1: top,
        },
        Edge {
            x0: right,
            y0: top,
            x1: right,
            y1: bottom,
        },
        Edge {
            x0: right,
            y0: bottom,
            x1: left,
            y1: bottom,
        },
        Edge {
            x0: left,
            y0: bottom,
            x1: left,
            y1: top,
        },
    ]
}

/// The same rectangle wound the other way.
fn reversed(edges: [Edge; 4]) -> [Edge; 4] {
    edges.map(|edge| Edge {
        x0: edge.x1,
        y0: edge.y1,
        x1: edge.x0,
        y1: edge.y0,
    })
}

fn whole(value: i32) -> Fixed {
    Fixed::from_i32(value)
}

/// `numerator / denominator` of a pixel.
fn part(numerator: i64, denominator: i64) -> Fixed {
    Fixed::ONE.mul_ratio(numerator, denominator).unwrap()
}

/// Fill `edges` into a fresh mask of this size and return its bytes.
fn coverage(edges: &mut [Edge], width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0_u8; usize::try_from(width * height).unwrap()];
    let mut cells = vec![Cell::default(); usize::try_from(width).unwrap() + 1];
    let mut mask = Surface::new(&mut bytes, width, height, width, Format::A8).unwrap();
    fill(edges, &mut mask, &mut cells).unwrap();
    bytes
}

#[test]
fn a_whole_pixel_rectangle_is_opaque_inside_and_clear_outside() {
    let mut edges = rectangle(whole(1), whole(1), whole(3), whole(3));
    let bytes = coverage(&mut edges, 4, 4);
    for y in 0..4_usize {
        for x in 0..4_usize {
            let inside = (1..3).contains(&x) && (1..3).contains(&y);
            assert_eq!(
                bytes[y * 4 + x],
                if inside { 255 } else { 0 },
                "pixel ({x}, {y})"
            );
        }
    }
}

#[test]
fn a_rectangle_inset_by_a_fraction_covers_that_fraction() {
    for numerator in [1_i64, 2, 3] {
        let inset = part(numerator, 4);
        let mut edges = rectangle(
            inset,
            inset,
            whole(3).checked_sub(inset).unwrap(),
            whole(3).checked_sub(inset).unwrap(),
        );
        let bytes = coverage(&mut edges, 3, 3);
        // The middle pixel is whole; the four sides carry the fraction the
        // inset leaves, and the corners carry its square.
        assert_eq!(bytes[4], 255, "{numerator}/4 middle");
        // A side pixel carries the part of it the inset leaves, rounded to
        // nearest with a tie away from zero.
        let side = u8::try_from(((4 - numerator) * 255 + 2) / 4).unwrap();
        for index in [1_usize, 3, 5, 7] {
            assert_eq!(bytes[index], side, "{numerator}/4 side at {index}");
        }
        let corner = u8::try_from((u32::from(side) * u32::from(side) + 127) / 255).unwrap();
        for index in [0_usize, 2, 6, 8] {
            assert!(
                bytes[index].abs_diff(corner) <= 1,
                "{numerator}/4 corner at {index}: {} against {corner}",
                bytes[index]
            );
        }
    }
}

#[test]
fn a_half_pixel_column_covers_half_of_its_pixels() {
    let mut edges = rectangle(part(1, 2), whole(0), whole(1), whole(2));
    let bytes = coverage(&mut edges, 2, 2);
    assert_eq!(bytes, [128, 0, 128, 0]);
}

#[test]
fn coverage_sums_to_the_area_of_a_triangle() {
    // A right triangle of legs eight, whose area is thirty-two pixels.
    let mut edges = [
        Edge {
            x0: whole(0),
            y0: whole(0),
            x1: whole(8),
            y1: whole(8),
        },
        Edge {
            x0: whole(8),
            y0: whole(8),
            x1: whole(0),
            y1: whole(8),
        },
        Edge {
            x0: whole(0),
            y0: whole(8),
            x1: whole(0),
            y1: whole(0),
        },
    ];
    let bytes = coverage(&mut edges, 8, 8);
    let total: u32 = bytes.iter().map(|byte| u32::from(*byte)).sum();
    let expected = 32 * 255;
    assert!(
        total.abs_diff(expected) <= u32::try_from(bytes.len()).unwrap(),
        "{total} against {expected}"
    );
}

#[test]
fn two_contours_of_one_direction_fill_their_overlap() {
    let mut edges = rectangle(whole(0), whole(0), whole(3), whole(3)).to_vec();
    edges.extend(rectangle(whole(1), whole(1), whole(4), whole(4)));
    let bytes = coverage(&mut edges, 4, 4);
    // The non-zero rule fills the union, the overlap included.
    for y in 0..4_usize {
        for x in 0..4_usize {
            let inside = (x < 3 && y < 3) || (x >= 1 && y >= 1);
            assert_eq!(bytes[y * 4 + x], if inside { 255 } else { 0 }, "({x}, {y})");
        }
    }
}

#[test]
fn two_contours_of_opposite_directions_clear_their_overlap() {
    let mut edges = rectangle(whole(0), whole(0), whole(4), whole(4)).to_vec();
    edges.extend(reversed(rectangle(whole(1), whole(1), whole(3), whole(3))));
    let bytes = coverage(&mut edges, 4, 4);
    for y in 0..4_usize {
        for x in 0..4_usize {
            let hole = (1..3).contains(&x) && (1..3).contains(&y);
            assert_eq!(bytes[y * 4 + x], if hole { 0 } else { 255 }, "({x}, {y})");
        }
    }
}

#[test]
fn a_doubly_wound_contour_does_not_exceed_full_coverage() {
    let mut edges = rectangle(whole(0), whole(0), whole(2), whole(2)).to_vec();
    edges.extend(rectangle(whole(0), whole(0), whole(2), whole(2)));
    let bytes = coverage(&mut edges, 2, 2);
    assert_eq!(bytes, [255; 4]);
}

#[test]
fn malformed_geometry_draws_and_does_not_panic() {
    let cases: [Vec<Edge>; 5] = [
        // Self-intersecting: a bow tie.
        vec![
            Edge {
                x0: whole(0),
                y0: whole(0),
                x1: whole(4),
                y1: whole(4),
            },
            Edge {
                x0: whole(4),
                y0: whole(4),
                x1: whole(4),
                y1: whole(0),
            },
            Edge {
                x0: whole(4),
                y0: whole(0),
                x1: whole(0),
                y1: whole(4),
            },
            Edge {
                x0: whole(0),
                y0: whole(4),
                x1: whole(0),
                y1: whole(0),
            },
        ],
        // A contour that does not return to its start.
        vec![
            Edge {
                x0: whole(0),
                y0: whole(0),
                x1: whole(4),
                y1: whole(0),
            },
            Edge {
                x0: whole(4),
                y0: whole(0),
                x1: whole(4),
                y1: whole(4),
            },
        ],
        // A single point.
        vec![Edge {
            x0: whole(2),
            y0: whole(2),
            x1: whole(2),
            y1: whole(2),
        }],
        // Two identical points, twice.
        vec![
            Edge {
                x0: whole(1),
                y0: whole(1),
                x1: whole(1),
                y1: whole(1),
            },
            Edge {
                x0: whole(1),
                y0: whole(1),
                x1: whole(1),
                y1: whole(1),
            },
        ],
        // Horizontal edges only.
        vec![
            Edge {
                x0: whole(0),
                y0: whole(2),
                x1: whole(4),
                y1: whole(2),
            },
            Edge {
                x0: whole(4),
                y0: whole(2),
                x1: whole(0),
                y1: whole(2),
            },
        ],
    ];
    for (index, mut edges) in cases.into_iter().enumerate() {
        let bytes = coverage(&mut edges, 4, 4);
        assert_eq!(bytes.len(), 16, "case {index}");
        if index >= 2 {
            assert!(bytes.iter().all(|byte| *byte == 0), "case {index} inked");
        }
    }
}

#[test]
fn nothing_outside_the_mask_is_written() {
    // The rectangle overhangs every edge of a mask with padding on each row.
    let mut bytes = vec![0xa5_u8; 5 * 4];
    let mut cells = vec![Cell::default(); 4];
    {
        let mut mask = Surface::new(&mut bytes, 3, 4, 5, Format::A8).unwrap();
        let mut edges = rectangle(whole(-4), whole(-4), whole(8), whole(8));
        fill(&mut edges, &mut mask, &mut cells).unwrap();
    }
    for row in 0..4_usize {
        assert_eq!(&bytes[row * 5..row * 5 + 3], &[255, 255, 255]);
        assert_eq!(&bytes[row * 5 + 3..row * 5 + 5], &[0xa5, 0xa5]);
    }
}

#[test]
fn geometry_entirely_outside_the_mask_leaves_it_clear() {
    for rect in [
        rectangle(whole(-8), whole(-8), whole(-4), whole(-4)),
        rectangle(whole(8), whole(8), whole(12), whole(12)),
        rectangle(whole(-8), whole(0), whole(-4), whole(4)),
    ] {
        let mut edges = rect;
        assert_eq!(coverage(&mut edges, 4, 4), [0; 16]);
    }
}

#[test]
fn a_column_left_of_the_mask_still_fills_it() {
    // An edge left of the first pixel bounds the whole mask on its right.
    let mut edges = rectangle(whole(-4), whole(0), whole(2), whole(4));
    assert_eq!(coverage(&mut edges, 2, 2), [255; 4]);
}

#[test]
fn a_mask_of_another_format_is_refused() {
    let mut bytes = vec![0_u8; 64];
    let mut cells = vec![Cell::default(); 8];
    let mut mask = Surface::new(&mut bytes, 4, 4, 16, Format::Rgbx8888).unwrap();
    let mut edges = rectangle(whole(0), whole(0), whole(2), whole(2));
    assert_eq!(
        fill(&mut edges, &mut mask, &mut cells).unwrap_err(),
        RasterError::Format
    );
}

#[test]
fn a_cell_buffer_too_small_is_refused() {
    let mut bytes = vec![0_u8; 16];
    let mut cells = vec![Cell::default(); 4];
    let mut mask = Surface::new(&mut bytes, 4, 4, 4, Format::A8).unwrap();
    let mut edges = rectangle(whole(0), whole(0), whole(2), whole(2));
    assert_eq!(
        fill(&mut edges, &mut mask, &mut cells).unwrap_err(),
        RasterError::BufferTooSmall
    );
}

#[test]
fn the_same_edges_fill_identically_twice() {
    let build = || {
        let mut edges = rectangle(part(3, 8), part(5, 8), part(29, 8), part(27, 8)).to_vec();
        edges.extend(reversed(rectangle(
            part(9, 8),
            part(11, 8),
            part(21, 8),
            part(19, 8),
        )));
        edges
    };
    let mut first = build();
    let mut second = build();
    second.reverse();
    assert_eq!(coverage(&mut first, 5, 5), coverage(&mut second, 5, 5));
}

#[test]
fn a_surface_texel_reads_back_the_coverage() {
    let mut bytes = vec![0_u8; 4];
    let mut cells = vec![Cell::default(); 3];
    let mut mask = Surface::new(&mut bytes, 2, 2, 2, Format::A8).unwrap();
    let mut edges = rectangle(whole(0), whole(0), whole(1), whole(2));
    fill(&mut edges, &mut mask, &mut cells).unwrap();
    assert_eq!(mask.texel(0, 0), Some(Texel::Coverage(255)));
    assert_eq!(mask.texel(1, 0), Some(Texel::Coverage(0)));
}

#[test]
fn a_shape_and_its_mirror_cover_mirrored_pixels() {
    // Regression: the interpolation of an edge's x at a row boundary biased
    // a half-tie by the sign of the denominator as well as the numerator, so
    // an edge running one way rounded differently from the same edge running
    // the other. Mirroring a shape must mirror its coverage exactly.
    let slant = |flip: bool| {
        let sign = if flip { -1 } else { 1 };
        let at = |x: i32| whole(4).checked_add(whole(sign * x)).unwrap();
        vec![
            Edge {
                x0: at(-3),
                y0: whole(0),
                x1: at(3),
                y1: whole(6),
            },
            Edge {
                x0: at(3),
                y0: whole(6),
                x1: at(3),
                y1: whole(0),
            },
            Edge {
                x0: at(3),
                y0: whole(0),
                x1: at(-3),
                y1: whole(0),
            },
        ]
    };
    let forward = coverage(&mut slant(false), 8, 6);
    let mirrored = coverage(&mut slant(true), 8, 6);
    for row in 0..6_usize {
        let left = &forward[row * 8..row * 8 + 8];
        let right: Vec<u8> = mirrored[row * 8..row * 8 + 8]
            .iter()
            .rev()
            .copied()
            .collect();
        assert_eq!(left, &right[..], "row {row} is not mirrored");
    }
}
