// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::state`.

use audhsos_abi::Error;
use audhsos_time::CivilTime;
use gfx::{Color, PixelFormat, Rect, Surface};
use user_proto::input::{BUTTON_LEFT, Event as Input, KeyCode, KeyEvent, PointerEvent};
use user_proto::window::{Event, Title};

use crate::bar;
use crate::state::{DESKTOP, Desk, ENDS, NOBODY, Outcome, SHOWS, WINDOWS};
use crate::window::{CLOSE_INK, FOCUSED, PLAIN, TITLE_HEIGHT, Window};

/// The screen every test works on.
const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

/// A desktop that has been painted once.
fn desk() -> Desk {
    let mut desk = Desk::new(WIDTH, HEIGHT);
    desk.show();
    desk
}

/// One key going down or coming up.
fn key(code: KeyCode, pressed: bool) -> Input {
    Input::Key(KeyEvent { code, pressed })
}

/// The pointer moving to `x`, `y` with `buttons` held.
fn to(desk: &mut Desk, x: u32, y: u32, buttons: u8) -> Outcome {
    let (from_x, from_y) = desk.pointer();
    let dx = i32::try_from(x)
        .unwrap()
        .saturating_sub(i32::try_from(from_x).unwrap());
    let dy = i32::try_from(y)
        .unwrap()
        .saturating_sub(i32::try_from(from_y).unwrap());
    desk.feed(Input::Pointer(PointerEvent {
        dx: i16::try_from(dx).unwrap(),
        dy: i16::try_from(dy).unwrap(),
        wheel: 0,
        buttons,
    }))
}

/// The button going down where the pointer already stands.
fn press(desk: &mut Desk) -> Outcome {
    desk.feed(Input::Pointer(PointerEvent {
        dx: 0,
        dy: 0,
        wheel: 0,
        buttons: BUTTON_LEFT,
    }))
}

/// The button coming up where the pointer already stands.
fn release(desk: &mut Desk) -> Outcome {
    desk.feed(Input::Pointer(PointerEvent {
        dx: 0,
        dy: 0,
        wheel: 0,
        buttons: 0,
    }))
}

/// A click at `x`, `y`: the pointer goes there, the button goes down.
fn click(desk: &mut Desk, x: u32, y: u32) -> Outcome {
    to(desk, x, y, 0);
    press(desk)
}

/// Opens a window of 200 by 100 for `badge`.
fn open(desk: &mut Desk, badge: u64, title: &[u8]) -> u32 {
    desk.open(
        badge,
        200,
        100,
        Title::new(title).unwrap(),
        &mut Outcome::nothing(),
    )
    .unwrap()
}

/// Every event waiting for the window of `badge`.
fn drain(desk: &mut Desk, badge: u64, id: u32) -> Vec<Event> {
    let mut events = Vec::new();
    while let Ok(Some(event)) = desk.poll(badge, id) {
        events.push(event);
    }
    events
}

/// A screen to paint on.
fn screen() -> Vec<u8> {
    vec![0; usize::try_from(WIDTH.saturating_mul(HEIGHT).saturating_mul(4)).unwrap()]
}

/// A surface over `bytes`.
fn surface(bytes: &mut [u8]) -> Surface<'_> {
    Surface::new(bytes, WIDTH, HEIGHT, WIDTH, PixelFormat::Rgbx8888).unwrap()
}

/// Paints everything the desktop paints, in the order the process paints
/// it, and answers with the surface bytes.
fn painted(desk: &Desk) -> Vec<u8> {
    let mut bytes = screen();
    {
        let mut screen = surface(&mut bytes);
        desk.paint_desktop(&mut screen, desk.bounds());
        for window in desk.iter() {
            desk.paint_chrome(&mut screen, window);
        }
        desk.paint_bar(&mut screen);
    }
    bytes
}

/// The color at `x`, `y` of a painted screen.
fn pixel(bytes: &[u8], x: u32, y: u32) -> Color {
    let mut copy = bytes.to_vec();
    let surface = surface(&mut copy);
    surface.pixel(x, y).unwrap()
}

/// How many pixels of `rect` carry `color`.
fn count(bytes: &[u8], rect: Rect, color: Color) -> u32 {
    let mut copy = bytes.to_vec();
    let surface = surface(&mut copy);
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
fn a_request_without_a_badge_gets_no_window() {
    let mut desk = desk();
    assert_eq!(
        desk.open(NOBODY, 100, 100, Title::empty(), &mut Outcome::nothing()),
        Err(Error::AccessDenied)
    );
}

#[test]
fn a_window_of_no_pixels_is_refused() {
    let mut desk = desk();
    assert_eq!(
        desk.open(1, 0, 100, Title::empty(), &mut Outcome::nothing()),
        Err(Error::InvalidArgument)
    );
    assert_eq!(
        desk.open(1, 100, 0, Title::empty(), &mut Outcome::nothing()),
        Err(Error::InvalidArgument)
    );
}

#[test]
fn a_screen_with_no_room_for_a_window_opens_none() {
    let mut desk = Desk::new(4, 4);
    assert_eq!(
        desk.open(1, 100, 100, Title::empty(), &mut Outcome::nothing()),
        Err(Error::InvalidArgument)
    );
}

#[test]
fn a_client_holds_one_window() {
    let mut desk = desk();
    open(&mut desk, 1, b"first");
    assert_eq!(
        desk.open(1, 100, 100, Title::empty(), &mut Outcome::nothing()),
        Err(Error::AlreadyExists)
    );
}

#[test]
fn the_desktop_holds_as_many_windows_as_it_holds() {
    let mut desk = desk();
    for badge in 1..=u64::try_from(WINDOWS).unwrap() {
        open(&mut desk, badge, b"window");
    }
    assert_eq!(desk.len(), WINDOWS);
    assert_eq!(
        desk.open(99, 100, 100, Title::empty(), &mut Outcome::nothing()),
        Err(Error::QuotaExceeded)
    );
}

#[test]
fn a_window_larger_than_the_screen_is_cut_down_to_it() {
    let mut desk = desk();
    let id = desk
        .open(
            1,
            10_000,
            10_000,
            Title::new(b"big").unwrap(),
            &mut Outcome::nothing(),
        )
        .unwrap();
    let window = desk.window(id).unwrap();
    assert_eq!(window.width(), desk.max_width());
    assert_eq!(window.height(), desk.max_height());
    assert!(window.frame().bottom() <= HEIGHT);
}

#[test]
fn every_window_stands_below_the_bar_and_further_right_than_the_one_before() {
    let mut desk = desk();
    let first = open(&mut desk, 1, b"first");
    let second = open(&mut desk, 2, b"second");
    let first = desk.window(first).unwrap().frame();
    let second = desk.window(second).unwrap().frame();
    assert!(first.y >= bar::HEIGHT);
    assert!(second.x > first.x && second.y > first.y);
}

#[test]
fn the_window_that_just_opened_stands_in_front_and_has_the_focus() {
    let mut desk = desk();
    open(&mut desk, 1, b"first");
    let second = open(&mut desk, 2, b"second");
    assert_eq!(desk.focused().map(Window::id), Some(second));
    assert_eq!(desk.iter().last().map(Window::id), Some(second));
}

#[test]
fn a_client_cannot_name_another_clients_window() {
    let mut desk = desk();
    let first = open(&mut desk, 1, b"first");
    open(&mut desk, 2, b"second");
    assert_eq!(desk.holding(2, first).err(), Some(Error::AccessDenied));
    assert_eq!(desk.holding_mut(2, first).err(), Some(Error::AccessDenied));
    assert_eq!(desk.holding(1, 99).err(), Some(Error::NotFound));
    assert_eq!(desk.poll(2, first).err(), Some(Error::AccessDenied));
    assert_eq!(
        desk.close(2, first, &mut Outcome::nothing()).err(),
        Some(Error::AccessDenied)
    );
    assert!(desk.holding(1, first).is_ok());
}

#[test]
fn a_window_that_is_closed_is_gone_and_the_one_behind_it_takes_the_focus() {
    let mut desk = desk();
    let first = open(&mut desk, 1, b"first");
    let second = open(&mut desk, 2, b"second");
    let frame = desk.window(second).unwrap().frame();
    assert_eq!(desk.close(2, second, &mut Outcome::nothing()), Ok(frame));
    assert_eq!(desk.len(), 1);
    assert_eq!(desk.focused().map(Window::id), Some(first));
    assert!(desk.window(second).is_none());
}

#[test]
fn a_client_that_is_gone_leaves_no_window_behind() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"first");
    let frame = desk.window(id).unwrap().frame();
    assert_eq!(desk.forget(1, &mut Outcome::nothing()), Some((id, frame)));
    assert!(desk.is_empty());
    assert_eq!(desk.forget(1, &mut Outcome::nothing()), None);
}

#[test]
fn nothing_is_painted_before_the_key_that_shows_the_desktop() {
    let mut desk = Desk::new(WIDTH, HEIGHT);
    assert!(!desk.is_shown());
    let outcome = desk.feed(key(KeyCode::A, true));
    assert!(outcome.repaint.is_empty());
    let outcome = desk.feed(key(SHOWS, true));
    assert!(outcome.repaint.is_empty());
    let outcome = desk.feed(key(SHOWS, false));
    assert_eq!(outcome.repaint, desk.bounds());
    assert!(desk.is_shown());
}

#[test]
fn the_pointer_moves_while_the_desktop_is_not_shown_and_changes_nothing() {
    let mut desk = Desk::new(WIDTH, HEIGHT);
    let outcome = to(&mut desk, 10, 10, BUTTON_LEFT);
    assert!(outcome.moved);
    assert!(outcome.repaint.is_empty());
    assert_eq!(desk.pointer(), (10, 10));
}

#[test]
fn the_key_that_ends_the_desktop_tells_every_client() {
    let mut desk = desk();
    let first = open(&mut desk, 1, b"first");
    let second = open(&mut desk, 2, b"second");
    drain(&mut desk, 1, first);
    drain(&mut desk, 2, second);
    let outcome = desk.feed(key(ENDS, true));
    assert!(!outcome.ended);
    let outcome = desk.feed(key(ENDS, false));
    assert!(outcome.ended);
    assert_eq!(outcome.woken.len(), 2);
    assert_eq!(drain(&mut desk, 1, first), vec![Event::Closed]);
    assert_eq!(drain(&mut desk, 2, second), vec![Event::Closed]);
}

#[test]
fn a_key_reaches_the_window_that_has_the_focus_and_no_other() {
    let mut desk = desk();
    let first = open(&mut desk, 1, b"first");
    let second = open(&mut desk, 2, b"second");
    drain(&mut desk, 1, first);
    drain(&mut desk, 2, second);
    let outcome = desk.feed(key(KeyCode::A, true));
    assert_eq!(
        outcome.woken.iter().copied().collect::<Vec<_>>(),
        vec![second]
    );
    assert_eq!(drain(&mut desk, 1, first), vec![]);
    assert_eq!(
        drain(&mut desk, 2, second),
        vec![Event::Key {
            code: KeyCode::A,
            pressed: true,
        }]
    );
}

#[test]
fn a_key_with_no_window_open_reaches_nobody() {
    let mut desk = desk();
    let outcome = desk.feed(key(KeyCode::A, true));
    assert!(outcome.woken.is_empty());
    assert!(outcome.repaint.is_empty());
}

#[test]
fn the_clock_paints_the_bar_when_it_moves_and_nothing_when_it_does_not() {
    let mut desk = desk();
    let noon = CivilTime::new(2026, 9, 16, 12, 0, 0).unwrap();
    assert_eq!(desk.tick(noon), bar::rect(WIDTH));
    assert_eq!(desk.tick(noon), Rect::EMPTY);
    assert_eq!(&desk.clock().text(), b"12:00:00");
}

#[test]
fn a_clock_that_moves_before_the_desktop_is_shown_paints_nothing() {
    let mut desk = Desk::new(WIDTH, HEIGHT);
    let noon = CivilTime::new(2026, 9, 16, 12, 0, 0).unwrap();
    assert_eq!(desk.tick(noon), Rect::EMPTY);
}

#[test]
fn the_pointer_over_the_content_reaches_the_client_in_its_own_coordinates() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"first");
    drain(&mut desk, 1, id);
    let content = desk.window(id).unwrap().content();
    let outcome = to(&mut desk, content.x + 5, content.y + 7, 0);
    assert_eq!(outcome.woken.iter().copied().collect::<Vec<_>>(), vec![id]);
    assert_eq!(
        drain(&mut desk, 1, id),
        vec![Event::Pointer {
            x: 5,
            y: 7,
            buttons: 0,
        }]
    );
}

#[test]
fn the_pointer_over_the_desktop_reaches_nobody() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"first");
    drain(&mut desk, 1, id);
    let outcome = to(&mut desk, WIDTH - 1, HEIGHT - 1, 0);
    assert!(outcome.woken.is_empty());
    assert_eq!(drain(&mut desk, 1, id), vec![]);
}

#[test]
fn the_pointer_over_the_border_of_a_window_reaches_nobody() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"first");
    drain(&mut desk, 1, id);
    let frame = desk.window(id).unwrap().frame();
    let outcome = to(&mut desk, frame.x, frame.bottom() - 1, 0);
    assert!(outcome.woken.is_empty());
}

#[test]
fn a_click_on_a_window_behind_puts_it_in_front_and_tells_both_clients() {
    let mut desk = desk();
    let first = open(&mut desk, 1, b"first");
    let second = open(&mut desk, 2, b"second");
    drain(&mut desk, 1, first);
    drain(&mut desk, 2, second);
    let bar_of_first = desk.window(first).unwrap().title_bar();
    let outcome = click(&mut desk, bar_of_first.x + 4, bar_of_first.y + 4);
    assert_eq!(desk.iter().last().map(Window::id), Some(first));
    assert_eq!(desk.focused().map(Window::id), Some(first));
    assert_eq!(outcome.woken.len(), 2);
    assert_eq!(drain(&mut desk, 1, first), vec![Event::Focus { has: true }]);
    assert_eq!(
        drain(&mut desk, 2, second),
        vec![Event::Focus { has: false }]
    );
    assert!(
        outcome
            .repaint
            .overlaps(desk.window(first).unwrap().frame())
    );
}

#[test]
fn a_click_on_the_window_in_front_changes_no_order_and_wakes_nobody() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"only");
    drain(&mut desk, 1, id);
    let title = desk.window(id).unwrap().title_bar();
    let outcome = click(&mut desk, title.x + 4, title.y + 4);
    assert!(outcome.woken.is_empty());
    assert_eq!(drain(&mut desk, 1, id), vec![]);
}

#[test]
fn a_window_is_dragged_by_its_title_bar() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"only");
    let before = desk.window(id).unwrap().frame();
    click(&mut desk, before.x + 10, before.y + 10);
    let outcome = to(&mut desk, before.x + 60, before.y + 40, BUTTON_LEFT);
    let after = desk.window(id).unwrap().frame();
    assert_eq!(after.x, before.x + 50);
    assert_eq!(after.y, before.y + 30);
    assert!(outcome.repaint.overlaps(before));
    assert!(outcome.repaint.overlaps(after));
    release(&mut desk);
    to(&mut desk, before.x, before.y, 0);
    assert_eq!(desk.window(id).unwrap().frame(), after);
}

#[test]
fn a_drag_keeps_the_window_below_the_bar_and_on_the_screen() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"only");
    let frame = desk.window(id).unwrap().frame();
    click(&mut desk, frame.x + 10, frame.y + 10);
    to(&mut desk, 0, 0, BUTTON_LEFT);
    assert_eq!(desk.window(id).unwrap().frame().y, bar::HEIGHT);
    to(&mut desk, WIDTH - 1, HEIGHT - 1, BUTTON_LEFT);
    let frame = desk.window(id).unwrap().frame();
    assert_eq!(frame.right(), WIDTH);
    assert_eq!(frame.bottom(), HEIGHT);
}

#[test]
fn a_click_on_the_content_of_a_window_is_no_drag() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"only");
    let content = desk.window(id).unwrap().content();
    let before = desk.window(id).unwrap().frame();
    click(&mut desk, content.x + 5, content.y + 5);
    to(&mut desk, content.x + 100, content.y + 50, BUTTON_LEFT);
    assert_eq!(desk.window(id).unwrap().frame(), before);
    let events = drain(&mut desk, 1, id);
    assert!(events.contains(&Event::Pointer {
        x: 5,
        y: 5,
        buttons: BUTTON_LEFT,
    }));
}

#[test]
fn a_click_on_the_close_box_tells_the_client_and_leaves_the_window_standing() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"only");
    drain(&mut desk, 1, id);
    let close = desk.window(id).unwrap().close_box();
    let outcome = click(&mut desk, close.x + 2, close.y + 2);
    assert_eq!(outcome.woken.iter().copied().collect::<Vec<_>>(), vec![id]);
    assert_eq!(drain(&mut desk, 1, id), vec![Event::Closed]);
    assert_eq!(desk.len(), 1);
}

#[test]
fn a_click_on_a_menu_title_opens_its_panel_and_a_click_beside_it_closes_it() {
    let mut desk = desk();
    let cell = bar::title_rect(bar::DESK_MENU);
    let outcome = click(&mut desk, cell.x + 2, 2);
    assert_eq!(desk.menu(), Some(bar::DESK_MENU));
    assert!(outcome.repaint.overlaps(bar::rect(WIDTH)));
    release(&mut desk);
    click(&mut desk, WIDTH / 2, HEIGHT / 2);
    assert_eq!(desk.menu(), None);
}

#[test]
fn a_click_on_the_bar_beside_every_title_opens_nothing() {
    let mut desk = desk();
    click(&mut desk, WIDTH - 2, 2);
    assert_eq!(desk.menu(), None);
}

#[test]
fn the_entry_that_quits_ends_the_desktop() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"only");
    drain(&mut desk, 1, id);
    let cell = bar::title_rect(bar::DESK_MENU);
    click(&mut desk, cell.x + 2, 2);
    release(&mut desk);
    let quit = bar::entry_rect(bar::DESK_MENU, desk.entries(bar::DESK_MENU), 1);
    let outcome = click(&mut desk, quit.x + 2, quit.y + 2);
    assert!(outcome.ended);
    assert_eq!(desk.menu(), None);
    assert_eq!(drain(&mut desk, 1, id), vec![Event::Closed]);
}

#[test]
fn the_entry_that_closes_a_window_tells_the_window_that_has_the_focus() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"only");
    drain(&mut desk, 1, id);
    let cell = bar::title_rect(bar::DESK_MENU);
    click(&mut desk, cell.x + 2, 2);
    release(&mut desk);
    let entry = bar::entry_rect(bar::DESK_MENU, desk.entries(bar::DESK_MENU), 0);
    let outcome = click(&mut desk, entry.x + 2, entry.y + 2);
    assert!(!outcome.ended);
    assert_eq!(drain(&mut desk, 1, id), vec![Event::Closed]);
}

#[test]
fn the_entry_that_closes_a_window_with_none_open_closes_the_menu() {
    let mut desk = desk();
    let cell = bar::title_rect(bar::DESK_MENU);
    click(&mut desk, cell.x + 2, 2);
    release(&mut desk);
    let entry = bar::entry_rect(bar::DESK_MENU, desk.entries(bar::DESK_MENU), 0);
    click(&mut desk, entry.x + 2, entry.y + 2);
    assert_eq!(desk.menu(), None);
}

#[test]
fn the_menu_of_the_windows_lists_them_from_the_front_and_raises_what_is_chosen() {
    let mut desk = desk();
    let first = open(&mut desk, 1, b"first");
    let second = open(&mut desk, 2, b"second");
    assert_eq!(desk.entries(bar::WINDOW_MENU), 2);
    assert_eq!(desk.entry_text(bar::WINDOW_MENU, 0), "second");
    assert_eq!(desk.entry_text(bar::WINDOW_MENU, 1), "first");
    assert_eq!(desk.entry_text(bar::WINDOW_MENU, 2), "");
    assert_eq!(desk.entry_text(bar::DESK_MENU, 0), bar::DESK_ENTRIES[0]);
    assert_eq!(desk.entry_text(bar::DESK_MENU, 9), "");
    let cell = bar::title_rect(bar::WINDOW_MENU);
    click(&mut desk, cell.x + 2, 2);
    release(&mut desk);
    let entry = bar::entry_rect(bar::WINDOW_MENU, 2, 1);
    let outcome = click(&mut desk, entry.x + 2, entry.y + 2);
    assert_eq!(desk.iter().last().map(Window::id), Some(first));
    assert_eq!(desk.focused().map(Window::id), Some(first));
    assert_eq!(outcome.repaint, desk.bounds());
    assert_eq!(desk.menu(), None);
    drain(&mut desk, 2, second);
}

#[test]
fn a_click_in_an_open_panel_that_names_no_entry_only_closes_it() {
    let mut desk = desk();
    let cell = bar::title_rect(bar::DESK_MENU);
    click(&mut desk, cell.x + 2, 2);
    release(&mut desk);
    let panel = bar::panel_rect(bar::DESK_MENU, desk.entries(bar::DESK_MENU));
    click(&mut desk, panel.x, panel.bottom() + 40);
    assert_eq!(desk.menu(), None);
}

#[test]
fn a_window_that_is_closed_while_its_menu_is_open_closes_the_menu() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"only");
    let cell = bar::title_rect(bar::WINDOW_MENU);
    click(&mut desk, cell.x + 2, 2);
    assert_eq!(desk.menu(), Some(bar::WINDOW_MENU));
    desk.close(1, id, &mut Outcome::nothing()).unwrap();
    assert_eq!(desk.menu(), None);
}

#[test]
fn a_window_dragged_away_while_it_is_closed_drags_nothing() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"only");
    let frame = desk.window(id).unwrap().frame();
    click(&mut desk, frame.x + 10, frame.y + 10);
    desk.close(1, id, &mut Outcome::nothing()).unwrap();
    let outcome = to(&mut desk, frame.x + 100, frame.y + 100, BUTTON_LEFT);
    assert!(outcome.repaint.is_empty());
}

#[test]
fn the_desktop_is_painted_under_everything() {
    let desk = desk();
    let bytes = painted(&desk);
    assert_eq!(pixel(&bytes, WIDTH / 2, HEIGHT / 2), DESKTOP);
    assert_eq!(pixel(&bytes, 0, HEIGHT - 1), DESKTOP);
}

#[test]
fn the_bar_is_painted_over_the_windows_with_the_clock_at_its_right_end() {
    let mut desk = desk();
    desk.open(
        1,
        600,
        400,
        Title::new(b"wide").unwrap(),
        &mut Outcome::nothing(),
    )
    .unwrap();
    desk.tick(CivilTime::new(2026, 9, 16, 11, 22, 33).unwrap());
    let bytes = painted(&desk);
    assert_eq!(pixel(&bytes, 0, 0), bar::BACKGROUND);
    assert_eq!(pixel(&bytes, WIDTH - 1, 0), bar::BACKGROUND);
    assert_eq!(pixel(&bytes, 0, bar::HEIGHT - 1), bar::EDGE);
    let clock = bar::clock_rect(WIDTH);
    assert!(count(&bytes, clock, bar::INK) > 0);
    assert!(count(&bytes, bar::title_rect(bar::DESK_MENU), bar::INK) > 0);
}

#[test]
fn the_frame_of_a_window_carries_its_title_and_its_close_box() {
    let mut desk = desk();
    let id = open(&mut desk, 1, b"shell");
    let bytes = painted(&desk);
    let window = desk.window(id).unwrap();
    let frame = window.frame();
    assert_eq!(pixel(&bytes, frame.x, frame.bottom() - 1), FOCUSED);
    let close = window.close_box();
    assert_eq!(count(&bytes, close, CLOSE_INK), close.w * close.h);
    let title = Rect::new(frame.x, frame.y, frame.w, TITLE_HEIGHT);
    assert!(count(&bytes, title, Color::WHITE) > 0);
}

#[test]
fn the_window_behind_carries_the_other_frame_color() {
    let mut desk = desk();
    let first = open(&mut desk, 1, b"first");
    open(&mut desk, 2, b"second");
    let bytes = painted(&desk);
    let frame = desk.window(first).unwrap().frame();
    assert_eq!(pixel(&bytes, frame.x, frame.bottom() - 1), PLAIN);
}

#[test]
fn an_open_panel_is_painted_under_the_bar() {
    let mut desk = desk();
    let cell = bar::title_rect(bar::DESK_MENU);
    click(&mut desk, cell.x + 2, 2);
    let bytes = painted(&desk);
    let panel = bar::panel_rect(bar::DESK_MENU, desk.entries(bar::DESK_MENU));
    assert_eq!(pixel(&bytes, panel.x, panel.y), bar::EDGE);
    assert!(count(&bytes, panel, bar::INK) > 0);
    assert!(count(&bytes, cell, bar::HIGHLIGHT) > 0);
}

#[test]
fn a_panel_of_no_entry_is_painted_as_nothing() {
    let mut desk = desk();
    let cell = bar::title_rect(bar::WINDOW_MENU);
    click(&mut desk, cell.x + 2, 2);
    assert_eq!(desk.entries(bar::WINDOW_MENU), 0);
    let bytes = painted(&desk);
    assert_eq!(pixel(&bytes, cell.x, bar::HEIGHT + 2), DESKTOP);
}

#[test]
fn the_window_that_opens_hears_that_it_has_the_focus_and_the_one_before_it_that_it_lost_it() {
    let mut desk = desk();
    let first = open(&mut desk, 1, b"first");
    assert_eq!(drain(&mut desk, 1, first), vec![Event::Focus { has: true }]);
    let mut outcome = Outcome::nothing();
    let second = desk
        .open(2, 200, 100, Title::new(b"second").unwrap(), &mut outcome)
        .unwrap();
    assert_eq!(outcome.woken.len(), 2);
    assert!(
        outcome
            .repaint
            .overlaps(desk.window(second).unwrap().frame())
    );
    assert_eq!(
        drain(&mut desk, 1, first),
        vec![Event::Focus { has: false }]
    );
    assert_eq!(
        drain(&mut desk, 2, second),
        vec![Event::Focus { has: true }]
    );
}

#[test]
fn the_desktop_ends_whether_it_was_painted_or_not() {
    let mut desk = Desk::new(WIDTH, HEIGHT);
    let id = desk
        .open(
            1,
            200,
            100,
            Title::new(b"only").unwrap(),
            &mut Outcome::nothing(),
        )
        .unwrap();
    drain(&mut desk, 1, id);
    assert!(!desk.is_shown());
    let outcome = desk.feed(key(ENDS, false));
    assert!(outcome.ended);
    assert_eq!(drain(&mut desk, 1, id), vec![Event::Closed]);
}

#[test]
fn the_window_that_takes_the_focus_from_a_window_that_closed_hears_that_it_has() {
    let mut desk = desk();
    let first = open(&mut desk, 1, b"first");
    let second = open(&mut desk, 2, b"second");
    drain(&mut desk, 1, first);
    drain(&mut desk, 2, second);
    let mut outcome = Outcome::nothing();
    desk.close(2, second, &mut outcome).unwrap();
    assert_eq!(
        outcome.woken.iter().copied().collect::<Vec<_>>(),
        vec![first]
    );
    assert_eq!(drain(&mut desk, 1, first), vec![Event::Focus { has: true }]);
}

#[test]
fn the_same_holds_for_a_window_the_desktop_forgets() {
    let mut desk = desk();
    let first = open(&mut desk, 1, b"first");
    let second = open(&mut desk, 2, b"second");
    drain(&mut desk, 1, first);
    drain(&mut desk, 2, second);
    let mut outcome = Outcome::nothing();
    desk.forget(2, &mut outcome).unwrap();
    assert_eq!(
        outcome.woken.iter().copied().collect::<Vec<_>>(),
        vec![first]
    );
    assert_eq!(drain(&mut desk, 1, first), vec![Event::Focus { has: true }]);
    assert_eq!(desk.focused().map(Window::id), Some(first));
}
