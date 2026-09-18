// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The inverse of the affine a `PaintOp::Fill` carries, which R9 maps a device
//! pixel back through.

use text_core::Fixed;

use crate::error::RasterError;

pub use text_core::colr::Affine;

/// Inverting a 2x3 affine.
pub trait Invertible: Sized {
    /// The transform that undoes this one, or `None` when it is singular and
    /// collapses the plane onto a line or a point.
    /// # Errors
    /// Returns `Overflow` when an intermediate exceeds Q32.32.
    fn inverse(self) -> Result<Option<Self>, RasterError>;
}

impl Invertible for Affine {
    fn inverse(self) -> Result<Option<Self>, RasterError> {
        let determinant = self
            .xx
            .checked_mul(self.yy)?
            .checked_sub(self.xy.checked_mul(self.yx)?)?;
        if determinant == Fixed::ZERO {
            return Ok(None);
        }
        let Ok(xx) = self.yy.checked_div(determinant) else {
            return Ok(None);
        };
        let Ok(xy) = self.xy.checked_neg()?.checked_div(determinant) else {
            return Ok(None);
        };
        let Ok(yx) = self.yx.checked_neg()?.checked_div(determinant) else {
            return Ok(None);
        };
        let Ok(yy) = self.xx.checked_div(determinant) else {
            return Ok(None);
        };
        let dx = xx
            .checked_mul(self.dx)?
            .checked_add(xy.checked_mul(self.dy)?)?
            .checked_neg()?;
        let dy = yx
            .checked_mul(self.dx)?
            .checked_add(yy.checked_mul(self.dy)?)?
            .checked_neg()?;
        Ok(Some(Self {
            xx,
            yx,
            xy,
            yy,
            dx,
            dy,
        }))
    }
}
