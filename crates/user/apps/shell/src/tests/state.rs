// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::state`.

use gfx::draw::{Command, LIST_CAPACITY, List};
use user_proto::input::KeyCode;
use user_proto::window::Event;

use crate::screen::ROWS;
use crate::state::{BACKGROUND, HEIGHT, INK, MAX_INPUT, PROMPT_INK, Paint, Shell, Step, WIDTH};
use user_proto::keyboard::Layout;

/// One key going down and coming up again.
fn tap(shell: &mut Shell, code: KeyCode) -> Step {
    let step = shell.feed(Event::Key {
        code,
        pressed: true,
    });
    shell.feed(Event::Key {
        code,
        pressed: false,
    });
    step
}

/// Types every character of `text`, which is letters and spaces.
fn type_in(shell: &mut Shell, text: &str) {
    for character in text.chars() {
        let code = code_of(character);
        tap(shell, code);
    }
}

/// The key that types `character` on the United States layout.
fn code_of(character: char) -> KeyCode {
    match character {
        ' ' => KeyCode::Space,
        '.' => KeyCode::Period,
        '/' => KeyCode::Slash,
        ':' => KeyCode::Semicolon,
        '0' => KeyCode::Digit0,
        '1' => KeyCode::Digit1,
        '2' => KeyCode::Digit2,
        other => KeyCode::ALL
            .iter()
            .copied()
            .find(|code| code.name().eq_ignore_ascii_case(&other.to_string()))
            .unwrap(),
    }
}

/// Every command of a full render, over as many lists as it takes.
fn rendered(shell: &Shell) -> Vec<Command> {
    let mut commands = Vec::new();
    let mut row = Some(0);
    while let Some(from) = row {
        let mut list = List::new();
        row = shell.render(from, &mut list);
        commands.extend(list.iter().copied());
        assert!(commands.len() < 200, "a render that never finishes");
    }
    commands
}

/// The text of every text command of `commands`.
fn texts(commands: &[Command]) -> Vec<String> {
    commands
        .iter()
        .filter_map(|command| match command {
            Command::Text { text, .. } => Some(text.as_str().to_owned()),
            _other => None,
        })
        .collect()
}

#[test]
fn a_new_shell_has_everything_to_paint_and_nothing_to_run() {
    let mut shell = Shell::new();
    assert_eq!(shell.paint(), Paint::All);
    assert_eq!(shell.input(), "");
    assert!(shell.screen().is_empty());
    assert!(!shell.has_ended());
    assert_eq!(shell.take_line(), None);
    shell.painted();
    assert_eq!(shell.paint(), Paint::Nothing);
    assert_eq!(Shell::default().paint(), Paint::All);
}

#[test]
fn what_is_typed_stands_in_the_line() {
    let mut shell = Shell::new();
    shell.painted();
    let step = tap(&mut shell, KeyCode::A);
    assert_eq!(step, Step::Painted);
    assert_eq!(shell.paint(), Paint::Input);
    type_in(&mut shell, "bc");
    assert_eq!(shell.input(), "abc");
}

#[test]
fn a_key_that_types_nothing_changes_nothing() {
    let mut shell = Shell::new();
    shell.painted();
    assert_eq!(tap(&mut shell, KeyCode::LeftShift), Step::Nothing);
    assert_eq!(shell.paint(), Paint::Nothing);
    assert_eq!(shell.input(), "");
}

#[test]
fn the_backspace_takes_the_last_character_away_and_stops_at_an_empty_line() {
    let mut shell = Shell::new();
    type_in(&mut shell, "ab");
    assert_eq!(tap(&mut shell, KeyCode::Backspace), Step::Painted);
    assert_eq!(shell.input(), "a");
    tap(&mut shell, KeyCode::Backspace);
    assert_eq!(shell.input(), "");
    shell.painted();
    assert_eq!(tap(&mut shell, KeyCode::Backspace), Step::Nothing);
    assert_eq!(shell.paint(), Paint::Nothing);
}

#[test]
fn a_line_as_long_as_the_row_takes_no_further_character() {
    let mut shell = Shell::new();
    for _ in 0..MAX_INPUT {
        tap(&mut shell, KeyCode::X);
    }
    assert_eq!(shell.input().len(), MAX_INPUT);
    shell.painted();
    assert_eq!(tap(&mut shell, KeyCode::X), Step::Nothing);
    assert_eq!(shell.input().len(), MAX_INPUT);
}

#[test]
fn what_is_entered_goes_into_the_scrollback_behind_the_prompt_and_waits() {
    let mut shell = Shell::new();
    type_in(&mut shell, "help");
    assert_eq!(tap(&mut shell, KeyCode::Enter), Step::Ready);
    assert_eq!(shell.paint(), Paint::All);
    assert_eq!(shell.input(), "");
    assert_eq!(shell.screen().row(0), "$ help");
    let line = shell.take_line().unwrap();
    assert_eq!(line.as_bytes(), b"help");
    assert_eq!(shell.take_line(), None);
}

#[test]
fn an_empty_line_is_entered_as_an_empty_line() {
    let mut shell = Shell::new();
    assert_eq!(tap(&mut shell, KeyCode::Enter), Step::Ready);
    assert_eq!(shell.screen().row(0), "$ ");
    assert_eq!(shell.take_line().unwrap().as_bytes(), b"");
}

#[test]
fn what_the_program_prints_stands_above_the_line_being_typed() {
    let mut shell = Shell::new();
    shell.painted();
    shell.print("one\ntwo\n");
    assert_eq!(shell.paint(), Paint::All);
    assert_eq!(shell.screen().row(0), "one");
    assert_eq!(shell.screen().row(1), "two");
}

#[test]
fn a_cleared_shell_holds_no_row() {
    let mut shell = Shell::new();
    shell.print("something\n");
    shell.clear();
    assert!(shell.screen().is_empty());
    assert_eq!(shell.paint(), Paint::All);
}

#[test]
fn the_pointer_changes_nothing_and_the_focus_paints_the_caret() {
    let mut shell = Shell::new();
    shell.painted();
    assert_eq!(
        shell.feed(Event::Pointer {
            x: 1,
            y: 2,
            buttons: 0,
        }),
        Step::Nothing
    );
    assert_eq!(shell.paint(), Paint::Nothing);
    assert_eq!(shell.feed(Event::Focus { has: true }), Step::Painted);
    assert_eq!(shell.paint(), Paint::Input);
}

#[test]
fn the_window_going_away_ends_the_shell() {
    let mut shell = Shell::new();
    assert_eq!(shell.feed(Event::Closed), Step::Ended);
    assert!(shell.has_ended());
}

#[test]
fn a_render_clears_the_window_and_writes_every_row_that_holds_text() {
    let mut shell = Shell::new();
    shell.print("one\ntwo\n");
    let commands = rendered(&shell);
    assert_eq!(
        commands.first(),
        Some(&Command::Clear { color: BACKGROUND })
    );
    assert_eq!(texts(&commands), vec!["one", "two", "$ "]);
}

#[test]
fn the_line_being_typed_carries_the_caret_while_the_window_has_the_focus() {
    let mut shell = Shell::new();
    shell.feed(Event::Focus { has: true });
    type_in(&mut shell, "ab");
    let commands = rendered(&shell);
    assert_eq!(texts(&commands).last().map(String::as_str), Some("$ ab_"));
    shell.feed(Event::Focus { has: false });
    let commands = rendered(&shell);
    assert_eq!(texts(&commands).last().map(String::as_str), Some("$ ab"));
}

#[test]
fn every_row_is_written_where_its_glyphs_stand() {
    let mut shell = Shell::new();
    shell.print("one\n");
    let commands = rendered(&shell);
    let rows: Vec<(u32, gfx::Color)> = commands
        .iter()
        .filter_map(|command| match command {
            Command::Text { y, color, .. } => Some((*y, *color)),
            _other => None,
        })
        .collect();
    assert_eq!(rows.first(), Some(&(0, INK)));
    assert_eq!(
        rows.last(),
        Some(&(u32::try_from(ROWS).unwrap() * 16, PROMPT_INK))
    );
}

#[test]
fn a_render_of_a_full_scrollback_takes_more_than_one_list() {
    let mut shell = Shell::new();
    for index in 0..ROWS {
        shell.print(&format!("row {index}\n"));
    }
    let mut list = List::new();
    let next = shell.render(0, &mut list).unwrap();
    assert!(next < ROWS);
    assert_eq!(list.len(), LIST_CAPACITY);
    let commands = rendered(&shell);
    assert_eq!(texts(&commands).len(), ROWS + 1);
}

#[test]
fn a_list_that_is_already_full_takes_no_row_and_says_where_to_go_on() {
    let shell = Shell::new();
    let mut list = List::new();
    for _ in 0..LIST_CAPACITY {
        list.push(Command::Clear { color: BACKGROUND });
    }
    assert_eq!(shell.render(0, &mut list), Some(0));
    assert_eq!(shell.render(ROWS, &mut list), Some(ROWS));
    assert!(!shell.render_input(&mut list));
}

#[test]
fn the_window_holds_one_glyph_per_column_and_per_row() {
    assert_eq!(WIDTH, 56 * 8);
    assert_eq!(HEIGHT, 23 * 16);
}

#[test]
fn a_key_that_types_a_control_character_types_nothing() {
    let mut shell = Shell::new();
    shell.painted();
    assert_eq!(tap(&mut shell, KeyCode::Tab), Step::Nothing);
    assert_eq!(shell.input(), "");
    assert_eq!(shell.paint(), Paint::Nothing);
}

#[test]
fn a_character_the_font_cannot_draw_is_not_taken() {
    let mut shell = Shell::with_layout(Layout::De);
    shell.painted();
    // The key that types `;` on the United States layout types `ö` on the
    // German one, and this system draws the printable characters of ASCII
    // and nothing else.
    assert_eq!(tap(&mut shell, KeyCode::Semicolon), Step::Nothing);
    assert_eq!(shell.input(), "");
    assert_eq!(tap(&mut shell, KeyCode::A), Step::Painted);
    assert_eq!(shell.input(), "a");
}
