// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::surface`.

use test_support::generators::pair;
use test_support::property::check;

use crate::format::{Color, PixelFormat};
use crate::rect::Rect;
use crate::strategies::{any_color, any_pixel_format, any_rect};
use crate::surface::{Surface, SurfaceError};

/// The bytes of a surface of this size, with room for the padding a stride
/// wider than the picture leaves.
fn buffer(height: u32, stride: u32) -> Vec<u8> {
    let pixels = usize::try_from(height)
        .unwrap()
        .saturating_mul(usize::try_from(stride).unwrap());
    vec![0; pixels.saturating_mul(4)]
}

/// A surface of `width` by `height` with a row of `stride` pixels.
fn surface(bytes: &mut [u8], width: u32, height: u32, stride: u32) -> Surface<'_> {
    Surface::new(bytes, width, height, stride, PixelFormat::Rgbx8888).unwrap()
}

#[test]
fn a_surface_of_zero_width_or_height_is_rejected() {
    let mut bytes = buffer(4, 4);
    assert_eq!(
        Surface::new(&mut bytes, 0, 4, 4, PixelFormat::Rgbx8888).unwrap_err(),
        SurfaceError::Empty
    );
    assert_eq!(
        Surface::new(&mut bytes, 4, 0, 4, PixelFormat::Rgbx8888).unwrap_err(),
        SurfaceError::Empty
    );
}

#[test]
fn a_row_shorter_than_the_width_is_rejected() {
    let mut bytes = buffer(4, 4);
    assert_eq!(
        Surface::new(&mut bytes, 4, 4, 3, PixelFormat::Rgbx8888).unwrap_err(),
        SurfaceError::Stride
    );
}

#[test]
fn a_slice_too_short_for_the_rows_is_rejected() {
    let mut bytes = buffer(4, 4);
    let error = Surface::new(&mut bytes, 4, 5, 4, PixelFormat::Rgbx8888).unwrap_err();
    assert_eq!(
        error,
        SurfaceError::TooShort {
            needed: 80,
            given: 64
        }
    );
    assert!(format!("{error}").contains("80"));
}

#[test]
fn dimensions_beyond_this_address_space_are_rejected() {
    let mut bytes = buffer(1, 1);
    assert_eq!(
        Surface::new(&mut bytes, 1, u32::MAX, u32::MAX, PixelFormat::Rgbx8888).unwrap_err(),
        SurfaceError::Overflow
    );
}

#[test]
fn every_reason_a_surface_is_rejected_reads_as_a_sentence() {
    for error in [
        SurfaceError::Empty,
        SurfaceError::Stride,
        SurfaceError::Overflow,
        SurfaceError::TooShort {
            needed: 8,
            given: 4,
        },
    ] {
        assert!(!format!("{error}").is_empty());
    }
}

#[test]
fn a_surface_reports_what_it_was_built_with() {
    let mut bytes = buffer(4, 8);
    let picture = surface(&mut bytes, 6, 4, 8);
    assert_eq!(picture.width(), 6);
    assert_eq!(picture.height(), 4);
    assert_eq!(picture.stride(), 8);
    assert_eq!(picture.format(), PixelFormat::Rgbx8888);
    assert_eq!(picture.bounds(), Rect::new(0, 0, 6, 4));
    assert_eq!(picture.bytes().len(), 128);
}

#[test]
fn a_fill_inside_the_surface_writes_exactly_its_rectangle() {
    let mut bytes = buffer(4, 4);
    let mut picture = surface(&mut bytes, 4, 4, 4);
    let written = picture.fill(Rect::new(1, 1, 2, 2), Color::WHITE);
    assert_eq!(written, Rect::new(1, 1, 2, 2));
    for y in 0..4 {
        for x in 0..4 {
            let inside = (1..3).contains(&x) && (1..3).contains(&y);
            let wanted = if inside { Color::WHITE } else { Color::BLACK };
            assert_eq!(picture.pixel(x, y), Some(wanted), "at {x},{y}");
        }
    }
}

#[test]
fn a_fill_that_reaches_past_the_surface_is_clipped_to_it() {
    let mut bytes = buffer(4, 4);
    let mut picture = surface(&mut bytes, 4, 4, 4);
    let written = picture.fill(Rect::new(2, 2, 10, 10), Color::WHITE);
    assert_eq!(written, Rect::new(2, 2, 2, 2));
    assert_eq!(picture.pixel(3, 3), Some(Color::WHITE));
    assert_eq!(picture.pixel(1, 1), Some(Color::BLACK));
}

#[test]
fn a_fill_wholly_outside_the_surface_writes_nothing() {
    let mut bytes = buffer(4, 4);
    let mut picture = surface(&mut bytes, 4, 4, 4);
    assert_eq!(
        picture.fill(Rect::new(4, 0, 4, 4), Color::WHITE),
        Rect::EMPTY
    );
    assert_eq!(
        picture.fill(Rect::new(0, 4, 4, 4), Color::WHITE),
        Rect::EMPTY
    );
    assert!(picture.damage().is_empty());
    assert!(picture.bytes().iter().all(|byte| *byte == 0));
}

#[test]
fn a_fill_of_zero_width_or_height_writes_nothing() {
    let mut bytes = buffer(4, 4);
    let mut picture = surface(&mut bytes, 4, 4, 4);
    assert_eq!(
        picture.fill(Rect::new(0, 0, 0, 4), Color::WHITE),
        Rect::EMPTY
    );
    assert_eq!(
        picture.fill(Rect::new(0, 0, 4, 0), Color::WHITE),
        Rect::EMPTY
    );
    assert!(picture.bytes().iter().all(|byte| *byte == 0));
}

#[test]
fn a_fill_writes_the_last_row_and_the_last_column() {
    let mut bytes = buffer(3, 3);
    let mut picture = surface(&mut bytes, 3, 3, 3);
    picture.fill(picture.bounds(), Color::WHITE);
    assert_eq!(picture.pixel(2, 2), Some(Color::WHITE));
    assert_eq!(picture.pixel(0, 2), Some(Color::WHITE));
    assert_eq!(picture.pixel(2, 0), Some(Color::WHITE));
}

#[test]
fn a_fill_leaves_the_padding_of_a_wider_row_untouched() {
    let mut bytes = buffer(2, 6);
    let mut picture = surface(&mut bytes, 4, 2, 6);
    picture.fill(picture.bounds(), Color::WHITE);
    let padding = [
        16, 17, 18, 19, 20, 21, 22, 23, 40, 41, 42, 43, 44, 45, 46, 47,
    ];
    for offset in padding {
        assert_eq!(picture.bytes().get(offset), Some(&0), "byte {offset}");
    }
}

#[test]
fn a_pixel_outside_the_surface_is_neither_read_nor_written() {
    let mut bytes = buffer(2, 2);
    let mut picture = surface(&mut bytes, 2, 2, 2);
    assert_eq!(picture.pixel(2, 0), None);
    assert_eq!(picture.pixel(0, 2), None);
    assert!(!picture.set_pixel(2, 0, Color::WHITE));
    assert!(!picture.set_pixel(0, 2, Color::WHITE));
    assert!(picture.set_pixel(1, 1, Color::WHITE));
    assert!(picture.bytes().iter().take(12).all(|byte| *byte == 0));
}

#[test]
fn a_row_that_reaches_past_the_width_is_refused() {
    let mut bytes = buffer(2, 4);
    let mut picture = surface(&mut bytes, 3, 2, 4);
    assert!(picture.row_bytes(1, 0, 3).is_none());
    assert!(picture.row_bytes(0, 2, 1).is_none());
    assert_eq!(picture.row_bytes(1, 0, 2).map(<[u8]>::len), Some(8));
    assert!(picture.row_bytes_mut(1, 1, 3).is_none());
    assert_eq!(
        picture.row_bytes_mut(0, 1, 3).map(|row| row.len()),
        Some(12)
    );
}

#[test]
fn a_blit_copies_the_source_rectangle_to_the_target_position() {
    let mut source_bytes = buffer(2, 2);
    let mut source = surface(&mut source_bytes, 2, 2, 2);
    source.fill(source.bounds(), Color::new(1, 2, 3));
    let mut target_bytes = buffer(4, 4);
    let mut target = surface(&mut target_bytes, 4, 4, 4);
    let written = target.blit(&source, source.bounds(), 1, 1);
    assert_eq!(written, Rect::new(1, 1, 2, 2));
    assert_eq!(target.pixel(1, 1), Some(Color::new(1, 2, 3)));
    assert_eq!(target.pixel(2, 2), Some(Color::new(1, 2, 3)));
    assert_eq!(target.pixel(0, 0), Some(Color::BLACK));
    assert_eq!(target.pixel(3, 3), Some(Color::BLACK));
}

#[test]
fn a_blit_that_reaches_past_the_target_is_clipped_to_it() {
    let mut source_bytes = buffer(4, 4);
    let mut source = surface(&mut source_bytes, 4, 4, 4);
    source.fill(source.bounds(), Color::WHITE);
    let mut target_bytes = buffer(4, 4);
    let mut target = surface(&mut target_bytes, 4, 4, 4);
    let written = target.blit(&source, source.bounds(), 3, 3);
    assert_eq!(written, Rect::new(3, 3, 1, 1));
    assert_eq!(target.pixel(3, 3), Some(Color::WHITE));
    assert_eq!(target.pixel(2, 3), Some(Color::BLACK));
}

#[test]
fn a_blit_wholly_outside_the_target_writes_nothing() {
    let mut source_bytes = buffer(2, 2);
    let mut source = surface(&mut source_bytes, 2, 2, 2);
    source.fill(source.bounds(), Color::WHITE);
    let mut target_bytes = buffer(2, 2);
    let mut target = surface(&mut target_bytes, 2, 2, 2);
    assert_eq!(target.blit(&source, source.bounds(), 2, 0), Rect::EMPTY);
    assert_eq!(target.blit(&source, Rect::EMPTY, 0, 0), Rect::EMPTY);
    assert!(target.bytes().iter().all(|byte| *byte == 0));
}

#[test]
fn a_blit_between_two_formats_keeps_the_color() {
    let mut source_bytes = buffer(1, 1);
    let mut source = Surface::new(&mut source_bytes, 1, 1, 1, PixelFormat::Rgbx8888).unwrap();
    source.fill(source.bounds(), Color::new(0x10, 0x20, 0x30));
    let mut target_bytes = buffer(1, 1);
    let mut target = Surface::new(&mut target_bytes, 1, 1, 1, PixelFormat::Bgrx8888).unwrap();
    target.blit(&source, source.bounds(), 0, 0);
    assert_eq!(target.pixel(0, 0), Some(Color::new(0x10, 0x20, 0x30)));
    assert_eq!(target.bytes(), &[0x30, 0x20, 0x10, 0x00]);
}

#[test]
fn drawing_records_damage_until_it_is_cleared() {
    let mut bytes = buffer(4, 4);
    let mut picture = surface(&mut bytes, 4, 4, 4);
    picture.fill(Rect::new(0, 0, 2, 2), Color::WHITE);
    picture.add_damage(Rect::new(3, 3, 4, 4));
    assert_eq!(picture.damage().len(), 2);
    assert_eq!(picture.damage().bounds(), Rect::new(0, 0, 4, 4));
    picture.clear_damage();
    assert!(picture.damage().is_empty());
}

#[test]
fn property_no_fill_writes_a_byte_outside_the_surface() {
    check(
        "fill_stays_inside",
        &pair(any_rect(24), pair(any_color(), any_pixel_format())),
        |(rect, (color, format))| {
            let mut bytes = vec![0_u8; 8 * 10 * 4];
            let mut picture =
                Surface::new(&mut bytes, 8, 6, 10, *format).map_err(|error| format!("{error}"))?;
            let written = picture.fill(*rect, *color);
            if !written.is_empty() && (written.right() > 8 || written.bottom() > 6) {
                return Err(format!("{written:?} reaches past the surface"));
            }
            for row in 0..6 {
                let start = row * 10 * 4 + 8 * 4;
                let padding = picture.bytes().get(start..start + 8).unwrap_or_default();
                if padding.iter().any(|byte| *byte != 0) {
                    return Err(format!("the padding of row {row} was written"));
                }
            }
            let past_the_last_row = picture.bytes().get(6 * 10 * 4..).unwrap_or_default();
            if past_the_last_row.iter().any(|byte| *byte != 0) {
                return Err("a byte past the last row was written".to_owned());
            }
            Ok(())
        },
    );
}
