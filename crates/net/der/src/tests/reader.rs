// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The reader against the rules it enforces.

use test_support::generators::bytes;
use test_support::property::check;

use crate::error::DerError;
use crate::reader::{MAX_DEPTH, Reader};
use crate::tag::Tag;

/// A value with the given tag and content, encoded.
fn encode(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let length = content.len();
    if length < 0x80 {
        out.push(u8::try_from(length).unwrap_or(0));
    } else {
        let bytes = length.to_be_bytes();
        let significant: Vec<u8> = bytes
            .iter()
            .copied()
            .skip_while(|byte| *byte == 0)
            .collect();
        out.push(0x80 | u8::try_from(significant.len()).unwrap_or(0));
        out.extend_from_slice(&significant);
    }
    out.extend_from_slice(content);
    out
}

#[test]
fn a_value_in_the_short_form_is_read() {
    let encoded = encode(0x02, &[0x05]);
    let mut reader = Reader::new(&encoded);
    assert_eq!(reader.read_integer(), Ok(&[0x05u8][..]));
    assert!(reader.is_empty());
    assert_eq!(reader.finish(), Ok(()));
}

#[test]
fn a_value_in_the_long_form_is_read() {
    let content = vec![0xABu8; 200];
    let encoded = encode(0x04, &content);
    assert_eq!(encoded.get(..3), Some(&[0x04, 0x81, 0xC8][..]));

    let mut reader = Reader::new(&encoded);
    assert_eq!(reader.read_octet_string(), Ok(&content[..]));

    let long = vec![0xCDu8; 300];
    let encoded = encode(0x04, &long);
    assert_eq!(encoded.get(..4), Some(&[0x04, 0x82, 0x01, 0x2C][..]));
    assert_eq!(Reader::new(&encoded).read_octet_string(), Ok(&long[..]));
}

#[test]
fn a_length_that_the_short_form_could_carry_must_use_it() {
    let encoded = [0x04, 0x81, 0x05, 1, 2, 3, 4, 5];
    assert_eq!(
        Reader::new(&encoded).read_octet_string(),
        Err(DerError::NonMinimalLength)
    );
}

#[test]
fn a_length_with_a_leading_zero_is_refused() {
    let encoded = [0x04, 0x82, 0x00, 0x80];
    assert_eq!(
        Reader::new(&encoded).read_octet_string(),
        Err(DerError::NonMinimalLength)
    );
}

#[test]
fn the_indefinite_form_is_refused() {
    let encoded = [0x30, 0x80, 0x00, 0x00];
    assert_eq!(
        Reader::new(&encoded).read_sequence().err(),
        Some(DerError::IndefiniteLength)
    );
}

#[test]
fn a_length_that_reaches_past_the_input_is_refused() {
    assert_eq!(
        Reader::new(&[0x04, 0x05, 1, 2]).read_octet_string(),
        Err(DerError::LengthOutOfRange)
    );
    assert_eq!(
        Reader::new(&[0x04, 0x81]).read_octet_string(),
        Err(DerError::Truncated)
    );
    assert_eq!(
        Reader::new(&[0x04]).read_octet_string(),
        Err(DerError::Truncated)
    );
    assert_eq!(
        Reader::new(&[]).read_octet_string(),
        Err(DerError::EndOfInput)
    );
}

#[test]
fn a_length_of_more_than_four_bytes_is_refused() {
    let encoded = [0x04, 0x85, 0x01, 0x00, 0x00, 0x00, 0x00];
    assert_eq!(
        Reader::new(&encoded).read_octet_string(),
        Err(DerError::LengthOutOfRange)
    );
}

#[test]
fn the_high_tag_number_form_is_refused() {
    let encoded = [0x1F, 0x81, 0x00, 0x01, 0x00];
    assert_eq!(
        Reader::new(&encoded).read_any().err(),
        Some(DerError::HighTagNumber)
    );
}

#[test]
fn an_integer_must_be_present_non_negative_and_shortest() {
    assert_eq!(
        Reader::new(&encode(0x02, &[])).read_integer(),
        Err(DerError::BadInteger),
        "empty"
    );
    assert_eq!(
        Reader::new(&encode(0x02, &[0x80])).read_integer(),
        Err(DerError::BadInteger),
        "negative"
    );
    assert_eq!(
        Reader::new(&encode(0x02, &[0x00, 0x01])).read_integer(),
        Err(DerError::BadInteger),
        "padded without need"
    );

    assert_eq!(
        Reader::new(&encode(0x02, &[0x00, 0x80])).read_integer(),
        Ok(&[0x80u8][..]),
        "the padding zero is dropped"
    );
    assert_eq!(
        Reader::new(&encode(0x02, &[0x00])).read_integer(),
        Ok(&[0x00u8][..]),
        "zero itself"
    );
    assert_eq!(
        Reader::new(&encode(0x02, &[0x7F, 0xFF])).read_integer(),
        Ok(&[0x7Fu8, 0xFF][..])
    );
}

#[test]
fn a_boolean_is_zero_or_all_ones_and_nothing_else() {
    assert_eq!(
        Reader::new(&encode(0x01, &[0x00])).read_boolean(),
        Ok(false)
    );
    assert_eq!(Reader::new(&encode(0x01, &[0xFF])).read_boolean(), Ok(true));
    for content in [&[0x01u8][..], &[][..], &[0xFF, 0xFF][..], &[0x7F][..]] {
        assert_eq!(
            Reader::new(&encode(0x01, content)).read_boolean(),
            Err(DerError::BadBoolean),
            "content {content:?}"
        );
    }
}

#[test]
fn a_null_carries_nothing() {
    assert_eq!(Reader::new(&encode(0x05, &[])).read_null(), Ok(()));
    assert_eq!(
        Reader::new(&encode(0x05, &[0x00])).read_null(),
        Err(DerError::BadNull)
    );
}

#[test]
fn a_bit_string_declares_how_many_bits_it_does_not_use() {
    let encoded = encode(0x03, &[0x00, 0xDE, 0xAD]);
    let string = Reader::new(&encoded)
        .read_bit_string()
        .expect("no unused bits");
    assert_eq!(string.unused, 0);
    assert_eq!(string.bytes, &[0xDE, 0xAD]);
    assert_eq!(string.whole_bytes(), Ok(&[0xDEu8, 0xAD][..]));

    let encoded = encode(0x03, &[0x03, 0xDE, 0xA8]);
    let string = Reader::new(&encoded)
        .read_bit_string()
        .expect("three unused bits, all zero");
    assert_eq!(string.unused, 3);
    assert_eq!(string.whole_bytes(), Err(DerError::BadBitString));

    for content in [
        &[][..],                 // no count at all
        &[0x08, 0xFF][..],       // a count above seven
        &[0x01][..],             // a count without bytes
        &[0x03, 0xDE, 0xAF][..], // unused bits that are not zero
    ] {
        assert_eq!(
            Reader::new(&encode(0x03, content)).read_bit_string().err(),
            Some(DerError::BadBitString),
            "content {content:?}"
        );
    }

    let empty = encode(0x03, &[0x00]);
    let string = Reader::new(&empty)
        .read_bit_string()
        .expect("an empty string with no unused bits");
    assert!(string.bytes.is_empty());
}

#[test]
fn an_object_identifier_is_compared_as_its_bytes() {
    let rsa = [0x2Au8, 0x86, 0x48, 0x86, 0xF7, 0x0D];
    let encoded = encode(0x06, &rsa);
    let oid = Reader::new(&encoded)
        .read_object_identifier()
        .expect("a well formed identifier");
    assert_eq!(oid.as_bytes(), &rsa);

    let other = encode(0x06, &[0x2Au8, 0x86, 0x48, 0x86, 0xF7, 0x0E]);
    let other = Reader::new(&other)
        .read_object_identifier()
        .expect("a well formed identifier");
    assert_ne!(oid, other);

    for content in [
        &[][..],                 // empty
        &[0x2A, 0x86][..],       // ends in a continuation byte
        &[0x2A, 0x80, 0x01][..], // a component with a leading zero
        &[0x80, 0x01][..],       // the first component with a leading zero
    ] {
        assert_eq!(
            Reader::new(&encode(0x06, content))
                .read_object_identifier()
                .err(),
            Some(DerError::BadObjectIdentifier),
            "content {content:?}"
        );
    }
}

#[test]
fn a_sequence_is_read_element_by_element_and_must_be_finished() {
    let inner = [encode(0x02, &[0x01]), encode(0x02, &[0x02])].concat();
    let encoded = encode(0x30, &inner);

    let mut outer = Reader::new(&encoded);
    let mut sequence = outer.read_sequence().expect("a well formed sequence");
    assert_eq!(sequence.read_integer(), Ok(&[0x01u8][..]));
    assert_eq!(sequence.read_integer(), Ok(&[0x02u8][..]));
    assert_eq!(sequence.finish(), Ok(()));
    assert_eq!(outer.finish(), Ok(()));

    let mut partial = Reader::new(&encoded);
    let mut sequence = partial.read_sequence().expect("a well formed sequence");
    assert_eq!(sequence.read_integer(), Ok(&[0x01u8][..]));
    assert_eq!(sequence.finish(), Err(DerError::TrailingData));
}

#[test]
fn a_set_and_an_explicit_context_value_are_read() {
    let inner = encode(0x02, &[0x07]);
    let set = encode(0x31, &inner);
    let mut reader = Reader::new(&set);
    let mut contents = reader.read_set().expect("a well formed set");
    assert_eq!(contents.read_integer(), Ok(&[0x07u8][..]));

    let tagged = encode(0xA3, &inner);
    let mut reader = Reader::new(&tagged);
    let mut contents = reader.read_context(3).expect("an explicit context value");
    assert_eq!(contents.read_integer(), Ok(&[0x07u8][..]));

    let mut reader = Reader::new(&tagged);
    assert_eq!(reader.read_context(2).err(), Some(DerError::UnexpectedTag));
}

#[test]
fn a_primitive_tag_cannot_be_read_as_a_constructed_one() {
    let encoded = encode(0x02, &[0x01]);
    let mut reader = Reader::new(&encoded);
    assert_eq!(
        reader.read_constructed(Tag::INTEGER).err(),
        Some(DerError::WrongForm)
    );
}

#[test]
fn an_optional_value_is_taken_only_when_its_tag_is_there() {
    let present = [encode(0x01, &[0xFF]), encode(0x02, &[0x09])].concat();
    let mut reader = Reader::new(&present);
    assert_eq!(reader.read_optional(Tag::BOOLEAN), Ok(Some(&[0xFFu8][..])));
    assert_eq!(reader.read_integer(), Ok(&[0x09u8][..]));

    let absent = encode(0x02, &[0x09]);
    let mut reader = Reader::new(&absent);
    assert_eq!(reader.read_optional(Tag::BOOLEAN), Ok(None));
    assert_eq!(reader.read_integer(), Ok(&[0x09u8][..]));
}

#[test]
fn nesting_is_bounded() {
    // A value nested exactly to the bound is read; one deeper is refused.
    let mut inside = encode(0x02, &[0x01]);
    for _ in 0..MAX_DEPTH.saturating_sub(1) {
        inside = encode(0x30, &inside);
    }
    let mut reader = Reader::new(&inside);
    for level in 0..MAX_DEPTH.saturating_sub(1) {
        reader = reader
            .read_sequence()
            .unwrap_or_else(|error| panic!("level {level}: {error}"));
    }
    assert_eq!(reader.read_integer(), Ok(&[0x01u8][..]));

    let deeper = encode(0x30, &inside);
    let mut reader = Reader::new(&deeper);
    let mut error = None;
    for _ in 0..=MAX_DEPTH {
        match reader.read_sequence() {
            Ok(inner) => reader = inner,
            Err(found) => {
                error = Some(found);
                break;
            }
        }
    }
    assert_eq!(error, Some(DerError::TooDeep));
}

#[test]
fn what_was_not_read_is_available_and_trailing_bytes_are_refused() {
    let encoded = [encode(0x02, &[0x01]), vec![0xFF, 0xFF]].concat();
    let mut reader = Reader::new(&encoded);
    assert_eq!(reader.read_integer(), Ok(&[0x01u8][..]));
    assert_eq!(reader.rest(), &[0xFF, 0xFF]);
    assert!(!reader.is_empty());
    assert_eq!(reader.finish(), Err(DerError::TrailingData));
}

#[test]
fn a_tag_that_was_not_expected_leaves_the_reader_where_it_was() {
    let encoded = encode(0x02, &[0x01]);
    let mut reader = Reader::new(&encoded);
    assert_eq!(reader.read_octet_string(), Err(DerError::UnexpectedTag));
    assert_eq!(
        reader.read_integer(),
        Ok(&[0x01u8][..]),
        "the value is still there"
    );
}

#[test]
fn the_tag_type_answers_what_it_is() {
    assert!(Tag::SEQUENCE.is_constructed());
    assert!(!Tag::INTEGER.is_constructed());
    assert!(Tag::context(0, true).is_context());
    assert!(!Tag::SEQUENCE.is_context());
    assert_eq!(Tag::context(3, true).octet(), 0xA3);
    assert_eq!(Tag::context(3, false).octet(), 0x83);
    assert!(Tag::new(0x1F).is_high_form());
    assert!(!format!("{:?}", Tag::SEQUENCE).is_empty());
}

#[test]
fn the_errors_render_a_message() {
    for error in [
        DerError::Truncated,
        DerError::EndOfInput,
        DerError::UnexpectedTag,
        DerError::HighTagNumber,
        DerError::WrongForm,
        DerError::IndefiniteLength,
        DerError::NonMinimalLength,
        DerError::LengthOutOfRange,
        DerError::TrailingData,
        DerError::TooDeep,
        DerError::BadInteger,
        DerError::BadBoolean,
        DerError::BadBitString,
        DerError::BadObjectIdentifier,
        DerError::BadNull,
        DerError::BadTime,
    ] {
        assert!(!format!("{error}").is_empty());
        assert!(!format!("{error:?}").is_empty());
    }
}

#[test]
fn property_no_input_makes_the_reader_panic() {
    check("der_arbitrary", &bytes(0..=64), |input| {
        let mut reader = Reader::new(input);
        for _ in 0..8 {
            match reader.read_any() {
                Ok((tag, content)) => {
                    // A value is a slice of the input, so it re-encodes to
                    // what it came from by construction.
                    if content.len() > input.len() {
                        return Err("a value longer than its input".to_owned());
                    }
                    if tag.is_constructed() {
                        let mut inner = Reader::new(content);
                        let _ = inner.read_any();
                    }
                }
                Err(_) => break,
            }
        }

        let mut typed = Reader::new(input);
        let _ = typed.read_integer();
        let _ = typed.read_boolean();
        let _ = typed.read_bit_string();
        let _ = typed.read_object_identifier();
        let _ = typed.read_time();
        let _ = typed.read_null();
        let _ = typed.read_sequence();
        Ok(())
    });
}
