// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::draw`.

use crate::draw::{
    Command, LIST_CAPACITY, List, TEXT_CAPACITY, Text, draw, draw_all, frame, line, text_bounds,
};
use crate::format::{Color, PixelFormat};
use crate::rect::Rect;
use crate::surface::Surface;

/// The ink of the tests, which is neither black nor the background.
const INK: Color = Color::new(0xF0, 0x20, 0x40);

/// The bytes of a surface of `width` by `height`.
fn buffer(width: u32, height: u32) -> Vec<u8> {
    let pixels = usize::try_from(width)
        .unwrap()
        .saturating_mul(usize::try_from(height).unwrap());
    vec![0; pixels.saturating_mul(4)]
}

/// A surface of `width` by `height` over `bytes`.
fn surface(bytes: &mut [u8], width: u32, height: u32) -> Surface<'_> {
    Surface::new(bytes, width, height, width, PixelFormat::Rgbx8888).unwrap()
}

/// How many pixels of `rect` carry `color`.
fn count(surface: &Surface<'_>, rect: Rect, color: Color) -> u32 {
    let mut found = 0_u32;
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if surface.pixel(x, y) == Some(color) {
                found = found.saturating_add(1);
            }
        }
    }
    found
}

#[test]
fn text_of_more_than_its_capacity_is_refused() {
    let long = "x".repeat(TEXT_CAPACITY.saturating_add(1));
    assert_eq!(Text::new(&long), None);
    let full = "y".repeat(TEXT_CAPACITY);
    assert_eq!(Text::new(&full).unwrap().len(), TEXT_CAPACITY);
}

#[test]
fn text_outside_ascii_is_refused_because_the_font_has_no_glyph_for_it() {
    assert_eq!(Text::new("hallöchen"), None);
    assert_eq!(Text::from_bytes(&[0x41, 0xC3, 0xA4]), None);
}

#[test]
fn text_answers_with_what_it_was_given() {
    let text = Text::new("shell").unwrap();
    assert_eq!(text.as_str(), "shell");
    assert_eq!(text.as_bytes(), b"shell");
    assert_eq!(text.len(), 5);
    assert!(!text.is_empty());
    assert_eq!(text.width(), 40);
    assert!(Text::empty().is_empty());
    assert_eq!(Text::default().as_str(), "");
    assert_eq!(Text::empty().width(), 0);
}

#[test]
fn a_list_takes_commands_until_it_is_full() {
    let mut list = List::default();
    assert!(list.is_empty());
    for index in 0..LIST_CAPACITY {
        assert!(list.push(Command::Fill {
            rect: Rect::new(u32::try_from(index).unwrap(), 0, 1, 1),
            color: INK,
        }));
    }
    assert!(list.is_full());
    assert_eq!(list.len(), LIST_CAPACITY);
    assert!(!list.push(Command::Clear { color: INK }));
    assert_eq!(list.iter().count(), LIST_CAPACITY);
    list.clear();
    assert!(list.is_empty());
    assert_eq!(list.iter().count(), 0);
}

#[test]
fn clearing_puts_one_color_on_every_pixel() {
    let mut bytes = buffer(8, 4);
    let mut surface = surface(&mut bytes, 8, 4);
    let written = draw(&mut surface, &Command::Clear { color: INK });
    assert_eq!(written, Rect::new(0, 0, 8, 4));
    assert_eq!(count(&surface, surface.bounds(), INK), 32);
}

#[test]
fn a_fill_writes_its_rectangle_and_nothing_around_it() {
    let mut bytes = buffer(8, 8);
    let mut surface = surface(&mut bytes, 8, 8);
    let written = draw(
        &mut surface,
        &Command::Fill {
            rect: Rect::new(2, 2, 3, 3),
            color: INK,
        },
    );
    assert_eq!(written, Rect::new(2, 2, 3, 3));
    assert_eq!(count(&surface, surface.bounds(), INK), 9);
}

#[test]
fn a_fill_that_reaches_past_the_surface_is_clipped_to_it() {
    let mut bytes = buffer(8, 8);
    let mut surface = surface(&mut bytes, 8, 8);
    let written = draw(
        &mut surface,
        &Command::Fill {
            rect: Rect::new(6, 6, 8, 8),
            color: INK,
        },
    );
    assert_eq!(written, Rect::new(6, 6, 2, 2));
}

#[test]
fn a_frame_draws_the_edge_and_leaves_the_inside_alone() {
    let mut bytes = buffer(8, 8);
    let mut surface = surface(&mut bytes, 8, 8);
    let written = draw(
        &mut surface,
        &Command::Frame {
            rect: Rect::new(1, 1, 5, 4),
            color: INK,
        },
    );
    assert_eq!(written, Rect::new(1, 1, 5, 4));
    // Two rows of five and two columns of four, of which the four corners
    // are counted once: 5 + 5 + 4 + 4 - 4.
    assert_eq!(count(&surface, surface.bounds(), INK), 14);
    assert_eq!(surface.pixel(2, 2), Some(Color::BLACK));
}

#[test]
fn a_frame_outside_the_surface_draws_nothing() {
    let mut bytes = buffer(8, 8);
    let mut surface = surface(&mut bytes, 8, 8);
    assert_eq!(frame(&mut surface, Rect::new(8, 8, 4, 4), INK), Rect::EMPTY);
    assert_eq!(count(&surface, surface.bounds(), INK), 0);
}

#[test]
fn a_segment_joins_its_two_ends() {
    let mut bytes = buffer(8, 8);
    let mut surface = surface(&mut bytes, 8, 8);
    let written = draw(
        &mut surface,
        &Command::Line {
            from: (0, 0),
            to: (7, 7),
            color: INK,
        },
    );
    assert_eq!(written, Rect::new(0, 0, 8, 8));
    assert_eq!(count(&surface, surface.bounds(), INK), 8);
    assert_eq!(surface.pixel(3, 3), Some(INK));
}

#[test]
fn a_segment_of_one_pixel_marks_that_pixel() {
    let mut bytes = buffer(8, 8);
    let mut surface = surface(&mut bytes, 8, 8);
    assert_eq!(
        line(&mut surface, (4, 5), (4, 5), INK),
        Rect::new(4, 5, 1, 1)
    );
    assert_eq!(count(&surface, surface.bounds(), INK), 1);
}

#[test]
fn a_segment_that_runs_backwards_draws_the_same_pixels() {
    let mut forward = buffer(8, 8);
    let mut backward = buffer(8, 8);
    line(&mut surface(&mut forward, 8, 8), (1, 6), (6, 2), INK);
    line(&mut surface(&mut backward, 8, 8), (6, 2), (1, 6), INK);
    assert_eq!(forward, backward);
}

#[test]
fn a_segment_outside_the_surface_writes_nothing() {
    let mut bytes = buffer(8, 8);
    let mut surface = surface(&mut bytes, 8, 8);
    assert_eq!(line(&mut surface, (9, 9), (12, 12), INK), Rect::EMPTY);
    assert_eq!(count(&surface, surface.bounds(), INK), 0);
}

#[test]
fn text_is_written_at_its_cell_and_the_background_fills_the_rest_of_it() {
    let mut bytes = buffer(32, 16);
    let mut surface = surface(&mut bytes, 32, 16);
    let text = Text::new("Hi").unwrap();
    let written = draw(
        &mut surface,
        &Command::Text {
            x: 0,
            y: 0,
            color: INK,
            background: Some(Color::WHITE),
            text,
        },
    );
    assert_eq!(written, Rect::new(0, 0, 16, 16));
    assert_eq!(text_bounds(0, 0, &text), Rect::new(0, 0, 16, 16));
    assert!(count(&surface, Rect::new(0, 0, 16, 16), INK) > 0);
    assert!(count(&surface, Rect::new(0, 0, 16, 16), Color::WHITE) > 0);
    assert_eq!(count(&surface, Rect::new(16, 0, 16, 16), Color::WHITE), 0);
}

#[test]
fn text_without_a_background_leaves_the_cell_as_it_was() {
    let mut bytes = buffer(16, 16);
    let mut surface = surface(&mut bytes, 16, 16);
    surface.fill(surface.bounds(), Color::WHITE);
    draw(
        &mut surface,
        &Command::Text {
            x: 0,
            y: 0,
            color: INK,
            background: None,
            text: Text::new("A").unwrap(),
        },
    );
    assert!(count(&surface, Rect::new(0, 0, 8, 16), Color::WHITE) > 0);
    assert!(count(&surface, Rect::new(0, 0, 8, 16), INK) > 0);
}

#[test]
fn a_list_is_carried_out_in_the_order_it_was_built() {
    let mut bytes = buffer(8, 8);
    let mut surface = surface(&mut bytes, 8, 8);
    let mut list = List::new();
    list.push(Command::Clear { color: INK });
    list.push(Command::Fill {
        rect: Rect::new(0, 0, 4, 4),
        color: Color::WHITE,
    });
    let written = draw_all(&mut surface, &list);
    assert_eq!(written, Rect::new(0, 0, 8, 8));
    assert_eq!(count(&surface, Rect::new(0, 0, 4, 4), Color::WHITE), 16);
    assert_eq!(count(&surface, surface.bounds(), INK), 48);
}

#[test]
fn an_empty_list_writes_nothing() {
    let mut bytes = buffer(8, 8);
    let mut surface = surface(&mut bytes, 8, 8);
    assert_eq!(draw_all(&mut surface, &List::new()), Rect::EMPTY);
    assert!(surface.damage().is_empty());
}

#[test]
fn what_a_command_wrote_is_in_the_damage_of_the_surface() {
    let mut bytes = buffer(16, 16);
    let mut surface = surface(&mut bytes, 16, 16);
    draw(
        &mut surface,
        &Command::Fill {
            rect: Rect::new(2, 2, 4, 4),
            color: INK,
        },
    );
    assert_eq!(surface.damage().bounds(), Rect::new(2, 2, 4, 4));
    surface.clear_damage();
    line(&mut surface, (0, 0), (5, 5), INK);
    assert_eq!(surface.damage().bounds(), Rect::new(0, 0, 6, 6));
}
