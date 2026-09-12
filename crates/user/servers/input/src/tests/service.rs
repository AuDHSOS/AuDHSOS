// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::service`.

use driver_i8042::controller::{Controller, MAX_POLLS, OUTPUT_FULL};

use crate::doubles::ScriptedDevice;
use crate::service::{AUX_FLAG, Line, pack, service, unpack};

/// A controller with `bytes` standing in its output buffer.
fn device(bytes: &[u8]) -> ScriptedDevice {
    let mut device = ScriptedDevice::new();
    device.ports().push_all(bytes);
    device
}

#[test]
fn a_byte_and_where_it_came_from_go_into_one_word_and_come_back_out_of_it() {
    for byte in [0x00, 0x1C, 0xFF] {
        for aux in [false, true] {
            assert_eq!(unpack(pack(byte, aux)), (byte, aux));
        }
    }
    assert_eq!(pack(0, true), AUX_FLAG);
    assert_eq!(pack(0, false), 0);
}

#[test]
fn the_buffer_is_emptied_and_every_byte_says_which_device_sent_it() {
    let mut device = ScriptedDevice::new();
    device.ports().push(0x1C);
    device.ports().push_aux(0x08);
    device.ports().push(0xF0);
    let mut controller = Controller::new(device);
    let mut into = [0u64; 8];
    let taken = service(&mut controller, &mut into);
    assert_eq!(taken, 3);
    let read: Vec<(u8, bool)> = into.iter().take(taken).map(|word| unpack(*word)).collect();
    assert_eq!(read, vec![(0x1C, false), (0x08, true), (0xF0, false)]);
}

#[test]
fn both_lines_are_acknowledged_after_the_buffer_is_drained() {
    let mut controller = Controller::new(device(&[0x1C]));
    let mut into = [0u64; 4];
    assert_eq!(service(&mut controller, &mut into), 1);
    assert_eq!(
        controller.ports().acknowledged(),
        Line::ALL,
        "one drain empties what either line filled, so both are let go"
    );
    assert_eq!(Line::ALL.len(), 2);
    assert_eq!(Line::Keyboard.name(), "keyboard");
    assert_eq!(Line::Mouse.name(), "mouse");
}

#[test]
fn an_empty_buffer_takes_nothing_and_still_lets_both_lines_assert_again() {
    let mut controller = Controller::new(ScriptedDevice::new());
    let mut into = [0u64; 4];
    assert_eq!(service(&mut controller, &mut into), 0);
    assert_eq!(
        controller.ports().acknowledged(),
        Line::ALL,
        "a line the kernel masked has to be let go whether there was a byte or not"
    );
}

#[test]
fn a_drain_stops_when_the_message_is_full_and_leaves_the_rest_standing() {
    let mut controller = Controller::new(device(&[1, 2, 3, 4, 5]));
    let mut into = [0u64; 2];
    assert_eq!(service(&mut controller, &mut into), 2);
    assert_eq!(into, [pack(1, false), pack(2, false)]);
    assert_eq!(
        controller.ports().ports().remaining(),
        3,
        "what did not fit is read the next time round"
    );
}

#[test]
fn a_controller_that_never_runs_dry_is_read_no_further_than_the_message() {
    let mut device = ScriptedDevice::new();
    device.ports().fixed_status(OUTPUT_FULL);
    let mut controller = Controller::new(device);
    // Longer than one round of the drain, so that both bounds are met: the
    // poll limit ends the first round and the full message ends the second.
    let mut into = vec![0u64; usize::try_from(MAX_POLLS).unwrap().saturating_add(16)];
    let len = into.len();
    assert_eq!(
        service(&mut controller, &mut into),
        len,
        "the thread stops at the message it can send and not at the controller"
    );
    assert_eq!(
        controller.ports().acknowledged(),
        [Line::Keyboard, Line::Mouse, Line::Keyboard, Line::Mouse],
        "the lines are let go once per round"
    );
}

#[test]
fn a_byte_that_arrives_while_the_lines_are_let_go_is_taken_all_the_same() {
    let mut device = ScriptedDevice::new();
    device.ports().push(0x1C);
    // The second byte stands in the buffer only after the first round has
    // read the first, which is the race the second look is there for: a
    // line raised into its own mask is a line that never raises again.
    device.push_late(0x1B);
    let mut controller = Controller::new(device);
    let mut into = [0u64; 8];
    assert_eq!(service(&mut controller, &mut into), 2);
    assert_eq!(
        into.get(..2),
        Some([pack(0x1C, false), pack(0x1B, false)].as_slice())
    );
    assert_eq!(
        controller.ports().acknowledged().len(),
        4,
        "two rounds, and both lines let go in each"
    );
}
