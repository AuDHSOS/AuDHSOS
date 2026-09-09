// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use crate::RadixInteger;
use std::{
    format,
    string::{String, ToString},
};

fn parse(text: &str, radix: u32) -> f64 {
    let mut integer = RadixInteger::new(radix).unwrap();
    for c in text.chars() {
        assert!(integer.push(c.to_digit(radix).unwrap()));
    }
    integer.to_f64()
}

#[test]
fn integer_boundaries_ties_sticky_carry_and_invalid_digits() {
    assert!(RadixInteger::new(0).is_none());
    assert!(RadixInteger::new(37).is_none());
    let mut n = RadixInteger::new(2).unwrap();
    assert_eq!(n.to_f64().to_bits(), 0);
    assert!(!n.push(2));
    assert!(n.push(1));
    assert!(!n.push(3));
    assert_eq!(n.to_f64(), 1.0);
    for (hex, expected) in [
        ("200000000000011", 144_115_188_075_855_900.0),
        ("1000000000000081", 1_152_921_504_606_847_200.0),
        ("ffffffffffffffff", 18_446_744_073_709_552_000.0),
    ] {
        assert_eq!(parse(hex, 16), expected);
    }
    // Two ties at 2^96, with an even versus odd retained significand, and
    // a sticky bit below the halfway bit which must change the first result.
    let base = 2f64.powi(96);
    let even = base.to_bits();
    assert_eq!(parse("1000000000000080000000000", 16).to_bits(), even);
    assert_eq!(parse("1000000000000080000000001", 16).to_bits(), even + 1);
    assert_eq!(parse("1000000000000180000000000", 16).to_bits(), even + 2);
    assert_eq!(parse(&"f".repeat(256), 16), f64::INFINITY);
    let threshold = format!("{}c{}", "f".repeat(13), "0".repeat(242));
    assert_eq!(parse(&threshold, 16), f64::INFINITY);
    let below = format!("{}b{}", "f".repeat(13), "f".repeat(242));
    assert_eq!(parse(&below, 16), f64::MAX);
    let mut overflow = RadixInteger::new(16).unwrap();
    for _ in 0..300 {
        assert!(overflow.push(15));
    }
    assert!(!overflow.push(16));
    assert_eq!(overflow.to_f64(), f64::INFINITY);
    assert_eq!(parse(&format!("{}1", "0".repeat(5000)), 10), 1.0);
}

#[test]
fn arbitrary_radices_agree_with_single_rounded_u128_and_decimal_reference() {
    let mut seed = 0x1234_5678_9abc_def0u128;
    for _ in 0..500 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let reference: f64 = seed.to_string().parse().unwrap();
        for radix in 2..=36u32 {
            let mut value = seed;
            let mut reversed = std::vec::Vec::new();
            while value != 0 {
                reversed.push(
                    char::from_digit(u32::try_from(value % u128::from(radix)).unwrap(), radix)
                        .unwrap(),
                );
                value /= u128::from(radix);
            }
            let text: String = reversed.into_iter().rev().collect();
            assert_eq!(
                parse(&text, radix).to_bits(),
                reference.to_bits(),
                "{text} radix {radix}"
            );
        }
    }
    for size in [20, 40, 100, 200, 308, 309, 500, 2000] {
        for _ in 0..20 {
            let mut text = String::new();
            for _ in 0..size {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                text.push(char::from_digit(u32::try_from(seed % 10).unwrap(), 10).unwrap());
            }
            assert_eq!(
                parse(&text, 10).to_bits(),
                text.parse::<f64>().unwrap().to_bits(),
                "{text}"
            );
        }
    }
}
