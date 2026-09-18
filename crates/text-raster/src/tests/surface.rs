// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::arithmetic_side_effects, reason = "bounded fixture shapes")]

//! R1: the surface, its construction rules and its bounds-checked access.

use crate::{Format, RasterError, Surface, Texel};

/// A byte pattern that is not zero and not a texel any test writes, so that a
/// byte still carrying it was never written.
const SENTINEL: u8 = 0xa5;

/// Bytes enough for `height` rows of `stride`, plus `extra`, filled with the
/// sentinel.
fn canvas(height: u32, stride: u32, extra: usize) -> Vec<u8> {
    let needed = usize::try_from(u64::from(height) * u64::from(stride)).unwrap();
    vec![SENTINEL; needed + extra]
}

/// One texel of each format, distinguishable from zero.
fn sample(format: Format) -> Texel {
    match format {
        Format::A8 => Texel::Coverage(0x5c),
        Format::Rgba16 => Texel::Linear([0x0102, 0x0304, 0x0506, 0x0708]),
        Format::Rgbx8888 | Format::Bgrx8888 => Texel::Display([0x11, 0x22, 0x33]),
    }
}

#[test]
fn surface_every_format_reports_its_shape() {
    for &format in Format::ALL {
        let stride = 7 * format.bytes_per_pixel();
        let mut bytes = canvas(5, stride, 0);
        let surface = Surface::new(&mut bytes, 7, 5, stride, format).unwrap();
        assert_eq!(surface.width(), 7);
        assert_eq!(surface.height(), 5);
        assert_eq!(surface.stride(), stride);
        assert_eq!(surface.format(), format);
    }
}

#[test]
fn surface_bytes_per_pixel_matches_the_format() {
    assert_eq!(Format::A8.bytes_per_pixel(), 1);
    assert_eq!(Format::Rgba16.bytes_per_pixel(), 8);
    assert_eq!(Format::Rgbx8888.bytes_per_pixel(), 4);
    assert_eq!(Format::Bgrx8888.bytes_per_pixel(), 4);
    assert_eq!(Format::default(), Format::A8);
}

#[test]
fn surface_longer_slice_is_accepted() {
    let mut bytes = canvas(3, 4, 1024);
    assert!(Surface::new(&mut bytes, 4, 3, 4, Format::A8).is_ok());
}

#[test]
fn surface_zero_width_or_height_is_refused() {
    let mut bytes = canvas(4, 4, 0);
    assert_eq!(
        Surface::new(&mut bytes, 0, 3, 4, Format::A8).unwrap_err(),
        RasterError::Empty
    );
    assert_eq!(
        Surface::new(&mut bytes, 4, 0, 4, Format::A8).unwrap_err(),
        RasterError::Empty
    );
}

#[test]
fn surface_stride_below_the_row_is_refused() {
    for &format in Format::ALL {
        let row = 4 * format.bytes_per_pixel();
        let mut bytes = canvas(3, row, 0);
        assert_eq!(
            Surface::new(&mut bytes, 4, 3, row - 1, format).unwrap_err(),
            RasterError::Stride
        );
        assert!(Surface::new(&mut bytes, 4, 3, row, format).is_ok());
    }
}

#[test]
fn surface_slice_shorter_than_the_shape_is_refused() {
    let mut bytes = canvas(3, 4, 0);
    let short = bytes.len() - 1;
    assert_eq!(
        Surface::new(&mut bytes[..short], 4, 3, 4, Format::A8).unwrap_err(),
        RasterError::TooShort {
            needed: 12,
            given: 11,
        }
    );
}

#[test]
fn surface_row_beyond_u32_is_refused() {
    let mut bytes = canvas(1, 1, 0);
    assert_eq!(
        Surface::new(&mut bytes, u32::MAX, 1, u32::MAX, Format::Rgba16).unwrap_err(),
        RasterError::Overflow
    );
}

#[test]
fn surface_shape_beyond_the_address_space_is_refused() {
    let mut bytes = canvas(1, 1, 0);
    let error = Surface::new(&mut bytes, 1, u32::MAX, u32::MAX, Format::A8).unwrap_err();
    assert!(matches!(error, RasterError::TooShort { .. }));
}

#[test]
fn surface_every_corner_round_trips() {
    for &format in Format::ALL {
        let stride = 6 * format.bytes_per_pixel();
        let mut bytes = canvas(4, stride, 0);
        let mut surface = Surface::new(&mut bytes, 6, 4, stride, format).unwrap();
        surface.clear();
        for (x, y) in [(0, 0), (5, 0), (0, 3), (5, 3)] {
            surface.set_texel(x, y, sample(format)).unwrap();
            assert_eq!(surface.texel(x, y), Some(sample(format)));
        }
        assert_eq!(surface.texel(3, 2), Some(format.zero()));
    }
}

#[test]
fn surface_one_past_each_edge_is_refused() {
    for &format in Format::ALL {
        let stride = 6 * format.bytes_per_pixel();
        let mut bytes = canvas(4, stride, 0);
        let mut surface = Surface::new(&mut bytes, 6, 4, stride, format).unwrap();
        for (x, y) in [(6, 0), (0, 4), (6, 4), (u32::MAX, 0), (0, u32::MAX)] {
            assert_eq!(surface.texel(x, y), None);
            assert_eq!(
                surface.set_texel(x, y, sample(format)).unwrap_err(),
                RasterError::OutOfBounds
            );
        }
        assert_eq!(surface.row(4), None);
        assert_eq!(surface.row_mut(4), None);
    }
}

#[test]
fn surface_padding_survives_a_clear_and_a_full_write() {
    for &format in Format::ALL {
        let row = 5 * format.bytes_per_pixel();
        let stride = row + 3;
        let mut bytes = canvas(4, stride, 9);
        let mut surface = Surface::new(&mut bytes, 5, 4, stride, format).unwrap();
        surface.clear();
        surface.fill(sample(format)).unwrap();
        for y in 0..4_u32 {
            let start = usize::try_from(y * stride + row).unwrap();
            let end = usize::try_from((y + 1) * stride).unwrap();
            assert!(
                bytes[start..end].iter().all(|byte| *byte == SENTINEL),
                "row padding written for {format:?}"
            );
        }
        let tail = usize::try_from(4 * stride).unwrap();
        assert!(bytes[tail..].iter().all(|byte| *byte == SENTINEL));
    }
}

#[test]
fn surface_fill_covers_every_visible_pixel() {
    for &format in Format::ALL {
        let stride = 3 * format.bytes_per_pixel() + 2;
        let mut bytes = canvas(2, stride, 0);
        let mut surface = Surface::new(&mut bytes, 3, 2, stride, format).unwrap();
        surface.fill(sample(format)).unwrap();
        for y in 0..2 {
            for x in 0..3 {
                assert_eq!(surface.texel(x, y), Some(sample(format)));
            }
        }
    }
}

#[test]
fn surface_coverage_round_trips_every_byte() {
    let mut bytes = canvas(1, 256, 0);
    let mut surface = Surface::new(&mut bytes, 256, 1, 256, Format::A8).unwrap();
    for value in 0..=u8::MAX {
        let x = u32::from(value);
        surface.set_texel(x, 0, Texel::Coverage(value)).unwrap();
    }
    for value in 0..=u8::MAX {
        assert_eq!(
            surface.texel(u32::from(value), 0),
            Some(Texel::Coverage(value))
        );
    }
}

#[test]
fn surface_linear_round_trips_the_channel_extremes() {
    let cases = [
        [0, 0, 0, 0],
        [u16::MAX; 4],
        [1, 2, 3, 4],
        [u16::MAX, 0, u16::MAX, 0],
        [0x00ff, 0xff00, 0x0f0f, 0xf0f0],
    ];
    let mut bytes = canvas(1, 40, 0);
    let mut surface = Surface::new(&mut bytes, 5, 1, 40, Format::Rgba16).unwrap();
    for (x, channels) in cases.iter().enumerate() {
        let x = u32::try_from(x).unwrap();
        surface.set_texel(x, 0, Texel::Linear(*channels)).unwrap();
        assert_eq!(surface.texel(x, 0), Some(Texel::Linear(*channels)));
    }
    assert_eq!(surface.row(0).unwrap()[0..2], [0, 0]);
    assert_eq!(surface.row(0).unwrap()[16..18], [1, 0]);
}

#[test]
fn surface_display_formats_store_opposite_channel_orders() {
    let mut rgb = canvas(1, 4, 0);
    let mut bgr = canvas(1, 4, 0);
    let texel = Texel::Display([0x11, 0x22, 0x33]);
    let mut first = Surface::new(&mut rgb, 1, 1, 4, Format::Rgbx8888).unwrap();
    first.set_texel(0, 0, texel).unwrap();
    let mut second = Surface::new(&mut bgr, 1, 1, 4, Format::Bgrx8888).unwrap();
    second.set_texel(0, 0, texel).unwrap();
    assert_eq!(first.texel(0, 0), Some(texel));
    assert_eq!(second.texel(0, 0), Some(texel));
    assert_eq!(rgb, [0x11, 0x22, 0x33, 0]);
    assert_eq!(bgr, [0x33, 0x22, 0x11, 0]);
}

#[test]
fn surface_texel_of_another_format_is_refused() {
    for &format in Format::ALL {
        let stride = 2 * format.bytes_per_pixel();
        let mut bytes = canvas(2, stride, 0);
        let mut surface = Surface::new(&mut bytes, 2, 2, stride, format).unwrap();
        for &other in Format::ALL {
            let texel = sample(other);
            if format.carries(texel) {
                assert!(surface.set_texel(0, 0, texel).is_ok());
                assert!(surface.fill(texel).is_ok());
            } else {
                assert_eq!(
                    surface.set_texel(0, 0, texel).unwrap_err(),
                    RasterError::Format
                );
                assert_eq!(surface.fill(texel).unwrap_err(), RasterError::Format);
            }
        }
    }
}

#[test]
fn surface_row_returns_the_visible_bytes_only() {
    let stride = 7;
    let mut bytes = canvas(3, stride, 0);
    let mut surface = Surface::new(&mut bytes, 4, 3, stride, Format::A8).unwrap();
    surface.clear();
    surface.row_mut(1).unwrap().fill(0x3c);
    assert_eq!(surface.row(0).unwrap(), &[0, 0, 0, 0]);
    assert_eq!(surface.row(1).unwrap(), &[0x3c; 4]);
    assert_eq!(bytes[11..14], [SENTINEL; 3]);
}

#[test]
fn surface_two_allocations_produce_identical_bytes() {
    for &format in Format::ALL {
        let stride = 9 * format.bytes_per_pixel();
        let mut first = canvas(6, stride, 0);
        let mut second = canvas(6, stride, 512);
        for bytes in [&mut first, &mut second] {
            let mut surface = Surface::new(bytes, 9, 6, stride, format).unwrap();
            surface.clear();
            for y in 0..6 {
                for x in 0..9 {
                    if (x + y) % 3 == 0 {
                        surface.set_texel(x, y, sample(format)).unwrap();
                    }
                }
            }
        }
        assert_eq!(first[..], second[..first.len()]);
    }
}

#[test]
fn raster_error_displays_every_variant() {
    let messages = [
        RasterError::Empty,
        RasterError::Stride,
        RasterError::Overflow,
        RasterError::TooShort {
            needed: 4,
            given: 2,
        },
        RasterError::Format,
        RasterError::OutOfBounds,
    ]
    .map(|error| format!("{error}"));
    for message in &messages {
        assert!(!message.is_empty());
    }
    assert_eq!(messages[3], "a surface of 4 bytes over a slice of 2");
}
