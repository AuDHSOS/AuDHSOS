// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::device`.

use crate::controller::{Controller, Error, WRITE_AUX};
use crate::device::{
    ACK, ENABLE_REPORTING, GET_ID, MAX_RESENDS, RESEND, RESET, RESET_PASSED, SCANCODE_SET_2,
    SET_SAMPLE_RATE, SET_SCANCODE_SET, WHEEL_ID, WHEEL_KNOCK, keyboard_command, mouse_command,
    start_keyboard, start_mouse,
};
use crate::doubles::ScriptedPorts;

/// A controller whose device answers `answers`, in order.
fn answering(answers: &[u8]) -> Controller<ScriptedPorts> {
    Controller::new(ScriptedPorts::with(answers))
}

#[test]
fn a_keyboard_that_answers_is_reset_set_to_set_two_and_told_to_report() {
    let mut controller = answering(&[ACK, RESET_PASSED, ACK, ACK, ACK]);
    assert_eq!(start_keyboard(&mut controller), Ok(()));
    assert_eq!(
        controller.ports().written(),
        vec![RESET, SET_SCANCODE_SET, SCANCODE_SET_2, ENABLE_REPORTING]
    );
    assert!(
        controller.ports().commands().is_empty(),
        "a byte for the keyboard needs no command before it"
    );
}

#[test]
fn a_keyboard_whose_self_test_fails_stops_the_sequence_there() {
    let mut controller = answering(&[ACK, 0xFC, ACK, ACK, ACK]);
    assert_eq!(
        start_keyboard(&mut controller),
        Err(Error::NotAcknowledged(0xFC))
    );
    assert_eq!(
        controller.ports().written(),
        vec![RESET],
        "nothing is sent after the reset that did not work"
    );
}

#[test]
fn a_device_that_asks_for_the_byte_again_gets_it_a_bounded_number_of_times() {
    let mut controller = answering(&[RESEND, RESEND, ACK]);
    assert_eq!(keyboard_command(&mut controller, ENABLE_REPORTING), Ok(()));
    assert_eq!(
        controller.ports().written(),
        vec![ENABLE_REPORTING; 3],
        "the byte went out once for every answer"
    );

    let asks: Vec<u8> = core::iter::repeat_n(
        RESEND,
        usize::try_from(MAX_RESENDS).unwrap_or(0).saturating_add(1),
    )
    .collect();
    let mut controller = answering(&asks);
    assert_eq!(
        keyboard_command(&mut controller, ENABLE_REPORTING),
        Err(Error::NotAcknowledged(RESEND)),
        "a device that never takes the byte is given up on"
    );
}

#[test]
fn a_mouse_hears_the_knock_and_answers_with_its_wheel() {
    let mut answers = vec![ACK, RESET_PASSED, 0x00];
    answers.extend_from_slice(&[ACK; 6]);
    answers.extend_from_slice(&[ACK, WHEEL_ID, ACK]);
    let mut controller = answering(&answers);
    assert_eq!(start_mouse(&mut controller), Ok(WHEEL_ID));

    let written = controller.ports().written();
    assert_eq!(
        written,
        vec![
            RESET,
            SET_SAMPLE_RATE,
            WHEEL_KNOCK[0],
            SET_SAMPLE_RATE,
            WHEEL_KNOCK[1],
            SET_SAMPLE_RATE,
            WHEEL_KNOCK[2],
            GET_ID,
            ENABLE_REPORTING,
        ]
    );
    assert_eq!(
        controller.ports().commands(),
        vec![WRITE_AUX; written.len()],
        "every byte for the mouse goes out behind the command that redirects it"
    );
}

#[test]
fn a_mouse_that_does_not_answer_its_reset_is_no_mouse() {
    let mut controller = answering(&[0x00]);
    assert_eq!(
        start_mouse(&mut controller),
        Err(Error::NotAcknowledged(0x00))
    );
}

#[test]
fn a_mouse_command_on_a_controller_that_stands_still_times_out() {
    let mut ports = ScriptedPorts::new();
    ports.fixed_status(0);
    let mut controller = Controller::new(ports);
    assert_eq!(
        mouse_command(&mut controller, GET_ID),
        Err(Error::Timeout),
        "the byte goes out and no answer ever comes"
    );
}
