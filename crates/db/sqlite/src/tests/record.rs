// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::record`.

#![allow(clippy::arithmetic_side_effects)]

use crate::error::Error;
use crate::record::{Record, Serial, Value};

/// A record of the serial types `types` over the body `body`, written the
/// way section 2.1 describes.
fn record(types: &[u64], body: &[u8]) -> Vec<u8> {
    let mut header = Vec::new();
    for code in types {
        let (bytes, len) = crate::tests::varint(*code);
        header.extend_from_slice(&bytes[..len]);
    }
    let (size, size_len) = crate::tests::varint(u64::try_from(header.len() + 1).unwrap());
    let mut out = Vec::new();
    out.extend_from_slice(&size[..size_len]);
    out.extend_from_slice(&header);
    out.extend_from_slice(body);
    out
}

#[test]
fn every_serial_code_is_the_type_and_the_length_the_format_gives_it() {
    let cases = [
        (0, Serial::Null, 0),
        (1, Serial::Int(1), 1),
        (2, Serial::Int(2), 2),
        (3, Serial::Int(3), 3),
        (4, Serial::Int(4), 4),
        (5, Serial::Int(6), 6),
        (6, Serial::Int(8), 8),
        (7, Serial::Real, 8),
        (8, Serial::Zero, 0),
        (9, Serial::One, 0),
        (12, Serial::Blob(0), 0),
        (13, Serial::Text(0), 0),
        (14, Serial::Blob(1), 1),
        (15, Serial::Text(1), 1),
        (100, Serial::Blob(44), 44),
        (101, Serial::Text(44), 44),
    ];
    for (code, serial, len) in cases {
        assert_eq!(Serial::from_code(code), Ok(serial), "code {code}");
        assert_eq!(serial.len(), len, "code {code}");
        assert_eq!(serial.is_empty(), len == 0);
    }
}

#[test]
fn the_two_reserved_serial_types_are_refused() {
    assert_eq!(Serial::from_code(10), Err(Error::SerialType(10)));
    assert_eq!(Serial::from_code(11), Err(Error::SerialType(11)));
}

#[test]
fn an_integer_is_read_at_the_width_it_was_stored_in_and_keeps_its_sign() {
    assert_eq!(Serial::Int(1).read(&[0xff]), Ok(Value::Int(-1)));
    assert_eq!(Serial::Int(1).read(&[0x7f]), Ok(Value::Int(127)));
    assert_eq!(Serial::Int(2).read(&[0xff, 0xfe]), Ok(Value::Int(-2)));
    assert_eq!(
        Serial::Int(3).read(&[0x01, 0x00, 0x00]),
        Ok(Value::Int(65536))
    );
    assert_eq!(
        Serial::Int(6).read(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]),
        Ok(Value::Int(-1))
    );
    assert_eq!(
        Serial::Int(8).read(&[0x80, 0, 0, 0, 0, 0, 0, 0]),
        Ok(Value::Int(i64::MIN))
    );
    assert_eq!(Serial::Zero.read(&[]), Ok(Value::Int(0)));
    assert_eq!(Serial::One.read(&[]), Ok(Value::Int(1)));
    assert_eq!(Serial::Null.read(&[]), Ok(Value::Null));
}

#[test]
fn a_double_is_eight_big_endian_bytes() {
    assert_eq!(
        Serial::Real.read(&1.5f64.to_be_bytes()),
        Ok(Value::Real(1.5))
    );
    assert_eq!(
        Serial::Real.read(&(-0.0f64).to_be_bytes()),
        Ok(Value::Real(-0.0))
    );
}

#[test]
fn text_and_blob_borrow_the_bytes_they_were_stored_in() {
    assert_eq!(Serial::Text(3).read(b"abcdef"), Ok(Value::Text(b"abc")));
    assert_eq!(
        Serial::Blob(2).read(b"\x01\x02\x03"),
        Ok(Value::Blob(b"\x01\x02"))
    );
    assert_eq!(Serial::Text(4).read(b"abc"), Err(Error::Overrun));
    assert_eq!(Serial::Int(8).read(b"short"), Err(Error::Overrun));
}

#[test]
fn a_record_answers_its_values_in_the_order_its_header_names_them() {
    // Types: NULL, one-byte integer, three-byte text, the constant one.
    let bytes = record(&[0, 1, 13 + 3 * 2, 9], b"\x2aabc");
    let parsed = Record::parse(&bytes).unwrap();
    let values: Vec<_> = parsed.values().collect::<Result<_, _>>().unwrap();
    assert_eq!(
        values.as_slice(),
        [
            Value::Null,
            Value::Int(42),
            Value::Text(b"abc"),
            Value::Int(1)
        ]
    );
    assert_eq!(parsed.value(2).unwrap(), Some(Value::Text(b"abc")));
    assert_eq!(parsed.value(4).unwrap(), None);
}

#[test]
fn a_header_that_reaches_past_the_payload_is_refused() {
    assert_eq!(Record::parse(&[]), Err(Error::Varint));
    // A header that claims more bytes than the payload holds.
    assert_eq!(Record::parse(&[40, 0, 0]), Err(Error::Overrun));
    // A header size smaller than the varint that holds it.
    assert_eq!(Record::parse(&[0, 0]), Err(Error::Overrun));
}

#[test]
fn a_value_that_runs_past_the_body_ends_the_walk_once() {
    let bytes = record(&[6], b"\x01\x02");
    let parsed = Record::parse(&bytes).unwrap();
    let mut values = parsed.values();
    assert_eq!(values.next(), Some(Err(Error::Overrun)));
    assert_eq!(values.next(), None);
    assert_eq!(parsed.value(0), Err(Error::Overrun));
}

#[test]
fn a_reserved_type_inside_a_record_ends_the_walk_once() {
    let bytes = record(&[10], b"");
    let parsed = Record::parse(&bytes).unwrap();
    let mut values = parsed.values();
    assert_eq!(values.next(), Some(Err(Error::SerialType(10))));
    assert_eq!(values.next(), None);
}
