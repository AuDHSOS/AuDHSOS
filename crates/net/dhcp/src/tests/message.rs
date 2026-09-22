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
    FILE_LEN, FIXED_LEN, HARDWARE_ETHERNET, MAGIC_COOKIE, MAX_MESSAGE_LEN, MIN_MESSAGE_LEN,
    Message, MessageType, Op, SNAME_LEN,
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
            for option in message.all_options() {
                let Ok((_, body)) = option else { break };
                assert!(body.len() <= input.len(), "a body longer than the message");
            }
            Ok(())
        },
    );
}

#[test]
fn no_overloaded_message_makes_the_option_walk_panic_or_run_on() {
    let header = with_fields(&[], &[], &[]);
    check(
        "dhcp_overloaded_options_are_read_inside_their_bytes",
        &bytes(1..=320),
        |input| {
            // The first byte picks the overload value; the next 192 fill
            // `sname` and `file`; the rest follows option 52.
            let mut names = input.get(1..).unwrap_or(&[]).to_vec();
            let tail = names.split_off(names.len().min(SNAME_LEN + FILE_LEN));
            names.resize(SNAME_LEN + FILE_LEN, 0);
            let mut bytes = header[..44].to_vec();
            bytes.extend_from_slice(&names);
            bytes.extend_from_slice(&MAGIC_COOKIE);
            bytes.extend_from_slice(&[52, 1, 1 + input[0] % 3]);
            bytes.extend_from_slice(&tail);
            let message = Message::parse(&bytes).expect("a message");
            let bound = SNAME_LEN + FILE_LEN + message.options.len() + 3;
            let mut count = 0usize;
            for option in message.all_options() {
                count += 1;
                assert!(count <= bound, "more options than bytes");
                let Ok((_, body)) = option else { break };
                assert!(body.len() <= bytes.len(), "a body longer than the message");
            }
            Ok(())
        },
    );
}

/// A reply with `options` in the option field, `file`, and `sname`.
fn with_fields(options: &[u8], file: &[u8], sname: &[u8]) -> Vec<u8> {
    let mut message = Message::request(1, MAC);
    message.op = Op::REPLY;
    message.options = options;
    message.file = file;
    message.sname = sname;
    let mut buffer = [0u8; MIN_MESSAGE_LEN];
    let mut writer = Writer::new(&mut buffer);
    message.write(&mut writer).expect("room");
    writer.finish().to_vec()
}

#[test]
fn sname_and_file_read_back_at_their_offsets() {
    let bytes = with_fields(&[255], b"boot.example", b"server");
    assert_eq!(&bytes[44..50], b"server");
    assert_eq!(&bytes[108..120], b"boot.example");
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.sname.len(), SNAME_LEN);
    assert_eq!(message.file.len(), FILE_LEN);
    assert_eq!(&message.sname[..6], b"server");
    assert_eq!(&message.file[..12], b"boot.example");
}

#[test]
fn a_field_longer_than_sname_or_file_is_refused() {
    let long = [0u8; FILE_LEN + 1];
    for (file, sname) in [(&long[..], &[][..]), (&[][..], &long[..=SNAME_LEN])] {
        let mut message = Message::request(1, MAC);
        message.file = file;
        message.sname = sname;
        let mut buffer = [0u8; MIN_MESSAGE_LEN];
        let mut writer = Writer::new(&mut buffer);
        assert_eq!(
            message.write(&mut writer),
            Err(DhcpError::Field(file.len().max(sname.len())))
        );
    }
}

#[test]
fn a_message_type_in_file_is_read_where_option_52_names_file() {
    let bytes = Reply::ack(1).in_file(OptionCode::MESSAGE_TYPE).bytes();
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.message_type(), Ok(MessageType::ACK));
}

#[test]
fn a_message_type_in_sname_is_read_where_option_52_names_sname() {
    let bytes = Reply::ack(1).in_sname(OptionCode::MESSAGE_TYPE).bytes();
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.message_type(), Ok(MessageType::ACK));
}

#[test]
fn sname_is_read_after_file() {
    let mut reply = Reply::ack(1).in_file(OptionCode::MESSAGE_TYPE);
    reply
        .sname
        .push(opt(OptionCode::MESSAGE_TYPE, &[MessageType::NAK.get()]));
    let bytes = reply.bytes();
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.message_type(), Ok(MessageType::NAK));
}

#[test]
fn file_and_sname_without_option_52_are_not_options() {
    let bytes = with_fields(&[255], &[53, 1, 5, 255], &[53, 1, 5, 255]);
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.message_type(), Err(DhcpError::MissingMessageType));
}

#[test]
fn option_52_in_file_is_not_read() {
    // Option 52 in `file` names `sname`; only the option field counts.
    let bytes = with_fields(&[52, 1, 1, 255], &[52, 1, 2, 255], &[53, 1, 5, 255]);
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.message_type(), Err(DhcpError::MissingMessageType));
}

#[test]
fn an_option_52_that_is_not_1_2_or_3_is_refused() {
    for value in [0, 4, 255] {
        let bytes = with_fields(&[53, 1, 5, 52, 1, value, 255], &[255], &[255]);
        let message = Message::parse(&bytes).expect("a message");
        assert_eq!(message.message_type(), Err(DhcpError::Overload(value)));
    }
    let bytes = with_fields(&[53, 1, 5, 52, 2, 1, 1, 255], &[255], &[255]);
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(
        message.message_type(),
        Err(DhcpError::OptionLength {
            code: OptionCode::OVERLOAD,
            len: 2
        })
    );
}

#[test]
fn a_file_block_without_an_end_marker_is_reported() {
    let file = [0u8; FILE_LEN];
    let bytes = with_fields(&[53, 1, 5, 52, 1, 1, 255], &file, &[]);
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.message_type(), Err(DhcpError::MissingEnd));
}

#[test]
fn a_truncated_option_in_sname_is_reported() {
    let sname = [1u8; SNAME_LEN];
    let bytes = with_fields(&[53, 1, 5, 52, 1, 2, 255], &[], &sname);
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(
        message.message_type(),
        Err(DhcpError::TruncatedOption(OptionCode::SUBNET_MASK))
    );
}

#[test]
fn an_error_in_the_option_field_is_reported_once() {
    // Padding to the minimum length leaves the field without an end marker.
    let bytes = with_fields(&[53, 1, 5], &[], &[]);
    let message = Message::parse(&bytes).expect("a message");
    let items: Vec<_> = message.all_options().collect();
    assert_eq!(items, vec![Err(DhcpError::MissingEnd)]);
}
