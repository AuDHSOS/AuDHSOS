// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::unreachable, reason = "a texel of a format the test built")]

//! R6: one coverage mask composited with one text colour, in linear light.

use text_core::Fixed;

use crate::{Format, Gamma, LINEAR_ONE, Mask, Paint, Surface, Texel, blend_mono};

/// A mask of these coverage bytes, one row per slice.
fn mask(rows: &[&[u8]]) -> (Vec<u8>, u32, u32) {
    let width = u32::try_from(rows.first().map_or(0, |row| row.len())).unwrap();
    let height = u32::try_from(rows.len()).unwrap();
    let mut bytes = Vec::new();
    for row in rows {
        bytes.extend_from_slice(row);
    }
    (bytes, width, height)
}

/// Draw one coverage byte in `paint` over a one-pixel destination of `format`
/// prefilled with `background`, and return the destination bytes.
fn one_pixel(format: Format, background: Texel, paint: Paint, coverage: u8) -> Vec<u8> {
    let gamma = Gamma::default_value().unwrap();
    let pixels = usize::try_from(format.bytes_per_pixel()).unwrap();
    let mut bytes = vec![0_u8; pixels];
    let mask_bytes = [coverage];
    let mask = Mask::new(&mask_bytes, 1, 1, 1).unwrap();
    {
        let mut destination =
            Surface::new(&mut bytes, 1, 1, u32::try_from(pixels).unwrap(), format).unwrap();
        destination.set_texel(0, 0, background).unwrap();
        blend_mono(&mut destination, (0, 0), mask, paint, &gamma).unwrap();
    }
    bytes
}

#[test]
fn zero_coverage_leaves_the_destination_alone() {
    for format in Format::ALL {
        let background = match format {
            Format::A8 => Texel::Coverage(0x44),
            Format::Rgba16 => Texel::Linear([0x1111, 0x2222, 0x3333, 0xffff]),
            Format::Rgbx8888 | Format::Bgrx8888 => Texel::Display([0x11, 0x22, 0x33]),
        };
        let before = one_pixel(*format, background, Paint::opaque(255, 0, 0), 0);
        let mut expected = vec![0_u8; usize::try_from(format.bytes_per_pixel()).unwrap()];
        {
            let mut surface =
                Surface::new(&mut expected, 1, 1, format.bytes_per_pixel(), *format).unwrap();
            surface.set_texel(0, 0, background).unwrap();
        }
        assert_eq!(before, expected, "{format:?}");
    }
}

#[test]
fn full_coverage_of_an_opaque_colour_replaces_the_destination() {
    let gamma = Gamma::default_value().unwrap();
    for format in [Format::Rgbx8888, Format::Bgrx8888, Format::Rgba16] {
        let bytes = one_pixel(
            format,
            match format {
                Format::Rgba16 => Texel::Linear([0, 0, 0, u16::MAX]),
                _ => Texel::Display([0, 0, 0]),
            },
            Paint::opaque(0x40, 0x80, 0xc0),
            255,
        );
        let mut copy = bytes.clone();
        let surface = Surface::new(&mut copy, 1, 1, format.bytes_per_pixel(), format).unwrap();
        match surface.texel(0, 0).unwrap() {
            Texel::Display(channels) => assert_eq!(channels, [0x40, 0x80, 0xc0], "{format:?}"),
            Texel::Linear([red, green, blue, alpha]) => {
                assert_eq!(u32::from(red), gamma.decode(0x40));
                assert_eq!(u32::from(green), gamma.decode(0x80));
                assert_eq!(u32::from(blue), gamma.decode(0xc0));
                assert_eq!(u32::from(alpha), LINEAR_ONE);
            }
            Texel::Coverage(_) => unreachable!(),
        }
    }
}

#[test]
fn a_middle_coverage_is_the_linear_light_average() {
    // Coverage 128 of white over black leaves `128/255` of linear light.
    let bytes = one_pixel(
        Format::Rgba16,
        Texel::Linear([0, 0, 0, u16::MAX]),
        Paint::opaque(255, 255, 255),
        128,
    );
    let mut copy = bytes;
    let surface = Surface::new(&mut copy, 1, 1, 8, Format::Rgba16).unwrap();
    let Texel::Linear([red, _, _, alpha]) = surface.texel(0, 0).unwrap() else {
        unreachable!()
    };
    let expected = u32::try_from(u64::from(LINEAR_ONE) * 128 / 255).unwrap();
    assert!(
        u32::from(red).abs_diff(expected) <= 1,
        "{red} of {expected}"
    );
    assert_eq!(u32::from(alpha), LINEAR_ONE);
}

#[test]
fn light_on_dark_and_dark_on_light_move_the_same_distance() {
    // D-183 exists for this: in linear light one coverage value moves the
    // destination by one amount whichever way round the two colours are. An
    // uncorrected blend in display space moves them by 51395 and 14386.
    let gamma = Gamma::default_value().unwrap();
    let dark = one_pixel(
        Format::Rgbx8888,
        Texel::Display([255, 255, 255]),
        Paint::opaque(0, 0, 0),
        128,
    );
    let light = one_pixel(
        Format::Rgbx8888,
        Texel::Display([0, 0, 0]),
        Paint::opaque(255, 255, 255),
        128,
    );
    let fell = LINEAR_ONE - gamma.decode(*dark.first().unwrap());
    let rose = gamma.decode(*light.first().unwrap());
    assert!(
        fell.abs_diff(rose) <= 512,
        "dark fell {fell} and light rose {rose}"
    );
}

#[test]
fn a_translucent_paint_weights_the_coverage() {
    let half = Paint {
        alpha: Fixed::ONE.mul_ratio(1, 2).unwrap(),
        ..Paint::opaque(255, 255, 255)
    };
    let bytes = one_pixel(
        Format::Rgba16,
        Texel::Linear([0, 0, 0, u16::MAX]),
        half,
        255,
    );
    let mut copy = bytes;
    let surface = Surface::new(&mut copy, 1, 1, 8, Format::Rgba16).unwrap();
    let Texel::Linear([red, _, _, _]) = surface.texel(0, 0).unwrap() else {
        unreachable!()
    };
    assert!(u32::from(red).abs_diff(LINEAR_ONE / 2) <= 2, "{red}");
}

#[test]
fn a_mask_overhanging_every_edge_writes_only_inside() {
    let gamma = Gamma::default_value().unwrap();
    let (mask_bytes, width, height) = mask(&[&[255; 6], &[255; 6], &[255; 6], &[255; 6]]);
    let plane = Mask::new(&mask_bytes, width, height, width).unwrap();
    for at in [(-2_i32, -2_i32), (1, 1), (-2, 1), (1, -2)] {
        let mut bytes = vec![0xa5_u8; 4 * 3];
        {
            let mut destination = Surface::new(&mut bytes, 2, 3, 4, Format::A8).unwrap();
            destination.clear();
            blend_mono(
                &mut destination,
                at,
                plane,
                Paint::opaque(255, 255, 255),
                &gamma,
            )
            .unwrap();
        }
        for row in 0..3_usize {
            assert_eq!(&bytes[row * 4 + 2..row * 4 + 4], &[0xa5, 0xa5], "{at:?}");
        }
    }
}

#[test]
fn the_same_draw_twice_gives_identical_bytes() {
    let gamma = Gamma::default_value().unwrap();
    let (mask_bytes, width, height) =
        mask(&[&[0, 64, 128, 255], &[255, 128, 64, 0], &[32, 96, 160, 224]]);
    let plane = Mask::new(&mask_bytes, width, height, width).unwrap();
    let paint = Paint {
        alpha: Fixed::ONE.mul_ratio(3, 4).unwrap(),
        ..Paint::opaque(0x20, 0x90, 0xf0)
    };
    let mut first = vec![0_u8; 4 * 4 * 3];
    let mut second = vec![0_u8; 4 * 4 * 3 + 64];
    for bytes in [&mut first, &mut second] {
        let mut destination = Surface::new(bytes, 4, 3, 16, Format::Rgbx8888).unwrap();
        destination.clear();
        blend_mono(&mut destination, (0, 0), plane, paint, &gamma).unwrap();
        blend_mono(&mut destination, (1, 1), plane, paint, &gamma).unwrap();
    }
    assert_eq!(first[..], second[..first.len()]);
}
