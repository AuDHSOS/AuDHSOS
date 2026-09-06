// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The limb operations, against the schoolbook reference and against the
//! carries that the widths of this crate make hard to reach by accident.

use test_support::generators::{bytes, vec};

use crate::limbs::{add_limbs, is_less, montgomery, subtract};
use crate::modulus::{MAX_LIMBS, Modulus};
use crate::tests::reference::Big;
use crate::tests::{WIDTHS, check_cases, hex, modulus_bytes};

#[test]
fn addition_carries_out_of_the_top_limb() {
    let mut value = [u64::MAX, u64::MAX];
    let carry = add_limbs(&mut value, &[1, 0]);
    assert_eq!(value, [0, 0]);
    assert_eq!(carry, 1);
}

#[test]
fn addition_carries_between_limbs_without_carrying_out() {
    let mut value = [u64::MAX, 0];
    let carry = add_limbs(&mut value, &[1, 0]);
    assert_eq!(value, [0, 1]);
    assert_eq!(carry, 0);
}

#[test]
fn subtraction_borrows_out_of_the_top_limb() {
    let mut value = [0u64, 0];
    let borrow = subtract(&mut value, &[1, 0]);
    assert_eq!(value, [u64::MAX, u64::MAX]);
    assert_eq!(borrow, 1);
}

#[test]
fn subtraction_borrows_between_limbs_without_borrowing_out() {
    let mut value = [0u64, 1];
    let borrow = subtract(&mut value, &[1, 0]);
    assert_eq!(value, [u64::MAX, 0]);
    assert_eq!(borrow, 0);
}

#[test]
fn the_comparison_is_decided_by_the_highest_limb_that_differs() {
    assert!(is_less(&[9, 1], &[0, 2]));
    assert!(!is_less(&[0, 2], &[9, 1]));
    assert!(is_less(&[1, 2], &[2, 2]));
    assert!(!is_less(&[2, 2], &[2, 2]));
    assert!(!is_less(&[3, 2], &[2, 2]));
}

/// The Montgomery product against the definition: what it returns, times
/// `2^(64*N)`, is the ordinary product modulo the modulus. The reference
/// has no Montgomery form at all, so the two never share a mistake.
#[test]
fn property_the_montgomery_product_agrees_with_the_reference() {
    for bits in WIDTHS {
        let width = bits / 8;
        let used = bits / 64;
        let name = format!("bignum_montgomery_{bits}");
        let generator = vec(bytes(width..=width), 3..=3);
        check_cases(cases_for(bits), &name, &generator, |parts| {
            let (Some(first), Some(second), Some(third)) =
                (parts.first(), parts.get(1), parts.get(2))
            else {
                return Err("the generator produced fewer than three values".to_owned());
            };
            let encoded = modulus_bytes(bits, first);
            let modulus = Modulus::new(&encoded).map_err(|error| error.to_string())?;
            let reference = Big::from_be_bytes(&encoded);
            let limbs = limbs_of(&reference, used);

            let left = Big::from_be_bytes(second).rem(&reference);
            let right = Big::from_be_bytes(third).rem(&reference);

            for (name, a, b) in [
                ("product", &left, &right),
                ("square", &left, &left),
                ("by one", &left, &Big::from_u64(1)),
            ] {
                let mut out = [0u64; MAX_LIMBS];
                montgomery(
                    &limbs_of(a, used)[..used],
                    &limbs_of(b, used)[..used],
                    &limbs[..used],
                    modulus.n0inv,
                    &mut out[..used],
                );
                let ours = Big::from_be_bytes(&be_of(&out[..used]));
                let lifted = ours.shl(used.saturating_mul(64)).rem(&reference);
                let theirs = a.mul(b).rem(&reference);
                if lifted != theirs {
                    return Err(format!(
                        "{name}: {} against {}",
                        hex(&lifted.to_be_bytes(width)),
                        hex(&theirs.to_be_bytes(width))
                    ));
                }
            }
            Ok(())
        });
    }
}

/// How many cases a width is worth. The reference reduces one bit at a
/// time over limbs of the modulus, so a case costs about the cube of the
/// width and the count comes down as the width goes up.
fn cases_for(bits: usize) -> u32 {
    match bits {
        1024 => 48,
        2048 => 24,
        3072 => 12,
        _ => 8,
    }
}

/// The limbs of a reference value, as an array of the crate's width.
pub(crate) fn limbs_of(value: &Big, used: usize) -> [u64; MAX_LIMBS] {
    let bytes = value.to_be_bytes(used.saturating_mul(8));
    let mut limbs = [0u64; MAX_LIMBS];
    let (chunks, _) = bytes.as_chunks::<8>();
    for (slot, chunk) in limbs.iter_mut().zip(chunks.iter().rev()) {
        *slot = u64::from_be_bytes(*chunk);
    }
    limbs
}

/// The big-endian encoding of limbs.
pub(crate) fn be_of(limbs: &[u64]) -> Vec<u8> {
    limbs
        .iter()
        .rev()
        .flat_map(|limb| limb.to_be_bytes())
        .collect()
}
