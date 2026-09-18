// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Premultiplied linear-light pixels: the one form every blend of this crate
//! runs on (D-178, D-183).
//!
//! Invariants: every channel is at most [`LINEAR_ONE`]; a colour channel never
//! exceeds the alpha it is premultiplied by, which is what keeps a blend of
//! two valid pixels valid.

use text_core::{Fixed, colr::ColorSource};

use crate::{
    error::RasterError,
    gamma::{Gamma, LINEAR_ONE},
    surface::{Format, Surface, Texel},
};

/// A colour the caller states in display space, with a linear alpha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Paint {
    /// Display-space red.
    pub red: u8,
    /// Display-space green.
    pub green: u8,
    /// Display-space blue.
    pub blue: u8,
    /// Alpha, in `[0, 1]`. Alpha is already linear and is not decoded.
    pub alpha: Fixed,
}

impl Paint {
    /// An opaque display-space colour.
    #[must_use]
    pub const fn opaque(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha: Fixed::ONE,
        }
    }

    /// This colour as a premultiplied linear-light pixel, weighted by a
    /// coverage byte.
    /// # Errors
    /// Returns `Overflow` when the alpha leaves Q32.32.
    pub fn pixel(self, coverage: u8, gamma: &Gamma) -> Result<Pixel, RasterError> {
        let alpha = self.alpha.clamp(Fixed::ZERO, Fixed::ONE);
        let scaled = alpha.mul_ratio(i64::from(coverage), 255)?;
        let alpha = level(scaled)?;
        Ok(Pixel {
            red: scale(gamma.decode(self.red), alpha),
            green: scale(gamma.decode(self.green), alpha),
            blue: scale(gamma.decode(self.blue), alpha),
            alpha,
        })
    }

    /// The colour a paint stream names, with `Foreground` resolved to the
    /// caller's text colour (D-173).
    #[must_use]
    pub const fn of(source: ColorSource, alpha: Fixed, foreground: Self) -> Self {
        match source {
            ColorSource::Palette { red, green, blue } => Self {
                red,
                green,
                blue,
                alpha,
            },
            ColorSource::Foreground => Self {
                red: foreground.red,
                green: foreground.green,
                blue: foreground.blue,
                alpha,
            },
        }
    }
}

/// One premultiplied linear-light pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pixel {
    /// Red, already multiplied by `alpha`.
    pub red: u32,
    /// Green, already multiplied by `alpha`.
    pub green: u32,
    /// Blue, already multiplied by `alpha`.
    pub blue: u32,
    /// Coverage of this pixel, in `[0, LINEAR_ONE]`.
    pub alpha: u32,
}

impl Pixel {
    /// A pixel that covers nothing.
    pub const CLEAR: Self = Self {
        red: 0,
        green: 0,
        blue: 0,
        alpha: 0,
    };

    /// `self` drawn over `backdrop`, which is Porter-Duff source-over.
    #[must_use]
    pub fn over(self, backdrop: Self) -> Self {
        let rest = LINEAR_ONE.saturating_sub(self.alpha);
        Self {
            red: self.red.saturating_add(scale(backdrop.red, rest)),
            green: self.green.saturating_add(scale(backdrop.green, rest)),
            blue: self.blue.saturating_add(scale(backdrop.blue, rest)),
            alpha: self.alpha.saturating_add(scale(backdrop.alpha, rest)),
        }
    }

    /// This pixel weighted by a mask value in `[0, LINEAR_ONE]`.
    #[must_use]
    pub fn weighted(self, weight: u32) -> Self {
        Self {
            red: scale(self.red, weight),
            green: scale(self.green, weight),
            blue: scale(self.blue, weight),
            alpha: scale(self.alpha, weight),
        }
    }

    /// The colour channels with the premultiplication undone, which is the
    /// form a blend mode is defined on (`docs/w3c/compositing-1.html:1785`).
    #[must_use]
    pub fn straight(self) -> [u32; 3] {
        if self.alpha == 0 {
            return [0; 3];
        }
        [self.red, self.green, self.blue].map(|channel| unscale(channel, self.alpha))
    }

    /// A pixel from straight colour channels and an alpha.
    #[must_use]
    pub fn premultiplied(channels: [u32; 3], alpha: u32) -> Self {
        let [red, green, blue] = channels.map(|value| scale(value.min(LINEAR_ONE), alpha));
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
}

/// `value * weight / LINEAR_ONE`, rounded to nearest.
#[must_use]
pub(crate) fn scale(value: u32, weight: u32) -> u32 {
    let product = u64::from(value).saturating_mul(u64::from(weight));
    let rounded = product
        .saturating_add(u64::from(LINEAR_ONE) / 2)
        .checked_div(u64::from(LINEAR_ONE))
        .unwrap_or(0);
    u32::try_from(rounded).unwrap_or(LINEAR_ONE).min(LINEAR_ONE)
}

/// `value * LINEAR_ONE / divisor`, rounded to nearest and clamped.
fn unscale(value: u32, divisor: u32) -> u32 {
    if divisor == 0 {
        return 0;
    }
    let product = u64::from(value).saturating_mul(u64::from(LINEAR_ONE));
    let rounded = product
        .saturating_add(u64::from(divisor) / 2)
        .checked_div(u64::from(divisor))
        .unwrap_or(0);
    u32::try_from(rounded).unwrap_or(LINEAR_ONE).min(LINEAR_ONE)
}

/// A Q32.32 value in `[0, 1]` as a linear channel level.
fn level(value: Fixed) -> Result<u32, RasterError> {
    let scaled = value
        .clamp(Fixed::ZERO, Fixed::ONE)
        .mul_ratio(i64::from(LINEAR_ONE), 1)?;
    let rounded = scaled.mul_ratio(1, Fixed::ONE.bits())?.bits();
    u32::try_from(rounded).map_err(|_| RasterError::Overflow)
}

impl Surface<'_> {
    /// The pixel at `(x, y)` in premultiplied linear light, or `None` outside
    /// the surface.
    ///
    /// An [`Format::A8`] surface carries coverage and no colour, so its pixel
    /// has black colour channels. A display-space surface is opaque, so its
    /// pixel has full alpha and needs no premultiplication.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32, gamma: &Gamma) -> Option<Pixel> {
        Some(match self.texel(x, y)? {
            Texel::Coverage(value) => Pixel {
                alpha: expand(value),
                ..Pixel::CLEAR
            },
            Texel::Linear([red, green, blue, alpha]) => Pixel {
                red: u32::from(red),
                green: u32::from(green),
                blue: u32::from(blue),
                alpha: u32::from(alpha),
            },
            Texel::Display([red, green, blue]) => Pixel {
                red: gamma.decode(red),
                green: gamma.decode(green),
                blue: gamma.decode(blue),
                alpha: LINEAR_ONE,
            },
        })
    }

    /// Write a premultiplied linear-light pixel at `(x, y)`.
    ///
    /// A display-space surface is opaque, so the premultiplication is undone
    /// before the channels are encoded.
    /// # Errors
    /// Returns `OutOfBounds` for a position beyond the visible pixels.
    pub fn set_pixel(
        &mut self,
        x: u32,
        y: u32,
        pixel: Pixel,
        gamma: &Gamma,
    ) -> Result<(), RasterError> {
        let texel = match self.format() {
            Format::A8 => Texel::Coverage(narrow(pixel.alpha)),
            Format::Rgba16 => Texel::Linear([
                channel(pixel.red),
                channel(pixel.green),
                channel(pixel.blue),
                channel(pixel.alpha),
            ]),
            Format::Rgbx8888 | Format::Bgrx8888 => {
                Texel::Display(pixel.straight().map(|value| gamma.encode(value)))
            }
        };
        self.set_texel(x, y, texel)
    }
}

/// A coverage byte as a linear level, with `255` reaching [`LINEAR_ONE`].
fn expand(value: u8) -> u32 {
    u32::from(value).saturating_mul(257)
}

/// A linear level as a coverage byte, rounded to nearest.
fn narrow(value: u32) -> u8 {
    let scaled = value.min(LINEAR_ONE).saturating_mul(255);
    u8::try_from(
        scaled
            .saturating_add(LINEAR_ONE / 2)
            .checked_div(LINEAR_ONE)
            .unwrap_or(0),
    )
    .unwrap_or(u8::MAX)
}

/// A linear level as the `u16` an [`Format::Rgba16`] surface stores.
fn channel(value: u32) -> u16 {
    u16::try_from(value.min(LINEAR_ONE)).unwrap_or(u16::MAX)
}
