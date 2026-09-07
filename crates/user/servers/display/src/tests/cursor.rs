// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::cursor`.

use gfx::{Color, PixelFormat, Rect, Surface};

use crate::cursor::{CURSOR_HEIGHT, CURSOR_SHAPE, CURSOR_WIDTH, Cursor};

/// The bytes of a screen of this size.
fn buffer(width: u32, height: u32) -> Vec<u8> {
    let pixels = usize::try_from(width)
        .unwrap()
        .saturating_mul(usize::try_from(height).unwrap());
    vec![0; pixels.saturating_mul(4)]
}

/// A screen over `bytes`, painted in one color.
fn screen(bytes: &mut [u8], width: u32, height: u32, ground: Color) -> Surface<'_> {
    let mut surface = Surface::new(bytes, width, height, width, PixelFormat::Rgbx8888).unwrap();
    surface.fill(surface.bounds(), ground);
    surface.clear_damage();
    surface
}

#[test]
fn the_shape_is_sixteen_rows_of_sixteen_pixels_and_has_a_tip() {
    assert_eq!(CURSOR_SHAPE.len(), usize::try_from(CURSOR_HEIGHT).unwrap());
    assert_eq!(CURSOR_WIDTH, 16);
    assert_ne!(CURSOR_SHAPE[0] & 0x8000, 0, "the tip is the first pixel");
    assert!(CURSOR_SHAPE.iter().any(|row| *row != 0));
}

#[test]
fn a_new_cursor_is_at_the_origin_shown_and_not_drawn() {
    let cursor = Cursor::new();
    assert_eq!(cursor.position(), (0, 0));
    assert!(cursor.is_visible());
    assert!(!cursor.is_drawn());
    assert_eq!(
        cursor.bounds(),
        Rect::new(0, 0, CURSOR_WIDTH, CURSOR_HEIGHT)
    );
    assert_eq!(Cursor::default().position(), (0, 0));
}

#[test]
fn the_tip_is_the_edge_color_and_the_inside_is_the_body_color() {
    let ground = Color::new(7, 8, 9);
    let mut bytes = buffer(32, 32);
    let mut surface = screen(&mut bytes, 32, 32, ground);
    let mut cursor = Cursor::new();
    cursor.place(4, 4, true, Rect::new(0, 0, 32, 32));
    cursor.draw(&mut surface);
    assert!(cursor.is_drawn());
    assert_eq!(
        surface.pixel(4, 4),
        Some(Color::BLACK),
        "the tip is an edge"
    );
    assert_eq!(
        surface.pixel(5, 8),
        Some(Color::WHITE),
        "a pixel with neighbours on every side is the body"
    );
    assert_eq!(
        surface.pixel(20, 20),
        Some(ground),
        "and nothing else moved"
    );
    assert!(!surface.damage().is_empty());
}

#[test]
fn erasing_puts_back_exactly_what_was_under_the_sprite() {
    let mut bytes = buffer(32, 32);
    let mut surface = screen(&mut bytes, 32, 32, Color::new(1, 2, 3));
    surface.fill(Rect::new(4, 4, 4, 4), Color::new(9, 9, 9));
    let mut cursor = Cursor::new();
    cursor.place(2, 2, true, Rect::new(0, 0, 32, 32));
    cursor.draw(&mut surface);
    cursor.erase(&mut surface);
    assert!(!cursor.is_drawn());
    for y in 0..32 {
        for x in 0..32 {
            let inside = (4..8).contains(&x) && (4..8).contains(&y);
            let wanted = if inside {
                Color::new(9, 9, 9)
            } else {
                Color::new(1, 2, 3)
            };
            assert_eq!(surface.pixel(x, y), Some(wanted), "at {x},{y}");
        }
    }
}

#[test]
fn erasing_a_cursor_that_stands_nowhere_changes_nothing() {
    let mut bytes = buffer(8, 8);
    let mut surface = screen(&mut bytes, 8, 8, Color::WHITE);
    let mut cursor = Cursor::new();
    cursor.erase(&mut surface);
    assert!(surface.damage().is_empty());
    assert_eq!(surface.pixel(0, 0), Some(Color::WHITE));
}

#[test]
fn a_cursor_at_the_edge_draws_what_fits_and_writes_nothing_beyond() {
    let mut bytes = buffer(8, 8);
    let mut surface = screen(&mut bytes, 8, 8, Color::new(3, 3, 3));
    let mut cursor = Cursor::new();
    cursor.place(20, 20, true, Rect::new(0, 0, 8, 8));
    assert_eq!(cursor.position(), (7, 7));
    cursor.draw(&mut surface);
    assert_eq!(surface.pixel(7, 7), Some(Color::BLACK));
    cursor.erase(&mut surface);
    assert_eq!(surface.pixel(7, 7), Some(Color::new(3, 3, 3)));
}

#[test]
fn a_cursor_that_is_not_shown_draws_nothing() {
    let mut bytes = buffer(32, 32);
    let mut surface = screen(&mut bytes, 32, 32, Color::new(5, 5, 5));
    let mut cursor = Cursor::new();
    cursor.place(1, 1, false, Rect::new(0, 0, 32, 32));
    cursor.draw(&mut surface);
    assert!(!cursor.is_drawn());
    assert_eq!(surface.pixel(1, 1), Some(Color::new(5, 5, 5)));
}
