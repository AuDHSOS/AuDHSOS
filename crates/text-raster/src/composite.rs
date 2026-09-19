// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R11: the twenty-eight compositing and blending modes a COLR glyph names.
//!
//! The thirteen compositing operators are section 9.1 of
//! `docs/w3c/compositing-1.html:1635` and run on premultiplied values. The
//! fifteen blend modes are section 10, `docs/w3c/compositing-1.html:1785`,
//! which requires unpremultiplied ones, so a blend divides its two operands by
//! their alphas first; that division is the only one here.
//!
//! The four non-separable modes are implemented from the auxiliary functions
//! of `docs/w3c/compositing-1.html:1967` and not as source-over, which D-180
//! refuses.

use text_core::{Fixed, colr::CompositeMode};

use crate::{
    gamma::LINEAR_ONE,
    math::sqrt,
    pixel::{Pixel, scale},
};

/// One whole channel as a signed value, for the intermediates of the
/// non-separable modes, which leave the range before they are clipped back.
const ONE: i64 = 65_535;

/// `source` combined with `backdrop` under `mode`.
///
/// Both are premultiplied linear-light pixels and so is the result.
#[must_use]
pub fn composite(mode: CompositeMode, source: Pixel, backdrop: Pixel) -> Pixel {
    if let Some((fa, fb)) = fractions(mode, source.alpha, backdrop.alpha) {
        return weighted(source, fa, backdrop, fb);
    }
    let straight = source.straight();
    let mixed = blend(mode, straight, backdrop.straight());
    // The mixing result is weighted by the backdrop alpha and the outcome is
    // then composited source-over, `docs/w3c/compositing-1.html:1785`.
    let rest = LINEAR_ONE.saturating_sub(backdrop.alpha);
    let colours = [0_usize, 1, 2].map(|index| {
        let own = straight.get(index).copied().unwrap_or(0);
        let mixed = mixed.get(index).copied().unwrap_or(0);
        scale(own, rest).saturating_add(scale(mixed, backdrop.alpha))
    });
    Pixel::premultiplied(colours, source.alpha).over(backdrop)
}

/// The `Fa` and `Fb` of one compositing operator, or `None` for a blend mode.
const fn fractions(mode: CompositeMode, source: u32, backdrop: u32) -> Option<(u32, u32)> {
    let rest_source = LINEAR_ONE.saturating_sub(source);
    let rest_backdrop = LINEAR_ONE.saturating_sub(backdrop);
    Some(match mode {
        CompositeMode::Clear => (0, 0),
        CompositeMode::Src => (LINEAR_ONE, 0),
        CompositeMode::Dest => (0, LINEAR_ONE),
        CompositeMode::SrcOver => (LINEAR_ONE, rest_source),
        CompositeMode::DestOver => (rest_backdrop, LINEAR_ONE),
        CompositeMode::SrcIn => (backdrop, 0),
        CompositeMode::DestIn => (0, source),
        CompositeMode::SrcOut => (rest_backdrop, 0),
        CompositeMode::DestOut => (0, rest_source),
        CompositeMode::SrcAtop => (backdrop, rest_source),
        CompositeMode::DestAtop => (rest_backdrop, source),
        CompositeMode::Xor => (rest_backdrop, rest_source),
        CompositeMode::Plus => (LINEAR_ONE, LINEAR_ONE),
        _ => return None,
    })
}

/// `source * fa + backdrop * fb`, the general Porter-Duff equation.
fn weighted(source: Pixel, fa: u32, backdrop: Pixel, fb: u32) -> Pixel {
    let mix = |a: u32, b: u32| scale(a, fa).saturating_add(scale(b, fb)).min(LINEAR_ONE);
    Pixel {
        red: mix(source.red, backdrop.red),
        green: mix(source.green, backdrop.green),
        blue: mix(source.blue, backdrop.blue),
        alpha: mix(source.alpha, backdrop.alpha),
    }
}

/// The mixing function `B(Cb, Cs)` of one blend mode, on straight colours.
fn blend(mode: CompositeMode, source: [u32; 3], backdrop: [u32; 3]) -> [u32; 3] {
    match mode {
        CompositeMode::Hue => set_luminosity(
            set_saturation(source, saturation(backdrop)),
            luminosity(backdrop),
        ),
        CompositeMode::Saturation => set_luminosity(
            set_saturation(backdrop, saturation(source)),
            luminosity(backdrop),
        ),
        CompositeMode::Color => set_luminosity(source, luminosity(backdrop)),
        CompositeMode::Luminosity => set_luminosity(backdrop, luminosity(source)),
        _ => {
            let mut out = [0_u32; 3];
            for (index, channel) in out.iter_mut().enumerate() {
                let (a, b) = (
                    source.get(index).copied().unwrap_or(0),
                    backdrop.get(index).copied().unwrap_or(0),
                );
                *channel = separable(mode, a, b);
            }
            out
        }
    }
}

/// One channel of a separable blend mode.
fn separable(mode: CompositeMode, source: u32, backdrop: u32) -> u32 {
    match mode {
        CompositeMode::Multiply => scale(source, backdrop),
        CompositeMode::Screen => screen(source, backdrop),
        CompositeMode::Overlay => hard_light(backdrop, source),
        CompositeMode::Darken => source.min(backdrop),
        CompositeMode::Lighten => source.max(backdrop),
        CompositeMode::ColorDodge => color_dodge(source, backdrop),
        CompositeMode::ColorBurn => color_burn(source, backdrop),
        CompositeMode::HardLight => hard_light(source, backdrop),
        CompositeMode::SoftLight => soft_light(source, backdrop),
        CompositeMode::Difference => source.abs_diff(backdrop),
        CompositeMode::Exclusion => exclusion(source, backdrop),
        // Every other value reached `fractions` above.
        _ => backdrop,
    }
}

/// `Cb + Cs - Cb * Cs`.
fn screen(source: u32, backdrop: u32) -> u32 {
    source
        .saturating_add(backdrop)
        .saturating_sub(scale(source, backdrop))
        .min(LINEAR_ONE)
}

/// `Cb + Cs - 2 * Cb * Cs`.
fn exclusion(source: u32, backdrop: u32) -> u32 {
    let product = scale(source, backdrop);
    source
        .saturating_add(backdrop)
        .saturating_sub(product)
        .saturating_sub(product)
        .min(LINEAR_ONE)
}

/// `min(1, Cb / (1 - Cs))`, with the two limits the specification names.
fn color_dodge(source: u32, backdrop: u32) -> u32 {
    if backdrop == 0 {
        return 0;
    }
    if source >= LINEAR_ONE {
        return LINEAR_ONE;
    }
    divide(backdrop, LINEAR_ONE.saturating_sub(source))
}

/// `1 - min(1, (1 - Cb) / Cs)`, with the two limits the specification names.
fn color_burn(source: u32, backdrop: u32) -> u32 {
    if backdrop >= LINEAR_ONE {
        return LINEAR_ONE;
    }
    if source == 0 {
        return 0;
    }
    LINEAR_ONE.saturating_sub(divide(LINEAR_ONE.saturating_sub(backdrop), source))
}

/// `Multiply(Cb, 2 * Cs)` below a half and `Screen(Cb, 2 * Cs - 1)` above it.
fn hard_light(source: u32, backdrop: u32) -> u32 {
    let doubled = source.saturating_mul(2);
    if doubled <= LINEAR_ONE {
        scale(backdrop, doubled)
    } else {
        screen(backdrop, doubled.saturating_sub(LINEAR_ONE))
    }
}

/// The soft light of `docs/w3c/compositing-1.html:1832`.
fn soft_light(source: u32, backdrop: u32) -> u32 {
    let doubled = source.saturating_mul(2);
    if doubled <= LINEAR_ONE {
        let weight = LINEAR_ONE.saturating_sub(doubled);
        let pull = scale(scale(weight, backdrop), LINEAR_ONE.saturating_sub(backdrop));
        backdrop.saturating_sub(pull)
    } else {
        let weight = doubled.saturating_sub(LINEAR_ONE);
        let target = soft_light_target(backdrop);
        let step = target.saturating_sub(backdrop.min(target));
        let drop = backdrop.saturating_sub(target.min(backdrop));
        backdrop
            .saturating_add(scale(weight, step))
            .saturating_sub(scale(weight, drop))
            .min(LINEAR_ONE)
    }
}

/// `D(Cb)` of the soft light formula: the polynomial
/// `((16 * x - 12) * x + 4) * x` below a quarter and `sqrt(x)` at and above
/// it. The polynomial leaves the channel range on the way, so it is evaluated
/// signed and unclamped.
fn soft_light_target(backdrop: u32) -> u32 {
    let level = i64::from(backdrop);
    if level.saturating_mul(4) <= ONE {
        let inner = level
            .saturating_mul(16)
            .saturating_sub(ONE.saturating_mul(12));
        let scaled = ratio(inner.saturating_mul(level), ONE).saturating_add(ONE.saturating_mul(4));
        return clamp(ratio(scaled.saturating_mul(level), ONE));
    }
    let value = Fixed::from_bits(ratio(level.saturating_mul(1 << 32), ONE));
    let Ok(root) = sqrt(value) else {
        return backdrop;
    };
    clamp(ratio(root.bits().saturating_mul(ONE), 1 << 32))
}

/// `value / divisor`, truncated, with a zero divisor giving zero.
fn ratio(value: i64, divisor: i64) -> i64 {
    value.checked_div(divisor).unwrap_or(0)
}

/// A signed level as a channel of `[0, LINEAR_ONE]`.
fn clamp(value: i64) -> u32 {
    u32::try_from(value.clamp(0, ONE)).unwrap_or(LINEAR_ONE)
}

/// `value * LINEAR_ONE / divisor`, clamped to one whole channel.
fn divide(value: u32, divisor: u32) -> u32 {
    if divisor == 0 {
        return LINEAR_ONE;
    }
    let scaled = u64::from(value).saturating_mul(u64::from(LINEAR_ONE));
    u32::try_from(scaled.checked_div(u64::from(divisor)).unwrap_or(0))
        .unwrap_or(LINEAR_ONE)
        .min(LINEAR_ONE)
}

/// `Lum(C) = 0.3 * R + 0.59 * G + 0.11 * B`.
fn luminosity(colour: [u32; 3]) -> i64 {
    signed_luminosity(colour.map(i64::from))
}

/// `Sat(C) = max(C) - min(C)`.
fn saturation(colour: [u32; 3]) -> i64 {
    let [red, green, blue] = colour.map(i64::from);
    red.max(green)
        .max(blue)
        .saturating_sub(red.min(green).min(blue))
}

/// `SetLum(C, l)`, which shifts every channel and then clips the colour.
fn set_luminosity(colour: [u32; 3], target: i64) -> [u32; 3] {
    let signed = colour.map(i64::from);
    let shift = target.saturating_sub(luminosity(colour));
    clip_color(signed.map(|channel| channel.saturating_add(shift)))
}

/// `ClipColor(C)`, which pulls a colour back into range about its luminosity.
fn clip_color(colour: [i64; 3]) -> [u32; 3] {
    let level = signed_luminosity(colour);
    let low = colour.iter().copied().min().unwrap_or(0);
    let high = colour.iter().copied().max().unwrap_or(0);
    let adjusted = colour.map(|channel| {
        let mut value = channel;
        if low < 0 && level != low {
            let pulled = value.saturating_sub(level).saturating_mul(level);
            value = level.saturating_add(ratio(pulled, level.saturating_sub(low)));
        }
        if high > ONE && high != level {
            let pushed = value
                .saturating_sub(level)
                .saturating_mul(ONE.saturating_sub(level));
            value = level.saturating_add(ratio(pushed, high.saturating_sub(level)));
        }
        value
    });
    adjusted.map(clamp)
}

/// `Lum` of a colour whose channels may be outside the range.
fn signed_luminosity(colour: [i64; 3]) -> i64 {
    let [red, green, blue] = colour;
    let sum = red
        .saturating_mul(30)
        .saturating_add(green.saturating_mul(59))
        .saturating_add(blue.saturating_mul(11));
    ratio(sum, 100)
}

/// `SetSat(C, s)`, which rescales the spread between the extreme channels.
fn set_saturation(colour: [u32; 3], target: i64) -> [u32; 3] {
    let signed = colour.map(i64::from);
    let low = signed.iter().copied().min().unwrap_or(0);
    let high = signed.iter().copied().max().unwrap_or(0);
    if high <= low {
        return [0; 3];
    }
    let span = high.saturating_sub(low);
    signed
        .map(|channel| ratio(channel.saturating_sub(low).saturating_mul(target), span))
        .map(clamp)
}
