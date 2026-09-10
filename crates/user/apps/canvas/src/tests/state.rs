// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::state`.

use gfx::{Color, GLYPH_HEIGHT, GLYPH_WIDTH, PixelFormat, Rect, Surface, glyph};
use user_proto::input::{BUTTON_LEFT, BUTTON_RIGHT, Event, KeyCode, KeyEvent, PointerEvent};

use crate::state::{BACKGROUND, Canvas, ENDS, INK, PEN, Step, TEXT_ORIGIN, span};

/// The size the tests draw on. It is small enough to walk pixel by pixel
/// and large enough for the text cursor to have somewhere to wrap to.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;

/// The bytes of a surface of this size.
fn buffer(width: u32, height: u32) -> Vec<u8> {
    let pixels = usize::try_from(width)
        .unwrap()
        .saturating_mul(usize::try_from(height).unwrap());
    vec![0; pixels.saturating_mul(4)]
}

/// A surface over `bytes` with the damage of the background already
/// forgotten, so a test sees only what the event under it wrote.
fn surface<'a>(bytes: &'a mut [u8], canvas: &mut Canvas) -> Surface<'a> {
    let mut surface = Surface::new(bytes, WIDTH, HEIGHT, WIDTH, PixelFormat::Rgbx8888).unwrap();
    canvas.clear(&mut surface);
    surface.clear_damage();
    surface
}

/// The event of the pointer moving by this much with these buttons.
const fn moved(dx: i16, dy: i16, buttons: u8) -> Event {
    Event::Pointer(PointerEvent {
        dx,
        dy,
        wheel: 0,
        buttons,
    })
}

/// The event of a key going down or coming up.
const fn key(code: KeyCode, pressed: bool) -> Event {
    Event::Key(KeyEvent { code, pressed })
}

#[test]
fn a_new_canvas_puts_the_pointer_in_the_middle_and_the_text_at_the_origin() {
    let canvas = Canvas::new(WIDTH, HEIGHT);
    assert_eq!(canvas.cursor(), (WIDTH / 2, HEIGHT / 2));
    assert_eq!(canvas.text_cursor(), TEXT_ORIGIN);
    assert!(!canvas.is_drawing());
}

#[test]
fn a_canvas_over_nothing_still_answers_with_a_position() {
    let canvas = Canvas::new(0, 0);
    assert_eq!(canvas.cursor(), (0, 0));
}

#[test]
fn clearing_puts_the_background_everywhere_and_starts_the_text_over() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let screen = surface(&mut bytes, &mut canvas);
    for row in 0..HEIGHT {
        for column in 0..WIDTH {
            assert_eq!(screen.pixel(column, row), Some(BACKGROUND));
        }
    }
    assert_eq!(canvas.text_cursor(), TEXT_ORIGIN);
}

#[test]
fn the_pointer_follows_the_deltas_and_says_it_moved() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    let (x, y) = canvas.cursor();
    assert_eq!(canvas.feed(moved(3, -2, 0), &mut screen), Step::Moved);
    assert_eq!(canvas.cursor(), (x + 3, y - 2));
}

#[test]
fn a_pointer_event_that_moves_nothing_and_draws_nothing_changes_nothing() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    assert_eq!(canvas.feed(moved(0, 0, 0), &mut screen), Step::Nothing);
    assert!(screen.damage().is_empty());
}

#[test]
fn the_pointer_stops_at_every_edge_however_far_it_is_pushed() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    canvas.feed(moved(i16::MIN, i16::MIN, 0), &mut screen);
    assert_eq!(canvas.cursor(), (0, 0));
    canvas.feed(moved(i16::MAX, i16::MAX, 0), &mut screen);
    assert_eq!(canvas.cursor(), (WIDTH - 1, HEIGHT - 1));
}

#[test]
fn a_pressed_button_marks_the_pixel_it_was_pressed_on() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    let at = canvas.cursor();
    let step = canvas.feed(moved(0, 0, BUTTON_LEFT), &mut screen);
    assert_eq!(step, Step::Drew { from: at, to: at });
    assert!(canvas.is_drawing());
    assert_eq!(screen.pixel(at.0, at.1), Some(PEN));
}

#[test]
fn a_held_button_joins_one_position_to_the_next() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    let from = canvas.cursor();
    canvas.feed(moved(0, 0, BUTTON_LEFT), &mut screen);
    let step = canvas.feed(moved(8, 4, BUTTON_LEFT), &mut screen);
    let to = canvas.cursor();
    assert_eq!(step, Step::Drew { from, to });
    // Every column between the two ends carries the pen in some row, which
    // is what a line without gaps means for a segment wider than it is
    // tall.
    for column in from.0..=to.0 {
        let painted = (from.1..=to.1).any(|row| screen.pixel(column, row) == Some(PEN));
        assert!(painted, "column {column} of the stroke is empty");
    }
    assert_eq!(screen.pixel(to.0, to.1), Some(PEN));
}

#[test]
fn a_stroke_upwards_and_to_the_left_is_drawn_the_same_way() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    let from = canvas.cursor();
    canvas.feed(moved(0, 0, BUTTON_LEFT), &mut screen);
    canvas.feed(moved(-9, -6, BUTTON_LEFT), &mut screen);
    let to = canvas.cursor();
    assert!(to.0 < from.0 && to.1 < from.1);
    for column in to.0..=from.0 {
        let painted = (to.1..=from.1).any(|row| screen.pixel(column, row) == Some(PEN));
        assert!(painted, "column {column} of the stroke is empty");
    }
}

#[test]
fn the_damage_of_a_stroke_holds_both_of_its_ends() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    let from = canvas.cursor();
    canvas.feed(moved(0, 0, BUTTON_LEFT), &mut screen);
    screen.clear_damage();
    canvas.feed(moved(-5, 7, BUTTON_LEFT), &mut screen);
    let to = canvas.cursor();
    let bounds = screen.damage().bounds();
    assert!(
        bounds.contains(from.0, from.1),
        "{bounds:?} misses {from:?}"
    );
    assert!(bounds.contains(to.0, to.1), "{bounds:?} misses {to:?}");
}

#[test]
fn a_button_that_is_not_the_one_that_draws_only_moves() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    assert_eq!(
        canvas.feed(moved(4, 0, BUTTON_RIGHT), &mut screen),
        Step::Moved
    );
    assert!(!canvas.is_drawing());
    assert!(screen.damage().is_empty());
}

#[test]
fn releasing_the_button_stops_the_drawing() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    canvas.feed(moved(0, 0, BUTTON_LEFT), &mut screen);
    assert!(canvas.is_drawing());
    let step = canvas.feed(moved(3, 3, 0), &mut screen);
    assert_eq!(step, Step::Moved);
    assert!(!canvas.is_drawing());
    let (x, y) = canvas.cursor();
    assert_eq!(screen.pixel(x, y), Some(BACKGROUND));
}

#[test]
fn a_typed_letter_stands_at_the_text_cursor_pixel_for_pixel() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    let at = canvas.text_cursor();
    let step = canvas.feed(key(KeyCode::A, true), &mut screen);
    assert_eq!(
        step,
        Step::Typed {
            x: at.0,
            y: at.1,
            character: 'a'
        }
    );
    let rows = glyph('a');
    for (row, bits) in rows.iter().enumerate() {
        for column in 0..GLYPH_WIDTH {
            let set = bits & (1 << (GLYPH_WIDTH - 1 - column)) != 0;
            let wanted = if set { INK } else { BACKGROUND };
            let y = at.1 + u32::try_from(row).unwrap();
            assert_eq!(
                screen.pixel(at.0 + column, y),
                Some(wanted),
                "pixel {column},{row} of the glyph"
            );
        }
    }
    assert_eq!(canvas.text_cursor(), (at.0 + GLYPH_WIDTH, at.1));
}

#[test]
fn a_release_types_nothing() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    canvas.feed(key(KeyCode::A, true), &mut screen);
    let at = canvas.text_cursor();
    assert_eq!(
        canvas.feed(key(KeyCode::A, false), &mut screen),
        Step::Nothing
    );
    assert_eq!(canvas.text_cursor(), at);
}

#[test]
fn shift_is_held_across_events_and_makes_the_letter_capital() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    canvas.feed(key(KeyCode::LeftShift, true), &mut screen);
    let step = canvas.feed(key(KeyCode::A, true), &mut screen);
    assert!(matches!(step, Step::Typed { character: 'A', .. }));
    canvas.feed(key(KeyCode::A, false), &mut screen);
    canvas.feed(key(KeyCode::LeftShift, false), &mut screen);
    let step = canvas.feed(key(KeyCode::A, true), &mut screen);
    assert!(matches!(step, Step::Typed { character: 'a', .. }));
}

#[test]
fn a_key_that_stands_for_no_character_writes_nothing() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    let at = canvas.text_cursor();
    assert_eq!(
        canvas.feed(key(KeyCode::F5, true), &mut screen),
        Step::Nothing
    );
    assert_eq!(canvas.text_cursor(), at);
}

#[test]
fn the_text_wraps_at_the_right_edge_instead_of_writing_off_it() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    let start = canvas.text_cursor();
    let cells = (WIDTH - start.0) / GLYPH_WIDTH;
    for _ in 0..cells {
        canvas.feed(key(KeyCode::A, true), &mut screen);
        canvas.feed(key(KeyCode::A, false), &mut screen);
    }
    let step = canvas.feed(key(KeyCode::A, true), &mut screen);
    assert_eq!(
        step,
        Step::Typed {
            x: TEXT_ORIGIN.0,
            y: start.1 + GLYPH_HEIGHT,
            character: 'a'
        }
    );
}

#[test]
fn the_return_key_starts_a_new_line_without_writing() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    canvas.feed(key(KeyCode::A, true), &mut screen);
    canvas.feed(key(KeyCode::A, false), &mut screen);
    assert_eq!(
        canvas.feed(key(KeyCode::Enter, true), &mut screen),
        Step::Nothing
    );
    assert_eq!(
        canvas.text_cursor(),
        (TEXT_ORIGIN.0, TEXT_ORIGIN.1 + GLYPH_HEIGHT)
    );
}

#[test]
fn the_text_goes_back_to_the_top_when_there_is_no_next_line() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    for _ in 0..HEIGHT {
        canvas.feed(key(KeyCode::Enter, true), &mut screen);
        canvas.feed(key(KeyCode::Enter, false), &mut screen);
        let (_x, y) = canvas.text_cursor();
        assert!(
            y + GLYPH_HEIGHT <= HEIGHT,
            "the text cursor left the surface"
        );
    }
}

#[test]
fn escape_puts_the_background_back_and_starts_the_text_over() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    canvas.feed(moved(0, 0, BUTTON_LEFT), &mut screen);
    canvas.feed(moved(6, 6, BUTTON_LEFT), &mut screen);
    canvas.feed(key(KeyCode::A, true), &mut screen);
    let drawn = canvas.cursor();
    assert_eq!(
        canvas.feed(key(KeyCode::Escape, true), &mut screen),
        Step::Cleared
    );
    assert_eq!(screen.pixel(drawn.0, drawn.1), Some(BACKGROUND));
    assert_eq!(
        screen.pixel(TEXT_ORIGIN.0 + 1, TEXT_ORIGIN.1 + 8),
        Some(BACKGROUND)
    );
    assert_eq!(canvas.text_cursor(), TEXT_ORIGIN);
}

#[test]
fn escape_coming_up_clears_nothing_a_second_time() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    canvas.feed(key(KeyCode::Escape, true), &mut screen);
    screen.clear_damage();
    assert_eq!(
        canvas.feed(key(KeyCode::Escape, false), &mut screen),
        Step::Nothing
    );
    assert!(screen.damage().is_empty());
}

#[test]
fn the_key_that_ends_the_program_ends_it_when_it_comes_up() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    assert_eq!(canvas.feed(key(ENDS, true), &mut screen), Step::Nothing);
    assert_eq!(canvas.feed(key(ENDS, false), &mut screen), Step::Ended);
}

#[test]
fn the_key_that_ends_the_program_types_nothing() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    let at = canvas.text_cursor();
    canvas.feed(key(ENDS, true), &mut screen);
    assert_eq!(canvas.text_cursor(), at);
}

#[test]
fn the_span_of_two_points_holds_both_of_them_whichever_way_round_they_are() {
    assert_eq!(span((2, 3), (6, 9)), Rect::new(2, 3, 5, 7));
    assert_eq!(span((6, 9), (2, 3)), Rect::new(2, 3, 5, 7));
    assert_eq!(span((4, 4), (4, 4)), Rect::new(4, 4, 1, 1));
}

#[test]
fn nothing_a_stroke_draws_leaves_the_surface() {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let mut bytes = buffer(WIDTH, HEIGHT);
    let mut screen = surface(&mut bytes, &mut canvas);
    // Along every edge and back through the middle, with the button held
    // throughout: a stroke that ran off the surface would be a pixel this
    // never wrote or a panic in the walk.
    for (dx, dy) in [
        (i16::MIN, i16::MIN),
        (i16::MAX, 0),
        (0, i16::MAX),
        (i16::MIN, i16::MIN),
        (30, 20),
    ] {
        canvas.feed(moved(dx, dy, BUTTON_LEFT), &mut screen);
        let (x, y) = canvas.cursor();
        assert!(x < WIDTH && y < HEIGHT);
    }
    let outside = screen
        .damage()
        .iter()
        .find(|rect| rect.right() > WIDTH || rect.bottom() > HEIGHT);
    assert!(outside.is_none(), "damage left the surface: {outside:?}");
}

#[test]
fn the_pen_the_ink_and_the_background_are_three_different_colors() {
    let colors = [BACKGROUND, PEN, INK, Color::WHITE, Color::BLACK];
    for (first, one) in colors.iter().enumerate() {
        for (second, other) in colors.iter().enumerate() {
            if first != second {
                assert_ne!(one, other, "{first} and {second} are the same color");
            }
        }
    }
}
