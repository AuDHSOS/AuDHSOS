// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::unreachable, reason = "a texel of a format the test built")]

//! R9: the three gradient shapes, the three extend modes, and interpolation on
//! premultiplied linear-light values.

use text_core::{
    Fixed,
    colr::{Color, ColorLine, ColorSource, ColorStop, Extend, Fill},
};

use crate::{Affine, Gamma, Gradient, Invertible, LINEAR_ONE, Paint, Pixel};

fn whole(value: i32) -> Fixed {
    Fixed::from_i32(value)
}

fn part(numerator: i64, denominator: i64) -> Fixed {
    Fixed::ONE.mul_ratio(numerator, denominator).unwrap()
}

fn stop(offset: Fixed, red: u8, green: u8, blue: u8) -> ColorStop {
    ColorStop {
        offset,
        color: Color {
            source: ColorSource::Palette { red, green, blue },
            alpha: Fixed::ONE,
        },
    }
}

fn line(count: usize, extend: Extend) -> ColorLine {
    ColorLine {
        extend,
        first: 0,
        count,
    }
}

/// Black to white, so that one channel reads the parameter directly.
fn ramp(extend: Extend) -> ([ColorStop; 2], ColorLine) {
    (
        [stop(Fixed::ZERO, 0, 0, 0), stop(Fixed::ONE, 255, 255, 255)],
        line(2, extend),
    )
}

fn context() -> Gamma {
    Gamma::default_value().unwrap()
}

/// The red channel of the gradient at a device point, as a linear level.
fn red_at(gradient: &Gradient<'_>, x: i32, y: i32) -> Option<u32> {
    gradient
        .at(whole(x), whole(y), Paint::opaque(255, 0, 0), &context())
        .unwrap()
        .map(|pixel| pixel.red)
}

#[test]
fn a_linear_gradient_reads_its_parameter_along_p0_p3() {
    let (stops, colours) = ramp(Extend::Pad);
    let fill = Fill::Linear {
        x0: whole(0),
        y0: whole(0),
        x1: whole(16),
        y1: whole(0),
        x2: whole(0),
        y2: whole(16),
        line: colours,
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    for step in 0..=16_i32 {
        let level = red_at(&gradient, step, 0).unwrap();
        let expected =
            u32::try_from(u64::from(LINEAR_ONE) * u64::try_from(step).unwrap() / 16).unwrap();
        assert!(
            level.abs_diff(expected) <= 64,
            "at {step}: {level} of {expected}"
        );
        // A point off the gradient line takes the same parameter, because the
        // parameter is a projection.
        assert_eq!(red_at(&gradient, step, 7), Some(level));
    }
}

#[test]
fn a_rotated_linear_gradient_uses_p3_and_not_a_naive_projection() {
    // p2 is not perpendicular to p0p1, so p3 differs from p1: the parameter
    // reaches one only where the perpendicular through p3 crosses.
    let (stops, colours) = ramp(Extend::Pad);
    let fill = Fill::Linear {
        x0: whole(0),
        y0: whole(0),
        x1: whole(16),
        y1: whole(0),
        x2: whole(16),
        y2: whole(16),
        line: colours,
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    // p0p2 is the diagonal, so its perpendicular is the other diagonal and p3
    // is the projection of (16, 0) onto it, which is (8, -8). The parameter is
    // therefore one at (8, -8) and a half at (4, -4).
    assert_eq!(red_at(&gradient, 8, -8), Some(LINEAR_ONE));
    let half = red_at(&gradient, 4, -4).unwrap();
    assert!(half.abs_diff(LINEAR_ONE / 2) <= 64, "{half}");
    // A naive projection onto p0p1 would make (16, 0) the end; here it is
    // already past it and pads.
    assert_eq!(red_at(&gradient, 16, 0), Some(LINEAR_ONE));
}

#[test]
fn a_linear_gradient_whose_p3_falls_on_p0_draws_nothing() {
    let (stops, colours) = ramp(Extend::Pad);
    for (x1, y1, x2, y2) in [
        (0, 0, 0, 16),  // p1 sits on p0
        (16, 0, 32, 0), // p0p1 is parallel to p0p2
    ] {
        let fill = Fill::Linear {
            x0: whole(0),
            y0: whole(0),
            x1: whole(x1),
            y1: whole(y1),
            x2: whole(x2),
            y2: whole(y2),
            line: colours,
        };
        assert!(
            Gradient::new(fill, &stops, Affine::IDENTITY)
                .unwrap()
                .is_none(),
            "({x1}, {y1}) with ({x2}, {y2})"
        );
    }
}

#[test]
fn a_radial_gradient_reads_the_distance_between_two_circles() {
    let (stops, colours) = ramp(Extend::Pad);
    let fill = Fill::Radial {
        x0: whole(0),
        y0: whole(0),
        r0: whole(0),
        x1: whole(0),
        y1: whole(0),
        r1: whole(16),
        line: colours,
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    // Concentric circles: the parameter is the distance over the radius.
    assert_eq!(red_at(&gradient, 0, 0), Some(0));
    for step in [4_i32, 8, 12, 16] {
        let level = red_at(&gradient, step, 0).unwrap();
        let expected =
            u32::try_from(u64::from(LINEAR_ONE) * u64::try_from(step).unwrap() / 16).unwrap();
        assert!(
            level.abs_diff(expected) <= 64,
            "at {step}: {level} of {expected}"
        );
    }
    // Outside the last circle the pad mode holds the end colour.
    assert_eq!(red_at(&gradient, 40, 0), Some(LINEAR_ONE));
}

#[test]
fn a_radial_cone_no_circle_reaches_draws_nothing() {
    // Two circles of the same radius whose centres differ describe a strip;
    // nothing outside it is on any circle of the family.
    let (stops, colours) = ramp(Extend::Pad);
    let fill = Fill::Radial {
        x0: whole(0),
        y0: whole(0),
        r0: whole(2),
        x1: whole(32),
        y1: whole(0),
        r1: whole(2),
        line: colours,
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    assert!(red_at(&gradient, 16, 0).is_some());
    assert!(red_at(&gradient, 16, 1).is_some());
    assert!(red_at(&gradient, 16, 40).is_none(), "far above the strip");
    assert!(red_at(&gradient, 16, -40).is_none(), "far below the strip");
}

#[test]
fn a_radial_root_with_a_negative_radius_is_rejected() {
    // A shrinking family: past the smaller circle the interpolated radius goes
    // negative, so that root is not taken.
    let (stops, colours) = ramp(Extend::Pad);
    let fill = Fill::Radial {
        x0: whole(0),
        y0: whole(0),
        r0: whole(16),
        x1: whole(0),
        y1: whole(0),
        r1: whole(0),
        line: colours,
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    // At the centre the only circle through the point is the last one.
    assert_eq!(red_at(&gradient, 0, 0), Some(LINEAR_ONE));
    // Halfway out the parameter is a half.
    let half = red_at(&gradient, 8, 0).unwrap();
    assert!(half.abs_diff(LINEAR_ONE / 2) <= 64, "{half}");
}

#[test]
fn a_sweep_gradient_reads_the_angle_about_its_centre() {
    let (stops, colours) = ramp(Extend::Pad);
    let fill = Fill::Sweep {
        x: whole(0),
        y: whole(0),
        start: Fixed::ZERO,
        end: whole(2),
        line: colours,
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    // Counter-clockwise from the positive x axis, over a whole turn.
    for (x, y, turns) in [(16_i32, 0_i32, 0_u64), (0, 16, 1), (-16, 0, 2), (0, -16, 3)] {
        let level = red_at(&gradient, x, y).unwrap();
        let expected = u32::try_from(u64::from(LINEAR_ONE) * turns / 4).unwrap();
        assert!(
            level.abs_diff(expected) <= 64,
            "({x}, {y}): {level} of {expected}"
        );
    }
}

#[test]
fn a_sweep_of_zero_span_takes_the_first_stop() {
    let (stops, colours) = ramp(Extend::Pad);
    let fill = Fill::Sweep {
        x: whole(0),
        y: whole(0),
        start: part(1, 2),
        end: part(1, 2),
        line: colours,
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    assert_eq!(red_at(&gradient, 4, 4), Some(0));
}

#[test]
fn every_extend_mode_folds_the_line_its_own_way() {
    // The device x values are -6 to 6 in twos and the line runs from zero to
    // four, so the parameter is -1.5, -1, -0.5, 0, 0.5, 1, 1.5. The table
    // below is the position each mode folds that to, in quarters.
    let cases = [
        (Extend::Pad, [0_u64, 0, 0, 0, 2, 4, 4]),
        (Extend::Repeat, [2_u64, 0, 2, 0, 2, 0, 2]),
        (Extend::Reflect, [2_u64, 4, 2, 0, 2, 4, 2]),
    ];
    for (mode, expected) in cases {
        let (stops, colours) = ramp(mode);
        // A line from zero to four, so the parameter is the device x over four.
        let fill = Fill::Linear {
            x0: whole(0),
            y0: whole(0),
            x1: whole(4),
            y1: whole(0),
            x2: whole(0),
            y2: whole(4),
            line: colours,
        };
        let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
            .unwrap()
            .unwrap();
        for (index, quarters) in expected.into_iter().enumerate() {
            let at = i32::try_from(index).unwrap() * 2 - 6;
            let level = red_at(&gradient, at, 0).unwrap();
            let want = u32::try_from(u64::from(LINEAR_ONE) * quarters / 4).unwrap();
            assert!(
                level.abs_diff(want) <= 96,
                "{mode:?} at {at}: {level} of {want}"
            );
        }
    }
}

#[test]
fn a_line_of_one_stop_is_that_colour_everywhere() {
    let stops = [stop(part(1, 2), 40, 80, 120)];
    let fill = Fill::Linear {
        x0: whole(0),
        y0: whole(0),
        x1: whole(8),
        y1: whole(0),
        x2: whole(0),
        y2: whole(8),
        line: line(1, Extend::Pad),
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    let gamma = context();
    let expected = Paint::opaque(40, 80, 120).pixel(255, &gamma).unwrap();
    for at in [-8_i32, 0, 4, 8, 32] {
        assert_eq!(
            gradient
                .at(whole(at), Fixed::ZERO, Paint::opaque(0, 0, 0), &gamma)
                .unwrap(),
            Some(expected),
            "at {at}"
        );
    }
}

#[test]
fn two_stops_at_one_offset_make_a_step() {
    let stops = [stop(part(1, 2), 0, 0, 0), stop(part(1, 2), 255, 255, 255)];
    let fill = Fill::Linear {
        x0: whole(0),
        y0: whole(0),
        x1: whole(8),
        y1: whole(0),
        x2: whole(0),
        y2: whole(8),
        line: line(2, Extend::Pad),
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    assert_eq!(red_at(&gradient, 0, 0), Some(0));
    assert_eq!(red_at(&gradient, 8, 0), Some(LINEAR_ONE));
}

#[test]
fn a_line_without_stops_draws_nothing() {
    let stops: [ColorStop; 0] = [];
    let fill = Fill::Linear {
        x0: whole(0),
        y0: whole(0),
        x1: whole(8),
        y1: whole(0),
        x2: whole(0),
        y2: whole(8),
        line: line(0, Extend::Pad),
    };
    assert!(
        Gradient::new(fill, &stops, Affine::IDENTITY)
            .unwrap()
            .is_none()
    );
    // A line naming more stops than the caller supplied is refused too.
    let one = [stop(Fixed::ZERO, 0, 0, 0)];
    let wide = Fill::Linear {
        x0: whole(0),
        y0: whole(0),
        x1: whole(8),
        y1: whole(0),
        x2: whole(0),
        y2: whole(8),
        line: line(4, Extend::Pad),
    };
    assert!(
        Gradient::new(wide, &one, Affine::IDENTITY)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_singular_placement_draws_nothing() {
    let (stops, colours) = ramp(Extend::Pad);
    let fill = Fill::Linear {
        x0: whole(0),
        y0: whole(0),
        x1: whole(8),
        y1: whole(0),
        x2: whole(0),
        y2: whole(8),
        line: colours,
    };
    let flat = Affine {
        xx: Fixed::ONE,
        yx: Fixed::ONE,
        xy: Fixed::ONE,
        yy: Fixed::ONE,
        dx: Fixed::ZERO,
        dy: Fixed::ZERO,
    };
    assert!(Gradient::new(fill, &stops, flat).unwrap().is_none());
    assert!(flat.inverse().unwrap().is_none());
}

#[test]
fn interpolation_runs_on_linear_light() {
    // Halfway along a black to white line, linear light gives half of 65535,
    // which encodes to 186 of 255. A display-space midpoint would be 128.
    let (stops, colours) = ramp(Extend::Pad);
    let fill = Fill::Linear {
        x0: whole(0),
        y0: whole(0),
        x1: whole(2),
        y1: whole(0),
        x2: whole(0),
        y2: whole(2),
        line: colours,
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    let gamma = context();
    let middle = red_at(&gradient, 1, 0).unwrap();
    assert!(middle.abs_diff(LINEAR_ONE / 2) <= 32, "{middle}");
    assert_eq!(gamma.encode(middle), 186);
}

#[test]
fn the_foreground_colour_reaches_a_stop() {
    let stops = [ColorStop {
        offset: Fixed::ZERO,
        color: Color {
            source: ColorSource::Foreground,
            alpha: Fixed::ONE,
        },
    }];
    let fill = Fill::Linear {
        x0: whole(0),
        y0: whole(0),
        x1: whole(4),
        y1: whole(0),
        x2: whole(0),
        y2: whole(4),
        line: line(1, Extend::Pad),
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    let gamma = context();
    let text = Paint::opaque(10, 20, 30);
    assert_eq!(
        gradient.at(whole(1), Fixed::ZERO, text, &gamma).unwrap(),
        Some(text.pixel(255, &gamma).unwrap())
    );
}

#[test]
fn a_solid_fill_is_not_a_gradient() {
    let stops = [stop(Fixed::ZERO, 0, 0, 0)];
    let solid = Fill::Solid(Color {
        source: ColorSource::Palette {
            red: 1,
            green: 2,
            blue: 3,
        },
        alpha: Fixed::ONE,
    });
    assert!(
        Gradient::new(solid, &stops, Affine::IDENTITY)
            .unwrap()
            .is_none()
    );
}

#[test]
fn the_same_gradient_evaluates_identically_twice() {
    let (stops, colours) = ramp(Extend::Reflect);
    let fill = Fill::Radial {
        x0: part(3, 2),
        y0: part(5, 2),
        r0: whole(1),
        x1: whole(9),
        y1: whole(7),
        r1: whole(5),
        line: colours,
    };
    let make = || {
        Gradient::new(fill, &stops, Affine::IDENTITY)
            .unwrap()
            .unwrap()
    };
    let (first, second) = (make(), make());
    let gamma = context();
    for y in -4..12_i32 {
        for x in -4..16_i32 {
            let a = first.at(whole(x), whole(y), Paint::opaque(0, 0, 0), &gamma);
            let b = second.at(whole(x), whole(y), Paint::opaque(0, 0, 0), &gamma);
            assert_eq!(a.unwrap(), b.unwrap(), "({x}, {y})");
        }
    }
}

#[test]
fn a_transform_moves_the_gradient_with_the_glyph() {
    let (stops, colours) = ramp(Extend::Pad);
    let fill = Fill::Linear {
        x0: whole(0),
        y0: whole(0),
        x1: whole(4),
        y1: whole(0),
        x2: whole(0),
        y2: whole(4),
        line: colours,
    };
    // Twice the size and shifted right by ten pixels.
    let placement = Affine {
        xx: whole(2),
        yx: Fixed::ZERO,
        xy: Fixed::ZERO,
        yy: whole(2),
        dx: whole(10),
        dy: Fixed::ZERO,
    };
    let gradient = Gradient::new(fill, &stops, placement).unwrap().unwrap();
    assert_eq!(red_at(&gradient, 10, 0), Some(0));
    assert_eq!(red_at(&gradient, 18, 0), Some(LINEAR_ONE));
    let middle = red_at(&gradient, 14, 0).unwrap();
    assert!(middle.abs_diff(LINEAR_ONE / 2) <= 64, "{middle}");
}

#[test]
fn a_pixel_of_the_gradient_is_premultiplied() {
    let stops = [ColorStop {
        offset: Fixed::ZERO,
        color: Color {
            source: ColorSource::Palette {
                red: 255,
                green: 255,
                blue: 255,
            },
            alpha: Fixed::ONE.mul_ratio(1, 2).unwrap(),
        },
    }];
    let fill = Fill::Linear {
        x0: whole(0),
        y0: whole(0),
        x1: whole(4),
        y1: whole(0),
        x2: whole(0),
        y2: whole(4),
        line: line(1, Extend::Pad),
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    let Some(Pixel { red, alpha, .. }) = gradient
        .at(whole(2), Fixed::ZERO, Paint::opaque(0, 0, 0), &context())
        .unwrap()
    else {
        unreachable!()
    };
    assert!(alpha.abs_diff(LINEAR_ONE / 2) <= 2, "{alpha}");
    assert!(red.abs_diff(LINEAR_ONE / 2) <= 2, "{red}");
}

#[test]
fn a_gradient_in_font_units_still_reads_its_ramp() {
    // Regression: the normalization of `Gradient::new` compared the largest
    // coordinate against a limit that halved with every shift instead of
    // doubling, so any geometry above 128 units drove the shift to 32, the
    // geometry to zero and every pixel to the last stop. COLR states a
    // gradient in font units, so that was every real gradient.
    for span in [128_i32, 130, 1024, 2048, 16_384] {
        let (stops, colours) = ramp(Extend::Pad);
        let fill = Fill::Linear {
            x0: whole(0),
            y0: whole(0),
            x1: whole(span),
            y1: whole(0),
            x2: whole(0),
            y2: whole(span),
            line: colours,
        };
        let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
            .unwrap()
            .unwrap();
        let mut previous = None;
        for step in 0..=4_i32 {
            let level = red_at(&gradient, span * step / 4, 0).unwrap();
            let expected =
                u32::try_from(u64::from(LINEAR_ONE) * u64::try_from(step).unwrap() / 4).unwrap();
            assert!(
                level.abs_diff(expected) <= 256,
                "span {span} at {step}/4: {level} of {expected}"
            );
            if let Some(previous) = previous {
                assert!(level > previous, "span {span} is not increasing");
            }
            previous = Some(level);
        }
    }
}

#[test]
fn a_radial_gradient_in_font_units_keeps_its_two_circles() {
    let (stops, colours) = ramp(Extend::Pad);
    let fill = Fill::Radial {
        x0: whole(512),
        y0: whole(512),
        r0: whole(0),
        x1: whole(512),
        y1: whole(512),
        r1: whole(512),
        line: colours,
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    assert_eq!(red_at(&gradient, 512, 512), Some(0));
    let half = red_at(&gradient, 768, 512).unwrap();
    assert!(half.abs_diff(LINEAR_ONE / 2) <= 256, "{half}");
    assert_eq!(red_at(&gradient, 1024, 512), Some(LINEAR_ONE));
}

#[test]
fn a_radial_family_whose_circles_share_one_apex_still_resolves() {
    // When the distance between the centres equals the difference of the
    // radii the quadratic degenerates to a linear equation, which is a
    // separate branch of the two-circle form.
    let (stops, colours) = ramp(Extend::Pad);
    let fill = Fill::Radial {
        x0: whole(0),
        y0: whole(0),
        r0: whole(0),
        x1: whole(4),
        y1: whole(0),
        r1: whole(4),
        line: colours,
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    // On the axis the parameter is the distance over the growth of the family.
    let quarter = red_at(&gradient, 2, 0).unwrap();
    assert!(quarter.abs_diff(LINEAR_ONE / 4) <= 256, "{quarter}");
    // At the apex the linear term vanishes as well and no circle passes.
    assert!(red_at(&gradient, 0, 2).is_none(), "the apex resolved");
}

#[test]
fn a_gradient_of_the_largest_geometry_takes_the_last_shift() {
    // The normalization runs out of shifts rather than looping; what it then
    // gives is a gradient that still reads in one direction.
    let (stops, colours) = ramp(Extend::Pad);
    let huge = Fixed::from_bits(i64::MAX / 4);
    let fill = Fill::Linear {
        x0: Fixed::ZERO,
        y0: Fixed::ZERO,
        x1: huge,
        y1: Fixed::ZERO,
        x2: Fixed::ZERO,
        y2: huge,
        line: colours,
    };
    let gradient = Gradient::new(fill, &stops, Affine::IDENTITY)
        .unwrap()
        .unwrap();
    assert_eq!(red_at(&gradient, 0, 0), Some(0));
}
