// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::message`.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE, WORD, WORD_COUNT};
use audhsos_abi::layout::{MAX_MESSAGE_HANDLES, MAX_MESSAGE_WORDS};
use audhsos_abi::{Error, Handle, MessageError};

use crate::message::{CodecError, MAX_BYTES, Reader, Writer};

/// A buffer of zeros to work on.
fn buffer() -> [u8; SIZE] {
    [0; SIZE]
}

/// A handle every table could hand out.
fn handle(index: u32) -> Handle {
    Handle::new(index, 1).unwrap()
}

/// The label the tests send under.
const LABEL: u64 = 0x1234;

#[test]
fn words_come_back_in_the_order_they_were_written() {
    let mut bytes = buffer();
    let mut writer = Writer::new();
    {
        let mut view = BufferMut::new(&mut bytes);
        writer.word(&mut view, 11).unwrap();
        writer.word(&mut view, 22).unwrap();
        writer.finish(&mut view, LABEL).unwrap();
    }
    assert_eq!(writer.words(), 2);
    assert_eq!(writer.handles(), 0);

    let mut reader = Reader::new(Buffer::new(&bytes)).unwrap();
    assert_eq!(reader.label(), LABEL);
    assert_eq!(reader.remaining_words(), 2);
    assert_eq!(reader.word().unwrap(), 11);
    assert_eq!(reader.word().unwrap(), 22);
    assert_eq!(reader.remaining_words(), 0);
    assert_eq!(reader.word(), Err(CodecError::Truncated));
}

#[test]
fn a_byte_string_comes_back_with_its_length() {
    let mut bytes = buffer();
    let mut writer = Writer::new();
    {
        let mut view = BufferMut::new(&mut bytes);
        writer.bytes(&mut view, b"console").unwrap();
        writer.finish(&mut view, LABEL).unwrap();
    }
    // One word for the length and one for the seven bytes.
    assert_eq!(writer.words(), 2);

    let mut reader = Reader::new(Buffer::new(&bytes)).unwrap();
    let mut into = [0u8; 32];
    let len = reader.bytes(&mut into).unwrap();
    assert_eq!(len, 7);
    assert_eq!(into.get(..len).unwrap(), b"console");
}

#[test]
fn a_byte_string_of_no_bytes_is_a_length_and_nothing_else() {
    let mut bytes = buffer();
    let mut writer = Writer::new();
    {
        let mut view = BufferMut::new(&mut bytes);
        writer.bytes(&mut view, b"").unwrap();
        writer.finish(&mut view, LABEL).unwrap();
    }
    assert_eq!(writer.words(), 1);
    let mut reader = Reader::new(Buffer::new(&bytes)).unwrap();
    let mut into = [0u8; 8];
    assert_eq!(reader.bytes(&mut into).unwrap(), 0);
}

#[test]
fn a_byte_string_that_fills_its_last_word_needs_no_padding_word() {
    let mut bytes = buffer();
    let mut writer = Writer::new();
    {
        let mut view = BufferMut::new(&mut bytes);
        writer.bytes(&mut view, b"12345678").unwrap();
        writer.finish(&mut view, LABEL).unwrap();
    }
    assert_eq!(writer.words(), 2);
    let mut reader = Reader::new(Buffer::new(&bytes)).unwrap();
    let mut into = [0u8; 8];
    assert_eq!(reader.bytes(&mut into).unwrap(), 8);
    assert_eq!(&into, b"12345678");
}

#[test]
fn two_byte_strings_follow_one_another() {
    let mut bytes = buffer();
    let mut writer = Writer::new();
    {
        let mut view = BufferMut::new(&mut bytes);
        writer.bytes(&mut view, b"abc").unwrap();
        writer.bytes(&mut view, b"defgh").unwrap();
        writer.word(&mut view, 9).unwrap();
        writer.finish(&mut view, LABEL).unwrap();
    }
    let mut reader = Reader::new(Buffer::new(&bytes)).unwrap();
    let mut into = [0u8; 8];
    assert_eq!(reader.bytes(&mut into).unwrap(), 3);
    assert_eq!(into.get(..3).unwrap(), b"abc");
    assert_eq!(reader.bytes(&mut into).unwrap(), 5);
    assert_eq!(into.get(..5).unwrap(), b"defgh");
    assert_eq!(reader.word().unwrap(), 9);
}

#[test]
fn handles_come_back_in_the_order_they_were_written() {
    let mut bytes = buffer();
    let mut writer = Writer::new();
    {
        let mut view = BufferMut::new(&mut bytes);
        writer.handle(&mut view, handle(1)).unwrap();
        writer.handle(&mut view, handle(2)).unwrap();
        writer.finish(&mut view, LABEL).unwrap();
    }
    assert_eq!(writer.handles(), 2);
    let mut reader = Reader::new(Buffer::new(&bytes)).unwrap();
    assert_eq!(reader.remaining_handles(), 2);
    assert_eq!(reader.handle().unwrap(), handle(1));
    assert_eq!(reader.handle().unwrap(), handle(2));
    assert_eq!(reader.handle(), Err(CodecError::Truncated));
}

#[test]
fn a_fifth_handle_has_no_slot() {
    let mut bytes = buffer();
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = Writer::new();
    for index in 0..MAX_MESSAGE_HANDLES {
        let raw = u32::try_from(index).unwrap().wrapping_add(1);
        writer.handle(&mut view, handle(raw)).unwrap();
    }
    assert_eq!(
        writer.handle(&mut view, handle(9)),
        Err(CodecError::NoHandleSlot)
    );
    assert_eq!(writer.handles(), MAX_MESSAGE_HANDLES);
}

#[test]
fn a_handle_word_that_is_no_handle_is_refused() {
    let mut bytes = buffer();
    {
        let mut view = BufferMut::new(&mut bytes);
        view.set_handle_word(0, 0);
        view.set_label(LABEL);
        view.set_counts(0, 1).unwrap();
    }
    let mut reader = Reader::new(Buffer::new(&bytes)).unwrap();
    assert_eq!(reader.handle(), Err(CodecError::BadHandle(0)));
}

#[test]
fn a_word_past_the_end_of_the_message_area_does_not_fit() {
    let mut bytes = buffer();
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = Writer::new();
    for _ in 0..MAX_MESSAGE_WORDS {
        writer.word(&mut view, 1).unwrap();
    }
    assert_eq!(writer.word(&mut view, 1), Err(CodecError::Full));
    assert_eq!(writer.words(), MAX_MESSAGE_WORDS);
}

#[test]
fn the_widest_byte_string_fits_and_one_byte_more_does_not() {
    let mut bytes = buffer();
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = Writer::new();
    let widest = vec![0x5Au8; MAX_BYTES];
    writer.bytes(&mut view, &widest).unwrap();
    assert_eq!(writer.words(), MAX_MESSAGE_WORDS);

    let mut bytes = buffer();
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = Writer::new();
    let wider = vec![0x5Au8; MAX_BYTES + 1];
    assert_eq!(writer.bytes(&mut view, &wider), Err(CodecError::Full));
    assert_eq!(writer.words(), 0, "a refused string writes no length");
}

#[test]
fn a_byte_string_longer_than_the_destination_is_refused_and_the_place_is_kept() {
    let mut bytes = buffer();
    {
        let mut view = BufferMut::new(&mut bytes);
        let mut writer = Writer::new();
        writer.bytes(&mut view, b"abcdefghij").unwrap();
        writer.finish(&mut view, LABEL).unwrap();
    }
    let mut reader = Reader::new(Buffer::new(&bytes)).unwrap();
    let mut small = [0u8; 4];
    assert_eq!(
        reader.bytes(&mut small),
        Err(CodecError::TooLong {
            len: 10,
            capacity: 4,
        })
    );
    // The reader stands where it stood, so a wider destination still works.
    let mut wide = [0u8; 16];
    assert_eq!(reader.bytes(&mut wide).unwrap(), 10);
    assert_eq!(wide.get(..10).unwrap(), b"abcdefghij");
}

#[test]
fn a_byte_string_whose_words_are_missing_is_truncated() {
    let mut bytes = buffer();
    {
        let mut view = BufferMut::new(&mut bytes);
        view.set_word(0, 16);
        view.set_label(LABEL);
        view.set_counts(1, 0).unwrap();
    }
    let mut reader = Reader::new(Buffer::new(&bytes)).unwrap();
    let mut into = [0u8; 16];
    assert_eq!(reader.bytes(&mut into), Err(CodecError::Truncated));
    // The length word is still where it was.
    assert_eq!(reader.remaining_words(), 1);
}

#[test]
fn a_length_no_message_can_carry_is_refused() {
    let mut bytes = buffer();
    {
        let mut view = BufferMut::new(&mut bytes);
        view.set_word(0, u64::MAX);
        view.set_label(LABEL);
        view.set_counts(1, 0).unwrap();
    }
    let mut reader = Reader::new(Buffer::new(&bytes)).unwrap();
    let mut into = [0u8; 16];
    assert_eq!(
        reader.bytes(&mut into),
        Err(CodecError::BadLength(u64::MAX))
    );

    let mut bytes = buffer();
    {
        let mut view = BufferMut::new(&mut bytes);
        let over = u64::try_from(MAX_BYTES).unwrap().wrapping_add(1);
        view.set_word(0, over);
        view.set_label(LABEL);
        view.set_counts(1, 0).unwrap();
    }
    let mut reader = Reader::new(Buffer::new(&bytes)).unwrap();
    let mut into = [0u8; 16];
    let over = u64::try_from(MAX_BYTES).unwrap().wrapping_add(1);
    assert_eq!(reader.bytes(&mut into), Err(CodecError::BadLength(over)));
}

#[test]
fn a_header_with_impossible_counts_makes_no_reader() {
    let mut bytes = buffer();
    let end = WORD_COUNT.checked_add(WORD).unwrap();
    bytes
        .get_mut(WORD_COUNT..end)
        .unwrap()
        .copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(
        Reader::new(Buffer::new(&bytes)).unwrap_err(),
        CodecError::Header(MessageError::TooManyWords)
    );
}

#[test]
fn the_reader_hands_out_the_header_it_was_built_from() {
    let mut bytes = buffer();
    {
        let mut view = BufferMut::new(&mut bytes);
        let mut writer = Writer::new();
        writer.word(&mut view, 1).unwrap();
        writer.handle(&mut view, handle(4)).unwrap();
        writer.finish(&mut view, LABEL).unwrap();
    }
    let reader = Reader::new(Buffer::new(&bytes)).unwrap();
    let message = reader.message();
    assert_eq!(message.label, LABEL);
    assert_eq!(message.word_count, 1);
    assert_eq!(message.handle_count, 1);
}

#[test]
fn every_error_renders_a_message_and_an_error_code() {
    let cases = [
        CodecError::Full,
        CodecError::NoHandleSlot,
        CodecError::Truncated,
        CodecError::TooLong {
            len: 9,
            capacity: 4,
        },
        CodecError::BadLength(99),
        CodecError::BadHandle(0),
        CodecError::Header(MessageError::TooManyWords),
        CodecError::Header(MessageError::TooManyHandles),
    ];
    for case in cases {
        assert!(!format!("{case}").is_empty(), "{case:?} has no message");
        assert_ne!(Error::from(case).code(), 0);
    }
    assert_eq!(Error::from(CodecError::Full), Error::BufferTooSmall);
    assert_eq!(Error::from(CodecError::Truncated), Error::InvalidArgument);
}
