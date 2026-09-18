// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R5: the transfer function of D-183 as two tables.

use text_core::Fixed;

use crate::{Gamma, LINEAR_ONE, RasterError};

/// `numerator / 10` as a gamma value.
fn value(numerator: i64) -> Fixed {
    Fixed::ONE.mul_ratio(numerator, 10).unwrap()
}

#[test]
fn every_display_code_round_trips_at_three_exponents() {
    for exponent in [value(10), value(18), value(22)] {
        let gamma = Gamma::new(exponent).unwrap();
        for code in 0..=u8::MAX {
            assert_eq!(
                gamma.encode(gamma.decode(code)),
                code,
                "code {code} at exponent {exponent:?}"
            );
        }
    }
}

#[test]
fn an_exponent_of_one_is_the_identity() {
    let gamma = Gamma::new(Fixed::ONE).unwrap();
    for code in 0..=u8::MAX {
        assert_eq!(gamma.decode(code), u32::from(code) * 257);
    }
    assert_eq!(gamma.decode(0), 0);
    assert_eq!(gamma.decode(255), LINEAR_ONE);
}

#[test]
fn decoding_is_strictly_increasing() {
    for exponent in [value(10), value(18), value(22), value(160)] {
        let gamma = Gamma::new(exponent).unwrap();
        let mut previous = 0_u32;
        for code in 1..=u8::MAX {
            let level = gamma.decode(code);
            assert!(
                level > previous,
                "code {code} at exponent {exponent:?}: {level} after {previous}"
            );
            previous = level;
        }
        assert_eq!(gamma.decode(0), 0);
        assert_eq!(gamma.decode(255), LINEAR_ONE);
    }
}

#[test]
fn encoding_picks_the_nearest_display_code() {
    let gamma = Gamma::default_value().unwrap();
    for code in 0..u8::MAX {
        let (low, high) = (gamma.decode(code), gamma.decode(code + 1));
        let middle = u32::midpoint(low, high);
        assert_eq!(gamma.encode(middle), code, "midpoint below {code}");
        assert_eq!(gamma.encode(middle + 1), code + 1, "midpoint above {code}");
    }
    assert_eq!(gamma.encode(0), 0);
    assert_eq!(gamma.encode(LINEAR_ONE), 255);
    assert_eq!(gamma.encode(LINEAR_ONE * 4), 255);
}

#[test]
fn two_contexts_of_one_exponent_hold_identical_tables() {
    let first = Gamma::new(value(22)).unwrap();
    let second = Gamma::new(Fixed::ONE.mul_ratio(11, 5).unwrap()).unwrap();
    assert_eq!(first, second);
    assert_eq!(first, Gamma::default_value().unwrap());
}

#[test]
fn an_exponent_outside_the_range_is_refused() {
    for exponent in [Fixed::ZERO, Fixed::from_i32(-1), Fixed::from_i32(17)] {
        assert_eq!(Gamma::new(exponent).unwrap_err(), RasterError::Gamma);
    }
    assert!(Gamma::new(Fixed::from_i32(16)).is_ok());
    assert!(Gamma::new(Fixed::from_bits(1)).is_ok());
}

#[test]
fn the_midtone_of_the_default_matches_the_transfer_function() {
    // `(128/255)^2.2` is `0.2195` of linear light, which is `14386` of 65535.
    let gamma = Gamma::default_value().unwrap();
    assert!(
        gamma.decode(128).abs_diff(14_386) <= 8,
        "{}",
        gamma.decode(128)
    );
    // The second display code is `0.33` of 65535 and would round onto the
    // first, so the monotone adjustment lifts it by one level.
    assert_eq!(gamma.decode(0), 0);
    assert_eq!(gamma.decode(1), 1);
    assert_eq!(gamma.decode(2), 2);
    // Half of linear light encodes to `0.7297`, which is `186` of 255.
    assert_eq!(gamma.encode(LINEAR_ONE / 2), 186);
}
