// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::graphics`, covering the conversion items of the
//! catalog 6.6.24.

#![allow(clippy::arithmetic_side_effects)]

use audhsos_abi::boot_info::{Framebuffer, FramebufferFormat};
use audhsos_abi::layout::PAGE_SIZE;

use crate::graphics::{framebuffer_format, to_framebuffer};
use crate::protocols::{GraphicsOutputModeInformation, GraphicsPixelFormat};

/// The base every test uses; frame-aligned, as the firmware reports it.
const BASE: u64 = 0xC000_0000;

fn mode(width: u32, height: u32, stride: u32, format: u32) -> GraphicsOutputModeInformation {
    GraphicsOutputModeInformation {
        version: 0,
        horizontal_resolution: width,
        vertical_resolution: height,
        pixel_format: format,
        pixel_information: [0; 4],
        pixels_per_scan_line: stride,
    }
}

fn linear(width: u32, height: u32) -> GraphicsOutputModeInformation {
    mode(
        width,
        height,
        width,
        GraphicsPixelFormat::RedGreenBlueReserved8.code(),
    )
}

fn size_of(info: &GraphicsOutputModeInformation) -> usize {
    usize::try_from(info.vertical_resolution)
        .unwrap()
        .saturating_mul(usize::try_from(info.pixels_per_scan_line).unwrap())
        .saturating_mul(4)
}

#[test]
fn the_two_linear_formats_become_the_two_framebuffer_formats() {
    assert_eq!(
        framebuffer_format(GraphicsPixelFormat::RedGreenBlueReserved8.code()),
        Some(FramebufferFormat::Rgbx8888)
    );
    assert_eq!(
        framebuffer_format(GraphicsPixelFormat::BlueGreenRedReserved8.code()),
        Some(FramebufferFormat::Bgrx8888)
    );
    assert_eq!(
        framebuffer_format(GraphicsPixelFormat::BitMask.code()),
        None,
        "a bit mask is not a format the loader can describe"
    );
    assert_eq!(
        framebuffer_format(GraphicsPixelFormat::BltOnly.code()),
        None
    );
    assert_eq!(framebuffer_format(4), None, "an unknown code");
    for format in GraphicsPixelFormat::ALL {
        assert_eq!(GraphicsPixelFormat::from_u32(format.code()), Some(format));
    }
    assert_eq!(GraphicsPixelFormat::from_u32(4), None);
}

#[test]
fn a_linear_mode_becomes_a_framebuffer_the_boot_information_accepts() {
    let info = linear(1024, 768);
    let size = size_of(&info);
    assert_eq!(
        to_framebuffer(BASE, size, &info),
        Some(Framebuffer {
            phys_start: BASE,
            len: 0x30_0000,
            width: 1024,
            height: 768,
            stride: 1024,
            format: FramebufferFormat::Rgbx8888,
        })
    );
}

#[test]
fn a_size_that_is_not_a_whole_number_of_frames_is_rounded_up() {
    let info = linear(800, 600);
    let size = size_of(&info);
    assert_eq!(size, 800 * 600 * 4);
    let framebuffer = to_framebuffer(BASE, size + 1, &info).unwrap();
    assert!(framebuffer.len.is_multiple_of(PAGE_SIZE));
    assert_eq!(
        framebuffer.len,
        (u64::try_from(size).unwrap() + 1).next_multiple_of(PAGE_SIZE)
    );
    let exact = to_framebuffer(BASE, size, &info).unwrap();
    assert_eq!(
        exact.len,
        u64::try_from(size).unwrap().next_multiple_of(PAGE_SIZE),
        "a size that is already a whole number of frames is unchanged"
    );
}

#[test]
fn a_stride_above_the_width_is_carried_through() {
    let info = mode(
        1000,
        768,
        1024,
        GraphicsPixelFormat::BlueGreenRedReserved8.code(),
    );
    let framebuffer = to_framebuffer(BASE, size_of(&info), &info).unwrap();
    assert_eq!(framebuffer.width, 1000);
    assert_eq!(framebuffer.stride, 1024);
    assert_eq!(framebuffer.format, FramebufferFormat::Bgrx8888);
}

#[test]
fn a_base_of_zero_or_an_unaligned_base_reports_no_framebuffer() {
    let info = linear(640, 480);
    let size = size_of(&info);
    assert_eq!(to_framebuffer(0, size, &info), None);
    assert_eq!(to_framebuffer(BASE + 1, size, &info), None);
    assert_eq!(to_framebuffer(BASE + 0x800, size, &info), None);
    assert!(to_framebuffer(BASE, size, &info).is_some());
}

#[test]
fn a_resolution_of_zero_or_a_short_stride_reports_no_framebuffer() {
    let zero_width = linear(0, 480);
    assert_eq!(to_framebuffer(BASE, 0x10_0000, &zero_width), None);
    let zero_height = linear(640, 0);
    assert_eq!(to_framebuffer(BASE, 0x10_0000, &zero_height), None);
    let short_stride = mode(
        640,
        480,
        639,
        GraphicsPixelFormat::RedGreenBlueReserved8.code(),
    );
    assert_eq!(to_framebuffer(BASE, 0x20_0000, &short_stride), None);
}

#[test]
fn a_size_below_the_visible_pixels_reports_no_framebuffer() {
    let info = linear(1024, 768);
    let size = size_of(&info);
    assert_eq!(to_framebuffer(BASE, size - 1, &info), None);
    assert!(to_framebuffer(BASE, size, &info).is_some());
    assert_eq!(to_framebuffer(BASE, 0, &info), None);
}

#[test]
fn a_framebuffer_that_would_leave_the_address_space_reports_none() {
    let info = linear(4, 1);
    let size = size_of(&info);
    assert_eq!(
        to_framebuffer(u64::MAX - PAGE_SIZE + 1, size, &info),
        None,
        "the end of the framebuffer must be representable"
    );
    assert!(to_framebuffer(u64::MAX - 2 * PAGE_SIZE + 1, size, &info).is_some());
}
