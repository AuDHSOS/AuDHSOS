// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use crate::{
    acos, acosh, asin, asinh, atan, atan2, atanh, cos, cosh, exp, ln, log2, log10, sin, sinh, sqrt,
    tan, tanh,
};

/// How far two doubles may stand apart in the last place, which is the
/// tolerance the differential tests here read.
const APART: u64 = 4;

/// Asserts that two doubles are the same or stand within [`APART`] of one
/// another, which a NaN on both sides counts as.
#[track_caller]
fn near(mine: f64, host: f64, what: &str) {
    if mine.is_nan() && host.is_nan() {
        return;
    }
    assert_eq!(mine.is_finite(), host.is_finite(), "{what}: {mine} {host}");
    if !mine.is_finite() {
        assert_eq!(mine, host, "{what}");
        return;
    }
    assert!(
        mine.to_bits().abs_diff(host.to_bits()) <= APART,
        "{what}: {mine:?} vs {host:?}"
    );
}

/// The arguments the differential tests read: a deterministic run of
/// doubles over the range each function is read on.
fn arguments(count: u32, scale: f64) -> std::vec::Vec<f64> {
    let mut seed = 0x0057_1a10_u64;
    let mut out = std::vec::Vec::new();
    for _ in 0..count {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let unit = f64::from(u32::try_from(seed >> 40).unwrap_or(0)) / 16_777_216.0;
        out.push((unit - 0.5) * 2.0 * scale);
    }
    out
}

#[test]
fn the_square_root_is_the_double_nearest_it() {
    for x in [0.0, -0.0, 1.0, 2.0, 4.0, 0.25, 1e300, 1e-300, f64::INFINITY] {
        assert_eq!(sqrt(x).to_bits(), x.sqrt().to_bits(), "sqrt({x:?})");
    }
    // The root of a number at either end of the range stands within the
    // tolerance: the root of the largest double falls halfway between two
    // doubles, which only a correctly rounded root picks between.
    for x in [f64::MIN_POSITIVE, f64::from_bits(1), f64::MAX] {
        near(sqrt(x), x.sqrt(), "sqrt");
    }
    assert!(sqrt(-1.0).is_nan());
    assert!(sqrt(f64::NAN).is_nan());
    for x in arguments(2000, 1e6) {
        let x = x.abs();
        assert_eq!(sqrt(x).to_bits(), x.sqrt().to_bits(), "sqrt({x:?})");
    }
    for x in arguments(500, 1e-290) {
        let x = x.abs();
        assert_eq!(sqrt(x).to_bits(), x.sqrt().to_bits(), "sqrt({x:?})");
    }
}

#[test]
fn the_exponential_and_the_logarithms_answer_what_the_host_answers() {
    for x in [0.0, 1.0, -1.0, 700.0, -700.0, 710.5, -746.5, f64::INFINITY] {
        near(exp(x), x.exp(), "exp");
    }
    assert!(exp(f64::NAN).is_nan());
    assert_eq!(exp(f64::NEG_INFINITY), 0.0);
    for x in arguments(2000, 700.0) {
        near(exp(x), x.exp(), "exp");
    }
    // A power of two and a power of ten are answered exactly, which the
    // logarithm of that base is read for.
    for n in -300..300 {
        assert_eq!(log2(2f64.powi(n)), f64::from(n), "log2(2 ** {n})");
    }
    for n in 0..23 {
        assert_eq!(log10(10f64.powi(n)), f64::from(n), "log10(10 ** {n})");
    }
    assert_eq!(ln(0.0), f64::NEG_INFINITY);
    assert_eq!(log2(0.0), f64::NEG_INFINITY);
    assert_eq!(log10(0.0), f64::NEG_INFINITY);
    assert!(ln(-1.0).is_nan());
    assert!(log2(-1.0).is_nan());
    assert!(log10(-1.0).is_nan());
    assert!(ln(f64::NAN).is_nan());
    assert_eq!(ln(f64::INFINITY), f64::INFINITY);
    assert_eq!(log2(f64::INFINITY), f64::INFINITY);
    for x in arguments(2000, 1e12) {
        let x = x.abs();
        near(ln(x), x.ln(), "ln");
        near(log2(x), x.log2(), "log2");
        near(log10(x), x.log10(), "log10");
    }
}

#[test]
fn the_circular_functions_answer_what_the_host_answers() {
    for x in [0.0, -0.0] {
        assert_eq!(sin(x).to_bits(), x.sin().to_bits(), "sin({x:?})");
    }
    for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            sin(x).is_nan() && cos(x).is_nan() && tan(x).is_nan(),
            "{x:?}"
        );
    }
    // A quarter turn and its multiples, where the reduction leaves the
    // whole distance from the turn.
    for turn in -8..9 {
        let x = f64::from(turn) * core::f64::consts::FRAC_PI_2;
        near(sin(x), x.sin(), "sin");
        near(cos(x), x.cos(), "cos");
        near(tan(x), x.tan(), "tan");
    }
    for x in arguments(4000, 100.0) {
        near(sin(x), x.sin(), "sin");
        near(cos(x), x.cos(), "cos");
        near(tan(x), x.tan(), "tan");
    }
    for x in arguments(500, 1e-100) {
        near(sin(x), x.sin(), "sin");
        near(cos(x), x.cos(), "cos");
    }
}

#[test]
fn the_inverse_circular_functions_answer_what_the_host_answers() {
    assert_eq!(asin(1.0), core::f64::consts::FRAC_PI_2);
    assert_eq!(asin(-1.0), -core::f64::consts::FRAC_PI_2);
    assert_eq!(acos(1.0), 0.0);
    assert_eq!(acos(-1.0), core::f64::consts::PI);
    assert_eq!(atan(f64::INFINITY), core::f64::consts::FRAC_PI_2);
    assert_eq!(atan(f64::NEG_INFINITY), -core::f64::consts::FRAC_PI_2);
    for x in [1.5, -1.5, f64::NAN] {
        assert!(asin(x).is_nan() && acos(x).is_nan(), "{x:?}");
    }
    assert!(atan(f64::NAN).is_nan());
    for x in arguments(2000, 1.0) {
        near(asin(x), x.asin(), "asin");
        near(acos(x), x.acos(), "acos");
        near(atan(x), x.atan(), "atan");
    }
    for x in arguments(2000, 1e8) {
        near(atan(x), x.atan(), "atan");
    }
}

#[test]
fn the_angle_of_a_point_is_the_angle_the_host_answers() {
    let held = [0.0, -0.0, f64::INFINITY, f64::NEG_INFINITY];
    for y in held {
        for x in held {
            assert_eq!(
                atan2(y, x).to_bits(),
                y.atan2(x).to_bits(),
                "atan2({y:?}, {x:?})"
            );
        }
        assert!(atan2(y, f64::NAN).is_nan());
        assert!(atan2(f64::NAN, y).is_nan());
    }
    for y in [1.0, -1.0, 1e300, -1e-300] {
        for x in [1.0, -1.0, 1e300, -1e-300, 0.0, -0.0] {
            near(atan2(y, x), y.atan2(x), "atan2");
        }
    }
    for (y, x) in arguments(1000, 1e3).into_iter().zip(arguments(1000, 1e3)) {
        near(atan2(y, x), y.atan2(x), "atan2");
    }
}

#[test]
fn the_hyperbolic_functions_answer_what_the_host_answers() {
    for x in [0.0, -0.0] {
        assert_eq!(sinh(x).to_bits(), x.sinh().to_bits(), "sinh({x:?})");
        assert_eq!(tanh(x).to_bits(), x.tanh().to_bits(), "tanh({x:?})");
        assert_eq!(asinh(x).to_bits(), x.asinh().to_bits(), "asinh({x:?})");
        assert_eq!(atanh(x).to_bits(), x.atanh().to_bits(), "atanh({x:?})");
    }
    assert_eq!(sinh(f64::INFINITY), f64::INFINITY);
    assert_eq!(cosh(f64::INFINITY), f64::INFINITY);
    assert_eq!(cosh(f64::NEG_INFINITY), f64::INFINITY);
    assert_eq!(tanh(f64::INFINITY), 1.0);
    assert_eq!(tanh(f64::NEG_INFINITY), -1.0);
    assert_eq!(asinh(f64::INFINITY), f64::INFINITY);
    assert_eq!(acosh(f64::INFINITY), f64::INFINITY);
    assert_eq!(acosh(1.0), 0.0);
    assert_eq!(atanh(1.0), f64::INFINITY);
    assert_eq!(atanh(-1.0), f64::NEG_INFINITY);
    for x in [f64::NAN, 0.5, -1.0] {
        assert!(acosh(x).is_nan(), "acosh({x:?})");
    }
    for x in [f64::NAN, 1.5, -1.5] {
        assert!(atanh(x).is_nan(), "atanh({x:?})");
    }
    assert!(sinh(f64::NAN).is_nan() && cosh(f64::NAN).is_nan());
    assert!(tanh(f64::NAN).is_nan() && asinh(f64::NAN).is_nan());
    for x in arguments(2000, 30.0) {
        near(sinh(x), x.sinh(), "sinh");
        near(cosh(x), x.cosh(), "cosh");
        near(tanh(x), x.tanh(), "tanh");
        near(asinh(x), x.asinh(), "asinh");
    }
    for x in arguments(500, 1e-40) {
        near(sinh(x), x.sinh(), "sinh");
        near(tanh(x), x.tanh(), "tanh");
        near(asinh(x), x.asinh(), "asinh");
        near(atanh(x), x.atanh(), "atanh");
    }
    // `f64::atanh` of the host reads `ln(1 + 2x/(1-x))`, which loses the
    // digits of the argument near one, so the host answers only for a
    // bounded argument and the hyperbolic tangent answers for the rest.
    for x in arguments(1000, 0.9) {
        near(atanh(x), x.atanh(), "atanh");
    }
    for x in arguments(1000, 0.999_999) {
        near(tanh(atanh(x)), x, "tanh of atanh");
    }
    for x in arguments(1000, 1e6) {
        near(acosh(x.abs() + 1.0), (x.abs() + 1.0).acosh(), "acosh");
        near(asinh(x), x.asinh(), "asinh");
    }
}
