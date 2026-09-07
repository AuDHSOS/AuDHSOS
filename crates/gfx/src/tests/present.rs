// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::present`, against a sink that records what it was
//! given instead of showing it.

use crate::format::{Color, PixelFormat};
use crate::present::{PixelSink, PresentError, present};
use crate::rect::{Damage, Rect};
use crate::surface::Surface;

/// One run of pixels a presentation wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Written {
    /// Where the run begins.
    x: u32,
    /// Which row it is in.
    y: u32,
    /// The bytes of it.
    bytes: Vec<u8>,
}

/// A sink that records the runs instead of showing them.
struct Recording {
    /// Visible columns.
    width: u32,
    /// Visible rows.
    height: u32,
    /// The order of the channels it expects.
    format: PixelFormat,
    /// What it was given, in order.
    runs: Vec<Written>,
}

impl Recording {
    fn new(width: u32, height: u32, format: PixelFormat) -> Self {
        Recording {
            width,
            height,
            format,
            runs: Vec::new(),
        }
    }

    /// Every pixel it was given, as a position and a color.
    fn pixels(&self) -> Vec<(u32, u32, Color)> {
        let mut pixels = Vec::new();
        for run in &self.runs {
            for (index, chunk) in run.bytes.as_chunks::<4>().0.iter().enumerate() {
                let x = run.x.saturating_add(u32::try_from(index).unwrap());
                pixels.push((x, run.y, Color::decode(self.format, *chunk)));
            }
        }
        pixels
    }
}

impl PixelSink for Recording {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn format(&self) -> PixelFormat {
        self.format
    }

    fn write_row(&mut self, x: u32, y: u32, bytes: &[u8]) {
        self.runs.push(Written {
            x,
            y,
            bytes: bytes.to_vec(),
        });
    }
}

/// The bytes of a surface of this size.
fn buffer(width: u32, height: u32) -> Vec<u8> {
    let pixels = usize::try_from(width)
        .unwrap()
        .saturating_mul(usize::try_from(height).unwrap());
    vec![0; pixels.saturating_mul(4)]
}

#[test]
fn a_presentation_copies_exactly_the_damaged_pixels() {
    let mut bytes = buffer(4, 4);
    let mut picture = Surface::new(&mut bytes, 4, 4, 4, PixelFormat::Rgbx8888).unwrap();
    picture.fill(Rect::new(1, 1, 2, 2), Color::WHITE);
    let mut sink = Recording::new(4, 4, PixelFormat::Rgbx8888);
    let written = present(&picture, &mut sink, picture.damage()).unwrap();
    assert_eq!(written, 4);
    assert_eq!(
        sink.pixels(),
        vec![
            (1, 1, Color::WHITE),
            (2, 1, Color::WHITE),
            (1, 2, Color::WHITE),
            (2, 2, Color::WHITE),
        ]
    );
}

#[test]
fn a_presentation_of_nothing_writes_nothing() {
    let mut bytes = buffer(4, 4);
    let picture = Surface::new(&mut bytes, 4, 4, 4, PixelFormat::Rgbx8888).unwrap();
    let mut sink = Recording::new(4, 4, PixelFormat::Rgbx8888);
    assert_eq!(present(&picture, &mut sink, &Damage::new()).unwrap(), 0);
    assert!(sink.runs.is_empty());
}

#[test]
fn a_presentation_writes_no_pixel_the_sink_does_not_hold() {
    let mut bytes = buffer(8, 8);
    let mut picture = Surface::new(&mut bytes, 8, 8, 8, PixelFormat::Rgbx8888).unwrap();
    picture.fill(picture.bounds(), Color::WHITE);
    let mut sink = Recording::new(4, 2, PixelFormat::Rgbx8888);
    let written = present(&picture, &mut sink, picture.damage()).unwrap();
    assert_eq!(written, 8);
    assert!(sink.pixels().iter().all(|(x, y, _)| *x < 4 && *y < 2));
}

#[test]
fn damage_outside_the_surface_presents_nothing() {
    let mut bytes = buffer(4, 4);
    let picture = Surface::new(&mut bytes, 4, 4, 4, PixelFormat::Rgbx8888).unwrap();
    let mut damage = Damage::new();
    damage.push(Rect::new(8, 8, 2, 2));
    let mut sink = Recording::new(4, 4, PixelFormat::Rgbx8888);
    assert_eq!(present(&picture, &mut sink, &damage).unwrap(), 0);
    assert!(sink.runs.is_empty());
}

#[test]
fn a_sink_of_another_format_is_refused_and_nothing_is_written() {
    let mut bytes = buffer(4, 4);
    let mut picture = Surface::new(&mut bytes, 4, 4, 4, PixelFormat::Rgbx8888).unwrap();
    picture.fill(picture.bounds(), Color::WHITE);
    let mut sink = Recording::new(4, 4, PixelFormat::Bgrx8888);
    let error = present(&picture, &mut sink, picture.damage()).unwrap_err();
    assert_eq!(
        error,
        PresentError::FormatMismatch {
            surface: PixelFormat::Rgbx8888,
            sink: PixelFormat::Bgrx8888,
        }
    );
    assert!(format!("{error}").contains("rgbx8888"));
    assert!(sink.runs.is_empty());
}

#[test]
fn a_surface_is_a_sink_and_takes_the_rows_of_another_one() {
    let mut back_bytes = buffer(4, 4);
    let mut back = Surface::new(&mut back_bytes, 4, 4, 4, PixelFormat::Rgbx8888).unwrap();
    back.fill(Rect::new(0, 0, 2, 4), Color::WHITE);
    let mut front_bytes = buffer(4, 4);
    let mut front = Surface::new(&mut front_bytes, 4, 4, 4, PixelFormat::Rgbx8888).unwrap();
    let written = present(&back, &mut front, back.damage()).unwrap();
    assert_eq!(written, 8);
    assert_eq!(front.pixel(0, 0), Some(Color::WHITE));
    assert_eq!(front.pixel(1, 3), Some(Color::WHITE));
    assert_eq!(front.pixel(2, 0), Some(Color::BLACK));
}

#[test]
fn a_row_that_does_not_fit_the_sink_is_dropped_by_it() {
    let mut bytes = buffer(2, 2);
    let mut front = Surface::new(&mut bytes, 2, 2, 2, PixelFormat::Rgbx8888).unwrap();
    let row = Color::WHITE.encode(PixelFormat::Rgbx8888).repeat(4);
    front.write_row(0, 0, &row);
    front.write_row(0, 5, &row);
    assert_eq!(front.pixel(0, 0), Some(Color::BLACK));
    assert_eq!(front.pixel(1, 1), Some(Color::BLACK));
}
