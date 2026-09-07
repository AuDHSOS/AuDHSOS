// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::state`, over byte arrays that stand in for the memory
//! of a client and for the framebuffer of the machine.

use audhsos_abi::{Error, FramebufferFormat};
use gfx::{Color, Damage, PixelFormat, Rect, Surface};
use user_proto::display::Mode;

use crate::state::{Display, NOBODY};

/// The screen of these tests.
const SCREEN: Mode = Mode {
    width: 8,
    height: 6,
    format: FramebufferFormat::Rgbx8888,
};

/// The badge of the client of these tests.
const CLIENT: u64 = 0x11;

/// The bytes of a surface of this size.
fn buffer(width: u32, height: u32) -> Vec<u8> {
    let pixels = usize::try_from(width)
        .unwrap()
        .saturating_mul(usize::try_from(height).unwrap());
    vec![0; pixels.saturating_mul(4)]
}

/// A surface over `bytes`.
fn surface(bytes: &mut [u8], width: u32, height: u32) -> Surface<'_> {
    Surface::new(bytes, width, height, width, PixelFormat::Rgbx8888).unwrap()
}

/// A display over the screen above.
fn display() -> Display<4> {
    Display::new(Some(SCREEN))
}

#[test]
fn a_machine_without_a_framebuffer_answers_that_there_is_none() {
    let mut display: Display<4> = Display::default();
    assert_eq!(display.screen(), Err(Error::NotFound));
    assert_eq!(display.create(CLIENT, 4, 4), Err(Error::NotFound));
    assert_eq!(display.format(), Err(Error::NotFound));
    assert_eq!(display.bounds(), Rect::EMPTY);
    let mut bytes = buffer(4, 4);
    let mut screen = surface(&mut bytes, 4, 4);
    assert_eq!(
        display.set_cursor(0, 0, true, &mut screen),
        Err(Error::NotFound)
    );
}

#[test]
fn a_surface_of_the_size_of_the_screen_is_made_and_numbered() {
    let mut display = display();
    let held = display.create(CLIENT, 8, 6).unwrap();
    assert_eq!(held.badge, CLIENT);
    assert_eq!(held.id, 1);
    assert_eq!((held.width, held.height), (8, 6));
    assert_eq!(held.bytes(), 192);
    assert_eq!(display.len(), 1);
    assert!(!display.is_empty());
    assert_eq!(display.of(CLIENT), Some(&held));
    assert_eq!(display.iter().count(), 1);
    assert_eq!(display.screen(), Ok(SCREEN));
    assert_eq!(display.format(), Ok(PixelFormat::Rgbx8888));
    assert_eq!(display.bounds(), Rect::new(0, 0, 8, 6));
}

#[test]
fn a_surface_larger_than_the_screen_or_of_no_pixels_is_refused() {
    let mut display = display();
    for (width, height) in [(9, 6), (8, 7), (0, 6), (8, 0)] {
        assert_eq!(
            display.create(CLIENT, width, height),
            Err(Error::InvalidArgument),
            "{width}x{height}"
        );
    }
    assert!(display.is_empty());
}

#[test]
fn a_client_holds_one_surface_and_no_second() {
    let mut display = display();
    display.create(CLIENT, 4, 4).unwrap();
    assert_eq!(display.create(CLIENT, 4, 4), Err(Error::AlreadyExists));
    assert_eq!(display.len(), 1);
}

#[test]
fn more_clients_than_the_server_holds_are_refused() {
    let mut display: Display<2> = Display::new(Some(SCREEN));
    display.create(1, 4, 4).unwrap();
    display.create(2, 4, 4).unwrap();
    assert_eq!(display.create(3, 4, 4), Err(Error::QuotaExceeded));
}

#[test]
fn a_client_that_names_another_clients_surface_is_refused() {
    let mut display = display();
    let mine = display.create(CLIENT, 4, 4).unwrap();
    display.create(CLIENT + 1, 4, 4).unwrap();
    assert_eq!(display.holding(CLIENT, mine.id), Ok(mine));
    assert_eq!(
        display.holding(CLIENT + 1, mine.id),
        Err(Error::AccessDenied)
    );
    assert_eq!(display.holding(CLIENT, 99), Err(Error::NotFound));
    assert_eq!(
        display.destroy(CLIENT + 1, mine.id),
        Err(Error::AccessDenied)
    );
}

#[test]
fn a_surface_that_is_given_up_is_gone_and_so_is_the_client_that_goes_away() {
    let mut display = display();
    let held = display.create(CLIENT, 4, 4).unwrap();
    assert_eq!(display.destroy(CLIENT, held.id), Ok(held));
    assert!(display.is_empty());
    assert_eq!(display.destroy(CLIENT, held.id), Err(Error::NotFound));

    let second = display.create(CLIENT, 4, 4).unwrap();
    assert_eq!(second.id, 2, "numbers are not handed out twice");
    assert_eq!(display.forget(CLIENT), Some(second));
    assert_eq!(display.forget(CLIENT), None);
    assert!(display.is_empty());
}

#[test]
fn a_presentation_copies_the_damaged_rectangles_and_nothing_else() {
    let mut display = display();
    let held = display.create(CLIENT, 8, 6).unwrap();
    let mut client_bytes = buffer(8, 6);
    let mut client = surface(&mut client_bytes, 8, 6);
    client.fill(Rect::new(1, 1, 2, 2), Color::WHITE);
    client.fill(Rect::new(5, 4, 2, 2), Color::new(1, 2, 3));
    let mut screen_bytes = buffer(8, 6);
    let mut screen = surface(&mut screen_bytes, 8, 6);
    // The sprite would stand over the pixels this test reads, and where it
    // stands has a test of its own below.
    display.set_cursor(0, 0, false, &mut screen).unwrap();
    let damage = *client.damage();
    let written = display
        .present(CLIENT, held.id, &damage, &client, &mut screen)
        .unwrap();
    assert_eq!(written, 8);
    assert_eq!(screen.pixel(1, 1), Some(Color::WHITE));
    assert_eq!(screen.pixel(2, 2), Some(Color::WHITE));
    assert_eq!(screen.pixel(5, 4), Some(Color::new(1, 2, 3)));
    assert_eq!(screen.pixel(4, 4), Some(Color::BLACK));
    assert_eq!(screen.pixel(0, 0), Some(Color::BLACK), "nothing was drawn");
}

#[test]
fn the_sprite_stands_on_top_of_what_was_presented_and_off_it_afterwards() {
    let mut display = display();
    let held = display.create(CLIENT, 8, 6).unwrap();
    let mut client_bytes = buffer(8, 6);
    let mut client = surface(&mut client_bytes, 8, 6);
    client.fill(client.bounds(), Color::new(2, 4, 6));
    let mut screen_bytes = buffer(8, 6);
    let mut screen = surface(&mut screen_bytes, 8, 6);
    let damage = *client.damage();
    display
        .present(CLIENT, held.id, &damage, &client, &mut screen)
        .unwrap();
    assert_eq!(
        screen.pixel(0, 0),
        Some(Color::BLACK),
        "the tip of the sprite"
    );
    assert_eq!(screen.pixel(7, 0), Some(Color::new(2, 4, 6)));

    // The next presentation takes the sprite off first, so what the client
    // drew stands there while the copy runs, and the sprite goes back on
    // afterwards.
    display
        .present(CLIENT, held.id, &damage, &client, &mut screen)
        .unwrap();
    assert_eq!(screen.pixel(0, 0), Some(Color::BLACK));
    assert_eq!(screen.pixel(1, 0), Some(Color::new(2, 4, 6)));
}

#[test]
fn presenting_a_surface_of_another_size_than_was_asked_for_is_refused() {
    let mut display = display();
    let held = display.create(CLIENT, 8, 6).unwrap();
    let mut client_bytes = buffer(4, 4);
    let client = surface(&mut client_bytes, 4, 4);
    let mut screen_bytes = buffer(8, 6);
    let mut screen = surface(&mut screen_bytes, 8, 6);
    assert_eq!(
        display.present(CLIENT, held.id, &Damage::new(), &client, &mut screen),
        Err(Error::InvalidArgument)
    );
}

#[test]
fn presenting_into_a_screen_of_another_format_is_refused() {
    let mut display = display();
    let held = display.create(CLIENT, 8, 6).unwrap();
    let mut client_bytes = buffer(8, 6);
    let client = surface(&mut client_bytes, 8, 6);
    let mut screen_bytes = buffer(8, 6);
    let mut screen = Surface::new(&mut screen_bytes, 8, 6, 8, PixelFormat::Bgrx8888).unwrap();
    assert_eq!(
        display.present(CLIENT, held.id, &Damage::new(), &client, &mut screen),
        Err(Error::InvalidArgument)
    );
}

#[test]
fn a_presentation_that_could_not_be_copied_leaves_the_cursor_on_the_screen() {
    let mut display = display();
    let held = display.create(CLIENT, 8, 6).unwrap();
    let mut client_bytes = buffer(8, 6);
    let client = surface(&mut client_bytes, 8, 6);
    let mut screen_bytes = buffer(8, 6);
    let mut screen = Surface::new(&mut screen_bytes, 8, 6, 8, PixelFormat::Bgrx8888).unwrap();
    display.set_cursor(2, 2, true, &mut screen).unwrap();
    assert!(
        display
            .present(CLIENT, held.id, &Damage::new(), &client, &mut screen)
            .is_err()
    );
    assert!(display.cursor().is_drawn(), "the pointer is still there");
    assert_eq!(
        screen.pixel(2, 2),
        Some(Color::BLACK),
        "and its tip with it"
    );
}

#[test]
fn a_presentation_of_a_surface_nobody_holds_is_refused() {
    let mut display = display();
    let mut client_bytes = buffer(8, 6);
    let client = surface(&mut client_bytes, 8, 6);
    let mut screen_bytes = buffer(8, 6);
    let mut screen = surface(&mut screen_bytes, 8, 6);
    assert_eq!(
        display.present(CLIENT, 1, &Damage::new(), &client, &mut screen),
        Err(Error::NotFound)
    );
}

#[test]
fn the_cursor_is_clamped_to_the_screen_and_leaves_what_was_under_it() {
    let mut display = display();
    let mut screen_bytes = buffer(8, 6);
    let mut screen = surface(&mut screen_bytes, 8, 6);
    screen.fill(screen.bounds(), Color::new(9, 9, 9));
    display.set_cursor(100, 100, true, &mut screen).unwrap();
    assert_eq!(display.cursor().position(), (7, 5));
    assert!(display.cursor().is_visible());
    assert!(display.cursor().is_drawn());
    assert_eq!(screen.pixel(7, 5), Some(Color::BLACK), "the tip is drawn");
    assert_eq!(screen.pixel(0, 0), Some(Color::new(9, 9, 9)));

    display.set_cursor(0, 0, true, &mut screen).unwrap();
    assert_eq!(
        screen.pixel(7, 5),
        Some(Color::new(9, 9, 9)),
        "what was under the sprite is back"
    );
}

#[test]
fn a_cursor_that_is_not_shown_leaves_the_screen_alone() {
    let mut display = display();
    let mut screen_bytes = buffer(8, 6);
    let mut screen = surface(&mut screen_bytes, 8, 6);
    screen.fill(screen.bounds(), Color::new(4, 5, 6));
    display.set_cursor(2, 2, false, &mut screen).unwrap();
    assert!(!display.cursor().is_visible());
    assert!(!display.cursor().is_drawn());
    for y in 0..6 {
        for x in 0..8 {
            assert_eq!(screen.pixel(x, y), Some(Color::new(4, 5, 6)), "at {x},{y}");
        }
    }
}

#[test]
fn a_client_that_carries_no_badge_gets_no_surface() {
    let mut display = display();
    assert_eq!(
        display.create(NOBODY, 4, 4),
        Err(Error::AccessDenied),
        "a capability found under a name names nobody"
    );
    assert!(display.is_empty());
}
