// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::arithmetic_side_effects, reason = "bounded fixture arithmetic")]
#![expect(
    clippy::type_complexity,
    reason = "the operator table of the specification"
)]
#![expect(
    clippy::float_cmp,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the reference formulas of docs/w3c/compositing-1.html are stated \
              in real arithmetic and are evaluated here to a tolerance, which \
              D-177 admits for a test and for no product path"
)]

//! R11: the twenty-eight compositing and blending modes.

use text_core::colr::CompositeMode;

use crate::{LINEAR_ONE, Pixel, composite};

/// Every mode value COLR defines, `docs/microsoft/colr.html:3094`.
const MODES: [CompositeMode; 28] = [
    CompositeMode::Clear,
    CompositeMode::Src,
    CompositeMode::Dest,
    CompositeMode::SrcOver,
    CompositeMode::DestOver,
    CompositeMode::SrcIn,
    CompositeMode::DestIn,
    CompositeMode::SrcOut,
    CompositeMode::DestOut,
    CompositeMode::SrcAtop,
    CompositeMode::DestAtop,
    CompositeMode::Xor,
    CompositeMode::Plus,
    CompositeMode::Screen,
    CompositeMode::Overlay,
    CompositeMode::Darken,
    CompositeMode::Lighten,
    CompositeMode::ColorDodge,
    CompositeMode::ColorBurn,
    CompositeMode::HardLight,
    CompositeMode::SoftLight,
    CompositeMode::Difference,
    CompositeMode::Exclusion,
    CompositeMode::Multiply,
    CompositeMode::Hue,
    CompositeMode::Saturation,
    CompositeMode::Color,
    CompositeMode::Luminosity,
];

/// A premultiplied pixel from straight channels and an alpha, both in
/// hundredths of a whole channel.
fn pixel(red: u32, green: u32, blue: u32, alpha: u32) -> Pixel {
    let level = |value: u32| LINEAR_ONE * value / 100;
    Pixel::premultiplied([level(red), level(green), level(blue)], level(alpha))
}

/// Whether two levels agree to within a thousandth of a channel.
fn near(left: u32, right: u32) -> bool {
    left.abs_diff(right) <= LINEAR_ONE / 1000 + 2
}

fn near_pixel(left: Pixel, right: Pixel) -> bool {
    near(left.red, right.red)
        && near(left.green, right.green)
        && near(left.blue, right.blue)
        && near(left.alpha, right.alpha)
}

/// The operand matrix every mode is checked over.
fn operands() -> [(Pixel, Pixel); 5] {
    [
        (pixel(100, 0, 0, 100), pixel(0, 0, 100, 100)),
        (pixel(100, 0, 0, 100), pixel(0, 0, 100, 0)),
        (pixel(100, 0, 0, 0), pixel(0, 0, 100, 100)),
        (pixel(80, 40, 20, 50), pixel(20, 60, 90, 75)),
        (pixel(25, 50, 75, 25), pixel(75, 50, 25, 50)),
    ]
}

#[test]
fn every_mode_stays_inside_one_channel_and_is_premultiplied() {
    for mode in MODES {
        for (source, backdrop) in operands() {
            let out = composite(mode, source, backdrop);
            for channel in [out.red, out.green, out.blue, out.alpha] {
                assert!(channel <= LINEAR_ONE, "{mode:?} gave {channel}");
            }
            // `Plus` is the one mode the specification lets exceed the
            // backdrop's own alpha, and even it stays inside one channel.
            if mode != CompositeMode::Plus {
                assert!(
                    out.red <= out.alpha + LINEAR_ONE / 1000 + 2,
                    "{mode:?} is not premultiplied: {} over {}",
                    out.red,
                    out.alpha
                );
            }
        }
    }
}

#[test]
fn the_thirteen_operators_follow_the_porter_duff_equation() {
    // `co = as * Fa * Cs + ab * Fb * Cb` with the `Fa` and `Fb` of each
    // operator, `docs/w3c/compositing-1.html:1635`, on premultiplied values.
    let table: [(CompositeMode, fn(u32, u32) -> (u32, u32)); 13] = [
        (CompositeMode::Clear, |_, _| (0, 0)),
        (CompositeMode::Src, |_, _| (LINEAR_ONE, 0)),
        (CompositeMode::Dest, |_, _| (0, LINEAR_ONE)),
        (CompositeMode::SrcOver, |source, _| {
            (LINEAR_ONE, LINEAR_ONE - source)
        }),
        (CompositeMode::DestOver, |_, backdrop| {
            (LINEAR_ONE - backdrop, LINEAR_ONE)
        }),
        (CompositeMode::SrcIn, |_, backdrop| (backdrop, 0)),
        (CompositeMode::DestIn, |source, _| (0, source)),
        (CompositeMode::SrcOut, |_, backdrop| {
            (LINEAR_ONE - backdrop, 0)
        }),
        (CompositeMode::DestOut, |source, _| (0, LINEAR_ONE - source)),
        (CompositeMode::SrcAtop, |source, backdrop| {
            (backdrop, LINEAR_ONE - source)
        }),
        (CompositeMode::DestAtop, |source, backdrop| {
            (LINEAR_ONE - backdrop, source)
        }),
        (CompositeMode::Xor, |source, backdrop| {
            (LINEAR_ONE - backdrop, LINEAR_ONE - source)
        }),
        (CompositeMode::Plus, |_, _| (LINEAR_ONE, LINEAR_ONE)),
    ];
    for (mode, weights) in table {
        for (source, backdrop) in operands() {
            let (fa, fb) = weights(source.alpha, backdrop.alpha);
            let mix = |a: u32, b: u32| {
                let sum = u64::from(a) * u64::from(fa) + u64::from(b) * u64::from(fb);
                u32::try_from((sum + u64::from(LINEAR_ONE) / 2) / u64::from(LINEAR_ONE))
                    .unwrap()
                    .min(LINEAR_ONE)
            };
            let want = Pixel {
                red: mix(source.red, backdrop.red),
                green: mix(source.green, backdrop.green),
                blue: mix(source.blue, backdrop.blue),
                alpha: mix(source.alpha, backdrop.alpha),
            };
            let got = composite(mode, source, backdrop);
            assert!(near_pixel(got, want), "{mode:?}: {got:?} against {want:?}");
        }
    }
}

/// The straight-colour mixing function of each separable mode, independently
/// written from `docs/w3c/compositing-1.html:1832`, on hundredths.
fn mixed(mode: CompositeMode, source: f64, backdrop: f64) -> f64 {
    match mode {
        CompositeMode::Multiply => backdrop * source,
        CompositeMode::Screen => backdrop + source - backdrop * source,
        CompositeMode::Overlay => mixed(CompositeMode::HardLight, backdrop, source),
        CompositeMode::Darken => backdrop.min(source),
        CompositeMode::Lighten => backdrop.max(source),
        CompositeMode::ColorDodge => {
            if backdrop == 0.0 {
                0.0
            } else if source == 1.0 {
                1.0
            } else {
                1.0_f64.min(backdrop / (1.0 - source))
            }
        }
        CompositeMode::ColorBurn => {
            if backdrop == 1.0 {
                1.0
            } else if source == 0.0 {
                0.0
            } else {
                1.0 - 1.0_f64.min((1.0 - backdrop) / source)
            }
        }
        CompositeMode::HardLight => {
            if source <= 0.5 {
                backdrop * (2.0 * source)
            } else {
                let doubled = 2.0 * source - 1.0;
                backdrop + doubled - backdrop * doubled
            }
        }
        CompositeMode::SoftLight => {
            if source <= 0.5 {
                backdrop - (1.0 - 2.0 * source) * backdrop * (1.0 - backdrop)
            } else {
                let target = if backdrop <= 0.25 {
                    ((16.0 * backdrop - 12.0) * backdrop + 4.0) * backdrop
                } else {
                    backdrop.sqrt()
                };
                backdrop + (2.0 * source - 1.0) * (target - backdrop)
            }
        }
        CompositeMode::Difference => (backdrop - source).abs(),
        CompositeMode::Exclusion => backdrop + source - 2.0 * backdrop * source,
        _ => backdrop,
    }
}

#[test]
fn the_eleven_separable_modes_follow_their_own_formulas() {
    let modes = [
        CompositeMode::Multiply,
        CompositeMode::Screen,
        CompositeMode::Overlay,
        CompositeMode::Darken,
        CompositeMode::Lighten,
        CompositeMode::ColorDodge,
        CompositeMode::ColorBurn,
        CompositeMode::HardLight,
        CompositeMode::SoftLight,
        CompositeMode::Difference,
        CompositeMode::Exclusion,
    ];
    for mode in modes {
        for step in 0..=10_u32 {
            for other in 0..=10_u32 {
                let (source, backdrop) = (
                    pixel(step * 10, step * 10, step * 10, 100),
                    pixel(other * 10, other * 10, other * 10, 100),
                );
                let want = mixed(mode, f64::from(step) / 10.0, f64::from(other) / 10.0);
                let level = (want.clamp(0.0, 1.0) * f64::from(LINEAR_ONE)).round() as u32;
                let got = composite(mode, source, backdrop).red;
                assert!(
                    got.abs_diff(level) <= LINEAR_ONE / 200,
                    "{mode:?} at {step}/{other}: {got} against {level}"
                );
            }
        }
    }
}

#[test]
fn the_four_non_separable_modes_are_not_source_over() {
    // A colour whose channels differ is what separates them; source-over would
    // return the source unchanged for every one of the four.
    let source = pixel(90, 20, 30, 100);
    let backdrop = pixel(10, 70, 40, 100);
    for mode in [
        CompositeMode::Hue,
        CompositeMode::Saturation,
        CompositeMode::Color,
        CompositeMode::Luminosity,
    ] {
        let out = composite(mode, source, backdrop);
        assert!(
            !near_pixel(out, source),
            "{mode:?} returned the source unchanged"
        );
    }
}

/// `Lum`, `Sat`, `ClipColor`, `SetLum` and `SetSat`, written independently.
mod reference {
    pub(super) fn lum(colour: [f64; 3]) -> f64 {
        0.30 * colour[0] + 0.59 * colour[1] + 0.11 * colour[2]
    }

    pub(super) fn clip(colour: [f64; 3]) -> [f64; 3] {
        let level = lum(colour);
        let low = colour[0].min(colour[1]).min(colour[2]);
        let high = colour[0].max(colour[1]).max(colour[2]);
        let mut out = colour;
        if low < 0.0 {
            for channel in &mut out {
                *channel = level + (*channel - level) * level / (level - low);
            }
        }
        if high > 1.0 {
            for channel in &mut out {
                *channel = level + (*channel - level) * (1.0 - level) / (high - level);
            }
        }
        out
    }

    pub(super) fn set_lum(colour: [f64; 3], target: f64) -> [f64; 3] {
        let shift = target - lum(colour);
        clip(colour.map(|channel| channel + shift))
    }

    pub(super) fn sat(colour: [f64; 3]) -> f64 {
        colour[0].max(colour[1]).max(colour[2]) - colour[0].min(colour[1]).min(colour[2])
    }

    pub(super) fn set_sat(colour: [f64; 3], target: f64) -> [f64; 3] {
        let low = colour[0].min(colour[1]).min(colour[2]);
        let high = colour[0].max(colour[1]).max(colour[2]);
        if high <= low {
            return [0.0; 3];
        }
        colour.map(|channel| (channel - low) * target / (high - low))
    }
}

#[test]
fn the_four_non_separable_modes_follow_their_auxiliary_functions() {
    let cases = [
        ([90_u32, 20, 30], [10_u32, 70, 40]),
        ([100, 100, 100], [0, 0, 0]),
        ([0, 0, 0], [100, 100, 100]),
        ([50, 50, 50], [20, 80, 60]),
        ([90, 10, 10], [10, 90, 90]),
        // A pair that drives `ClipColor` through both of its branches.
        ([100, 0, 0], [5, 5, 5]),
        ([0, 0, 100], [95, 95, 95]),
        // A grey source, whose saturation is zero, for the `SetSat` branch.
        ([40, 40, 40], [10, 60, 90]),
    ];
    for (source_levels, backdrop_levels) in cases {
        let source = pixel(source_levels[0], source_levels[1], source_levels[2], 100);
        let backdrop = pixel(
            backdrop_levels[0],
            backdrop_levels[1],
            backdrop_levels[2],
            100,
        );
        let cs = source_levels.map(|value| f64::from(value) / 100.0);
        let cb = backdrop_levels.map(|value| f64::from(value) / 100.0);
        let expected = [
            (
                CompositeMode::Hue,
                reference::set_lum(
                    reference::set_sat(cs, reference::sat(cb)),
                    reference::lum(cb),
                ),
            ),
            (
                CompositeMode::Saturation,
                reference::set_lum(
                    reference::set_sat(cb, reference::sat(cs)),
                    reference::lum(cb),
                ),
            ),
            (
                CompositeMode::Color,
                reference::set_lum(cs, reference::lum(cb)),
            ),
            (
                CompositeMode::Luminosity,
                reference::set_lum(cb, reference::lum(cs)),
            ),
        ];
        for (mode, want) in expected {
            let got = composite(mode, source, backdrop);
            for (index, channel) in [got.red, got.green, got.blue].into_iter().enumerate() {
                let level = (want[index].clamp(0.0, 1.0) * f64::from(LINEAR_ONE)).round() as u32;
                assert!(
                    channel.abs_diff(level) <= LINEAR_ONE / 100,
                    "{mode:?} channel {index} of {source_levels:?} over \
                     {backdrop_levels:?}: {channel} against {level}"
                );
            }
        }
    }
}

#[test]
fn a_transparent_backdrop_leaves_a_blend_at_the_source() {
    // The mixing result is weighted by the backdrop alpha, so with none of it
    // every blend mode is source-over.
    let source = pixel(80, 40, 20, 100);
    let clear = Pixel::CLEAR;
    for mode in MODES {
        if matches!(
            mode,
            CompositeMode::Clear
                | CompositeMode::Dest
                | CompositeMode::SrcIn
                | CompositeMode::DestIn
                | CompositeMode::DestOut
                | CompositeMode::SrcAtop
                | CompositeMode::DestAtop
        ) {
            continue;
        }
        let out = composite(mode, source, clear);
        assert!(
            near_pixel(out, source),
            "{mode:?} moved the source over nothing: {out:?}"
        );
    }
}

#[test]
fn clear_and_the_two_copies_ignore_what_they_are_told_to_ignore() {
    let source = pixel(100, 0, 0, 100);
    let backdrop = pixel(0, 100, 0, 100);
    assert_eq!(
        composite(CompositeMode::Clear, source, backdrop),
        Pixel::CLEAR
    );
    assert!(near_pixel(
        composite(CompositeMode::Src, source, backdrop),
        source
    ));
    assert!(near_pixel(
        composite(CompositeMode::Dest, source, backdrop),
        backdrop
    ));
}

#[test]
fn the_same_operands_composite_identically_twice() {
    for mode in MODES {
        for (source, backdrop) in operands() {
            assert_eq!(
                composite(mode, source, backdrop),
                composite(mode, source, backdrop),
                "{mode:?}"
            );
        }
    }
}
