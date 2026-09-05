// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The group of P-384.
//!
//! The formulas are [`mod@crate::jacobian`]; what is here is the curve
//! constant and the base point, in Montgomery form. Both come from RFC
//! 5903, section 3.2, which is kept at `docs/rfc/rfc5903.txt`.

use crate::jacobian::Curve;
use crate::p384::field::{BYTES, LIMBS, Prime};

/// The curve `y^2 = x^3 - 3x + b` over [`Prime`].
#[derive(Clone, Copy, Debug)]
pub struct P384;

impl Curve<LIMBS> for P384 {
    type Field = Prime;

    const B: [u64; LIMBS] = [
        0x0811_8871_9d41_2dcc,
        0xf729_add8_7a4c_32ec,
        0x77f2_209b_1920_022e,
        0xe337_4bee_9493_8ae2,
        0xb62b_21f4_1f02_2094,
        0xcd08_114b_604f_bff9,
    ];
    const GENERATOR_X: [u64; LIMBS] = [
        0x3dd0_7566_49c0_b528,
        0x20e3_78e2_a0d6_ce38,
        0x879c_3afc_541b_4d6e,
        0x6454_8684_59a3_0eff,
        0x812f_f723_614e_de2b,
        0x4d3a_adc2_299e_1513,
    ];
    const GENERATOR_Y: [u64; LIMBS] = [
        0x2304_3dad_4b03_a4fe,
        0xa1bf_a8bf_7bb4_a9ac,
        0x8bad_e756_2e83_b050,
        0xc6c3_5219_68f4_ffd9,
        0xdd80_0226_3969_a840,
        0x2b78_abc2_5a15_c5e9,
    ];
}

/// A point of P-384 in Jacobian coordinates.
pub type Point = crate::jacobian::Point<P384, LIMBS, BYTES>;
