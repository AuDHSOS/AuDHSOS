// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::bar`.

use audhsos_time::CivilTime;
use gfx::{GLYPH_HEIGHT, GLYPH_WIDTH, Rect};

use crate::bar::{
    CLOCK_LEN, Clock, DESK_ENTRIES, DESK_MENU, ENTRY_HEIGHT, HEIGHT, MARGIN, PANEL_WIDTH, TITLES,
    WINDOW_MENU, cell_width, clock_rect, entry_at, entry_rect, panel_rect, rect, text_top,
    title_at, title_rect,
};

#[test]
fn the_bar_is_a_strip_across_the_top_of_the_screen() {
    assert_eq!(rect(1024), Rect::new(0, 0, 1024, HEIGHT));
}

#[test]
fn the_titles_stand_one_after_the_other_from_the_left() {
    let first = title_rect(DESK_MENU);
    let second = title_rect(WINDOW_MENU);
    assert_eq!(first.x, MARGIN);
    assert_eq!(first.w, cell_width(TITLES[DESK_MENU]));
    assert_eq!(second.x, first.right());
    assert_eq!(first.h, HEIGHT);
}

#[test]
fn a_title_the_bar_does_not_have_occupies_nothing() {
    assert_eq!(title_rect(TITLES.len()), Rect::EMPTY);
    assert_eq!(panel_rect(TITLES.len(), 2), Rect::EMPTY);
    assert_eq!(entry_rect(TITLES.len(), 2, 0), Rect::EMPTY);
}

#[test]
fn a_pixel_of_a_cell_names_its_title_and_one_beside_it_names_none() {
    let cell = title_rect(WINDOW_MENU);
    assert_eq!(title_at(cell.x, 0), Some(WINDOW_MENU));
    assert_eq!(title_at(cell.right(), 0), None);
    assert_eq!(title_at(0, 0), None);
    assert_eq!(title_at(cell.x, HEIGHT), None);
}

#[test]
fn the_clock_stands_at_the_right_end_of_the_bar() {
    let clock = clock_rect(1024);
    let text = u32::try_from(CLOCK_LEN).unwrap() * GLYPH_WIDTH;
    assert_eq!(clock.right(), 1024 - MARGIN);
    assert_eq!(clock.w, text);
    assert_eq!(clock.h, GLYPH_HEIGHT);
    assert_eq!(clock.y, text_top());
}

#[test]
fn a_panel_hangs_below_the_title_it_belongs_to() {
    let panel = panel_rect(DESK_MENU, DESK_ENTRIES.len());
    assert_eq!(panel.x, title_rect(DESK_MENU).x);
    assert_eq!(panel.y, HEIGHT);
    assert_eq!(panel.w, PANEL_WIDTH);
    assert_eq!(panel.h, ENTRY_HEIGHT * 2);
}

#[test]
fn the_entries_of_a_panel_stand_one_below_the_other() {
    let first = entry_rect(DESK_MENU, 2, 0);
    let second = entry_rect(DESK_MENU, 2, 1);
    assert_eq!(first.y, HEIGHT);
    assert_eq!(second.y, first.bottom());
    assert_eq!(entry_rect(DESK_MENU, 2, 2), Rect::EMPTY);
}

#[test]
fn a_pixel_of_a_panel_names_its_entry() {
    let second = entry_rect(DESK_MENU, 2, 1);
    assert_eq!(entry_at(DESK_MENU, 2, second.x, second.y), Some(1));
    assert_eq!(entry_at(DESK_MENU, 2, second.x, second.bottom()), None);
    assert_eq!(entry_at(DESK_MENU, 2, second.right(), second.y), None);
    assert_eq!(entry_at(DESK_MENU, 0, 0, HEIGHT), None);
}

#[test]
fn a_clock_starts_at_midnight() {
    assert_eq!(&Clock::new().text(), b"00:00:00");
    assert_eq!(Clock::default(), Clock::new());
}

#[test]
fn a_clock_shows_the_time_it_was_put_at() {
    let mut clock = Clock::new();
    assert!(clock.set(CivilTime::new(2026, 9, 16, 7, 5, 9).unwrap()));
    assert_eq!(&clock.text(), b"07:05:09");
    assert!(clock.set(CivilTime::new(2026, 9, 16, 23, 59, 59).unwrap()));
    assert_eq!(&clock.text(), b"23:59:59");
}

#[test]
fn a_clock_put_at_the_time_it_shows_did_not_move() {
    let mut clock = Clock::new();
    let noon = CivilTime::new(2026, 9, 16, 12, 0, 0).unwrap();
    assert!(clock.set(noon));
    assert!(!clock.set(noon));
}

#[test]
fn a_clock_takes_the_wall_clock_apart() {
    let mut clock = Clock::new();
    // 1970-01-01T01:02:03Z.
    assert!(clock.set_unix(3723));
    assert_eq!(&clock.text(), b"01:02:03");
}

#[test]
fn a_wall_clock_this_system_cannot_take_apart_leaves_the_clock_alone() {
    let mut clock = Clock::new();
    assert!(clock.set_unix(3723));
    assert!(!clock.set_unix(i64::MIN));
    assert_eq!(&clock.text(), b"01:02:03");
}
