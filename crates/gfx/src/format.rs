// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Pixel formats and colors.
//!
//! Both formats the Graphics Output Protocol reports for a linear
//! framebuffer are four bytes per pixel with eight bits of each channel
//! and one byte that is not shown; they differ in whether red or blue
//! comes first. The unused byte is written as zero, because a firmware
//! that shows it as an alpha channel would otherwise show nothing.
//!
//! Invariant: `encode` and `decode` of one format are inverse for every
//! color, so a pixel read out of a surface and written back is unchanged.

use audhsos_abi::FramebufferFormat;

/// How many bytes one pixel occupies in every format.
pub const BYTES_PER_PIXEL: u32 = 4;

/// The order of the channels in memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// Red, green, blue, unused.
    #[default]
    Rgbx8888,
    /// Blue, green, red, unused.
    Bgrx8888,
}

impl PixelFormat {
    /// Both formats, in the order of their codes.
    pub const ALL: &'static [PixelFormat] = &[PixelFormat::Rgbx8888, PixelFormat::Bgrx8888];

    /// The format the boot information reports.
    #[must_use]
    pub const fn from_boot(format: FramebufferFormat) -> Self {
        match format {
            FramebufferFormat::Rgbx8888 => PixelFormat::Rgbx8888,
            FramebufferFormat::Bgrx8888 => PixelFormat::Bgrx8888,
        }
    }

    /// The format as the boot information writes it.
    #[must_use]
    pub const fn to_boot(self) -> FramebufferFormat {
        match self {
            PixelFormat::Rgbx8888 => FramebufferFormat::Rgbx8888,
            PixelFormat::Bgrx8888 => FramebufferFormat::Bgrx8888,
        }
    }

    /// The name of the format, as the kernel prints it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            PixelFormat::Rgbx8888 => "rgbx8888",
            PixelFormat::Bgrx8888 => "bgrx8888",
        }
    }
}

/// A color of eight bits per channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl Color {
    /// Black, which is what a cleared surface holds.
    pub const BLACK: Color = Color::new(0, 0, 0);
    /// White.
    pub const WHITE: Color = Color::new(0xFF, 0xFF, 0xFF);

    /// The color of these three channels.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }

    /// The four bytes of this color in `format`, in memory order.
    #[must_use]
    pub const fn encode(self, format: PixelFormat) -> [u8; 4] {
        match format {
            PixelFormat::Rgbx8888 => [self.r, self.g, self.b, 0],
            PixelFormat::Bgrx8888 => [self.b, self.g, self.r, 0],
        }
    }

    /// The color four bytes of `format` stand for. The byte that is not
    /// shown is ignored.
    #[must_use]
    pub const fn decode(format: PixelFormat, bytes: [u8; 4]) -> Self {
        let [first, second, third, _unused] = bytes;
        match format {
            PixelFormat::Rgbx8888 => Color::new(first, second, third),
            PixelFormat::Bgrx8888 => Color::new(third, second, first),
        }
    }
}
