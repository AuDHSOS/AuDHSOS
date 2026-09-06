// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The fixed part, the magic cookie, and what the client refuses to read.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test edits a message at known offsets"
)]

use net_wire::{Ipv4Addr, MacAddr, WireError, Writer};
use test_support::generators::bytes;
use test_support::property::check;

use crate::error::DhcpError;
use crate::message::{
    FIXED_LEN, HARDWARE_ETHERNET, MAGIC_COOKIE, MAX_MESSAGE_LEN, MIN_MESSAGE_LEN, Message,
    MessageType, Op,
};
use crate::option::OptionCode;
use crate::tests::harness::{MAC, OFFERED, Reply, opt};

#[test]
fn a_reply_reads_back_as_what_the_server_wrote() {
    let bytes = Reply::ack(0x1234_5678).bytes();
    assert_eq!(bytes.len(), MIN_MESSAGE_LEN);
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.op, Op::REPLY);
    assert_eq!(message.xid, 0x1234_5678);
    assert_eq!(message.yours, OFFERED);
    assert_eq!(message.hardware, MAC);
    assert!(!message.broadcast);
    assert_eq!(message.client, Ipv4Addr::UNSPECIFIED);
    assert_eq!(message.next_server, Ipv4Addr::UNSPECIFIED);
    assert_eq!(message.relay, Ipv4Addr::UNSPECIFIED);
    assert_eq!(message.message_type(), Ok(MessageType::ACK));
}

#[test]
fn a_request_this_client_writes_reads_back_as_itself() {
    let mut message = Message::request(7, MAC);
    message.broadcast = true;
    message.secs = 12;
    message.options = &[53, 1, 1, 255];
    let mut buffer = [0u8; MIN_MESSAGE_LEN];
    let mut writer = Writer::new(&mut buffer);
    message.write(&mut writer).expect("room");
    let bytes = writer.finish().to_vec();
    assert_eq!(bytes.len(), MIN_MESSAGE_LEN);
    assert_eq!(bytes[1], HARDWARE_ETHERNET);
    assert_eq!(usize::from(bytes[2]), MacAddr::LEN);
    assert_eq!(&bytes[FIXED_LEN..FIXED_LEN + 4], &MAGIC_COOKIE);
    let again = Message::parse(&bytes).expect("a message");
    assert_eq!(again.op, Op::REQUEST);
    assert_eq!(again.xid, 7);
    assert_eq!(again.secs, 12);
    assert!(again.broadcast);
    assert_eq!(again.message_type(), Ok(MessageType::DISCOVER));
    assert_eq!(Op::REQUEST.get(), 1);
    assert_eq!(Op::REPLY.get(), 2);
}

/// A reply whose option block is exactly `options`.
fn with_block(options: &[u8]) -> Vec<u8> {
    let mut message = Message::request(1, MAC);
    message.op = Op::REPLY;
    message.options = options;
    let mut buffer = [0u8; MIN_MESSAGE_LEN];
    let mut writer = Writer::new(&mut buffer);
    message.write(&mut writer).expect("room");
    writer.finish().to_vec()
}

#[test]
fn a_message_with_no_message_type_is_no_dhcp_message() {
    let bytes = with_block(&[1, 4, 255, 255, 255, 0, 255]);
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.message_type(), Err(DhcpError::MissingMessageType));
}

#[test]
fn a_message_type_that_is_not_one_byte_is_refused() {
    let bytes = with_block(&[53, 3, 5, 0, 0, 255]);
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(
        message.message_type(),
        Err(DhcpError::OptionLength {
            code: OptionCode::MESSAGE_TYPE,
            len: 3
        })
    );
}

#[test]
fn a_message_type_option_of_no_body_is_refused() {
    let bytes = with_block(&[53, 0, 255]);
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(
        message.message_type(),
        Err(DhcpError::OptionLength {
            code: OptionCode::MESSAGE_TYPE,
            len: 0
        })
    );
}

#[test]
fn an_option_block_that_cannot_be_walked_is_reported_and_not_stepped_over() {
    let mut bytes = Reply::ack(1).bytes();
    bytes[FIXED_LEN + 4] = 12;
    bytes[FIXED_LEN + 5] = 255;
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(
        message.message_type(),
        Err(DhcpError::TruncatedOption(OptionCode::new(12)))
    );
}

#[test]
fn the_magic_cookie_is_what_tells_options_from_the_vendor_field() {
    let mut bytes = Reply::ack(1).bytes();
    bytes[FIXED_LEN] = 0;
    assert_eq!(
        Message::parse(&bytes).err(),
        Some(DhcpError::Cookie([0, 130, 83, 99]))
    );
}

#[test]
fn a_link_this_client_has_no_frames_for_is_refused() {
    let mut bytes = Reply::ack(1).bytes();
    bytes[1] = 6;
    assert_eq!(
        Message::parse(&bytes).err(),
        Some(DhcpError::Hardware { htype: 6, hlen: 6 })
    );
    let mut bytes = Reply::ack(1).bytes();
    bytes[2] = 8;
    assert_eq!(
        Message::parse(&bytes).err(),
        Some(DhcpError::Hardware { htype: 1, hlen: 8 })
    );
}

#[test]
fn an_op_code_that_is_neither_direction_is_refused() {
    let mut bytes = Reply::ack(1).bytes();
    bytes[0] = 3;
    assert_eq!(Message::parse(&bytes).err(), Some(DhcpError::Op(3)));
}

#[test]
fn a_message_shorter_than_the_fixed_part_is_no_message() {
    let bytes = Reply::ack(1).bytes();
    for cut in [0usize, 1, 100, FIXED_LEN, FIXED_LEN + 3] {
        assert!(
            matches!(
                Message::parse(&bytes[..cut]),
                Err(DhcpError::Wire(WireError::OutOfBounds { .. }))
            ),
            "a message read out of {cut} bytes"
        );
    }
    // The fixed part and the cookie and nothing else is a message with an
    // option block that has no end marker.
    let message = Message::parse(&bytes[..FIXED_LEN + 4]).expect("a message");
    assert_eq!(message.message_type(), Err(DhcpError::MissingEnd));
}

#[test]
fn a_message_that_does_not_fit_its_buffer_says_so() {
    let message = Message::request(1, MAC);
    for room in [0usize, 100, MIN_MESSAGE_LEN - 1] {
        let mut buffer = vec![0u8; room];
        let mut writer = Writer::new(&mut buffer);
        assert!(matches!(
            message.write(&mut writer),
            Err(DhcpError::Wire(WireError::OutOfBounds { .. }))
        ));
    }
}

#[test]
fn an_option_block_longer_than_the_padding_is_written_whole() {
    let block = vec![OptionCode::PAD.get(); 200];
    let mut message = Message::request(1, MAC);
    message.options = &block;
    let mut buffer = [0u8; MAX_MESSAGE_LEN];
    let mut writer = Writer::new(&mut buffer);
    message.write(&mut writer).expect("room");
    assert_eq!(writer.position(), FIXED_LEN + 4 + 200);
}

#[test]
fn every_message_type_has_the_number_rfc_2132_gives_it() {
    let cases = [
        (MessageType::DISCOVER, 1),
        (MessageType::OFFER, 2),
        (MessageType::REQUEST, 3),
        (MessageType::DECLINE, 4),
        (MessageType::ACK, 5),
        (MessageType::NAK, 6),
        (MessageType::RELEASE, 7),
        (MessageType::INFORM, 8),
    ];
    for (kind, number) in cases {
        assert_eq!(kind.get(), number);
        assert_eq!(MessageType::new(number), kind);
    }
}

#[test]
fn an_option_that_stands_twice_is_read_as_the_last_of_them() {
    let bytes = Reply::ack(1)
        .with(opt(OptionCode::MESSAGE_TYPE, &[MessageType::NAK.get()]))
        .bytes();
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.message_type(), Ok(MessageType::NAK));
}

#[test]
fn no_byte_stream_makes_the_parser_panic_or_step_outside_it() {
    check(
        "dhcp_message_is_read_inside_its_bytes",
        &bytes(0..=320),
        |input| {
            let Ok(message) = Message::parse(input) else {
                return Ok(());
            };
            assert!(
                message.options.len() <= input.len(),
                "an option block longer than the message"
            );
            for option in message.options() {
                let Ok((_, body)) = option else { break };
                assert!(body.len() <= input.len(), "a body longer than the message");
            }
            Ok(())
        },
    );
}
