// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::doubles`, which the tests of the state stand on.

use user_proto::input::{Event, KeyCode, KeyEvent, RingWriter};

use crate::doubles::{RecordingClients, ScriptedDevice};
use crate::service::{Line, Lines as _};
use crate::state::Clients as _;

#[test]
fn a_ring_of_the_double_is_made_and_reads_back_what_was_pushed_into_it() {
    let mut clients = RecordingClients::new(2);
    let event = Event::Key(KeyEvent {
        code: KeyCode::A,
        pressed: true,
    });
    {
        let bytes = clients.ring(0).unwrap();
        let mut ring = RingWriter::adopt(bytes).unwrap();
        assert!(ring.push(event));
    }
    assert_eq!(clients.drain(0), vec![event]);
    assert_eq!(clients.drain(0), Vec::new());
    assert_eq!(clients.overflow(0), 0);
    assert_eq!(clients.drain(9), Vec::new(), "a slot nobody holds is empty");
    assert_eq!(clients.overflow(9), 0);
}

#[test]
fn a_client_that_has_ended_cannot_be_woken_and_the_others_still_can() {
    let mut clients = RecordingClients::new(2);
    assert!(clients.wake(0));
    clients.end(1);
    assert!(!clients.wake(1));
    assert!(clients.wake(0));
    assert_eq!(clients.woken(), [0, 0]);
    clients.clear();
    assert!(clients.woken().is_empty());
}

#[test]
fn the_device_records_every_acknowledgement_in_order_and_answers_its_script() {
    let mut device = ScriptedDevice::new();
    device.ports().push(0x76);
    device.acknowledge(Line::Mouse);
    device.acknowledge(Line::Keyboard);
    assert_eq!(device.acknowledged(), [Line::Mouse, Line::Keyboard]);
    assert_eq!(device.ports().remaining(), 1);
}
