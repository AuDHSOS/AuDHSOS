// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R5: the transfer function of D-183, as two tables built once per value.
//!
//! Every blend of this crate runs on linear-light values. A display-space
//! channel is decoded on the way in and encoded on the way out; coverage
//! becomes alpha unchanged, because the format states that alpha is already on
//! a linear scale (`docs/microsoft/cpal.html:985`).
//!
//! `server-display` owns the value (D-175). This crate reads it from nowhere:
//! the caller builds a context and passes it per call.

use text_core::Fixed;

use crate::{
    error::RasterError,
    math::{MAX_EXPONENT, pow},
};

/// One whole linear-light channel. Sixteen bits, because eight place the two
/// darkest display codes on top of each other (D-178).
pub const LINEAR_ONE: u32 = 65_535;

/// Display-space levels per channel.
const LEVELS: usize = 256;

/// The transfer function of one gamma value, as tables.
///
/// The decode table is strictly increasing by construction. At 2.2 the second
/// display code is `0.33` of 65535 and rounds onto the first, so one level is
/// added where a code would otherwise collide with the code below it; the
/// adjustment is at most a few parts in 65535, which is far below one display
/// code anywhere. Without it a code would have no linear value of its own and
/// could not be encoded back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gamma {
    /// The linear value of each display code.
    decode: [u16; LEVELS],
    /// The linear value halfway to the next display code, so that encoding is
    /// a search for the nearest code and needs no wider table.
    edges: [u16; LEVELS],
}

impl Gamma {
    /// The value D-175 gives as the default, 2.2.
    /// # Errors
    /// Never; the default is inside the accepted range.
    pub fn default_value() -> Result<Self, RasterError> {
        Self::new(Fixed::ONE.mul_ratio(22, 10)?)
    }

    /// Build the tables of one gamma value.
    /// # Errors
    /// Returns `Gamma` for a value outside `(0, 16]`.
    pub fn new(gamma: Fixed) -> Result<Self, RasterError> {
        if gamma <= Fixed::ZERO || gamma > MAX_EXPONENT {
            return Err(RasterError::Gamma);
        }
        let mut decode = [0_u16; LEVELS];
        let mut previous = 0_u16;
        for (code, entry) in decode.iter_mut().enumerate() {
            let code = i64::try_from(code).map_err(|_| RasterError::Overflow)?;
            let level = Fixed::ONE.mul_ratio(code, 255)?;
            let linear = pow(level, gamma)?.mul_ratio(i64::from(LINEAR_ONE), 1)?;
            let value = u16::try_from(round(linear)?).map_err(|_| RasterError::Overflow)?;
            *entry = if code == 0 {
                value
            } else {
                value.max(previous.saturating_add(1))
            };
            previous = *entry;
        }
        let mut edges = [u16::MAX; LEVELS];
        for code in 0..LEVELS.saturating_sub(1) {
            let (low, high) = (
                u32::from(*decode.get(code).ok_or(RasterError::Overflow)?),
                u32::from(
                    *decode
                        .get(code.checked_add(1).ok_or(RasterError::Overflow)?)
                        .ok_or(RasterError::Overflow)?,
                ),
            );
            *edges.get_mut(code).ok_or(RasterError::Overflow)? =
                u16::try_from(low.checked_add(high).ok_or(RasterError::Overflow)? / 2)
                    .map_err(|_| RasterError::Overflow)?;
        }
        Ok(Self { decode, edges })
    }

    /// The linear-light value of a display-space channel.
    #[must_use]
    pub fn decode(&self, display: u8) -> u32 {
        u32::from(*self.decode.get(usize::from(display)).unwrap_or(&0))
    }

    /// The display-space channel nearest a linear-light value.
    #[must_use]
    pub fn encode(&self, linear: u32) -> u8 {
        let clamped = u16::try_from(linear.min(LINEAR_ONE)).unwrap_or(u16::MAX);
        let code = self.edges.partition_point(|edge| *edge < clamped);
        u8::try_from(code.min(LEVELS.saturating_sub(1))).unwrap_or(u8::MAX)
    }
}

/// Round a Q32.32 value to the nearest integer, ties to even.
fn round(value: Fixed) -> Result<i64, RasterError> {
    Ok(value.mul_ratio(1, Fixed::ONE.bits())?.bits())
}
