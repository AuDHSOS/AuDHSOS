// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two moduli of P-256: the field of the curve and the order of its
//! group.
//!
//! The arithmetic is [`mod@crate::montgomery`]; what is here is the four
//! constants each modulus needs, and the width the encodings have.

use crate::montgomery::Params;

/// Bytes of an encoded value.
pub const BYTES: usize = 32;
/// Limbs of sixty-four bits a value occupies.
pub const LIMBS: usize = 4;

/// An element of one of the two moduli of this curve.
pub type Element<P> = crate::montgomery::Element<P, LIMBS, BYTES>;

/// The field of the curve: `2^256 - 2^224 + 2^192 + 2^96 - 1`.
#[derive(Clone, Copy, Debug)]
pub struct Prime;

impl Params<LIMBS> for Prime {
    const MODULUS: [u64; LIMBS] = [
        0xffff_ffff_ffff_ffff,
        0x0000_0000_ffff_ffff,
        0x0000_0000_0000_0000,
        0xffff_ffff_0000_0001,
    ];
    const N0INV: u64 = 0x0000_0000_0000_0001;
    const R2: [u64; LIMBS] = [
        0x0000_0000_0000_0003,
        0xffff_fffb_ffff_ffff,
        0xffff_ffff_ffff_fffe,
        0x0000_0004_ffff_fffd,
    ];
    const ONE: [u64; LIMBS] = [
        0x0000_0000_0000_0001,
        0xffff_ffff_0000_0000,
        0xffff_ffff_ffff_ffff,
        0x0000_0000_ffff_fffe,
    ];
}

/// The order of the group of the curve.
#[derive(Clone, Copy, Debug)]
pub struct Order;

impl Params<LIMBS> for Order {
    const MODULUS: [u64; LIMBS] = [
        0xf3b9_cac2_fc63_2551,
        0xbce6_faad_a717_9e84,
        0xffff_ffff_ffff_ffff,
        0xffff_ffff_0000_0000,
    ];
    const N0INV: u64 = 0xccd1_c8aa_ee00_bc4f;
    const R2: [u64; LIMBS] = [
        0x8324_4c95_be79_eea2,
        0x4699_799c_49bd_6fa6,
        0x2845_b239_2b6b_ec59,
        0x66e1_2d94_f3d9_5620,
    ];
    const ONE: [u64; LIMBS] = [
        0x0c46_353d_039c_daaf,
        0x4319_0552_58e8_617b,
        0x0000_0000_0000_0000,
        0x0000_0000_ffff_ffff,
    ];
}
