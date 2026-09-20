// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{Secret, wipe};

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

/// Regression, issue #44: the barrier `wipe` places after the fill takes
/// the buffer, not the `&&mut [u8]` that named it.
///
/// Both spellings zero the same bytes, so no call can observe the
/// difference; the guard reads the source, as the one for issue #43 does.
#[test]
fn wipe_places_the_barrier_on_the_buffer() {
    let source = include_str!("../secret.rs");
    assert!(source.contains("let _ = black_box(&*bytes);"));
    assert!(!source.contains("black_box(&bytes)"));
}

#[test]
fn wipe_overwrites_every_byte_of_a_slice() {
    let mut buffer = [0xABu8; 64];
    wipe(&mut buffer);
    assert_eq!(buffer, [0u8; 64]);

    let mut padded = [0xFFu8; 131];
    if let Some(tail) = padded.get_mut(3..) {
        wipe(tail);
    }
    assert_eq!(padded.get(..3), Some(&[0xFFu8, 0xFF, 0xFF][..]));
    assert!(
        padded
            .get(3..)
            .is_some_and(|tail| tail.iter().all(|b| *b == 0))
    );

    wipe(&mut []);
}

#[test]
fn debug_shows_the_length_and_no_content() {
    let secret = Secret::new([0x41u8; 3]);
    let rendered = format!("{secret:?}");
    assert_eq!(rendered, "Secret<3>");
    assert!(!rendered.contains('A'));
    assert!(!rendered.contains("65"));
}
