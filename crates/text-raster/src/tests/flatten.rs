// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::arithmetic_side_effects, reason = "bounded fixture arithmetic")]

//! R2: flattening to the tolerance of D-181, for both curve kinds.

use text_core::{
    Fixed,
    cff::{Command, Position},
    colr::Affine,
    glyf::Point,
};

use crate::{
    Edge, MAX_SEGMENTS, RasterError, TOLERANCE, cubic_segments, flatten_cff, flatten_glyf,
    quadratic_segments,
};

/// A transform that scales font units by `scale` and flips y, which is what a
/// glyph transform of R13 does.
fn scaled(scale: i64, divisor: i64) -> Affine {
    Affine {
        xx: Fixed::ONE.mul_ratio(scale, divisor).unwrap(),
        yx: Fixed::ZERO,
        xy: Fixed::ZERO,
        yy: Fixed::ONE.mul_ratio(-scale, divisor).unwrap(),
        dx: Fixed::ZERO,
        dy: Fixed::ZERO,
    }
}

fn point(x: i32, y: i32, on_curve: bool) -> Point {
    Point::new(Fixed::from_i32(x), Fixed::from_i32(y), on_curve)
}

fn at(x: i32, y: i32) -> Position {
    Position {
        x: Fixed::from_i32(x),
        y: Fixed::from_i32(y),
    }
}

/// The vertices of a flattened open curve, in Q32.32 bits: the start of the
/// first edge and then the end of each.
fn vertices(edges: &[Edge]) -> Vec<(i128, i128)> {
    let mut out = Vec::new();
    if let Some(first) = edges.first() {
        out.push((i128::from(first.x0.bits()), i128::from(first.y0.bits())));
    }
    for edge in edges {
        out.push((i128::from(edge.x1.bits()), i128::from(edge.y1.bits())));
    }
    out
}

/// The perpendicular distance from `p` to the segment `a`–`b`, in Q32.32 bits.
fn distance_to_segment(p: (i128, i128), a: (i128, i128), b: (i128, i128)) -> i128 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let square = dx * dx + dy * dy;
    if square == 0 {
        let (ex, ey) = (p.0 - a.0, p.1 - a.1);
        return (ex * ex + ey * ey).isqrt();
    }
    let cross = dx * (p.1 - a.1) - dy * (p.0 - a.0);
    cross.abs() / square.isqrt()
}

/// The largest perpendicular distance between the curve and the polyline that
/// replaced it, sampled sixteen times inside every segment.
fn deviation(edges: &[Edge], curve: &dyn Fn(i128, i128) -> (i128, i128)) -> i128 {
    let points = vertices(edges);
    let segments = i128::try_from(points.len()).unwrap() - 1;
    assert!(segments >= 1);
    let samples = segments * 16;
    let mut worst = 0_i128;
    for step in 0..=samples {
        let index = usize::try_from((step / 16).min(segments - 1)).unwrap();
        let point = curve(step, samples);
        worst = worst.max(distance_to_segment(point, points[index], points[index + 1]));
    }
    worst
}

/// A quadratic in Q32.32 bits, evaluated at `step / total`.
fn quadratic_at(
    start: (i128, i128),
    control: (i128, i128),
    end: (i128, i128),
) -> impl Fn(i128, i128) -> (i128, i128) {
    move |step, total| {
        let rest = total - step;
        let square = total * total;
        (
            (rest * rest * start.0 + 2 * rest * step * control.0 + step * step * end.0) / square,
            (rest * rest * start.1 + 2 * rest * step * control.1 + step * step * end.1) / square,
        )
    }
}

/// A cubic in Q32.32 bits, evaluated at `step / total`.
fn cubic_at(
    start: (i128, i128),
    first: (i128, i128),
    second: (i128, i128),
    end: (i128, i128),
) -> impl Fn(i128, i128) -> (i128, i128) {
    move |step, total| {
        let rest = total - step;
        let cube = total * total * total;
        let mix = |a: i128, b: i128, c: i128, d: i128| {
            (rest * rest * rest * a
                + 3 * rest * rest * step * b
                + 3 * rest * step * step * c
                + step * step * step * d)
                / cube
        };
        (
            mix(start.0, first.0, second.0, end.0),
            mix(start.1, first.1, second.1, end.1),
        )
    }
}

/// One outline point in Q32.32 device bits under `transform`.
fn placed(transform: Affine, x: i32, y: i32) -> (i128, i128) {
    let (px, py) = transform
        .apply(Fixed::from_i32(x), Fixed::from_i32(y))
        .unwrap();
    (i128::from(px.bits()), i128::from(py.bits()))
}

#[test]
fn segment_count_follows_the_closed_form() {
    // `n = ceil(sqrt(d / (4 * TOLERANCE)))` with `TOLERANCE` an eighth, so
    // `n = ceil(sqrt(2 * d))` for a quadratic and `ceil(sqrt(6 * d))` for a
    // cubic. Two device units of second difference therefore need two
    // segments, and anything above two needs three.
    assert_eq!(quadratic_segments(Fixed::ZERO), 1);
    assert_eq!(quadratic_segments(Fixed::from_i32(2)), 2);
    assert_eq!(
        quadratic_segments(
            Fixed::from_i32(2)
                .checked_add(Fixed::from_bits(1 << 20))
                .unwrap()
        ),
        3
    );
    assert_eq!(
        quadratic_segments(
            Fixed::from_i32(2)
                .checked_sub(Fixed::from_bits(1 << 20))
                .unwrap()
        ),
        2
    );
    assert_eq!(cubic_segments(Fixed::ZERO), 1);
    // `6 * d = 24` at `d = 4`, whose root is between four and five.
    assert_eq!(cubic_segments(Fixed::from_i32(4)), 5);
    // `6 * d = 36` at `d = 6`, whose root is exactly six.
    assert_eq!(cubic_segments(Fixed::from_i32(6)), 6);
}

#[test]
fn segment_count_is_clamped_at_the_maximum() {
    // The clamp bites where `2 * d` exceeds `MAX_SEGMENTS` squared.
    let clamp = i32::try_from(MAX_SEGMENTS).unwrap();
    // `2 * d` is `MAX_SEGMENTS` squared here, so the formula itself reaches the
    // maximum; one unit more is what the clamp holds back.
    assert_eq!(
        quadratic_segments(Fixed::from_i32(clamp * clamp / 2)),
        MAX_SEGMENTS
    );
    assert_eq!(
        quadratic_segments(Fixed::from_i32(clamp * clamp / 2 + 1)),
        MAX_SEGMENTS
    );
    assert_eq!(quadratic_segments(Fixed::from_i32(32_000)), 253);
    assert_eq!(quadratic_segments(Fixed::from_bits(i64::MAX)), MAX_SEGMENTS);
    assert_eq!(cubic_segments(Fixed::from_bits(i64::MAX)), MAX_SEGMENTS);
    assert_eq!(quadratic_segments(Fixed::from_i32(-1)), 1);
}

#[test]
fn segment_count_doubles_when_the_size_quadruples() {
    // The count scales as the square root of the size, because the second
    // difference is measured after the transform.
    let base = quadratic_segments(Fixed::from_i32(32));
    let four = quadratic_segments(Fixed::from_i32(128));
    assert_eq!(four, base * 2);
}

#[test]
fn quadratic_stays_within_the_tolerance_at_four_sizes() {
    // A quarter circle of one em, as the one quadratic a TrueType face states
    // it with, at four sizes.
    for size in [8_i64, 16, 64, 256] {
        let points = [
            point(1000, 0, true),
            point(1000, 552, false),
            point(0, 1000, true),
        ];
        let mut edges = vec![Edge::default(); 1024];
        let transform = scaled(size, 1000);
        let count = flatten_glyf(&points, &[2], transform, &mut edges).unwrap();
        // The last edge closes the contour and is not part of the curve.
        let curve = &edges[..count - 1];
        let worst = deviation(
            curve,
            &quadratic_at(
                placed(transform, 1000, 0),
                placed(transform, 1000, 552),
                placed(transform, 0, 1000),
            ),
        );
        assert!(
            worst <= i128::from(TOLERANCE.bits()),
            "size {size} deviated {worst} of {}",
            TOLERANCE.bits()
        );
        // The count follows the size: four times the size, twice the segments.
        assert!(!curve.is_empty());
    }
}

#[test]
fn the_segment_count_doubles_with_four_times_the_size() {
    let points = [
        point(1000, 0, true),
        point(1000, 552, false),
        point(0, 1000, true),
    ];
    let mut edges = vec![Edge::default(); 1024];
    let small = flatten_glyf(&points, &[2], scaled(16, 1000), &mut edges).unwrap();
    let large = flatten_glyf(&points, &[2], scaled(64, 1000), &mut edges).unwrap();
    assert_eq!((large - 1) * 10, (small - 1) * 20);
}

#[test]
fn quadratic_with_coincident_control_points_gives_no_edge() {
    let points = [
        point(0, 0, true),
        point(0, 0, false),
        point(0, 0, true),
        point(10, 0, true),
    ];
    let mut edges = [Edge::default(); 16];
    let count = flatten_glyf(&points, &[3], scaled(1, 1), &mut edges).unwrap();
    // The degenerate quadratic encloses nothing and is dropped; what is left
    // is the line out to the last point and the closing edge back.
    assert_eq!(count, 2);
    assert_eq!(edges[0].x1, Fixed::from_i32(10));
    assert_eq!(edges[1].x1, Fixed::ZERO);
}

#[test]
fn contour_of_one_point_produces_no_edge() {
    let points = [point(5, 5, true)];
    let mut edges = [Edge::default(); 4];
    assert_eq!(
        flatten_glyf(&points, &[0], scaled(1, 1), &mut edges).unwrap(),
        0
    );
    let commands = [Command::Move(at(5, 5)), Command::Close];
    assert_eq!(flatten_cff(&commands, scaled(1, 1), &mut edges).unwrap(), 0);
}

#[test]
fn contour_without_an_on_curve_point_starts_at_an_implied_midpoint() {
    // Four off-curve points describe a rounded square whose corners are the
    // implied midpoints of adjacent control points.
    let points = [
        point(-100, -100, false),
        point(100, -100, false),
        point(100, 100, false),
        point(-100, 100, false),
    ];
    let mut edges = vec![Edge::default(); 1024];
    let count = flatten_glyf(&points, &[3], scaled(1, 1), &mut edges).unwrap();
    assert!(count > 4);
    // The polyline closes: the last endpoint is the first start.
    assert_eq!(edges[0].x0, edges[count - 1].x1);
    assert_eq!(edges[0].y0, edges[count - 1].y1);
    // The start is the midpoint of the last and first control points.
    assert_eq!(edges[0].x0, Fixed::from_i32(-100));
    assert_eq!(edges[0].y0, Fixed::ZERO);
}

#[test]
fn a_square_closes_without_a_closing_command() {
    let points = [
        point(0, 0, true),
        point(10, 0, true),
        point(10, 10, true),
        point(0, 10, true),
    ];
    let mut edges = [Edge::default(); 16];
    let count = flatten_glyf(&points, &[3], scaled(1, 1), &mut edges).unwrap();
    assert_eq!(count, 4);
    assert_eq!(edges[3].x1, edges[0].x0);
    assert_eq!(edges[3].y1, edges[0].y0);
}

#[test]
fn two_contours_are_both_closed() {
    let points = [
        point(0, 0, true),
        point(10, 0, true),
        point(10, 10, true),
        point(2, 2, true),
        point(8, 2, true),
        point(8, 8, true),
    ];
    let mut edges = [Edge::default(); 16];
    let count = flatten_glyf(&points, &[2, 5], scaled(1, 1), &mut edges).unwrap();
    assert_eq!(count, 6);
    assert_eq!(edges[2].x1, edges[0].x0);
    assert_eq!(edges[5].x1, edges[3].x0);
}

#[test]
fn contour_ends_that_do_not_partition_the_points_are_refused() {
    let points = [point(0, 0, true), point(1, 1, true)];
    let mut edges = [Edge::default(); 16];
    for ends in [&[9_usize][..], &[1, 0][..], &[0, 0][..]] {
        assert!(matches!(
            flatten_glyf(&points, ends, scaled(1, 1), &mut edges),
            Err(RasterError::Font(_))
        ));
    }
}

#[test]
fn cubic_stays_within_the_tolerance_at_four_sizes() {
    for size in [8_i64, 16, 64, 256] {
        let commands = [
            Command::Move(at(1000, 0)),
            Command::Curve(at(1000, 552), at(552, 1000), at(0, 1000)),
        ];
        let mut edges = vec![Edge::default(); 1024];
        let transform = scaled(size, 1000);
        let count = flatten_cff(&commands, transform, &mut edges).unwrap();
        let curve = &edges[..count - 1];
        let worst = deviation(
            curve,
            &cubic_at(
                placed(transform, 1000, 0),
                placed(transform, 1000, 552),
                placed(transform, 552, 1000),
                placed(transform, 0, 1000),
            ),
        );
        assert!(
            worst <= i128::from(TOLERANCE.bits()),
            "size {size} deviated {worst} of {}",
            TOLERANCE.bits()
        );
    }
}

#[test]
fn cff_close_returns_to_the_contour_start() {
    let commands = [
        Command::Move(at(0, 0)),
        Command::Line(at(10, 0)),
        Command::Line(at(10, 10)),
        Command::Close,
        Command::Move(at(20, 20)),
        Command::Line(at(30, 20)),
    ];
    let mut edges = [Edge::default(); 16];
    let count = flatten_cff(&commands, scaled(1, 1), &mut edges).unwrap();
    // Three edges for the first contour, two for the second.
    assert_eq!(count, 5);
    assert_eq!(edges[2].x1, Fixed::ZERO);
    assert_eq!(edges[4].x1, Fixed::from_i32(20));
}

#[test]
fn a_buffer_too_small_is_refused() {
    let points = [
        point(0, 0, true),
        point(10, 0, true),
        point(10, 10, true),
        point(0, 10, true),
    ];
    let mut edges = [Edge::default(); 3];
    assert_eq!(
        flatten_glyf(&points, &[3], scaled(1, 1), &mut edges).unwrap_err(),
        RasterError::BufferTooSmall
    );
    let commands = [Command::Move(at(0, 0)), Command::Line(at(1, 1))];
    let mut none: [Edge; 0] = [];
    assert_eq!(
        flatten_cff(&commands, scaled(1, 1), &mut none).unwrap_err(),
        RasterError::BufferTooSmall
    );
}

#[test]
fn the_same_outline_flattens_to_identical_edges() {
    let points = [
        point(0, 0, true),
        point(300, 700, false),
        point(600, 0, true),
    ];
    let mut first = vec![Edge::default(); 256];
    let mut second = vec![Edge::default(); 512];
    let transform = scaled(32, 1000);
    let a = flatten_glyf(&points, &[2], transform, &mut first).unwrap();
    let b = flatten_glyf(&points, &[2], transform, &mut second).unwrap();
    assert_eq!(a, b);
    assert_eq!(first[..a], second[..b]);
}

#[test]
fn a_curve_beyond_the_arithmetic_takes_the_clamp() {
    // Regression: the length of the second difference was computed with
    // checked products, so a curve whose control points are further apart
    // than the square root of Q32.32 returned an error rather than taking the
    // clamp D-181 states.
    let far = i32::try_from(1_i64 << 20).unwrap();
    let points = [point(0, 0, true), point(far, far, false), point(1, 1, true)];
    let mut edges = vec![Edge::default(); 1024];
    let count = flatten_glyf(&points, &[2], scaled(1, 1), &mut edges).unwrap();
    // The clamp gives 256 segments, and the closing edge makes one more.
    assert_eq!(count, usize::try_from(MAX_SEGMENTS).unwrap() + 1);
}

#[test]
fn a_quadratic_whose_control_point_lies_off_the_chord_stays_within_the_tolerance() {
    // Regression: the segment count of a quadratic came from
    // `max(|start - control|, |end - control|)` instead of the curve's own
    // second difference `start - 2*control + end`. The two agree only when
    // the control point sits on the chord's perpendicular bisector; for a
    // control point the start and the end both point away from, the count was
    // a factor of `sqrt(2)` too small and the polyline left the tolerance.
    for (control, end) in [
        ((10, 10), (0, 20)),
        ((20, 20), (0, 30)),
        ((40, 5), (1, 10)),
        ((95, 40), (10, 60)),
    ] {
        let points = [
            point(0, 0, true),
            point(control.0, control.1, false),
            point(end.0, end.1, true),
        ];
        let mut edges = vec![Edge::default(); 1024];
        let transform = scaled(1, 1);
        let count = flatten_glyf(&points, &[2], transform, &mut edges).unwrap();
        // The last edge closes the contour and is not part of the curve.
        let worst = deviation(
            &edges[..count - 1],
            &quadratic_at(
                placed(transform, 0, 0),
                placed(transform, control.0, control.1),
                placed(transform, end.0, end.1),
            ),
        );
        assert!(
            worst <= i128::from(TOLERANCE.bits()),
            "control {control:?} end {end:?} deviated {worst} of {}",
            TOLERANCE.bits()
        );
    }
}

#[test]
fn a_flat_quadratic_takes_the_segments_its_second_difference_asks_for() {
    // Regression: a nearly straight quadratic has a second difference of two
    // units and needs two segments, but the chord-based count gave eleven.
    let points = [point(0, 0, true), point(50, 1, false), point(100, 0, true)];
    let mut edges = vec![Edge::default(); 1024];
    let count = flatten_glyf(&points, &[2], scaled(1, 1), &mut edges).unwrap();
    assert_eq!(quadratic_segments(Fixed::from_i32(2)), 2);
    // Two segments of curve and one closing edge.
    assert_eq!(count, 3);
}
