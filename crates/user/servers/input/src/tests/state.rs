// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::state`.

use audhsos_abi::Error;
use driver_i8042::device::WHEEL_ID;
use driver_i8042::mouse::{BUTTON_LEFT, BUTTON_RIGHT, SYNC};
use user_proto::input::{Event, KeyCode, KeyEvent, PointerEvent, RING_CAPACITY};

use crate::doubles::RecordingClients;
use crate::state::{Clients as _, Input, NOBODY, Subscriber};

/// How many clients the servers of these tests hold.
const CLIENTS: usize = 4;

/// A server nobody listens to, with a mouse that has a wheel.
fn server() -> Input<CLIENTS> {
    Input::new(WHEEL_ID)
}

/// Every event the bytes produce.
fn produced(input: &mut Input<CLIENTS>, bytes: &[(u8, bool)]) -> Vec<Event> {
    bytes
        .iter()
        .flat_map(|(byte, aux)| input.feed(*byte, *aux).iter().copied().collect::<Vec<_>>())
        .collect()
}

#[test]
fn a_client_that_carries_no_badge_gets_no_ring() {
    let mut input = server();
    assert_eq!(input.subscribe(NOBODY), Err(Error::AccessDenied));
    assert!(input.is_empty());
    assert_eq!(input.len(), 0);
}

#[test]
fn every_subscription_gets_a_slot_of_its_own_and_the_second_of_a_badge_is_refused() {
    let mut input = server();
    assert_eq!(input.subscribe(1), Ok(0));
    assert_eq!(input.subscribe(2), Ok(1));
    assert_eq!(input.subscribe(1), Err(Error::AlreadyExists));
    assert_eq!(input.len(), 2);
    assert_eq!(input.of(1), Some(&Subscriber { badge: 1, slot: 0 }));
    assert_eq!(input.of(3), None);
    let slots: Vec<usize> = input.iter().map(|held| held.slot).collect();
    assert_eq!(slots, vec![0, 1]);
}

#[test]
fn a_slot_that_was_let_go_of_is_given_out_again() {
    let mut input = server();
    for badge in 1..=4 {
        assert!(input.subscribe(badge).is_ok());
    }
    assert_eq!(
        input.subscribe(5),
        Err(Error::QuotaExceeded),
        "the server holds as many rings as it can"
    );
    assert_eq!(input.unsubscribe(2), Ok(1));
    assert_eq!(input.unsubscribe(2), Err(Error::NotFound));
    assert_eq!(input.subscribe(5), Ok(1), "the slot is free again");
}

#[test]
fn a_client_that_goes_away_is_forgotten_and_one_that_never_was_is_nothing() {
    let mut input = server();
    assert!(input.subscribe(7).is_ok());
    assert_eq!(input.forget(7), Some(Subscriber { badge: 7, slot: 0 }));
    assert_eq!(input.forget(7), None);
    assert!(input.is_empty());
}

#[test]
fn the_aux_bit_routes_a_byte_to_the_mouse_and_everything_else_to_the_keyboard() {
    let mut input = server();
    assert_eq!(
        produced(&mut input, &[(0x1C, false)]),
        vec![Event::Key(KeyEvent {
            code: KeyCode::A,
            pressed: true
        })]
    );
    // A byte marked as the mouse's reaches no decoder of keys: it is the
    // first byte of a packet, and the packet is what makes the event.
    let mut input = server();
    assert_eq!(
        produced(&mut input, &[(SYNC | BUTTON_LEFT, true)]),
        Vec::new(),
        "one byte of four is no packet yet"
    );
    assert_eq!(
        produced(&mut input, &[(0x02, true), (0x03, true), (0x00, true)]),
        vec![Event::Pointer(PointerEvent {
            dx: 2,
            dy: -3,
            wheel: 0,
            buttons: BUTTON_LEFT,
        })]
    );
}

#[test]
fn a_release_without_a_press_is_delivered_as_a_release() {
    let mut input = server();
    assert_eq!(
        produced(&mut input, &[(0xF0, false), (0x1C, false)]),
        vec![Event::Key(KeyEvent {
            code: KeyCode::A,
            pressed: false
        })],
        "the server keeps no note of what is down; the client does"
    );
}

#[test]
fn the_button_state_is_carried_by_every_packet() {
    let mut input = server();
    let held = produced(
        &mut input,
        &[
            (SYNC | BUTTON_LEFT, true),
            (1, true),
            (0, true),
            (0, true),
            (SYNC | BUTTON_LEFT | BUTTON_RIGHT, true),
            (1, true),
            (0, true),
            (0, true),
            (SYNC, true),
            (0, true),
            (0, true),
            (0, true),
        ],
    );
    let buttons: Vec<u8> = held
        .iter()
        .filter_map(|event| match event {
            Event::Pointer(pointer) => Some(pointer.buttons),
            Event::Key(_) => None,
        })
        .collect();
    assert_eq!(
        buttons,
        vec![BUTTON_LEFT, BUTTON_LEFT | BUTTON_RIGHT, 0],
        "a press and a release are two packets and the state stands in each"
    );
}

#[test]
fn a_wheel_delta_is_delivered_as_an_event_of_its_own() {
    let mut input = server();
    let events = produced(&mut input, &[(SYNC, true), (5, true), (0, true), (1, true)]);
    assert_eq!(
        events,
        vec![
            Event::Pointer(PointerEvent {
                dx: 5,
                dy: 0,
                wheel: 0,
                buttons: 0,
            }),
            Event::Pointer(PointerEvent {
                dx: 0,
                dy: 0,
                wheel: 1,
                buttons: 0,
            }),
        ],
        "a client that reads the wheel does not have to look at the movement to find it"
    );

    // A packet without a wheel is one event.
    let events = produced(&mut input, &[(SYNC, true), (1, true), (0, true), (0, true)]);
    assert_eq!(events.len(), 1);
}

#[test]
fn what_the_decoders_make_reaches_every_ring_and_wakes_every_client() {
    let mut input = server();
    let mut clients = RecordingClients::new(CLIENTS);
    assert!(input.subscribe(1).is_ok());
    assert!(input.subscribe(2).is_ok());
    let event = Event::Key(KeyEvent {
        code: KeyCode::Escape,
        pressed: true,
    });
    assert!(input.deliver(event, &mut clients).is_empty());
    assert_eq!(clients.woken(), [0, 1]);
    assert_eq!(clients.drain(0), vec![event]);
    assert_eq!(clients.drain(1), vec![event]);
    assert_eq!(clients.drain(2), Vec::new(), "and nobody else's");
}

#[test]
fn a_subscriber_that_cannot_be_woken_is_gone() {
    let mut input = server();
    let mut clients = RecordingClients::new(CLIENTS);
    assert!(input.subscribe(1).is_ok());
    assert!(input.subscribe(2).is_ok());
    clients.end(0);
    let event = Event::Key(KeyEvent {
        code: KeyCode::A,
        pressed: true,
    });
    let gone = input.deliver(event, &mut clients);
    assert_eq!(gone.len(), 1);
    assert_eq!(gone.get(0), Some(&Subscriber { badge: 1, slot: 0 }));
    assert_eq!(input.len(), 1, "and it is no longer subscribed");
    assert_eq!(input.of(1), None);
    assert_eq!(clients.woken(), [1], "the one that is left was woken");

    // Its slot is free for the next client.
    assert_eq!(input.subscribe(3), Ok(0));
}

#[test]
fn a_ring_the_process_does_not_hold_costs_the_event_and_not_the_subscription() {
    let mut input = server();
    // One client, and a process that holds no ring for it at all.
    let mut clients = RecordingClients::new(0);
    assert!(input.subscribe(1).is_ok());
    let event = Event::Key(KeyEvent {
        code: KeyCode::A,
        pressed: true,
    });
    assert!(input.deliver(event, &mut clients).is_empty());
    assert_eq!(input.len(), 1);
    assert_eq!(clients.woken(), [0]);
    assert_eq!(clients.ring(0), None);
}

#[test]
fn a_ring_that_is_full_drops_the_event_and_the_subscription_stays() {
    let mut input = server();
    let mut clients = RecordingClients::new(CLIENTS);
    assert!(input.subscribe(1).is_ok());
    let event = Event::Key(KeyEvent {
        code: KeyCode::A,
        pressed: true,
    });
    for _ in 0..=RING_CAPACITY {
        assert!(input.deliver(event, &mut clients).is_empty());
    }
    assert_eq!(clients.overflow(0), 1);
    assert_eq!(input.len(), 1);
}

#[test]
fn a_server_without_a_wheel_reads_three_byte_packets() {
    let mut input: Input<CLIENTS> = Input::default();
    assert_eq!(
        produced(&mut input, &[(SYNC, true), (1, true), (2, true)]),
        vec![Event::Pointer(PointerEvent {
            dx: 1,
            dy: -2,
            wheel: 0,
            buttons: 0,
        })]
    );
}
