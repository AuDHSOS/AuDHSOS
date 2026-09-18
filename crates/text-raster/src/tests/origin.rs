// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R4: the four horizontal subpixel positions and the whole vertical pixel.

use text_core::Fixed;

use crate::{Origin, SUBPIXEL_POSITIONS, quantize};

/// `numerator / 8` of a pixel.
fn eighth(numerator: i64) -> Fixed {
    Fixed::ONE.mul_ratio(numerator, 8).unwrap()
}

#[test]
fn four_positions_cover_every_eighth_of_two_pixels() {
    // An eighth is half a quantization step, so successive eighths alternate
    // between landing on a position and rounding to the nearer one.
    let expected = [
        (0_i32, 0_u8),
        (0, 0),
        (0, 1),
        (0, 2),
        (0, 2),
        (0, 2),
        (0, 3),
        (1, 0),
        (1, 0),
        (1, 0),
        (1, 1),
        (1, 2),
        (1, 2),
        (1, 2),
        (1, 3),
        (2, 0),
        (2, 0),
    ];
    for (step, (pixel, subpixel)) in expected.into_iter().enumerate() {
        let origin = quantize(eighth(i64::try_from(step).unwrap()), Fixed::ZERO).unwrap();
        assert_eq!(
            (origin.x, origin.subpixel),
            (pixel, subpixel),
            "{step} eighths"
        );
    }
}

#[test]
fn negative_positions_quantize_the_same_way() {
    for step in 0..17_i64 {
        let origin = quantize(eighth(-step), Fixed::ZERO).unwrap();
        let mirrored = quantize(eighth(step), Fixed::ZERO).unwrap();
        let position =
            i64::from(origin.x) * i64::from(SUBPIXEL_POSITIONS) + i64::from(origin.subpixel);
        let opposite =
            i64::from(mirrored.x) * i64::from(SUBPIXEL_POSITIONS) + i64::from(mirrored.subpixel);
        // A tie rounds to even in both signs, so the two sides agree in
        // magnitude except where the tie falls the same way on both.
        assert!(
            (position + opposite).abs() <= 1,
            "{step} eighths gave {position} and {opposite}"
        );
    }
}

#[test]
fn a_tie_rounds_to_even_in_both_signs() {
    // An eighth of a pixel is exactly half a quantization step.
    assert_eq!(quantize(eighth(1), Fixed::ZERO).unwrap().subpixel, 0);
    assert_eq!(quantize(eighth(3), Fixed::ZERO).unwrap().subpixel, 2);
    assert_eq!(quantize(eighth(5), Fixed::ZERO).unwrap().subpixel, 2);
    assert_eq!(quantize(eighth(-1), Fixed::ZERO).unwrap().subpixel, 0);
    assert_eq!(quantize(eighth(-3), Fixed::ZERO).unwrap().subpixel, 2);
    // A whole half pixel vertically is a tie between two rows.
    assert_eq!(quantize(Fixed::ZERO, eighth(4)).unwrap().y, 0);
    assert_eq!(quantize(Fixed::ZERO, eighth(12)).unwrap().y, 2);
    assert_eq!(quantize(Fixed::ZERO, eighth(-4)).unwrap().y, 0);
    assert_eq!(quantize(Fixed::ZERO, eighth(-12)).unwrap().y, -2);
}

#[test]
fn the_vertical_origin_is_a_whole_pixel() {
    for step in -16..=16_i64 {
        let origin = quantize(Fixed::ZERO, eighth(step)).unwrap();
        let rounded = quantize(Fixed::ZERO, Fixed::from_i32(origin.y)).unwrap();
        assert_eq!(origin.y, rounded.y);
        assert_eq!(origin.subpixel, 0);
    }
}

#[test]
fn the_offset_of_a_position_is_that_fraction_of_a_pixel() {
    for subpixel in 0..u8::try_from(SUBPIXEL_POSITIONS).unwrap() {
        let origin = Origin {
            x: 0,
            y: 0,
            subpixel,
        };
        assert_eq!(
            origin.offset(),
            Fixed::ONE
                .mul_ratio(i64::from(subpixel), i64::from(SUBPIXEL_POSITIONS))
                .unwrap()
        );
    }
}

#[test]
fn a_position_beyond_the_pixel_grid_is_refused() {
    assert!(quantize(Fixed::from_bits(i64::MAX), Fixed::ZERO).is_err());
    assert!(quantize(Fixed::ZERO, Fixed::from_bits(i64::MAX)).is_err());
    assert!(quantize(Fixed::from_i32(1000), Fixed::from_i32(-1000)).is_ok());
}
