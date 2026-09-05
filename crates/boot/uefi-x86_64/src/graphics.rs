// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The framebuffer the firmware has already set up.
//!
//! Invariant: the loader reads the mode and never changes it, and it never
//! writes to the framebuffer; a machine without a usable mode boots
//! without one.

use audhsos_abi::boot_info::Framebuffer;
use audhsos_uefi::graphics::to_framebuffer;
use audhsos_uefi::protocols::{GRAPHICS_OUTPUT_PROTOCOL, GraphicsOutputProtocol};

use crate::firmware::Firmware;

/// The framebuffer of the mode the firmware has set, or `None` if the
/// protocol is missing or the mode is one the loader cannot describe.
pub(crate) fn framebuffer(firmware: &Firmware<'_>) -> Option<Framebuffer> {
    let protocol: &GraphicsOutputProtocol =
        firmware.locate_protocol(GRAPHICS_OUTPUT_PROTOCOL).ok()?;
    if protocol.mode.is_null() {
        return None;
    }
    // SAFETY: the firmware owns the mode structure of a protocol it
    // reported for as long as boot services run, and the loader reads it
    // before it leaves them.
    let mode = unsafe { &*protocol.mode };
    if mode.info.is_null() {
        return None;
    }
    // SAFETY: the mode structure of a protocol the firmware reported
    // points at the description of the mode that is set, which the
    // firmware owns for as long as boot services run.
    let info = unsafe { &*mode.info };
    to_framebuffer(mode.frame_buffer_base, mode.frame_buffer_size, info)
}
