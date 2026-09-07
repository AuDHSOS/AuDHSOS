// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::startup`.

use crate::ipc_buffer::{Buffer, BufferMut, KERNEL_LABEL_BASE, SIZE, WORD, WORDS};
use crate::layout::MAX_MESSAGE_WORDS;
use crate::startup::{Given, Role, STARTUP_LABEL, Screen, StartupError, Writer, read};
use crate::{Error, FramebufferFormat, Handle};

/// A buffer of zeros to work on.
fn buffer() -> [u8; SIZE] {
    [0; SIZE]
}

/// The handle with `index` and generation one, which is a handle every
/// table could hand out.
fn handle(index: u32) -> Handle {
    Handle::new(index, 1).unwrap()
}

/// Writes `pairs` into `bytes` and finishes the message.
fn write(bytes: &mut [u8; SIZE], pairs: &[(Role, Handle)]) -> Writer {
    let mut buffer = BufferMut::new(bytes);
    let mut writer = Writer::new();
    for (role, handle) in pairs {
        writer.give(&mut buffer, *role, *handle).unwrap();
    }
    writer.finish(&mut buffer).unwrap();
    writer
}

/// Every pair of the message in `bytes`.
fn pairs(bytes: &[u8; SIZE]) -> Result<Vec<Given>, StartupError> {
    let view = Buffer::new(bytes);
    read(view).map(Iterator::collect)
}

/// Writes the word at `index` of the message area past the writer.
fn write_raw_word(bytes: &mut [u8; SIZE], index: usize, value: u64) {
    let offset = WORDS.checked_add(index.checked_mul(WORD).unwrap()).unwrap();
    let end = offset.checked_add(WORD).unwrap();
    bytes
        .get_mut(offset..end)
        .unwrap()
        .copy_from_slice(&value.to_le_bytes());
}

#[test]
fn the_label_spells_startup_and_is_no_kernel_label() {
    assert_eq!(STARTUP_LABEL.to_be_bytes(), *b"STARTUP\0");
    // The bound itself is a const assertion in the module; this checks
    // that the constant the module asserts about is the one read here.
    assert!(STARTUP_LABEL.checked_add(1).unwrap() < KERNEL_LABEL_BASE);
}

#[test]
fn every_role_has_a_unique_code_and_a_name() {
    for (index, role) in Role::ALL.iter().enumerate() {
        assert_ne!(role.code(), 0, "{} has code zero", role.name());
        assert_eq!(Role::from_code(role.code()), Some(*role));
        assert!(!role.name().is_empty());
        for other in Role::ALL.iter().skip(index.wrapping_add(1)) {
            assert_ne!(
                role.code(),
                other.code(),
                "{} and {}",
                role.name(),
                other.name()
            );
        }
    }
}

#[test]
fn an_unknown_code_names_no_role() {
    let highest = Role::ALL.iter().map(|role| role.code()).max().unwrap();
    assert_eq!(Role::from_code(0), None);
    assert_eq!(Role::from_code(highest.checked_add(1).unwrap()), None);
    assert_eq!(Role::from_code(u32::MAX), None);
}

#[test]
fn a_message_without_pairs_reads_as_no_pairs() {
    let mut bytes = buffer();
    let writer = write(&mut bytes, &[]);
    assert!(writer.is_empty());
    assert_eq!(writer.len(), 0);
    assert_eq!(pairs(&bytes).unwrap(), Vec::new());
}

#[test]
fn the_pairs_come_back_in_the_order_they_were_given() {
    let mut bytes = buffer();
    let given = [
        (Role::OwnProcess, handle(1)),
        (Role::NameServer, handle(2)),
        (Role::MemoryServer, handle(3)),
    ];
    let writer = write(&mut bytes, &given);
    assert_eq!(writer.len(), 3);
    let read = pairs(&bytes).unwrap();
    assert_eq!(read.len(), 3);
    for (slot, (role, handle)) in read.iter().zip(given) {
        assert_eq!(slot.role, role);
        assert_eq!(slot.handle(), Some(handle));
    }
}

#[test]
fn one_role_may_appear_more_than_once() {
    let mut bytes = buffer();
    write(
        &mut bytes,
        &[
            (Role::Ram, handle(7)),
            (Role::Ram, handle(8)),
            (Role::Ram, handle(9)),
        ],
    );
    let read = pairs(&bytes).unwrap();
    assert_eq!(read.len(), 3);
    assert!(read.iter().all(|given| given.role == Role::Ram));
    let handles: Vec<Handle> = read.iter().filter_map(|given| given.handle()).collect();
    assert_eq!(handles, vec![handle(7), handle(8), handle(9)]);
}

#[test]
fn a_message_with_another_label_is_no_startup_message() {
    let mut bytes = buffer();
    write(&mut bytes, &[(Role::OwnProcess, handle(1))]);
    BufferMut::new(&mut bytes).set_label(STARTUP_LABEL.wrapping_add(1));
    assert_eq!(pairs(&bytes), Err(StartupError::NotStartup));
}

#[test]
fn an_odd_word_count_is_an_incomplete_pair() {
    let mut bytes = buffer();
    write(&mut bytes, &[(Role::OwnProcess, handle(1))]);
    BufferMut::new(&mut bytes).set_counts(1, 0).unwrap();
    assert_eq!(pairs(&bytes), Err(StartupError::OddWordCount(1)));
}

#[test]
fn a_word_that_names_no_role_is_refused_before_any_pair_is_handed_out() {
    let mut bytes = buffer();
    write(
        &mut bytes,
        &[(Role::OwnProcess, handle(1)), (Role::NameServer, handle(2))],
    );
    // The second pair, so that a reader that hands out the first one
    // before it checks the second would pass this test and does not.
    write_raw_word(&mut bytes, 2, 0xDEAD);
    assert_eq!(pairs(&bytes), Err(StartupError::UnknownRole(0xDEAD)));
}

#[test]
fn a_role_word_above_a_u32_names_no_role() {
    let mut bytes = buffer();
    write(&mut bytes, &[(Role::OwnProcess, handle(1))]);
    write_raw_word(&mut bytes, 0, u64::from(u32::MAX).wrapping_add(1));
    assert_eq!(
        pairs(&bytes),
        Err(StartupError::UnknownRole(
            u64::from(u32::MAX).wrapping_add(1)
        ))
    );
}

#[test]
fn a_word_that_is_no_handle_is_refused() {
    let mut bytes = buffer();
    write(&mut bytes, &[(Role::OwnProcess, handle(1))]);
    write_raw_word(&mut bytes, 1, 0);
    assert_eq!(pairs(&bytes), Err(StartupError::BadHandle(0)));
}

#[test]
fn the_widest_message_holds_half_as_many_pairs_as_the_area_holds_words() {
    let mut bytes = buffer();
    let mut buffer = BufferMut::new(&mut bytes);
    let mut writer = Writer::new();
    let capacity = MAX_MESSAGE_WORDS.wrapping_div(2);
    for index in 0..capacity {
        let handle = handle(u32::try_from(index).unwrap().wrapping_add(1));
        writer.give(&mut buffer, Role::Ram, handle).unwrap();
    }
    assert_eq!(writer.len(), capacity);
    assert_eq!(
        writer.give(&mut buffer, Role::Ram, handle(1)),
        Err(StartupError::Full)
    );
    writer.finish(&mut buffer).unwrap();
    assert_eq!(pairs(&bytes).unwrap().len(), capacity);
}

#[test]
fn a_full_message_leaves_the_count_at_what_fitted() {
    let mut bytes = buffer();
    let mut buffer = BufferMut::new(&mut bytes);
    let mut writer = Writer::new();
    let capacity = MAX_MESSAGE_WORDS.wrapping_div(2);
    for _ in 0..capacity {
        writer.give(&mut buffer, Role::Ram, handle(1)).unwrap();
    }
    let refused = writer.give(&mut buffer, Role::Log, handle(2));
    assert_eq!(refused, Err(StartupError::Full));
    assert_eq!(writer.len(), capacity);
}

#[test]
fn every_error_has_a_message_and_an_error_code() {
    let cases = [
        StartupError::NotStartup,
        StartupError::OddWordCount(3),
        StartupError::UnknownRole(9999),
        StartupError::BadHandle(0),
        StartupError::Message(crate::MessageError::TooManyWords),
        StartupError::Message(crate::MessageError::TooManyHandles),
        StartupError::Full,
    ];
    for case in cases {
        let text = format!("{case}");
        assert!(!text.is_empty(), "{case:?} has no message");
        let error = Error::from(case);
        assert_ne!(error.code(), 0);
    }
    assert_eq!(Error::from(StartupError::Full), Error::BufferTooSmall);
    assert_eq!(
        Error::from(StartupError::NotStartup),
        Error::InvalidArgument
    );
}

#[test]
fn a_header_with_too_many_words_is_reported_as_a_message_error() {
    let mut bytes = buffer();
    write(&mut bytes, &[(Role::OwnProcess, handle(1))]);
    // Past `set_counts`, which refuses the count itself.
    let offset = crate::ipc_buffer::WORD_COUNT;
    let end = offset.checked_add(WORD).unwrap();
    bytes
        .get_mut(offset..end)
        .unwrap()
        .copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(
        pairs(&bytes),
        Err(StartupError::Message(crate::MessageError::TooManyWords))
    );
}

#[test]
fn a_role_carries_a_handle_or_a_value_and_says_which() {
    let values: Vec<&str> = Role::ALL
        .iter()
        .filter(|role| role.carries_value())
        .map(|role| role.name())
        .collect();
    assert_eq!(values, vec!["FramebufferGeometry", "FramebufferLine"]);
    assert!(!Role::Framebuffer.carries_value());
}

#[test]
fn a_value_role_is_read_as_a_number_and_not_as_a_handle() {
    let mut bytes = buffer();
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = Writer::new();
    writer
        .give(&mut view, Role::Framebuffer, handle(7))
        .unwrap();
    writer
        .tell(&mut view, Role::FramebufferGeometry, 0)
        .unwrap();
    writer.finish(&mut view).unwrap();
    let read = pairs(&bytes).unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(read.first().unwrap().handle(), Some(handle(7)));
    assert_eq!(read.first().unwrap().value(), None);
    assert_eq!(read.get(1).unwrap().value(), Some(0));
    assert_eq!(read.get(1).unwrap().handle(), None);
}

#[test]
fn a_word_that_is_no_handle_is_a_value_under_a_value_role() {
    let mut bytes = buffer();
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = Writer::new();
    writer
        .tell(&mut view, Role::FramebufferLine, u64::MAX)
        .unwrap();
    writer.finish(&mut view).unwrap();
    assert_eq!(
        pairs(&bytes).unwrap().first().unwrap().value(),
        Some(u64::MAX)
    );
}

#[test]
fn a_handle_under_a_value_role_and_a_value_under_a_handle_role_are_refused() {
    let mut bytes = buffer();
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = Writer::new();
    let error = writer
        .give(&mut view, Role::FramebufferGeometry, handle(1))
        .unwrap_err();
    assert_eq!(error, StartupError::WrongKind(Role::FramebufferGeometry));
    assert!(format!("{error}").contains("a value"));
    let error = writer.tell(&mut view, Role::Framebuffer, 4).unwrap_err();
    assert_eq!(error, StartupError::WrongKind(Role::Framebuffer));
    assert!(format!("{error}").contains("a handle"));
    assert_eq!(Error::from(error), Error::InvalidArgument);
    assert!(writer.is_empty());
}

#[test]
fn the_mode_of_a_framebuffer_survives_the_two_words() {
    let screen = Screen {
        width: 1280,
        height: 800,
        stride: 1360,
        format: FramebufferFormat::Bgrx8888,
    };
    assert_eq!(
        Screen::from_words(screen.geometry(), screen.line()),
        Some(screen)
    );
    assert_eq!(screen.geometry(), (1280 << 32) | 0x0320);
    assert_eq!(screen.line(), (1360 << 32) | 0x02);
}

#[test]
fn a_mode_of_an_unknown_format_is_no_mode() {
    assert_eq!(Screen::from_words((4 << 32) | 0x04, (4 << 32) | 0x09), None);
    assert_eq!(Screen::from_words(0, 0), None);
}

#[test]
fn the_widest_mode_the_words_hold_reads_back() {
    let screen = Screen {
        width: u32::MAX,
        height: u32::MAX,
        stride: u32::MAX,
        format: FramebufferFormat::Rgbx8888,
    };
    assert_eq!(
        Screen::from_words(screen.geometry(), screen.line()),
        Some(screen)
    );
}
