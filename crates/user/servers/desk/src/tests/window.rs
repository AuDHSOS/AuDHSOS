// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::window`.

use gfx::Rect;
use user_proto::input::KeyCode;
use user_proto::window::{Event, Title};

use crate::window::{
    BORDER, CLOSE_MARGIN, CLOSE_SIZE, EVENTS, FOCUSED, PLAIN, TITLE_HEIGHT, Window,
};

/// A window of 200 by 100 content at 40, 60.
fn window() -> Window {
    Window::new(1, 7, 40, 60, 200, 100, Title::new(b"shell").unwrap())
}

#[test]
fn a_window_answers_with_what_it_was_opened_with() {
    let window = window();
    assert_eq!(window.id(), 1);
    assert_eq!(window.badge(), 7);
    assert_eq!(window.width(), 200);
    assert_eq!(window.height(), 100);
    assert_eq!(window.title().as_bytes(), b"shell");
    assert!(!window.is_focused());
    assert_eq!(window.bytes(), 200 * 100 * 4);
    assert_eq!(window.waiting(), 0);
    assert_eq!(window.dropped(), 0);
}

#[test]
fn the_frame_holds_the_title_bar_the_border_and_the_content() {
    let window = window();
    assert_eq!(window.frame(), Rect::new(40, 60, 204, 122));
    assert_eq!(window.title_bar(), Rect::new(40, 60, 204, TITLE_HEIGHT));
    assert_eq!(window.content(), Rect::new(42, 80, 200, 100));
    assert_eq!(Window::frame_width(200), 204);
    assert_eq!(Window::frame_height(100), 122);
    assert_eq!(
        window.content().right() + BORDER,
        window.frame().right(),
        "the border stands right of the content"
    );
}

#[test]
fn the_close_box_stands_at_the_right_end_of_the_title_bar() {
    let window = window();
    let close = window.close_box();
    assert_eq!(close.w, CLOSE_SIZE);
    assert_eq!(close.right(), window.frame().right() - CLOSE_MARGIN);
    assert!(window.title_bar().contains(close.x, close.y));
}

#[test]
fn a_pixel_of_the_content_is_a_pixel_of_the_client() {
    let window = window();
    assert_eq!(window.inside(42, 80), Some((0, 0)));
    assert_eq!(window.inside(50, 90), Some((8, 10)));
    assert_eq!(window.inside(241, 179), Some((199, 99)));
    assert_eq!(window.inside(41, 80), None);
    assert_eq!(window.inside(42, 79), None);
    assert_eq!(window.inside(242, 180), None);
}

#[test]
fn a_window_is_placed_with_its_whole_frame_on_the_screen_and_below_the_bar() {
    let mut window = window();
    window.place(10, 100, 640, 480, 24);
    assert_eq!(window.frame(), Rect::new(10, 100, 204, 122));
    window.place(600, 400, 640, 480, 24);
    assert_eq!(window.frame(), Rect::new(436, 358, 204, 122));
    window.place(0, 0, 640, 480, 24);
    assert_eq!(window.frame(), Rect::new(0, 24, 204, 122));
}

#[test]
fn a_screen_with_no_room_for_the_frame_puts_it_at_the_corner() {
    let mut window = window();
    window.place(100, 100, 100, 100, 24);
    assert_eq!(window.frame().x, 0);
    assert_eq!(window.frame().y, 24);
}

#[test]
fn the_frame_of_the_window_in_front_is_drawn_in_another_color() {
    let mut window = window();
    assert_eq!(window.edge(), PLAIN);
    assert!(window.focus(true));
    assert_eq!(window.edge(), FOCUSED);
    assert!(!window.focus(true));
    assert!(window.focus(false));
}

#[test]
fn events_come_out_in_the_order_they_went_in() {
    let mut window = window();
    window.push(Event::Focus { has: true });
    window.push(Event::Key {
        code: KeyCode::A,
        pressed: true,
    });
    assert_eq!(window.waiting(), 2);
    assert_eq!(window.pop(), Some(Event::Focus { has: true }));
    assert_eq!(
        window.pop(),
        Some(Event::Key {
            code: KeyCode::A,
            pressed: true,
        })
    );
    assert_eq!(window.pop(), None);
}

#[test]
fn a_full_queue_drops_the_oldest_event_and_counts_it() {
    let mut window = window();
    for index in 0..EVENTS {
        window.push(Event::Pointer {
            x: u32::try_from(index).unwrap(),
            y: 0,
            buttons: 0,
        });
    }
    assert_eq!(window.waiting(), EVENTS);
    window.push(Event::Closed);
    assert_eq!(window.waiting(), EVENTS);
    assert_eq!(window.dropped(), 1);
    assert_eq!(
        window.pop(),
        Some(Event::Pointer {
            x: 1,
            y: 0,
            buttons: 0,
        })
    );
}
