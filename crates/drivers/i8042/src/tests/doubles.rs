// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::doubles`, which the tests of the controller stand on:
//! a double that answered something else would make them say nothing.

use crate::controller::{AUX, INPUT_FULL, OUTPUT_FULL, Ports as _};
use crate::doubles::{Access, ScriptedPorts};

#[test]
fn the_status_follows_the_script_and_says_which_device_the_byte_is_from() {
    let mut ports = ScriptedPorts::new();
    ports.push(0x76);
    ports.push_aux(0x08);
    assert_eq!(ports.read_status(), OUTPUT_FULL);
    assert_eq!(ports.read_data(), 0x76);
    assert_eq!(ports.read_status(), OUTPUT_FULL | AUX);
    assert_eq!(ports.read_data(), 0x08);
    assert_eq!(ports.read_status(), 0, "and then the buffer is empty");
    assert_eq!(ports.read_data(), 0, "a read of an empty buffer is zero");
    assert_eq!(ports.remaining(), 0);
}

#[test]
fn an_answer_stays_out_of_the_buffer_until_the_controller_has_been_asked() {
    let mut ports = ScriptedPorts::new();
    ports.answer(0x55);
    assert_eq!(
        ports.read_status(),
        0,
        "a controller nobody asked has nothing to say"
    );
    ports.write_command(crate::controller::DISABLE_KBD);
    assert_eq!(
        ports.read_status(),
        0,
        "and a command it answers nothing to changes that not at all"
    );
    ports.write_command(crate::controller::SELF_TEST);
    assert_eq!(ports.read_status(), OUTPUT_FULL);
    assert_eq!(ports.read_data(), 0x55);
    assert_eq!(ports.read_status(), 0);
}

#[test]
fn a_byte_written_to_a_device_makes_its_answer_readable() {
    let mut ports = ScriptedPorts::new();
    ports.answer_all(&[0xFA, 0xAA]);
    assert_eq!(ports.read_status(), 0);
    ports.write_data(0xFF);
    assert_eq!(ports.read_data(), 0xFA);
    assert_eq!(ports.read_data(), 0xAA, "and the one after it as well");
    assert_eq!(ports.remaining(), 0);
}

#[test]
fn a_fixed_status_answers_that_and_the_script_is_never_reached() {
    let mut ports = ScriptedPorts::with(&[0x76]);
    ports.fixed_status(INPUT_FULL);
    assert_eq!(ports.read_status(), INPUT_FULL);
    assert_eq!(ports.read_status(), INPUT_FULL);
    assert_eq!(ports.remaining(), 1);
}

#[test]
fn every_access_is_recorded_in_order_and_the_writes_are_told_apart() {
    let mut ports = ScriptedPorts::with(&[0xFA]);
    ports.write_command(0xAA);
    ports.write_data(0xF4);
    let value = ports.read_data();
    assert_eq!(value, 0xFA);
    assert_eq!(
        ports.log(),
        [
            Access::WriteCommand(0xAA),
            Access::WriteData(0xF4),
            Access::Data(0xFA),
        ]
    );
    assert_eq!(ports.commands(), vec![0xAA]);
    assert_eq!(ports.written(), vec![0xF4]);
    assert_eq!(ports.reads(), 1);
    ports.clear();
    assert!(ports.log().is_empty());
}
