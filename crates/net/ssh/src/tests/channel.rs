// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The session channel: the open, the window, the two data messages, the
//! requests that start a program, and the close sequence.
//!
//! RFC 4254 publishes no vector, so every message is read back with a
//! reader of this crate and every rule of sections 5.1 to 5.3 is checked
//! against what the channel does with it.

use crate::channel::{
    Channel, ENV, EXEC, EXIT_SIGNAL, EXIT_STATUS, Event, Message, SESSION, SHELL, STDERR,
};
use crate::error::SshError;
use crate::msg;
use crate::tests::ssh_string;
use crate::wire::Reader;

/// This side's channel number.
const LOCAL: u32 = 0;
/// The peer's.
const REMOTE: u32 = 7;
/// What this side grants the peer.
const WINDOW: u32 = 64;
/// What this side takes in one data message.
const MAX_PACKET: u32 = 32;
/// The same, as a length.
const MAX_PACKET_BYTES: usize = 32;
/// Room for what these tests write.
const ROOM: usize = 256;

/// A channel the peer has confirmed, with `window` to send into.
fn opened(window: u32, max_packet: u32) -> Channel {
    let mut channel = Channel::new(LOCAL, WINDOW, MAX_PACKET);
    channel
        .apply(&Message {
            channel: LOCAL,
            event: Event::Opened {
                remote: REMOTE,
                window,
                max_packet,
            },
        })
        .expect("the confirmation opens the channel");
    channel
}

/// One message for this channel.
fn message(event: Event<'_>) -> Message<'_> {
    Message {
        channel: LOCAL,
        event,
    }
}

/// The bytes of a channel request as a server writes it.
fn request(kind: &str, want_reply: bool, tail: &[u8]) -> Vec<u8> {
    let mut payload = vec![msg::CHANNEL_REQUEST];
    payload.extend_from_slice(&LOCAL.to_be_bytes());
    payload.extend_from_slice(&ssh_string(kind.as_bytes()));
    payload.push(u8::from(want_reply));
    payload.extend_from_slice(tail);
    payload
}

#[test]
fn the_open_names_the_type_the_number_and_what_this_side_takes() {
    let channel = Channel::new(LOCAL, WINDOW, MAX_PACKET);
    let mut out = [0u8; ROOM];
    let len = channel.write_open(&mut out).expect("there is room");

    let mut reader = Reader::new(out.get(..len).unwrap_or_default());
    assert_eq!(reader.read_byte(), Ok(msg::CHANNEL_OPEN));
    assert_eq!(reader.read_string(), Ok(SESSION.as_bytes()));
    assert_eq!(reader.read_u32(), Ok(LOCAL));
    assert_eq!(reader.read_u32(), Ok(WINDOW));
    assert_eq!(reader.read_u32(), Ok(MAX_PACKET));
    assert!(reader.is_empty());
    assert!(!channel.is_open());
}

#[test]
fn the_confirmation_gives_the_peers_number_and_the_window_to_send_into() {
    let channel = opened(100, 16);

    assert!(channel.is_open());
    assert_eq!(channel.remote(), Some(REMOTE));
    assert_eq!(channel.remote_window(), 100);
    assert_eq!(channel.local_window(), WINDOW);
}

#[test]
fn a_second_confirmation_is_no_message_this_channel_takes() {
    let mut channel = opened(100, 16);

    assert_eq!(
        channel.apply(&message(Event::Opened {
            remote: 9,
            window: 1,
            max_packet: 1,
        })),
        Err(SshError::Channel)
    );
    assert_eq!(channel.remote(), Some(REMOTE));
}

#[test]
fn a_message_for_another_channel_is_refused() {
    let mut channel = opened(100, 16);

    assert_eq!(
        channel.apply(&Message {
            channel: LOCAL.wrapping_add(1),
            event: Event::Eof,
        }),
        Err(SshError::Channel)
    );
}

#[test]
fn a_refused_open_closes_the_channel_without_a_close_message() {
    let mut channel = Channel::new(LOCAL, WINDOW, MAX_PACKET);

    channel
        .apply(&message(Event::OpenFailed {
            reason: msg::open::UNKNOWN_CHANNEL_TYPE,
            description: b"no such channel type",
            language: b"",
        }))
        .expect("a refusal ends the channel");

    assert!(!channel.is_open());
    assert!(channel.is_closed());
}

#[test]
fn data_spends_the_window_and_stops_at_it() {
    let mut channel = opened(40, MAX_PACKET);
    let mut out = [0u8; ROOM];

    let len = channel
        .write_data(&[0xaa; 30], &mut out)
        .expect("thirty bytes fit in the window and in a packet");
    assert_eq!(channel.remote_window(), 10);

    let mut reader = Reader::new(out.get(..len).unwrap_or_default());
    assert_eq!(reader.read_byte(), Ok(msg::CHANNEL_DATA));
    assert_eq!(reader.read_u32(), Ok(REMOTE));
    assert_eq!(reader.read_string(), Ok([0xaa; 30].as_slice()));

    assert_eq!(
        channel.write_data(&[0xbb; 11], &mut out),
        Err(SshError::Window)
    );
    assert_eq!(channel.remote_window(), 10);
}

#[test]
fn no_data_message_is_larger_than_the_maximum_packet_size() {
    assert_eq!(usize::try_from(MAX_PACKET), Ok(MAX_PACKET_BYTES));
    let mut channel = opened(1024, MAX_PACKET);
    let mut out = [0u8; ROOM];

    assert_eq!(
        channel.write_data(&[0xcc; MAX_PACKET_BYTES + 1], &mut out),
        Err(SshError::Window)
    );
    assert!(
        channel
            .write_data(&[0xcc; MAX_PACKET_BYTES], &mut out)
            .is_ok()
    );
}

#[test]
fn extended_data_spends_the_same_window_as_ordinary_data() {
    let mut channel = opened(40, MAX_PACKET);
    let mut out = [0u8; ROOM];

    let len = channel
        .write_extended_data(STDERR, &[0xdd; 10], &mut out)
        .expect("ten bytes fit");
    assert_eq!(channel.remote_window(), 30);

    let mut reader = Reader::new(out.get(..len).unwrap_or_default());
    assert_eq!(reader.read_byte(), Ok(msg::CHANNEL_EXTENDED_DATA));
    assert_eq!(reader.read_u32(), Ok(REMOTE));
    assert_eq!(reader.read_u32(), Ok(STDERR));
    assert_eq!(reader.read_string(), Ok([0xdd; 10].as_slice()));
}

#[test]
fn what_arrives_spends_the_window_this_side_granted() {
    let mut channel = opened(100, 16);

    channel
        .apply(&message(Event::Data(&[0x11; 40])))
        .expect("forty bytes are within the window");
    assert_eq!(channel.local_window(), WINDOW - 40);

    channel
        .apply(&message(Event::ExtendedData {
            kind: STDERR,
            data: &[0x22; 24],
        }))
        .expect("the last of the window");
    assert_eq!(channel.local_window(), 0);

    assert_eq!(
        channel.apply(&message(Event::Data(&[0x33; 1]))),
        Err(SshError::Window)
    );
}

#[test]
fn a_window_adjust_grants_and_never_passes_two_to_the_thirty_second() {
    let mut channel = opened(10, 16);
    let mut out = [0u8; ROOM];

    channel
        .apply(&message(Event::WindowAdjust(90)))
        .expect("ninety more bytes to send");
    assert_eq!(channel.remote_window(), 100);

    assert_eq!(
        channel.apply(&message(Event::WindowAdjust(u32::MAX))),
        Err(SshError::Window)
    );

    let len = channel
        .write_window_adjust(1000, &mut out)
        .expect("there is room");
    assert_eq!(channel.local_window(), WINDOW + 1000);

    let mut reader = Reader::new(out.get(..len).unwrap_or_default());
    assert_eq!(reader.read_byte(), Ok(msg::CHANNEL_WINDOW_ADJUST));
    assert_eq!(reader.read_u32(), Ok(REMOTE));
    assert_eq!(reader.read_u32(), Ok(1000));

    assert_eq!(
        channel.write_window_adjust(u32::MAX, &mut out),
        Err(SshError::Window)
    );
}

#[test]
fn the_requests_that_start_a_program_carry_what_the_document_prints() {
    let channel = opened(100, 16);
    let mut out = [0u8; ROOM];

    let len = channel
        .write_exec(b"uname -a", true, &mut out)
        .expect("there is room");
    let mut reader = Reader::new(out.get(..len).unwrap_or_default());
    assert_eq!(reader.read_byte(), Ok(msg::CHANNEL_REQUEST));
    assert_eq!(reader.read_u32(), Ok(REMOTE));
    assert_eq!(reader.read_string(), Ok(EXEC.as_bytes()));
    assert_eq!(reader.read_boolean(), Ok(true));
    assert_eq!(reader.read_string(), Ok(b"uname -a".as_slice()));
    assert!(reader.is_empty());

    let len = channel.write_shell(false, &mut out).expect("there is room");
    let mut reader = Reader::new(out.get(..len).unwrap_or_default());
    assert_eq!(reader.read_byte(), Ok(msg::CHANNEL_REQUEST));
    assert_eq!(reader.read_u32(), Ok(REMOTE));
    assert_eq!(reader.read_string(), Ok(SHELL.as_bytes()));
    assert_eq!(reader.read_boolean(), Ok(false));
    assert!(reader.is_empty());

    let len = channel
        .write_env(b"LANG", b"C", false, &mut out)
        .expect("there is room");
    let mut reader = Reader::new(out.get(..len).unwrap_or_default());
    assert_eq!(reader.read_byte(), Ok(msg::CHANNEL_REQUEST));
    assert_eq!(reader.read_u32(), Ok(REMOTE));
    assert_eq!(reader.read_string(), Ok(ENV.as_bytes()));
    assert_eq!(reader.read_boolean(), Ok(false));
    assert_eq!(reader.read_string(), Ok(b"LANG".as_slice()));
    assert_eq!(reader.read_string(), Ok(b"C".as_slice()));
}

#[test]
fn a_request_spends_no_window() {
    let channel = opened(0, 16);
    let mut out = [0u8; ROOM];

    assert!(channel.write_exec(b"true", true, &mut out).is_ok());
    assert!(channel.write_shell(true, &mut out).is_ok());
    assert_eq!(channel.remote_window(), 0);
}

#[test]
fn the_exit_status_and_the_exit_signal_read_as_what_they_are() {
    let status = request(EXIT_STATUS, false, &1u32.to_be_bytes());
    let mut signal = request(EXIT_SIGNAL, false, &ssh_string(b"TERM"));
    signal.push(1);
    signal.extend_from_slice(&ssh_string(b"killed"));
    signal.extend_from_slice(&ssh_string(b"en"));

    assert_eq!(
        Message::read(&status).map(|message| message.event),
        Ok(Event::ExitStatus(1))
    );
    assert_eq!(
        Message::read(&signal).map(|message| message.event),
        Ok(Event::ExitSignal {
            signal: b"TERM",
            core_dumped: true,
            message: b"killed",
            language: b"en",
        })
    );
}

#[test]
fn another_request_keeps_its_name_and_whether_it_wants_an_answer() {
    let pty = request("pty-req", true, &[]);

    assert_eq!(
        Message::read(&pty).map(|message| message.event),
        Ok(Event::Request {
            kind: b"pty-req",
            want_reply: true,
        })
    );
}

#[test]
fn the_close_sequence_is_both_directions_and_a_close_is_answered_once() {
    let mut channel = opened(100, 16);
    let mut out = [0u8; ROOM];

    channel
        .apply(&message(Event::Eof))
        .expect("the peer will send no more data");
    assert!(channel.eof_received());
    assert!(!channel.is_closed());

    let len = channel.write_eof(&mut out).expect("there is room");
    let mut reader = Reader::new(out.get(..len).unwrap_or_default());
    assert_eq!(reader.read_byte(), Ok(msg::CHANNEL_EOF));
    assert_eq!(reader.read_u32(), Ok(REMOTE));

    channel
        .apply(&message(Event::Close))
        .expect("the peer closes");
    assert!(!channel.is_closed());

    let len = channel.write_close(&mut out).expect("there is room");
    let mut reader = Reader::new(out.get(..len).unwrap_or_default());
    assert_eq!(reader.read_byte(), Ok(msg::CHANNEL_CLOSE));
    assert_eq!(reader.read_u32(), Ok(REMOTE));
    assert!(channel.is_closed());

    assert_eq!(channel.write_close(&mut out), Err(SshError::Channel));
}

#[test]
fn a_close_may_arrive_with_no_end_of_file_before_it() {
    let mut channel = opened(100, 16);

    channel
        .apply(&message(Event::Close))
        .expect("a close needs no eof before it");

    assert!(!channel.eof_received());
}

#[test]
fn nothing_is_sent_before_the_open_is_confirmed_or_after_a_close() {
    let mut before = Channel::new(LOCAL, WINDOW, MAX_PACKET);
    let mut out = [0u8; ROOM];

    assert_eq!(before.write_data(b"x", &mut out), Err(SshError::Channel));
    assert_eq!(before.write_eof(&mut out), Err(SshError::Channel));
    assert_eq!(before.write_close(&mut out), Err(SshError::Channel));
    assert_eq!(
        before.write_window_adjust(1, &mut out),
        Err(SshError::Channel)
    );
    assert_eq!(before.write_shell(true, &mut out), Err(SshError::Channel));
    assert_eq!(
        before.apply(&message(Event::Data(b"x"))),
        Err(SshError::Channel)
    );

    let mut after = opened(100, 16);
    after.write_close(&mut out).expect("there is room");
    assert_eq!(after.write_data(b"x", &mut out), Err(SshError::Channel));
    assert_eq!(after.write_eof(&mut out), Err(SshError::Channel));
    assert_eq!(
        after.write_exec(b"true", true, &mut out),
        Err(SshError::Channel)
    );
}

#[test]
fn data_after_an_end_of_file_this_side_sent_is_refused() {
    let mut channel = opened(100, 16);
    let mut out = [0u8; ROOM];

    channel.write_eof(&mut out).expect("there is room");

    assert_eq!(channel.write_data(b"x", &mut out), Err(SshError::Channel));
}

#[test]
fn a_buffer_too_small_writes_nothing_and_spends_nothing() {
    let mut channel = opened(100, 16);
    let mut out = [0u8; 4];

    assert!(matches!(
        channel.write_data(b"hello", &mut out),
        Err(SshError::OutOfBounds { .. })
    ));
    assert_eq!(channel.remote_window(), 100);

    assert!(matches!(
        channel.write_window_adjust(10, &mut out),
        Err(SshError::OutOfBounds { .. })
    ));
    assert_eq!(channel.local_window(), WINDOW);

    assert!(matches!(
        channel.write_open(&mut out),
        Err(SshError::OutOfBounds { .. })
    ));
    assert!(matches!(
        channel.write_exec(b"true", true, &mut out),
        Err(SshError::OutOfBounds { .. })
    ));
    assert!(matches!(
        channel.write_eof(&mut out),
        Err(SshError::OutOfBounds { .. })
    ));
    assert!(matches!(
        channel.write_close(&mut out),
        Err(SshError::OutOfBounds { .. })
    ));
    assert!(!channel.is_closed());
}

#[test]
fn every_message_of_the_layer_reads_and_nothing_else_does() {
    let mut confirmation = vec![msg::CHANNEL_OPEN_CONFIRMATION];
    confirmation.extend_from_slice(&LOCAL.to_be_bytes());
    confirmation.extend_from_slice(&REMOTE.to_be_bytes());
    confirmation.extend_from_slice(&256u32.to_be_bytes());
    confirmation.extend_from_slice(&16u32.to_be_bytes());

    let mut failure = vec![msg::CHANNEL_OPEN_FAILURE];
    failure.extend_from_slice(&LOCAL.to_be_bytes());
    failure.extend_from_slice(&msg::open::RESOURCE_SHORTAGE.to_be_bytes());
    failure.extend_from_slice(&ssh_string(b"no room"));
    failure.extend_from_slice(&ssh_string(b""));

    let mut data = vec![msg::CHANNEL_DATA];
    data.extend_from_slice(&LOCAL.to_be_bytes());
    data.extend_from_slice(&ssh_string(b"out"));

    assert_eq!(
        Message::read(&confirmation).map(|message| message.event),
        Ok(Event::Opened {
            remote: REMOTE,
            window: 256,
            max_packet: 16,
        })
    );
    assert_eq!(
        Message::read(&failure).map(|message| message.event),
        Ok(Event::OpenFailed {
            reason: msg::open::RESOURCE_SHORTAGE,
            description: b"no room",
            language: b"",
        })
    );
    assert_eq!(
        Message::read(&data).map(|message| message.event),
        Ok(Event::Data(b"out"))
    );
    assert_eq!(
        Message::read(&[msg::CHANNEL_SUCCESS, 0, 0, 0, 0]).map(|message| message.event),
        Ok(Event::Success)
    );
    assert_eq!(
        Message::read(&[msg::CHANNEL_FAILURE, 0, 0, 0, 0]).map(|message| message.event),
        Ok(Event::Failure)
    );
    assert_eq!(
        Message::read(&[msg::KEXINIT, 0, 0, 0, 0]),
        Err(SshError::Message(msg::KEXINIT))
    );
    assert!(matches!(
        Message::read(&[msg::CHANNEL_EOF]),
        Err(SshError::OutOfBounds { .. })
    ));
}
