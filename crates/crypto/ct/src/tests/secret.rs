// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::Secret;

#[test]
fn a_new_secret_holds_the_bytes_it_was_given() {
    let secret = Secret::new([1u8, 2, 3, 4]);
    assert_eq!(secret.as_bytes(), &[1, 2, 3, 4]);
}

#[test]
fn a_zero_secret_starts_empty_and_can_be_filled() {
    let mut secret = Secret::<8>::zero();
    assert_eq!(secret.as_bytes(), &[0u8; 8]);
    for (slot, value) in secret.as_bytes_mut().iter_mut().zip(1u8..) {
        *slot = value;
    }
    assert_eq!(secret.as_bytes(), &[1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn the_default_secret_is_the_zero_secret() {
    assert_eq!(Secret::<4>::default().as_bytes(), &[0u8; 4]);
}

#[test]
fn equality_is_the_constant_time_comparison() {
    let secret = Secret::new([9u8; 16]);
    let same = Secret::new([9u8; 16]);
    let mut other = Secret::new([9u8; 16]);
    if let Some(byte) = other.as_bytes_mut().get_mut(15) {
        *byte = 8;
    }
    assert!(secret.ct_eq(&same).is_true());
    assert!(!secret.ct_eq(&other).is_true());
}

#[test]
fn clearing_overwrites_the_buffer() {
    let mut secret = Secret::new([0xABu8; 32]);
    secret.clear();
    assert_eq!(secret.as_bytes(), &[0u8; 32]);
}

#[test]
fn a_clone_is_independent_of_its_original() {
    let secret = Secret::new([5u8; 4]);
    let mut clone = secret.clone();
    clone.clear();
    assert_eq!(secret.as_bytes(), &[5u8; 4]);
    assert_eq!(clone.as_bytes(), &[0u8; 4]);
}

#[test]
fn debug_shows_the_length_and_no_content() {
    let secret = Secret::new([0x41u8; 3]);
    let rendered = format!("{secret:?}");
    assert_eq!(rendered, "Secret<3>");
    assert!(!rendered.contains('A'));
    assert!(!rendered.contains("65"));
}

#[test]
fn wiping_words_overwrites_every_one_of_them() {
    let mut words = [0xDEAD_BEEFu32; 8];
    crate::wipe_u32(&mut words);
    assert_eq!(words, [0u32; 8]);
}

#[test]
fn wiping_wide_words_overwrites_every_one_of_them() {
    let mut words = [0xDEAD_BEEF_CAFE_F00Du64; 64];
    crate::wipe_u64(&mut words);
    assert_eq!(words, [0u64; 64]);
}

#[test]
fn wiping_an_empty_slice_changes_nothing() {
    crate::wipe_u32(&mut []);
    crate::wipe_u64(&mut []);
}
