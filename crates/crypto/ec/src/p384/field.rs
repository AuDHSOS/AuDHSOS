// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two moduli of P-384: the field of the curve and the order of its
//! group.
//!
//! The arithmetic is [`mod@crate::montgomery`]; what is here is the four
//! constants each modulus needs, and the width the encodings have.
//!
//! The prime and the order are those of RFC 5903, section 3.2, which
//! `docs/rfc/rfc5903.txt` holds verbatim. RFC 5114, section 2.7, states
//! the same values independently, and the two agree.

use crate::montgomery::Params;

/// Bytes of an encoded value.
pub const BYTES: usize = 48;
/// Limbs of sixty-four bits a value occupies.
pub const LIMBS: usize = 6;

/// An element of one of the two moduli of this curve.
pub type Element<P> = crate::montgomery::Element<P, LIMBS, BYTES>;

/// The field of the curve: `2^384 - 2^128 - 2^96 + 2^32 - 1`.
#[derive(Clone, Copy, Debug)]
pub struct Prime;

impl Params<LIMBS> for Prime {
    const MODULUS: [u64; LIMBS] = [
        0x0000_0000_ffff_ffff,
        0xffff_ffff_0000_0000,
        0xffff_ffff_ffff_fffe,
        0xffff_ffff_ffff_ffff,
        0xffff_ffff_ffff_ffff,
        0xffff_ffff_ffff_ffff,
    ];
    const N0INV: u64 = 0x0000_0001_0000_0001;
    const R2: [u64; LIMBS] = [
        0xffff_fffe_0000_0001,
        0x0000_0002_0000_0000,
        0xffff_fffe_0000_0000,
        0x0000_0002_0000_0000,
        0x0000_0000_0000_0001,
        0x0000_0000_0000_0000,
    ];
    const ONE: [u64; LIMBS] = [
        0xffff_ffff_0000_0001,
        0x0000_0000_ffff_ffff,
        0x0000_0000_0000_0001,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ];
}

/// The order of the group of the curve.
#[derive(Clone, Copy, Debug)]
pub struct Order;

impl Params<LIMBS> for Order {
    const MODULUS: [u64; LIMBS] = [
        0xecec_196a_ccc5_2973,
        0x581a_0db2_48b0_a77a,
        0xc763_4d81_f437_2ddf,
        0xffff_ffff_ffff_ffff,
        0xffff_ffff_ffff_ffff,
        0xffff_ffff_ffff_ffff,
    ];
    const N0INV: u64 = 0x6ed4_6089_e88f_dc45;
    const R2: [u64; LIMBS] = [
        0x2d31_9b24_19b4_09a9,
        0xff3d_81e5_df1a_a419,
        0xbc3e_483a_fcb8_2947,
        0xd40d_4917_4aab_1cc5,
        0x3fb0_5b7a_2826_6895,
        0x0c84_ee01_2b39_bf21,
    ];
    const ONE: [u64; LIMBS] = [
        0x1313_e695_333a_d68d,
        0xa7e5_f24d_b74f_5885,
        0x389c_b27e_0bc8_d220,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ];
}
