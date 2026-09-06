// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The header, the question section, and what a message that is not one
//! does.

use net_wire::{Ipv4Addr, WireError, Writer};
use test_support::generators::bytes;
use test_support::property::check;

use crate::error::DnsError;
use crate::message::{
    Flags, HEADER_LEN, Header, MAX_QUERY_LEN, Message, Opcode, Question, ResponseCode, write_query,
};
use crate::name::Name;
use crate::record::{Class, RecordData, RecordType};
use crate::tests::harness::{Response, a, name};

#[test]
fn a_query_carries_one_question_and_asks_for_recursion() {
    let mut buffer = [0u8; MAX_QUERY_LEN];
    let mut writer = Writer::new(&mut buffer);
    write_query(&mut writer, 0x1234, &name("example.com"), RecordType::A).expect("room");
    let written = writer.finish();
    let message = Message::parse(written).expect("a message");
    assert_eq!(message.header.id, 0x1234);
    assert_eq!(message.header.questions, 1);
    assert_eq!(message.header.answers, 0);
    assert!(!message.header.flags.is_response());
    assert!(message.header.flags.recursion_desired());
    assert_eq!(message.header.flags.opcode(), Opcode::QUERY);
    assert_eq!(message.header.flags.response_code(), ResponseCode::NO_ERROR);
    let question = message.question().expect("a question").expect("one");
    assert_eq!(question.name, name("example.com"));
    assert_eq!(question.record_type, RecordType::A);
    assert_eq!(question.class, Class::IN);
    assert_eq!(message.answers().count(), 0);
}

#[test]
fn an_answer_reads_back_as_what_was_written() {
    let response = Response::new(7, "example.com", RecordType::A)
        .with(&a("example.com", Ipv4Addr::new(93, 184, 216, 34)));
    let bytes = response.bytes();
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.header.id, 7);
    assert!(message.header.flags.is_response());
    assert!(!message.header.flags.is_truncated());
    assert_eq!(message.questions().count(), 1);
    let answers: Vec<_> = message
        .answers()
        .map(|record| record.expect("one"))
        .collect();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].name, name("example.com"));
    assert_eq!(answers[0].ttl, 300);
    assert_eq!(
        answers[0].data,
        RecordData::A(Ipv4Addr::new(93, 184, 216, 34))
    );
}

#[test]
fn a_response_with_no_answers_is_a_message() {
    let bytes = Response::new(9, "example.com", RecordType::AAAA)
        .coded(ResponseCode::NAME_ERROR)
        .bytes();
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.header.answers, 0);
    assert_eq!(
        message.header.flags.response_code(),
        ResponseCode::NAME_ERROR
    );
    assert_eq!(message.answers().count(), 0);
}

#[test]
fn every_flag_sits_where_the_header_says() {
    let flags = Flags::new(0x8580);
    assert!(flags.is_response());
    assert_eq!(flags.opcode(), Opcode::QUERY);
    assert!(flags.is_authoritative());
    assert!(!flags.is_truncated());
    assert!(flags.recursion_desired());
    assert!(flags.recursion_available());
    assert_eq!(flags.response_code(), ResponseCode::NO_ERROR);
    assert_eq!(Flags::new(0x1000).opcode(), Opcode::new(2));
    assert_eq!(Flags::new(0x0203).response_code(), ResponseCode::new(3));
    assert!(Flags::new(0x0200).is_truncated());
    assert_eq!(Flags::default().get(), 0);
    assert_eq!(Flags::QUERY.truncated().get(), 0x0300);
    assert_eq!(
        Flags::QUERY
            .with_response_code(ResponseCode::REFUSED)
            .response_code(),
        ResponseCode::REFUSED
    );
}

#[test]
fn every_response_code_reads_as_a_sentence() {
    let cases = [
        (ResponseCode::NO_ERROR, "no error"),
        (ResponseCode::FORMAT_ERROR, "format error"),
        (ResponseCode::SERVER_FAILURE, "server failure"),
        (ResponseCode::NAME_ERROR, "no such name"),
        (ResponseCode::NOT_IMPLEMENTED, "not implemented"),
        (ResponseCode::REFUSED, "refused"),
        (ResponseCode::new(11), "response code 11"),
    ];
    for (code, text) in cases {
        assert_eq!(code.to_string(), text);
    }
    assert_eq!(ResponseCode::REFUSED.get(), 5);
    assert_eq!(Opcode::QUERY.get(), 0);
}

#[test]
fn a_header_reads_back_as_what_was_written() {
    let header = Header {
        id: 0xBEEF,
        flags: Flags::new(0x8180),
        questions: 1,
        answers: 2,
        authorities: 3,
        additionals: 4,
    };
    let mut buffer = [0u8; HEADER_LEN];
    let mut writer = Writer::new(&mut buffer);
    header.write(&mut writer).expect("room");
    assert_eq!(Header::parse(writer.finish()), Ok(header));
}

#[test]
fn a_message_shorter_than_a_header_is_no_message() {
    assert_eq!(
        Message::parse(&[0u8; 11]).err(),
        Some(DnsError::Wire(WireError::OutOfBounds {
            needed: HEADER_LEN,
            available: 11
        }))
    );
    let mut buffer = [0u8; 8];
    let mut writer = Writer::new(&mut buffer);
    assert!(matches!(
        Header::default().write(&mut writer),
        Err(DnsError::Wire(WireError::OutOfBounds { .. }))
    ));
}

#[test]
fn a_question_section_that_is_not_one_ends_the_parse() {
    // A header that promises a question and a message that stops behind
    // it.
    let mut bytes = vec![0u8; HEADER_LEN];
    bytes[4] = 0;
    bytes[5] = 1;
    assert!(matches!(
        Message::parse(&bytes),
        Err(DnsError::Wire(WireError::OutOfBounds { .. }))
    ));
    // And a question whose name is there and whose two fields are not.
    bytes.extend_from_slice(b"\x03www\x00\x00");
    assert!(matches!(
        Message::parse(&bytes),
        Err(DnsError::Wire(WireError::OutOfBounds { .. }))
    ));
}

#[test]
fn a_question_of_no_questions_is_nothing_and_not_an_error() {
    let mut buffer = [0u8; HEADER_LEN];
    let mut writer = Writer::new(&mut buffer);
    Header::default().write(&mut writer).expect("room");
    let bytes = writer.finish().to_vec();
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(message.question(), Ok(None));
}

#[test]
fn a_question_that_does_not_fit_its_buffer_says_so() {
    let question = Question::new(name("example.com"), RecordType::A);
    let mut buffer = [0u8; 4];
    let mut writer = Writer::new(&mut buffer);
    assert!(matches!(
        question.write(&mut writer),
        Err(DnsError::Wire(WireError::OutOfBounds { .. }))
    ));
    let mut small = [0u8; 8];
    let mut writer = Writer::new(&mut small);
    assert!(matches!(
        write_query(&mut writer, 1, &Name::ROOT, RecordType::A),
        Err(DnsError::Wire(WireError::OutOfBounds { .. }))
    ));
}

#[test]
fn no_byte_stream_makes_the_parser_panic_or_step_outside_it() {
    check(
        "dns_message_is_read_inside_its_bytes",
        &bytes(0..=200),
        |input| {
            let Ok(message) = Message::parse(input) else {
                return Ok(());
            };
            for question in message.questions() {
                let Ok(question) = question else { break };
                assert!(question.name.as_bytes().len() <= input.len().max(1) * 2 + 2);
            }
            for record in message.answers() {
                let Ok(record) = record else { break };
                if let RecordData::Other(body) = record.data {
                    assert!(body.len() <= input.len(), "a body longer than the message");
                }
            }
            Ok(())
        },
    );
}
