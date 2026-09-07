// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::keyboard`.

use crate::input::{KeyCode, KeyEvent};
use crate::keyboard::{Keyboard, Layout, Modifiers};

/// Types the keys and answers with what came out.
fn typed(layout: Layout, keys: &[(KeyCode, bool)]) -> String {
    let mut keyboard = Keyboard::new(layout);
    keys.iter()
        .filter_map(|(code, pressed)| {
            keyboard.feed(KeyEvent {
                code: *code,
                pressed: *pressed,
            })
        })
        .collect()
}

/// A key pressed and let go of.
fn tap(code: KeyCode) -> [(KeyCode, bool); 2] {
    [(code, true), (code, false)]
}

#[test]
fn a_word_typed_on_the_us_layout_comes_out_as_that_word() {
    let mut keys = Vec::new();
    for code in [KeyCode::H, KeyCode::E, KeyCode::L, KeyCode::L, KeyCode::O] {
        keys.extend_from_slice(&tap(code));
    }
    assert_eq!(typed(Layout::Us, &keys), "hello");
}

#[test]
fn the_german_layout_swaps_the_two_letters_and_carries_the_umlauts() {
    assert_eq!(typed(Layout::De, &tap(KeyCode::Z)), "y");
    assert_eq!(typed(Layout::De, &tap(KeyCode::Y)), "z");
    assert_eq!(typed(Layout::Us, &tap(KeyCode::Z)), "z");
    assert_eq!(typed(Layout::De, &tap(KeyCode::Semicolon)), "ö");
    assert_eq!(typed(Layout::De, &tap(KeyCode::Quote)), "ä");
    assert_eq!(typed(Layout::De, &tap(KeyCode::LeftBracket)), "ü");
    assert_eq!(typed(Layout::De, &tap(KeyCode::Minus)), "ß");
    assert_eq!(typed(Layout::Us, &tap(KeyCode::Minus)), "-");
}

#[test]
fn a_key_held_with_shift_types_the_other_character_of_its_pair() {
    let mut keys = vec![(KeyCode::LeftShift, true)];
    keys.extend_from_slice(&tap(KeyCode::A));
    keys.extend_from_slice(&tap(KeyCode::Digit1));
    keys.push((KeyCode::LeftShift, false));
    keys.extend_from_slice(&tap(KeyCode::A));
    assert_eq!(typed(Layout::Us, &keys), "A!a");

    let mut keys = vec![(KeyCode::RightShift, true)];
    keys.extend_from_slice(&tap(KeyCode::Digit7));
    keys.push((KeyCode::RightShift, false));
    assert_eq!(typed(Layout::De, &keys), "/", "the German seven is a slash");
}

#[test]
fn the_lock_turns_over_on_the_press_and_changes_the_letters_only() {
    let mut keys = vec![(KeyCode::CapsLock, true), (KeyCode::CapsLock, false)];
    keys.extend_from_slice(&tap(KeyCode::A));
    keys.extend_from_slice(&tap(KeyCode::Digit1));
    keys.push((KeyCode::LeftShift, true));
    keys.extend_from_slice(&tap(KeyCode::A));
    keys.push((KeyCode::LeftShift, false));
    keys.extend_from_slice(&tap(KeyCode::CapsLock));
    keys.extend_from_slice(&tap(KeyCode::A));
    assert_eq!(
        typed(Layout::Us, &keys),
        "A1aa",
        "the lock is the shift of the letters and shift takes it back"
    );
}

#[test]
fn the_modifier_state_follows_press_and_release() {
    let mut keyboard = Keyboard::new(Layout::Us);
    assert_eq!(keyboard.modifiers(), Modifiers::default());
    assert_eq!(keyboard.layout(), Layout::Us);
    for (code, held) in [
        (KeyCode::LeftShift, "shift"),
        (KeyCode::LeftControl, "control"),
        (KeyCode::LeftAlt, "alt"),
        (KeyCode::RightAlt, "alt_graph"),
        (KeyCode::LeftMeta, "meta"),
    ] {
        assert_eq!(
            keyboard.feed(KeyEvent {
                code,
                pressed: true
            }),
            None,
            "a modifier types nothing"
        );
        let modifiers = keyboard.modifiers();
        let set = match held {
            "shift" => modifiers.shift,
            "control" => modifiers.control,
            "alt" => modifiers.alt,
            "alt_graph" => modifiers.alt_graph,
            _other => modifiers.meta,
        };
        assert!(set, "{held} is held");
        keyboard.feed(KeyEvent {
            code,
            pressed: false,
        });
    }
    assert_eq!(
        keyboard.modifiers(),
        Modifiers::default(),
        "and nothing is held afterwards"
    );
}

#[test]
fn a_release_without_a_press_is_no_event_and_leaves_the_state_where_it_was() {
    let mut keyboard = Keyboard::new(Layout::Us);
    assert_eq!(
        keyboard.feed(KeyEvent {
            code: KeyCode::A,
            pressed: false
        }),
        None,
        "a release types nothing"
    );
    assert_eq!(
        keyboard.feed(KeyEvent {
            code: KeyCode::RightShift,
            pressed: false
        }),
        None
    );
    assert_eq!(keyboard.modifiers(), Modifiers::default());
}

#[test]
fn a_key_that_stands_for_no_character_answers_nothing() {
    let keyboard = Keyboard::new(Layout::Us);
    for code in [
        KeyCode::Escape,
        KeyCode::F5,
        KeyCode::ArrowUp,
        KeyCode::Pause,
        KeyCode::Insert,
        KeyCode::NumLock,
        KeyCode::PrintScreen,
    ] {
        assert_eq!(keyboard.character(code), None, "{}", code.name());
    }
    assert_eq!(keyboard.character(KeyCode::Space), Some(' '));
    assert_eq!(keyboard.character(KeyCode::Enter), Some('\n'));
    assert_eq!(keyboard.character(KeyCode::Tab), Some('\t'));
    assert_eq!(keyboard.character(KeyCode::NumpadEnter), Some('\n'));
    assert_eq!(keyboard.character(KeyCode::Numpad7), Some('7'));
}

#[test]
fn both_layouts_name_themselves_and_carry_the_same_keys() {
    assert_eq!(Layout::ALL.len(), 2);
    assert_eq!(Layout::default(), Layout::Us);
    let us: Vec<KeyCode> = Layout::Us
        .table()
        .iter()
        .map(|(code, _low, _high)| *code)
        .collect();
    let de: Vec<KeyCode> = Layout::De
        .table()
        .iter()
        .map(|(code, _low, _high)| *code)
        .collect();
    assert_eq!(us, de, "the layouts differ in the characters, not the keys");
    for layout in Layout::ALL {
        assert!(!layout.name().is_empty());
        let mut sorted = layout
            .table()
            .iter()
            .map(|(code, _low, _high)| *code)
            .collect::<Vec<_>>();
        let count = sorted.len();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), count, "{} names a key twice", layout.name());
    }
}
