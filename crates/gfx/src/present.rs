// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Presentation: the damaged rectangles of a surface, copied to what shows
//! them.
//!
//! What shows them is a [`PixelSink`] — the framebuffer of the machine
//! through a [`Surface`] over its mapping, or a recording double in a test.
//! Only the rectangles the damage set names are copied, and only the part
//! of each that both the surface and the sink hold, so a picture larger
//! than the screen presents what fits and a picture smaller than the screen
//! leaves the rest of it alone.
//!
//! Invariant: presentation writes no pixel that was not damaged.

use crate::format::PixelFormat;
use crate::rect::{Damage, Rect};
use crate::surface::Surface;

/// Why a presentation did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresentError {
    /// The surface and the sink order their channels differently. A
    /// presentation copies rows of bytes and does not convert them.
    FormatMismatch {
        /// What the surface holds.
        surface: PixelFormat,
        /// What the sink expects.
        sink: PixelFormat,
    },
}

impl core::fmt::Display for PresentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PresentError::FormatMismatch { surface, sink } => write!(
                f,
                "a surface in {} presented to a sink in {}",
                surface.name(),
                sink.name()
            ),
        }
    }
}

/// What a presentation writes into.
pub trait PixelSink {
    /// Visible columns.
    fn width(&self) -> u32;

    /// Visible rows.
    fn height(&self) -> u32;

    /// The order of the channels it expects.
    fn format(&self) -> PixelFormat;

    /// Writes the bytes of a run of pixels at `x` in row `y`. Bytes that do
    /// not fit the row are dropped.
    fn write_row(&mut self, x: u32, y: u32, bytes: &[u8]);
}

/// Copies the damaged rectangles of `surface` into `sink` and answers with
/// how many pixels it wrote.
///
/// # Errors
///
/// [`PresentError::FormatMismatch`] when the two order their channels
/// differently; nothing is written then.
pub fn present(
    surface: &Surface<'_>,
    sink: &mut impl PixelSink,
    damage: &Damage,
) -> Result<u64, PresentError> {
    if surface.format() != sink.format() {
        return Err(PresentError::FormatMismatch {
            surface: surface.format(),
            sink: sink.format(),
        });
    }
    let shared = Rect::new(
        0,
        0,
        surface.width().min(sink.width()),
        surface.height().min(sink.height()),
    );
    let mut written = 0_u64;
    for rect in damage.iter() {
        let part = rect.intersect(shared);
        for row in 0..part.h {
            let y = part.y.saturating_add(row);
            let bytes = surface.row_bytes(part.x, y, part.w).unwrap_or_default();
            sink.write_row(part.x, y, bytes);
            written = written.saturating_add(u64::from(part.w));
        }
    }
    Ok(written)
}

impl PixelSink for Surface<'_> {
    fn width(&self) -> u32 {
        Surface::width(self)
    }

    fn height(&self) -> u32 {
        Surface::height(self)
    }

    fn format(&self) -> PixelFormat {
        Surface::format(self)
    }

    fn write_row(&mut self, x: u32, y: u32, bytes: &[u8]) {
        let count = u32::try_from(bytes.len().wrapping_div(4)).unwrap_or(u32::MAX);
        let row = self.row_bytes_mut(x, y, count).unwrap_or_default();
        let given = bytes.as_chunks::<4>().0;
        for (target, source) in row.as_chunks_mut::<4>().0.iter_mut().zip(given) {
            *target = *source;
        }
    }
}
