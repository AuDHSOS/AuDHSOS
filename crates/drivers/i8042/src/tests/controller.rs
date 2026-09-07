// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::controller`.

use crate::controller::{
    AUX, CONFIG_AUX_INTERRUPT, CONFIG_KBD_INTERRUPT, CONFIG_TRANSLATE, Controller, DISABLE_AUX,
    DISABLE_KBD, Devices, ENABLE_AUX, ENABLE_KBD, Error, INPUT_FULL, OUTPUT_FULL, READ_CONFIG,
    SELF_TEST, SELF_TEST_PASSED, TEST_AUX, TEST_KBD, WRITE_AUX, WRITE_CONFIG,
};
use crate::device::{
    ACK, ENABLE_REPORTING, GET_ID, RESET, RESET_PASSED, SET_SAMPLE_RATE, WHEEL_ID,
};
use crate::doubles::ScriptedPorts;

/// What the firmware left in the configuration byte: translation on and
/// both interrupts on, which is what the driver has to turn off.
const FIRMWARE_CONFIG: u8 = CONFIG_TRANSLATE | CONFIG_KBD_INTERRUPT | CONFIG_AUX_INTERRUPT;

/// Queues what a keyboard that works answers to `start_keyboard`.
fn keyboard_answers(ports: &mut ScriptedPorts) {
    ports.answer_all(&[ACK, RESET_PASSED, ACK, ACK, ACK]);
}

/// Queues what a mouse with a wheel answers to `start_mouse`.
fn mouse_answers(ports: &mut ScriptedPorts, id: u8) {
    ports.answer_all(&[ACK, RESET_PASSED, 0x00]);
    // The three sample rates of the knock, each a command and a value.
    ports.answer_all(&[ACK; 6]);
    ports.answer_all(&[ACK, id, ACK]);
}

/// A controller on which everything works, with `leftover` standing in its
/// output buffer the way the firmware leaves it.
fn healthy(leftover: &[u8], self_test: u8, kbd_test: u8, aux_test: u8) -> ScriptedPorts {
    let mut ports = ScriptedPorts::new();
    ports.push_all(leftover);
    ports.answer_all(&[FIRMWARE_CONFIG, self_test]);
    if self_test != SELF_TEST_PASSED {
        return ports;
    }
    ports.answer_all(&[kbd_test, aux_test]);
    if kbd_test == 0 {
        keyboard_answers(&mut ports);
    }
    if aux_test == 0 {
        mouse_answers(&mut ports, WHEEL_ID);
    }
    ports
}

#[test]
fn a_controller_that_works_finds_both_devices_and_the_wheel_of_the_mouse() {
    let mut controller = Controller::new(healthy(&[], SELF_TEST_PASSED, 0, 0));
    assert_eq!(
        controller.init(),
        Ok(Devices {
            keyboard: true,
            mouse: true,
            mouse_id: WHEEL_ID,
        })
    );
    assert_eq!(
        controller.ports().remaining(),
        0,
        "every answer of the script was read"
    );
}

#[test]
fn the_output_buffer_is_flushed_before_the_self_test() {
    let mut controller = Controller::new(healthy(&[0xAA, 0x55, 0x12], SELF_TEST_PASSED, 0, 0));
    assert!(controller.init().is_ok());
    let commands = controller.ports().commands();
    let flushed = controller
        .ports()
        .log()
        .iter()
        .take_while(|access| !matches!(access, crate::doubles::Access::WriteCommand(SELF_TEST)))
        .filter(|access| matches!(access, crate::doubles::Access::Data(_)))
        .count();
    assert!(
        flushed >= 3,
        "the three bytes the firmware left were read before the self-test, not {flushed}"
    );
    let self_test = commands.iter().position(|byte| *byte == SELF_TEST);
    let read_config = commands.iter().position(|byte| *byte == READ_CONFIG);
    assert!(read_config < self_test, "the configuration is read first");
}

#[test]
fn translation_and_both_interrupts_are_off_while_the_devices_come_up() {
    let mut controller = Controller::new(healthy(&[], SELF_TEST_PASSED, 0, 0));
    assert!(controller.init().is_ok());
    let written = controller.ports().written();
    let first = written.first().copied().unwrap_or(0xFF);
    assert_eq!(
        first & (CONFIG_TRANSLATE | CONFIG_KBD_INTERRUPT | CONFIG_AUX_INTERRUPT),
        0,
        "the first configuration byte written turns translation and both interrupts off"
    );
}

#[test]
fn both_interrupts_are_enabled_only_after_both_devices_are_initialized() {
    let mut controller = Controller::new(healthy(&[], SELF_TEST_PASSED, 0, 0));
    assert!(controller.init().is_ok());
    let log = controller.ports().log().to_vec();
    let last_config = log
        .iter()
        .rposition(|access| matches!(access, crate::doubles::Access::WriteCommand(WRITE_CONFIG)))
        .unwrap();
    let started = log
        .iter()
        .rposition(|access| matches!(access, crate::doubles::Access::WriteData(ENABLE_REPORTING)))
        .unwrap();
    assert!(
        started < last_config,
        "the last configuration byte is written after the last device command"
    );
    let running = controller.ports().written().last().copied().unwrap_or(0);
    assert_eq!(
        running & (CONFIG_KBD_INTERRUPT | CONFIG_AUX_INTERRUPT),
        CONFIG_KBD_INTERRUPT | CONFIG_AUX_INTERRUPT,
        "and it turns both interrupts on"
    );
    assert_eq!(running & CONFIG_TRANSLATE, 0, "and leaves translation off");
}

#[test]
fn a_failed_self_test_is_an_error_and_nothing_is_brought_up() {
    let mut controller = Controller::new(healthy(&[], 0xFC, 0, 0));
    assert_eq!(controller.init(), Err(Error::SelfTest(0xFC)));
    assert!(
        !controller.ports().commands().contains(&ENABLE_KBD),
        "no port is enabled after a controller that failed its own test"
    );
}

#[test]
fn a_missing_keyboard_is_reported_and_the_mouse_still_works() {
    let mut controller = Controller::new(healthy(&[], SELF_TEST_PASSED, 0x01, 0));
    assert_eq!(
        controller.init(),
        Ok(Devices {
            keyboard: false,
            mouse: true,
            mouse_id: WHEEL_ID,
        })
    );
    let running = controller.ports().written().last().copied().unwrap_or(0);
    assert_eq!(
        running & (CONFIG_KBD_INTERRUPT | CONFIG_AUX_INTERRUPT),
        CONFIG_AUX_INTERRUPT,
        "only the line of the device that answered is armed"
    );
}

#[test]
fn a_missing_mouse_is_reported_and_the_keyboard_still_works() {
    let mut controller = Controller::new(healthy(&[], SELF_TEST_PASSED, 0, 0x02));
    assert_eq!(
        controller.init(),
        Ok(Devices {
            keyboard: true,
            mouse: false,
            mouse_id: 0,
        })
    );
    assert!(
        !controller.ports().commands().contains(&ENABLE_AUX),
        "the port of a device that is not there is not enabled"
    );
}

#[test]
fn a_keyboard_that_does_not_acknowledge_is_reported_as_absent() {
    let mut ports = ScriptedPorts::new();
    ports.answer_all(&[FIRMWARE_CONFIG, SELF_TEST_PASSED, 0x00, 0x00]);
    // The reset of the keyboard is answered with nonsense, and the mouse
    // answers everything.
    ports.answer(0x11);
    mouse_answers(&mut ports, 0x00);
    let mut controller = Controller::new(ports);
    assert_eq!(
        controller.init(),
        Ok(Devices {
            keyboard: false,
            mouse: true,
            mouse_id: 0,
        })
    );
}

#[test]
fn a_mouse_without_a_wheel_keeps_the_identifier_it_gave() {
    let mut ports = ScriptedPorts::new();
    ports.answer_all(&[FIRMWARE_CONFIG, SELF_TEST_PASSED, 0x00, 0x00]);
    keyboard_answers(&mut ports);
    mouse_answers(&mut ports, 0x00);
    let mut controller = Controller::new(ports);
    assert_eq!(controller.init().map(|found| found.mouse_id), Ok(0x00));
}

#[test]
fn an_output_buffer_that_never_fills_runs_into_the_poll_limit() {
    let mut ports = ScriptedPorts::new();
    ports.fixed_status(0);
    let mut controller = Controller::new(ports);
    assert_eq!(controller.read(), Err(Error::Timeout));
    assert_eq!(controller.init(), Err(Error::Timeout));
}

#[test]
fn an_input_buffer_that_never_empties_runs_into_the_poll_limit() {
    let mut ports = ScriptedPorts::new();
    ports.fixed_status(INPUT_FULL);
    let mut controller = Controller::new(ports);
    assert_eq!(controller.command(SELF_TEST), Err(Error::Timeout));
    assert_eq!(controller.write(0), Err(Error::Timeout));
    assert_eq!(controller.write_aux(0), Err(Error::Timeout));
    assert_eq!(controller.init(), Err(Error::Timeout));
    assert!(
        controller.ports().commands().is_empty(),
        "a controller that never takes a byte is never written to"
    );
}

#[test]
fn the_aux_bit_of_the_status_says_which_device_sent_the_byte() {
    let mut ports = ScriptedPorts::new();
    ports.push(0x1C);
    ports.push_aux(0x08);
    let mut controller = Controller::new(ports);
    assert_eq!(controller.take(), Some((0x1C, false)));
    assert_eq!(controller.take(), Some((0x08, true)));
}

#[test]
fn a_read_with_the_output_buffer_empty_answers_nothing() {
    let mut controller = Controller::new(ScriptedPorts::new());
    assert_eq!(controller.take(), None);
    assert_eq!(
        controller.ports().reads(),
        0,
        "and the data register was never read"
    );
}

#[test]
fn a_flush_of_a_controller_that_stands_still_stops_at_the_poll_limit() {
    let mut ports = ScriptedPorts::new();
    ports.fixed_status(OUTPUT_FULL | AUX);
    let mut controller = Controller::new(ports);
    controller.flush();
    let reads = controller.ports().reads();
    assert!(
        u32::try_from(reads).unwrap_or(u32::MAX) >= crate::controller::MAX_POLLS,
        "the flush ran to its bound and stopped, not {reads} reads"
    );
}

#[test]
fn a_byte_for_the_mouse_goes_out_behind_the_command_that_redirects_it() {
    let mut controller = Controller::new(ScriptedPorts::new());
    assert!(controller.write_aux(GET_ID).is_ok());
    assert_eq!(controller.ports().commands(), vec![WRITE_AUX]);
    assert_eq!(controller.ports().written(), vec![GET_ID]);
}

#[test]
fn the_ports_are_disabled_before_anything_else_is_asked_of_the_controller() {
    let mut controller = Controller::new(healthy(&[], SELF_TEST_PASSED, 0, 0));
    assert!(controller.init().is_ok());
    let commands = controller.ports().commands();
    assert_eq!(
        commands.first().copied(),
        Some(DISABLE_KBD),
        "the keyboard port goes off first"
    );
    assert_eq!(commands.get(1).copied(), Some(DISABLE_AUX));
    assert!(
        commands.contains(&TEST_KBD) && commands.contains(&TEST_AUX),
        "and both ports are tested"
    );
}

#[test]
fn the_ports_are_given_back_and_a_controller_is_told_apart_by_its_error() {
    let controller = Controller::new(ScriptedPorts::with(&[0x76]));
    let mut ports = controller.into_ports();
    assert_eq!(ports.remaining(), 1);
    ports.clear();
    assert!(ports.log().is_empty());
    assert_eq!(
        format!("{}", Error::Timeout),
        "the controller did not become ready"
    );
    assert_eq!(
        format!("{}", Error::SelfTest(0xFC)),
        "the self-test answered 0xfc and not 0x55"
    );
    assert_eq!(
        format!("{}", Error::NotAcknowledged(0xFE)),
        "a device answered 0xfe and not an acknowledgement"
    );
}

#[test]
fn the_sample_rate_knock_and_the_identifier_question_are_both_sent_to_the_mouse() {
    let mut controller = Controller::new(healthy(&[], SELF_TEST_PASSED, 0, 0));
    assert!(controller.init().is_ok());
    let written = controller.ports().written();
    let knock: Vec<u8> = written
        .iter()
        .copied()
        .skip_while(|byte| *byte != SET_SAMPLE_RATE)
        .collect();
    assert_eq!(
        knock.first().copied(),
        Some(SET_SAMPLE_RATE),
        "the knock begins with the sample rate command"
    );
    assert!(
        knock.contains(&200) && knock.contains(&100) && knock.contains(&80),
        "the three rates of the knock are sent: {knock:?}"
    );
    assert!(knock.contains(&GET_ID), "and the identifier is asked for");
    assert!(written.contains(&RESET), "and both devices were reset");
}
