// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The types of RFC 4251, section 5: the vectors that section prints, the
//! rules it states, and what a cursor does at the end of a buffer.

use test_support::generators::bytes;
use test_support::property::check;

use crate::error::SshError;
use crate::tests::unhex;
use crate::wire::{Mpint, NameList, Reader, Writer};

/// The five `mpint` examples of RFC 4251, section 5: the value as the
/// document writes it, and the whole string it is stored as.
const MPINTS: [(&str, &str); 5] = [
    ("", "00000000"),
    ("09a378f9b2e332a7", "0000000809a378f9b2e332a7"),
    ("0080", "0000000200 80"),
    ("edcc", "00000002edcc"),
    ("ff21524111", "00000005ff21524111"),
];

/// The three `name-list` examples of the same section.
const NAME_LISTS: [(&[&str], &str); 3] = [
    (&[], "00000000"),
    (&["zlib"], "000000047a6c6962"),
    (&["zlib", "none"], "000000097a6c69622c6e6f6e65"),
];

#[test]
fn integers_come_out_most_significant_byte_first() {
    // The `uint32` example of section 5 is 699921578 as 29 b7 f4 aa.
    let field = unhex("29b7f4aa0102030405060708");
    let mut reader = Reader::new(&field);
    assert_eq!(reader.read_u32(), Ok(699_921_578));
    assert_eq!(reader.read_u64(), Ok(0x0102_0304_0506_0708));
    assert!(reader.is_empty());
    assert_eq!(reader.remaining(), 0);
    assert_eq!(reader.rest(), &[]);
}

#[test]
fn a_string_is_a_length_and_that_many_bytes() {
    // Section 5: the US-ASCII string "testing" is 00 00 00 07 t e s t i n g.
    let field = unhex("0000000774657374696e67");
    let mut reader = Reader::new(&field);
    assert_eq!(reader.read_string(), Ok(b"testing".as_slice()));
    assert_eq!(reader.position(), field.len());

    let mut buffer = [0u8; 11];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(writer.write_string(b"testing"), Ok(()));
    assert_eq!(writer.written(), field.as_slice());
}

#[test]
fn a_byte_and_a_boolean_are_one_byte_each() {
    let field = [0x00, 0x01, 0x2a];
    let mut reader = Reader::new(&field);
    assert_eq!(reader.read_boolean(), Ok(false));
    assert_eq!(reader.read_boolean(), Ok(true));
    // Section 5: every non-zero value MUST be read as true.
    assert_eq!(reader.read_boolean(), Ok(true));

    let mut buffer = [0u8; 3];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(writer.write_boolean(false), Ok(()));
    assert_eq!(writer.write_boolean(true), Ok(()));
    assert_eq!(writer.write_byte(0x2a), Ok(()));
    assert_eq!(writer.written(), &field);
}

#[test]
fn the_mpint_examples_of_the_document_read_and_write_as_it_prints_them() {
    for (value, encoded) in MPINTS {
        let data = unhex(value);
        let field = unhex(encoded);
        let mut reader = Reader::new(&field);
        let read = reader.read_mpint();
        assert_eq!(read.map(|mpint| mpint.as_bytes()), Ok(data.as_slice()));
        assert!(reader.is_empty());

        let mut buffer = [0u8; 16];
        let mut writer = Writer::new(&mut buffer);
        let mpint = Mpint::new(&data);
        assert_eq!(mpint.map(|value| writer.write_mpint(value)), Ok(Ok(())));
        assert_eq!(writer.written(), field.as_slice());
    }
}

#[test]
fn zero_is_a_string_of_no_bytes_and_a_zero_byte_is_not_zero() {
    let zero = Mpint::new(&[]);
    assert_eq!(zero.map(|value| value.is_zero()), Ok(true));
    assert_eq!(zero.map(|value| value.is_negative()), Ok(false));
    assert_eq!(zero.and_then(|value| value.magnitude()), Ok([].as_slice()));
    assert_eq!(Mpint::new(&[0x00]), Err(SshError::Mpint));
}

#[test]
fn an_unnecessary_leading_byte_is_refused_and_a_necessary_one_is_not() {
    // Section 5: a zero byte goes in front of a positive number whose top
    // bit is set, and `ff` in front of a negative one; a second is not
    // unnecessary, it is wrong.
    for (value, negative) in [("0080", false), ("ff21524111", true), ("ff", true)] {
        let data = unhex(value);
        assert_eq!(
            Mpint::new(&data).map(|mpint| mpint.is_negative()),
            Ok(negative)
        );
    }
    assert_eq!(Mpint::new(&unhex("007f")), Err(SshError::Mpint));
    assert_eq!(Mpint::new(&unhex("ffff")), Err(SshError::Mpint));
    assert_eq!(Mpint::new(&unhex("ff80")), Err(SshError::Mpint));
    assert_eq!(Mpint::new(&unhex("000080")), Err(SshError::Mpint));
}

#[test]
fn a_magnitude_is_the_value_without_the_byte_the_top_bit_earned() {
    for (value, magnitude) in [("0080", "80"), ("09a378f9b2e332a7", "09a378f9b2e332a7")] {
        let data = unhex(value);
        assert_eq!(
            Mpint::new(&data).and_then(|mpint| mpint.magnitude()),
            Ok(unhex(magnitude).as_slice())
        );
    }
    let negative = unhex("edcc");
    assert_eq!(
        Mpint::new(&negative).and_then(|mpint| mpint.magnitude()),
        Err(SshError::Negative)
    );
}

#[test]
fn an_unsigned_value_gets_the_mpint_form_of_rfc_8731() {
    // A value whose top bit is set is written with a zero byte in front,
    // one whose top bit is clear without, and leading zeros come off.
    let cases: [(&str, &str); 5] = [
        ("80", "0000000200 80"),
        ("7f", "000000017f"),
        ("00000080", "0000000200 80"),
        ("00", "00000000"),
        ("", "00000000"),
    ];
    for (magnitude, encoded) in cases {
        let mut buffer = [0u8; 8];
        let mut writer = Writer::new(&mut buffer);
        assert_eq!(writer.write_unsigned(&unhex(magnitude)), Ok(()));
        assert_eq!(writer.written(), unhex(encoded).as_slice());
    }
}

#[test]
fn a_written_unsigned_value_reads_back_as_the_same_number() {
    check(
        "an unsigned mpint round-trips",
        &bytes(0..=32),
        |magnitude| {
            let mut buffer = [0u8; 64];
            let mut writer = Writer::new(&mut buffer);
            writer
                .write_unsigned(magnitude)
                .map_err(|error| format!("{error}"))?;
            let written = writer.position();
            let mut reader = Reader::new(buffer.get(..written).unwrap_or(&[]));
            let read = reader.read_mpint().map_err(|error| format!("{error}"))?;
            let value = read.magnitude().map_err(|error| format!("{error}"))?;
            let start = magnitude
                .iter()
                .position(|byte| *byte != 0)
                .unwrap_or(magnitude.len());
            if value == magnitude.get(start..).unwrap_or(&[]) {
                Ok(())
            } else {
                Err(format!("{value:02x?} is not the value that was written"))
            }
        },
    );
}

#[test]
fn the_name_list_examples_of_the_document_read_and_write_as_it_prints_them() {
    for (names, encoded) in NAME_LISTS {
        let field = unhex(encoded);
        let mut reader = Reader::new(&field);
        let list = reader.read_name_list();
        assert_eq!(
            list.map(|list| list.iter().collect::<Vec<&str>>()),
            Ok(names.to_vec())
        );
        assert_eq!(list.map(|list| list.is_empty()), Ok(names.is_empty()));

        let mut buffer = [0u8; 16];
        let mut writer = Writer::new(&mut buffer);
        assert_eq!(writer.write_name_list(names), Ok(()));
        assert_eq!(writer.written(), field.as_slice());
    }
}

#[test]
fn a_list_says_what_it_holds_and_in_which_order() {
    let list = NameList::new(b"curve25519-sha256,diffie-hellman-group14-sha256");
    assert_eq!(
        list.map(|list| list.contains("curve25519-sha256")),
        Ok(true)
    );
    assert_eq!(list.map(|list| list.contains("ssh-dss")), Ok(false));
    assert_eq!(
        list.map(|list| list.as_str()),
        Ok("curve25519-sha256,diffie-hellman-group14-sha256")
    );
    assert_eq!(
        list.map(|list| (&list).into_iter().next()),
        Ok(Some("curve25519-sha256"))
    );
}

#[test]
fn a_name_of_no_length_is_not_a_name() {
    // Section 5: a name MUST have a non-zero length, so a leading, a
    // trailing, and a doubled comma each end a name that is not there.
    assert_eq!(NameList::new(b",zlib"), Err(SshError::NameList));
    assert_eq!(NameList::new(b"zlib,"), Err(SshError::NameList));
    assert_eq!(NameList::new(b"zlib,,none"), Err(SshError::NameList));
    assert_eq!(NameList::new(b","), Err(SshError::NameList));
}

#[test]
fn a_name_outside_us_ascii_is_not_a_name() {
    // Valid UTF-8 that is not US-ASCII, and bytes that are not UTF-8 at
    // all, are both refused, as is the null the section forbids.
    assert_eq!(
        NameList::new("zlib,nöne".as_bytes()),
        Err(SshError::NameList)
    );
    assert_eq!(NameList::new(&[0xff, 0xfe]), Err(SshError::NameList));
    assert_eq!(NameList::new(b"zlib\0"), Err(SshError::NameList));
}

#[test]
fn a_name_the_writer_cannot_separate_is_refused() {
    let mut buffer = [0u8; 32];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        writer.write_name_list(&["zlib,none"]),
        Err(SshError::NameList)
    );
    assert_eq!(writer.write_name_list(&[""]), Err(SshError::NameList));
    assert_eq!(writer.write_name_list(&["nöne"]), Err(SshError::NameList));
    assert_eq!(writer.write_name_list(&["zlib\0"]), Err(SshError::NameList));
    assert_eq!(writer.position(), 0);
}

#[test]
fn a_read_past_the_end_leaves_the_position_where_it_was() {
    let field = unhex("000000097a6c69622c6e6f6e");
    let mut reader = Reader::new(&field);
    assert_eq!(
        reader.read_string(),
        Err(SshError::OutOfBounds {
            needed: 9,
            available: 8
        })
    );
    assert_eq!(
        reader.read_name_list(),
        Err(SshError::OutOfBounds {
            needed: 9,
            available: 8
        })
    );
    assert_eq!(
        reader.read_mpint(),
        Err(SshError::OutOfBounds {
            needed: 9,
            available: 8
        })
    );
    assert_eq!(reader.position(), 0);
    assert_eq!(reader.read_bytes(4), Ok(unhex("00000009").as_slice()));
    assert_eq!(reader.read_u64(), Ok(0x7a6c_6962_2c6e_6f6e));
    assert_eq!(
        reader.read_byte(),
        Err(SshError::OutOfBounds {
            needed: 1,
            available: 0
        })
    );
    assert_eq!(reader.position(), 12);
}

#[test]
fn an_empty_reader_answers_every_read_with_the_bytes_it_wanted() {
    let mut reader = Reader::new(&[]);
    assert_eq!(
        reader.read_byte(),
        Err(SshError::OutOfBounds {
            needed: 1,
            available: 0
        })
    );
    assert_eq!(
        reader.read_boolean(),
        Err(SshError::OutOfBounds {
            needed: 1,
            available: 0
        })
    );
    assert_eq!(
        reader.read_u32(),
        Err(SshError::OutOfBounds {
            needed: 4,
            available: 0
        })
    );
    assert!(reader.is_empty());
}

#[test]
fn a_mpint_that_is_not_canonical_is_refused_where_it_is_read() {
    let field = unhex("00000002007f");
    let mut reader = Reader::new(&field);
    assert_eq!(reader.read_mpint(), Err(SshError::Mpint));
    assert_eq!(reader.position(), 0);

    let list = unhex("000000047a2c2c6c");
    let mut reader = Reader::new(&list);
    assert_eq!(reader.read_name_list(), Err(SshError::NameList));
    assert_eq!(reader.position(), 0);
}

#[test]
fn a_write_that_does_not_fit_writes_nothing() {
    let mut buffer = [0u8; 6];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(writer.write_u32(0x0102_0304), Ok(()));
    assert_eq!(writer.remaining(), 2);
    assert_eq!(
        writer.write_u32(0x0506_0708),
        Err(SshError::OutOfBounds {
            needed: 4,
            available: 2
        })
    );
    assert_eq!(
        writer.write_string(b"no"),
        Err(SshError::OutOfBounds {
            needed: 6,
            available: 2
        })
    );
    assert_eq!(
        writer.write_unsigned(&[0x80]),
        Err(SshError::OutOfBounds {
            needed: 6,
            available: 2
        })
    );
    assert_eq!(
        writer.write_name_list(&["none"]),
        Err(SshError::OutOfBounds {
            needed: 8,
            available: 2
        })
    );
    assert_eq!(writer.written(), &[0x01, 0x02, 0x03, 0x04]);
    assert_eq!(
        writer.write_u64(0),
        Err(SshError::OutOfBounds {
            needed: 8,
            available: 2
        })
    );
}

#[test]
fn every_field_written_reads_back_as_itself() {
    let mut buffer = [0u8; 64];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(writer.write_byte(0x14), Ok(()));
    assert_eq!(writer.write_u64(0x0011_2233_4455_6677), Ok(()));
    assert_eq!(writer.write_string(b"ssh-ed25519"), Ok(()));
    assert_eq!(writer.write_name_list(&["none"]), Ok(()));
    assert_eq!(writer.write_bytes(&[0xde, 0xad]), Ok(()));
    let written = writer.position();

    let mut reader = Reader::new(buffer.get(..written).unwrap_or(&[]));
    assert_eq!(reader.read_byte(), Ok(0x14));
    assert_eq!(reader.read_u64(), Ok(0x0011_2233_4455_6677));
    assert_eq!(reader.read_string(), Ok(b"ssh-ed25519".as_slice()));
    assert_eq!(
        reader.read_name_list().map(|list| list.as_str()),
        Ok("none")
    );
    assert_eq!(reader.rest(), &[0xde, 0xad]);
    assert_eq!(reader.read_bytes(2), Ok([0xde, 0xad].as_slice()));
    assert!(reader.is_empty());
}
