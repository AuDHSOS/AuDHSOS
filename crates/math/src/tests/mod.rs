// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

mod integer;
use super::*;

#[test]
fn ecmascript_special_values_sign_and_integer_parity() {
    for (x, y, z) in [
        (f64::NAN, 0.0, 1.0),
        (-0.0, 3.0, -0.0),
        (-0.0, -3.0, f64::NEG_INFINITY),
        (-0.0, 0.5, 0.0),
        (-0.0, -0.5, f64::INFINITY),
        (f64::NEG_INFINITY, 3.0, f64::NEG_INFINITY),
        (f64::NEG_INFINITY, -3.0, -0.0),
        (f64::NEG_INFINITY, 0.5, f64::INFINITY),
        (f64::NEG_INFINITY, -0.5, 0.0),
        (2.0, f64::INFINITY, f64::INFINITY),
        (0.5, f64::INFINITY, 0.0),
        (2.0, f64::NEG_INFINITY, 0.0),
        (0.5, f64::NEG_INFINITY, f64::INFINITY),
        (-1.0, 9_007_199_254_740_991.0, -1.0),
        (-1.0, 9_007_199_254_740_992.0, 1.0),
        (3.0, 1.0, 3.0),
        (3.0, 2.0, 9.0),
        (2.0, -1.0, 0.5),
    ] {
        assert_eq!(pow(x, y).to_bits(), z.to_bits(), "{x} ** {y}");
    }
    for (x, y) in [
        (1.0, f64::NAN),
        (-1.0, f64::INFINITY),
        (1.0, f64::NEG_INFINITY),
        (-2.0, 0.5),
        (f64::NAN, 1.0),
    ] {
        assert!(pow(x, y).is_nan());
    }
}
#[test]
fn exact_integer_powers_and_range_edges() {
    for n in -1074..=1023 {
        let result = pow(2.0, f64::from(n));
        let expected = if n < -1022 {
            f64::from_bits(1u64 << u32::try_from(n + 1074).unwrap())
        } else {
            f64::from_bits(u64::try_from(n + 1023).unwrap() << 52)
        };
        assert_eq!(result.to_bits(), expected.to_bits(), "2 ** {n}");
    }
    for (x, y, z) in [
        (3.0, 3.0, 27.0),
        (2.0, 10.0, 1024.0),
        (-3.0, 3.0, -27.0),
        (4.0, 0.5, 2.0),
        (9.0, 0.5, 3.0),
        (0.25, 0.5, 0.5),
        (2.0, 1024.0, f64::INFINITY),
        (2.0, -1075.0, 0.0),
        (1e300, 100.0, f64::INFINITY),
        (1e-300, 100.0, 0.0),
    ] {
        assert_eq!(pow(x, y).to_bits(), z.to_bits(), "{x} ** {y}");
    }
}
#[test]
fn deterministic_binary64_differential_against_host_math() {
    let mut seed = 0x004a_5253_u64;
    let mut count = 0;
    for _ in 0..20000 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let x = f64::from_bits(seed & 0x7fff_ffff_ffff_ffff);
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let y = f64::from(i32::try_from(seed >> 33).unwrap_or(0) % 20000 - 10000) / 1024.0;
        if x.is_finite() && x > 0.0 {
            let actual = pow(x, y);
            let expected = x.powf(y);
            assert!(
                actual.to_bits().abs_diff(expected.to_bits()) <= 4,
                "{x:?} ** {y:?}: {actual:?} vs {expected:?}"
            );
            count += 1;
        }
    }
    assert!(count > 19000);
    for x in [
        f64::from_bits(1),
        f64::MIN_POSITIVE,
        f64::MAX,
        1.0 + f64::EPSILON,
        1.0 - f64::EPSILON / 2.0,
    ] {
        for y in [-1e17, -1e10, -0.1, 0.1, 1e10, 1e17] {
            let a = pow(x, y);
            let b = x.powf(y);
            assert!(
                a.to_bits().abs_diff(b.to_bits()) <= 4,
                "{x:?} ** {y:?}: {a:?} vs {b:?}"
            );
        }
    }
}

#[test]
fn exceptional_matrix_and_small_integer_fast_path() {
    for x in [
        0.0,
        -0.0,
        1.0,
        -1.0,
        2.0,
        -2.0,
        0.5,
        -0.5,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ] {
        for y in [
            0.0,
            -0.0,
            1.0,
            2.0,
            3.0,
            -1.0,
            -2.0,
            -3.0,
            0.5,
            -0.5,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ] {
            let expected = if y.is_nan() || x.abs().to_bits() == 1.0f64.to_bits() && y.is_infinite()
            {
                f64::NAN
            } else {
                x.powf(y)
            };
            let actual = pow(x, y);
            if expected.is_nan() {
                assert!(actual.is_nan());
            } else {
                assert_eq!(actual.to_bits(), expected.to_bits(), "{x} ** {y}");
            }
        }
    }
    for i in 1..100 {
        for n in -16..=16 {
            let x = f64::from(i) / 7.0;
            let actual = pow(x, f64::from(n));
            let expected = x.powf(f64::from(n));
            assert!(
                actual.to_bits().abs_diff(expected.to_bits()) <= 2,
                "{x} ** {n}"
            );
        }
    }
}
