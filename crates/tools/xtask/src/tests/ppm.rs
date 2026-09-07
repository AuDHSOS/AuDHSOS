// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::ppm`, covering the catalog items 6.6.28.

use crate::ppm::{Image, PpmError};

/// A picture of `width` by `height` pixels, every one of them `color`.
fn file(width: u32, height: u32, color: (u8, u8, u8)) -> Vec<u8> {
    let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
    for _ in 0..width.saturating_mul(height) {
        bytes.extend_from_slice(&[color.0, color.1, color.2]);
    }
    bytes
}

#[test]
fn a_picture_reads_back_with_its_size_and_its_pixels() {
    let image = Image::parse(&file(4, 3, (1, 2, 3))).unwrap();
    assert_eq!((image.width(), image.height()), (4, 3));
    assert_eq!(image.pixel(0, 0), Ok((1, 2, 3)));
    assert_eq!(image.pixel(3, 2), Ok((1, 2, 3)));
}

#[test]
fn the_pixels_stand_row_by_row() {
    let mut bytes = b"P6\n2 2\n255\n".to_vec();
    bytes.extend_from_slice(&[1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 4]);
    let image = Image::parse(&bytes).unwrap();
    assert_eq!(image.pixel(0, 0), Ok((1, 1, 1)));
    assert_eq!(image.pixel(1, 0), Ok((2, 2, 2)));
    assert_eq!(image.pixel(0, 1), Ok((3, 3, 3)));
    assert_eq!(image.pixel(1, 1), Ok((4, 4, 4)));
}

#[test]
fn comments_and_extra_spaces_in_the_header_are_stepped_over() {
    let mut bytes = b"P6\n# taken by qemu\n 2  2 \n# and this too\n255\n".to_vec();
    bytes.extend_from_slice(&[9; 12]);
    let image = Image::parse(&bytes).unwrap();
    assert_eq!((image.width(), image.height()), (2, 2));
    assert_eq!(image.pixel(1, 1), Ok((9, 9, 9)));
}

#[test]
fn a_file_that_is_no_picture_is_refused() {
    assert_eq!(Image::parse(b"P5\n1 1\n255\n\0\0\0"), Err(PpmError::Magic));
    assert_eq!(Image::parse(b""), Err(PpmError::Magic));
    assert_eq!(
        Image::parse(b"P6\nx 1\n255\n"),
        Err(PpmError::Header("width"))
    );
    assert_eq!(
        Image::parse(b"P6\n1 x\n255\n"),
        Err(PpmError::Header("height"))
    );
    assert_eq!(
        Image::parse(b"P6\n1 1\n"),
        Err(PpmError::Header("largest channel value"))
    );
    assert_eq!(
        Image::parse(b"P6\n1 1\n65535\n"),
        Err(PpmError::MaxValue(65535))
    );
}

#[test]
fn a_truncated_picture_is_an_error_and_not_a_black_pixel() {
    let mut bytes = file(4, 4, (7, 7, 7));
    bytes.truncate(bytes.len() - 3);
    assert_eq!(
        Image::parse(&bytes),
        Err(PpmError::Short {
            needed: 48,
            given: 45
        })
    );
}

#[test]
fn a_pixel_outside_the_picture_is_an_error() {
    let image = Image::parse(&file(2, 2, (0, 0, 0))).unwrap();
    assert_eq!(image.pixel(2, 0), Err(PpmError::Outside));
    assert_eq!(image.pixel(0, 2), Err(PpmError::Outside));
    assert_eq!(image.pixel(u32::MAX, u32::MAX), Err(PpmError::Outside));
}

#[test]
fn a_color_is_counted_over_the_rectangle_it_is_asked_about() {
    let mut bytes = b"P6\n2 2\n255\n".to_vec();
    bytes.extend_from_slice(&[1, 1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1]);
    let image = Image::parse(&bytes).unwrap();
    assert_eq!(image.count_of(0, 0, 2, 2, (1, 1, 1)), Ok(3));
    assert_eq!(image.count_of(1, 0, 1, 1, (1, 1, 1)), Ok(0));
    assert_eq!(image.count_of(0, 0, 0, 0, (1, 1, 1)), Ok(0));
}

#[test]
fn a_rectangle_that_reaches_past_the_picture_is_an_error() {
    let image = Image::parse(&file(2, 2, (0, 0, 0))).unwrap();
    assert_eq!(
        image.count_of(1, 0, 2, 1, (0, 0, 0)),
        Err(PpmError::Outside)
    );
    assert_eq!(
        image.count_of(0, 1, 1, 2, (0, 0, 0)),
        Err(PpmError::Outside)
    );
}

#[test]
fn every_reason_a_file_is_no_picture_reads_as_a_sentence() {
    let errors = [
        PpmError::Magic,
        PpmError::Header("width"),
        PpmError::MaxValue(7),
        PpmError::Short {
            needed: 8,
            given: 4,
        },
        PpmError::Outside,
    ];
    for error in errors {
        assert!(!format!("{error}").is_empty());
    }
}
