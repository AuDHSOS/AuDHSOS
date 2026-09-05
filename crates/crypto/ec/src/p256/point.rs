// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The group of P-256.
//!
//! The formulas are [`mod@crate::jacobian`]; what is here is the curve
//! constant and the base point, in Montgomery form.

use crate::jacobian::Curve;
use crate::p256::field::{BYTES, LIMBS, Prime};

/// The curve `y^2 = x^3 - 3x + b` over [`Prime`].
#[derive(Clone, Copy, Debug)]
pub struct P256;

impl Curve<LIMBS> for P256 {
    type Field = Prime;

    const B: [u64; LIMBS] = [
        0xd89c_df62_29c4_bddf,
        0xacf0_05cd_7884_3090,
        0xe5a2_20ab_f721_2ed6,
        0xdc30_061d_0487_4834,
    ];
    const GENERATOR_X: [u64; LIMBS] = [
        0x79e7_30d4_18a9_143c,
        0x75ba_95fc_5fed_b601,
        0x79fb_732b_7762_2510,
        0x1890_5f76_a537_55c6,
    ];
    const GENERATOR_Y: [u64; LIMBS] = [
        0xddf2_5357_ce95_560a,
        0x8b4a_b8e4_ba19_e45c,
        0xd2e8_8688_dd21_f325,
        0x8571_ff18_2588_5d85,
    ];
}

/// A point of P-256 in Jacobian coordinates.
pub type Point = crate::jacobian::Point<P256, LIMBS, BYTES>;
