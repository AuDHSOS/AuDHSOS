// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Turning the mode the firmware has set into the framebuffer description
//! the boot information carries.
//!
//! Invariant: a description that comes out of here satisfies the
//! framebuffer rules of the boot information, so that the loader cannot
//! write one the kernel would reject.

use audhsos_abi::boot_info::{Framebuffer, FramebufferFormat};
use audhsos_abi::layout::PAGE_SIZE;

use crate::protocols::{GraphicsOutputModeInformation, GraphicsPixelFormat};

/// The framebuffer format for a pixel format the loader can address, or
/// `None` for the formats it cannot.
#[must_use]
pub const fn framebuffer_format(pixel_format: u32) -> Option<FramebufferFormat> {
    match GraphicsPixelFormat::from_u32(pixel_format) {
        Some(GraphicsPixelFormat::RedGreenBlueReserved8) => Some(FramebufferFormat::Rgbx8888),
        Some(GraphicsPixelFormat::BlueGreenRedReserved8) => Some(FramebufferFormat::Bgrx8888),
        // With a bit mask or without a linear framebuffer the loader
        // cannot describe the memory, so it reports no framebuffer at all.
        _ => None,
    }
}

/// The framebuffer description for the mode the firmware has set, or
/// `None` when the loader cannot address it. The length is `size` rounded
/// up to whole frames.
#[must_use]
pub fn to_framebuffer(
    base: u64,
    size: usize,
    info: &GraphicsOutputModeInformation,
) -> Option<Framebuffer> {
    let format = framebuffer_format(info.pixel_format)?;
    if base == 0 || !base.is_multiple_of(PAGE_SIZE) {
        return None;
    }
    let width = info.horizontal_resolution;
    let height = info.vertical_resolution;
    let stride = info.pixels_per_scan_line;
    if width == 0 || height == 0 || stride < width {
        return None;
    }
    let visible = u64::from(height)
        .checked_mul(u64::from(stride))?
        .checked_mul(4)?;
    let size = u64::try_from(size).ok()?;
    if size < visible {
        return None;
    }
    let len = size.checked_next_multiple_of(PAGE_SIZE)?;
    base.checked_add(len)?;
    Some(Framebuffer {
        phys_start: base,
        len,
        width,
        height,
        stride,
        format,
    })
}
