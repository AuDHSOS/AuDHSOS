// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The device configuration and the header of a frame.

#![allow(clippy::indexing_slicing)]

use crate::error::NetError;
use crate::net::{HEADER_LEN, write_header};

#[test]
fn a_header_is_twelve_bytes_of_zero_and_reaches_no_further() {
    let mut bytes = [0xFFu8; 16];
    write_header(&mut bytes, 0).expect("the buffer holds a header");
    assert_eq!(&bytes[..HEADER_LEN], &[0u8; HEADER_LEN]);
    assert_eq!(bytes[HEADER_LEN], 0xFF);
}

#[test]
fn a_buffer_shorter_than_the_header_is_refused_and_names_itself() {
    let mut bytes = [0xFFu8; HEADER_LEN - 1];
    assert_eq!(write_header(&mut bytes, 3), Err(NetError::Buffer(3)));
}

#[test]
fn an_area_of_no_buffers_says_so() {
    use crate::doubles::RamFrames;
    use crate::frames::Frames;
    let empty = RamFrames::new(0, 64, 0x1000);
    assert!(empty.is_empty());
    assert_eq!(empty.count(), 0);
    assert_eq!(empty.address(0), None);
    assert_eq!(empty.bytes(0), None);
    let held = RamFrames::new(2, 64, 0x1000);
    assert!(!held.is_empty());
    assert_eq!(held.address(1), Some(0x1040));
    assert_eq!(held.address(2), None);
}
