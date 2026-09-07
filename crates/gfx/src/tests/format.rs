// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::format`.

use audhsos_abi::FramebufferFormat;

use crate::format::{BYTES_PER_PIXEL, Color, PixelFormat};

#[test]
fn a_pixel_is_four_bytes_in_every_format() {
    assert_eq!(BYTES_PER_PIXEL, 4);
    assert_eq!(PixelFormat::ALL.len(), 2);
}

#[test]
fn rgbx_writes_red_first() {
    let color = Color::new(0x11, 0x22, 0x33);
    assert_eq!(
        color.encode(PixelFormat::Rgbx8888),
        [0x11, 0x22, 0x33, 0x00]
    );
}

#[test]
fn bgrx_writes_blue_first() {
    let color = Color::new(0x11, 0x22, 0x33);
    assert_eq!(
        color.encode(PixelFormat::Bgrx8888),
        [0x33, 0x22, 0x11, 0x00]
    );
}

#[test]
fn a_color_decodes_back_to_itself_in_both_formats() {
    for format in PixelFormat::ALL {
        for value in [
            Color::BLACK,
            Color::WHITE,
            Color::new(1, 2, 3),
            Color::new(0xFF, 0x00, 0x80),
        ] {
            assert_eq!(Color::decode(*format, value.encode(*format)), value);
        }
    }
}

#[test]
fn the_byte_that_is_not_shown_is_ignored_when_decoding() {
    let bytes = [0x11, 0x22, 0x33, 0xFF];
    assert_eq!(
        Color::decode(PixelFormat::Rgbx8888, bytes),
        Color::new(0x11, 0x22, 0x33)
    );
    assert_eq!(
        Color::decode(PixelFormat::Bgrx8888, bytes),
        Color::new(0x33, 0x22, 0x11)
    );
}

#[test]
fn every_format_of_the_boot_information_has_one_here_and_back() {
    for format in [FramebufferFormat::Rgbx8888, FramebufferFormat::Bgrx8888] {
        assert_eq!(PixelFormat::from_boot(format).to_boot(), format);
    }
}

#[test]
fn every_format_has_a_name() {
    assert_eq!(PixelFormat::Rgbx8888.name(), "rgbx8888");
    assert_eq!(PixelFormat::Bgrx8888.name(), "bgrx8888");
    assert_eq!(PixelFormat::default(), PixelFormat::Rgbx8888);
}
