// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::screen`.

use crate::screen::{COLUMNS, REPLACEMENT, ROWS, Screen};

#[test]
fn a_new_scrollback_holds_nothing() {
    let screen = Screen::new();
    assert!(screen.is_empty());
    assert_eq!(screen.len(), 0);
    assert_eq!(screen.row(0), "");
    assert_eq!(Screen::default().len(), 0);
}

#[test]
fn what_is_printed_stands_in_the_order_it_was_printed() {
    let mut screen = Screen::new();
    screen.print("first\nsecond\n");
    assert_eq!(screen.len(), 2);
    assert_eq!(screen.row(0), "first");
    assert_eq!(screen.row(1), "second");
    assert_eq!(screen.row(2), "");
}

#[test]
fn a_print_without_a_line_break_is_a_row_of_its_own() {
    let mut screen = Screen::new();
    screen.print("no break");
    assert_eq!(screen.len(), 1);
    assert_eq!(screen.row(0), "no break");
}

#[test]
fn an_empty_line_is_a_row() {
    let mut screen = Screen::new();
    screen.print("\n\n");
    assert_eq!(screen.len(), 2);
    assert_eq!(screen.row(0), "");
}

#[test]
fn a_row_longer_than_the_window_continues_on_the_next() {
    let mut screen = Screen::new();
    let long = "x".repeat(COLUMNS + 3);
    screen.print(&long);
    assert_eq!(screen.len(), 2);
    assert_eq!(screen.row(0).len(), COLUMNS);
    assert_eq!(screen.row(1), "xxx");
}

#[test]
fn a_byte_the_font_has_no_glyph_for_is_written_as_a_full_stop() {
    let mut screen = Screen::new();
    screen.print("a\tb\u{7f}c");
    assert_eq!(
        screen.row(0),
        format!("a{}b{}c", char::from(REPLACEMENT), char::from(REPLACEMENT))
    );
}

#[test]
fn the_oldest_row_drops_off_the_top_when_the_scrollback_is_full() {
    let mut screen = Screen::new();
    for index in 0..ROWS + 2 {
        screen.print(&format!("row {index}\n"));
    }
    assert_eq!(screen.len(), ROWS);
    assert_eq!(screen.row(0), "row 2");
    assert_eq!(screen.row(ROWS - 1), &format!("row {}", ROWS + 1));
    assert_eq!(screen.row(ROWS), "");
}

#[test]
fn a_cleared_scrollback_holds_nothing_again() {
    let mut screen = Screen::new();
    screen.print("something\n");
    screen.clear();
    assert!(screen.is_empty());
    assert_eq!(screen.row(0), "");
}
