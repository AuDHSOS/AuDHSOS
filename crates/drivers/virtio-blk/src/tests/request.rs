// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::request`.

#![allow(clippy::indexing_slicing)]

use crate::error::BlkError;
use crate::request::{HEADER_LEN, Kind, Request, S_IOERR, S_OK, S_UNSUPP, status};

#[test]
fn a_header_is_the_type_the_reserved_word_and_the_sector() {
    let mut bytes = [0xFFu8; 16];
    Request::read(0x1234_5678_9ABC)
        .write_header(&mut bytes)
        .expect("a header");
    assert_eq!(&bytes[0..4], &0u32.to_le_bytes(), "VIRTIO_BLK_T_IN");
    assert_eq!(&bytes[4..8], &0u32.to_le_bytes(), "the reserved word");
    assert_eq!(&bytes[8..16], &0x1234_5678_9ABCu64.to_le_bytes());
}

#[test]
fn each_kind_carries_the_number_the_specification_gives_it() {
    assert_eq!(Kind::In.code(), 0);
    assert_eq!(Kind::Out.code(), 1);
    assert_eq!(Kind::Flush.code(), 4);
    let mut bytes = [0u8; 16];
    Request::write(7)
        .write_header(&mut bytes)
        .expect("a header");
    assert_eq!(&bytes[0..4], &1u32.to_le_bytes());
    assert_eq!(&bytes[8..16], &7u64.to_le_bytes());
}

#[test]
fn only_a_read_has_the_device_writing_the_data() {
    assert!(Kind::In.writes_data());
    assert!(!Kind::Out.writes_data());
    assert!(!Kind::Flush.writes_data());
    assert!(!Kind::In.changes_device());
    assert!(Kind::Out.changes_device());
    assert!(Kind::Flush.changes_device());
}

#[test]
fn a_flush_is_of_sector_zero_and_one_of_any_other_is_refused() {
    let mut bytes = [0u8; 16];
    Request::flush().write_header(&mut bytes).expect("a header");
    assert_eq!(&bytes[0..4], &4u32.to_le_bytes());
    assert_eq!(&bytes[8..16], &0u64.to_le_bytes());
    let wrong = Request {
        kind: Kind::Flush,
        sector: 1,
    };
    assert_eq!(
        wrong.write_header(&mut bytes),
        Err(BlkError::FlushSector(1))
    );
}

#[test]
fn a_buffer_shorter_than_the_header_is_refused_and_left_alone() {
    let mut bytes = [0xAAu8; 15];
    assert_eq!(
        Request::read(1).write_header(&mut bytes),
        Err(BlkError::Buffer(15))
    );
    assert_eq!(bytes, [0xAAu8; 15]);
    let mut room = [0xAAu8; 17];
    Request::read(1).write_header(&mut room).expect("a header");
    assert_eq!(room[16], 0xAA, "nothing past the header is touched");
}

#[test]
fn the_header_is_the_length_the_specification_gives_it() {
    assert_eq!(HEADER_LEN, 16);
}

#[test]
fn only_the_ok_status_is_success() {
    assert_eq!(status(S_OK), Ok(()));
    assert_eq!(status(S_IOERR), Err(BlkError::Status(S_IOERR)));
    assert_eq!(status(S_UNSUPP), Err(BlkError::Status(S_UNSUPP)));
    assert_eq!(status(200), Err(BlkError::Status(200)));
    assert_eq!((S_OK, S_IOERR, S_UNSUPP), (0, 1, 2));
}
