// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The re-exchange: when one is due, what each side's `SSH_MSG_KEXINIT`
//! does to the state, what may be sent while one runs, and the
//! disconnect that ends a connection.

use crate::error::SshError;
use crate::msg::{self, Disconnect};
use crate::rekey::{Answer, BYTES, Exchange, MICROSECONDS, PACKETS, Rekey};
use crate::tests::ssh_string;
use crate::wire::Reader;

/// The moment the connection began.
const START: u64 = 1_000;

/// Room for a disconnect.
const ROOM: usize = 128;

/// A connection whose first exchange is over.
fn settled() -> Rekey {
    let mut rekey = Rekey::new(START);
    rekey.settled(START).expect("the first exchange ends");
    rekey
}

#[test]
fn a_connection_begins_inside_its_first_key_exchange() {
    let rekey = Rekey::new(START);

    assert_eq!(rekey.state(), Exchange::Running);
    assert!(rekey.is_running());
    assert!(!rekey.due(START + MICROSECONDS));
}

#[test]
fn a_gigabyte_an_hour_or_half_the_sequence_numbers_ask_for_one() {
    let mut by_bytes = settled();
    for _ in 0..1024 {
        by_bytes.note(1024 * 1024);
    }
    assert_eq!(by_bytes.bytes(), BYTES);
    assert!(by_bytes.due(START));

    let by_time = settled();
    assert!(!by_time.due(START + MICROSECONDS - 1));
    assert!(by_time.due(START + MICROSECONDS));

    let mut by_packets = settled();
    for _ in 0..4 {
        by_packets.note(0);
    }
    assert_eq!(by_packets.packets(), 4);
    assert!(!by_packets.due(START));
    assert!(PACKETS < u64::from(u32::MAX));
}

#[test]
fn a_clock_that_went_backwards_asks_for_nothing() {
    let rekey = settled();

    assert!(!rekey.due(START - 1));
}

#[test]
fn this_side_asks_once_and_the_peers_answer_ends_the_asking() {
    let mut rekey = settled();

    rekey.ask().expect("nothing was running");
    assert_eq!(rekey.state(), Exchange::Asked);
    assert_eq!(rekey.ask(), Err(SshError::Exchange));

    assert_eq!(rekey.peer_asked(), Ok(Answer::Nothing));
    assert_eq!(rekey.state(), Exchange::Running);
    assert_eq!(rekey.peer_asked(), Err(SshError::Exchange));
}

#[test]
fn the_peer_may_start_one_and_this_side_answers_it() {
    let mut rekey = settled();

    assert_eq!(rekey.peer_asked(), Ok(Answer::SendKexInit));
    assert_eq!(rekey.state(), Exchange::Running);

    rekey.answered().expect("this side sent its own");
    assert_eq!(rekey.state(), Exchange::Running);
}

#[test]
fn an_answer_with_nothing_running_is_no_answer() {
    let mut rekey = settled();

    assert_eq!(rekey.answered(), Err(SshError::Exchange));
}

#[test]
fn the_new_keys_end_the_exchange_and_start_the_counting_again() {
    let mut rekey = settled();
    rekey.note(4096);
    rekey.ask().expect("nothing was running");
    rekey.peer_asked().expect("the peer answers");

    assert!(!rekey.due(START + MICROSECONDS));

    rekey
        .settled(START + MICROSECONDS)
        .expect("the keys are in use");

    assert_eq!(rekey.state(), Exchange::Settled);
    assert_eq!(rekey.bytes(), 0);
    assert_eq!(rekey.packets(), 1);
    assert!(!rekey.due(START + MICROSECONDS));
    assert!(rekey.due(START + 2 * MICROSECONDS));
}

#[test]
fn new_keys_with_no_exchange_running_are_refused() {
    let mut rekey = settled();

    assert_eq!(rekey.settled(START), Err(SshError::Exchange));
}

#[test]
fn while_an_exchange_runs_only_the_transport_may_send() {
    let mut rekey = settled();

    assert!(rekey.may_send(msg::CHANNEL_DATA));
    assert!(rekey.may_send(msg::USERAUTH_REQUEST));

    rekey.ask().expect("nothing was running");

    assert!(rekey.may_send(msg::KEXINIT));
    assert!(rekey.may_send(msg::NEWKEYS));
    assert!(rekey.may_send(msg::DISCONNECT));
    assert!(!rekey.may_send(msg::USERAUTH_REQUEST));
    assert!(!rekey.may_send(msg::CHANNEL_DATA));
}

#[test]
fn a_disconnect_carries_the_reason_code_of_the_numbers_document() {
    let disconnect = Disconnect {
        reason: msg::disconnect::HOST_KEY_NOT_VERIFIABLE,
        description: b"no rule admits this host key",
        language: b"",
    };
    let mut out = [0u8; ROOM];
    let len = disconnect.write(&mut out).expect("there is room");

    let mut reader = Reader::new(out.get(..len).unwrap_or_default());
    assert_eq!(reader.read_byte(), Ok(msg::DISCONNECT));
    assert_eq!(reader.read_u32(), Ok(9));
    assert_eq!(
        reader.read_string(),
        Ok(b"no rule admits this host key".as_slice())
    );
    assert_eq!(reader.read_string(), Ok(b"".as_slice()));
    assert!(reader.is_empty());

    assert_eq!(
        Disconnect::read(out.get(..len).unwrap_or_default()),
        Ok(disconnect)
    );
}

#[test]
fn a_disconnect_that_is_not_one_is_refused() {
    let mut payload = vec![msg::DEBUG];
    payload.extend_from_slice(&3u32.to_be_bytes());
    payload.extend_from_slice(&ssh_string(b"key exchange failed"));

    assert_eq!(
        Disconnect::read(&payload),
        Err(SshError::Message(msg::DEBUG))
    );
    assert!(matches!(
        Disconnect::read(&[msg::DISCONNECT]),
        Err(SshError::OutOfBounds { .. })
    ));

    let disconnect = Disconnect {
        reason: msg::disconnect::KEY_EXCHANGE_FAILED,
        description: b"the shared secret is zero",
        language: b"",
    };
    assert!(matches!(
        disconnect.write(&mut [0u8; 8]),
        Err(SshError::OutOfBounds { .. })
    ));
}
