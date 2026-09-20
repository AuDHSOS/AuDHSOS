// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! X25519 from RFC 7748.
//!
//! The scalar is secret, so the ladder is constant time: it visits all 255
//! bits, every step is the same arithmetic, and the choice between the two
//! working points is a masked exchange rather than a branch. Nothing here
//! branches on a bit of the scalar or indexes with one.
//!
//! Invariant: the scalar is clamped before it is used, so that the
//! multiplication is by a multiple of the cofactor within the right range,
//! which is what makes the ladder safe against small-order peer values in
//! the first place.
//!
//! Invariant: the clamped scalar, the four working registers, and the
//! intermediates of the last round are overwritten before the ladder
//! returns; those intermediates alone recompute the shared secret.

use crypto_ct::{Choice, wipe};

use crate::error::EcError;
use crate::fe25519::Fe;

/// Bytes of a scalar.
pub const SCALAR_LEN: usize = 32;
/// Bytes of a public value.
pub const PUBLIC_LEN: usize = 32;

/// The `u` coordinate of the base point, which is nine.
const BASE: u8 = 9;

/// The public value of `scalar`: the ladder applied to the base point.
#[must_use]
pub fn base_point(scalar: &[u8; SCALAR_LEN]) -> [u8; PUBLIC_LEN] {
    let mut base = [0u8; PUBLIC_LEN];
    base[0] = BASE;
    ladder(scalar, &base)
}

/// The shared value of `scalar` and the peer's `point`.
///
/// # Errors
///
/// [`EcError::ZeroSharedSecret`] when the result is all zero, which happens
/// exactly when the peer sent a point of small order. RFC 8446 requires the
/// handshake to abort then, so this is an error and not a value the caller
/// has to remember to check.
pub fn x25519(
    scalar: &[u8; SCALAR_LEN],
    point: &[u8; PUBLIC_LEN],
) -> Result<[u8; PUBLIC_LEN], EcError> {
    let shared = ladder(scalar, point);
    let mut difference = 0u8;
    for byte in shared {
        difference |= byte;
    }
    if Choice::is_zero_u8(difference).is_true() {
        return Err(EcError::ZeroSharedSecret);
    }
    Ok(shared)
}

/// The Montgomery ladder.
#[expect(
    clippy::many_single_char_names,
    reason = "the working values are named as in RFC 7748, section 5"
)]
fn ladder(scalar: &[u8; SCALAR_LEN], point: &[u8; PUBLIC_LEN]) -> [u8; PUBLIC_LEN] {
    let mut clamped = clamp(scalar);
    let x1 = Fe::from_bytes(point);

    let mut x2 = Fe::ONE;
    let mut z2 = Fe::ZERO;
    let mut x3 = x1;
    let mut z3 = Fe::ONE;
    let mut swap = Choice::NO;

    // Declared here rather than in the loop, so that the last round's
    // values, which recompute the shared secret, are still named after the
    // loop and can be overwritten.
    let mut a = Fe::ZERO;
    let mut b = Fe::ZERO;
    let mut aa = Fe::ZERO;
    let mut bb = Fe::ZERO;
    let mut e = Fe::ZERO;
    let mut c = Fe::ZERO;
    let mut d = Fe::ZERO;
    let mut da = Fe::ZERO;
    let mut cb = Fe::ZERO;

    for position in (0..255u8).rev() {
        let bit = bit_of(&clamped, position);
        swap = swap ^ bit;
        Fe::swap(swap, &mut x2, &mut x3);
        Fe::swap(swap, &mut z2, &mut z3);
        swap = bit;

        a = x2.add(z2);
        b = x2.sub(z2);
        aa = a.square();
        bb = b.square();
        e = aa.sub(bb);
        c = x3.add(z3);
        d = x3.sub(z3);
        da = d.mul(a);
        cb = c.mul(b);

        x3 = da.add(cb).square();
        z3 = x1.mul(da.sub(cb).square());
        x2 = aa.mul(bb);
        z2 = e.mul(aa.add(e.mul_a24()));
    }

    Fe::swap(swap, &mut x2, &mut x3);
    Fe::swap(swap, &mut z2, &mut z3);
    let shared = x2.mul(z2.invert()).to_bytes();

    wipe(&mut clamped);
    for register in [
        &mut x2, &mut z2, &mut x3, &mut z3, &mut a, &mut b, &mut aa, &mut bb, &mut e, &mut c,
        &mut d, &mut da, &mut cb,
    ] {
        register.clear();
    }
    shared
}

/// The scalar with the bits RFC 7748 prescribes cleared and set.
const fn clamp(scalar: &[u8; SCALAR_LEN]) -> [u8; SCALAR_LEN] {
    let mut clamped = *scalar;
    clamped[0] &= 0xF8;
    clamped[31] &= 0x7F;
    clamped[31] |= 0x40;
    clamped
}

/// One bit of the scalar, without a branch and without an index that
/// depends on the scalar: every byte is visited, and the one that is wanted
/// is selected with a mask.
fn bit_of(scalar: &[u8; SCALAR_LEN], position: u8) -> Choice {
    let wanted = position.wrapping_shr(3);
    let within = u32::from(position & 7);
    let mut value = 0u8;
    for (index, candidate) in (0u8..).zip(scalar) {
        value |= Choice::is_zero_u8(index ^ wanted).mask_u8() & candidate;
    }
    Choice::from_lsb(value.wrapping_shr(within))
}
