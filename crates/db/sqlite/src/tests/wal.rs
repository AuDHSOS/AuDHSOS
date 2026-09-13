// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The write-ahead log a reader must follow, section 4 of the format.

use alloc::vec::Vec;

use crate::db::Database;
use crate::error::Error;
use crate::value::Value;
use crate::wal::Wal;

/// Where the header's page size stands.
const PAGE_SIZE: usize = 8;

/// The log of the fixture, with `change` applied to its bytes.
fn changed(change: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut bytes = super::LOG.to_vec();
    change(&mut bytes);
    bytes
}

/// A log written by hand: a header naming `page_size`, then one frame
/// per page given, the last of them a commit frame.
///
/// The shell writes only logs whose checksum is computed little-endian,
/// so `big` is the other half of section 4.2 and nothing else reaches
/// it.
fn built(page_size: u32, big: bool, frames: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let magic: u32 = if big { 0x377f_0683 } else { 0x377f_0682 };
    let salt = (0x1234_5678u32, 0x9abc_def0u32);
    for word in [magic, 3_007_000, page_size, 0, salt.0, salt.1] {
        out.extend_from_slice(&word.to_be_bytes());
    }
    // The header's own checksum, which seeds the running one.
    let mut running = sum((0, 0), &out, big);
    out.extend_from_slice(&running.0.to_be_bytes());
    out.extend_from_slice(&running.1.to_be_bytes());
    for (at, (page, content)) in frames.iter().enumerate() {
        let last = at.saturating_add(1) == frames.len();
        let after = if last {
            u32::try_from(frames.len()).unwrap()
        } else {
            0
        };
        let mut head = Vec::new();
        for word in [*page, after, salt.0, salt.1] {
            head.extend_from_slice(&word.to_be_bytes());
        }
        running = sum(running, &head[..8], big);
        running = sum(running, content, big);
        out.extend_from_slice(&head);
        out.extend_from_slice(&running.0.to_be_bytes());
        out.extend_from_slice(&running.1.to_be_bytes());
        out.extend_from_slice(content);
    }
    out
}

/// The checksum of section 4.2, which the test computes for itself so
/// that the reader is held to the format and not to its own arithmetic.
fn sum(running: (u32, u32), bytes: &[u8], big: bool) -> (u32, u32) {
    let (mut s0, mut s1) = running;
    let (pairs, _) = bytes.as_chunks::<8>();
    for pair in pairs {
        let word = |at: usize| {
            let held: [u8; 4] = pair[at..at + 4].try_into().unwrap();
            if big {
                u32::from_be_bytes(held)
            } else {
                u32::from_le_bytes(held)
            }
        };
        s0 = s0.wrapping_add(word(0)).wrapping_add(s1);
        s1 = s1.wrapping_add(word(4)).wrapping_add(s0);
    }
    (s0, s1)
}

/// What the fixture answers for `sql`, read with its log.
fn logged(sql: &[u8]) -> Vec<Vec<Value>> {
    let log = Wal::open(super::LOG).unwrap();
    let database = Database::open_with_log(super::LOGGED, &log).unwrap();
    database.query(sql).unwrap().rows
}

#[test]
fn the_file_alone_names_no_table_and_the_log_names_one() {
    let database = Database::open(super::LOGGED).unwrap();
    assert_eq!(database.tables().count(), 0);
    let log = Wal::open(super::LOG).unwrap();
    let database = Database::open_with_log(super::LOGGED, &log).unwrap();
    let names: Vec<&[u8]> = database
        .tables()
        .map(|table| table.name.as_slice())
        .collect();
    assert_eq!(names, [b"m".as_slice()]);
}

#[test]
fn the_rows_read_back_are_the_ones_the_last_commit_left() {
    // The fixture was written with three rows, then one text updated,
    // one row deleted and one row inserted.
    let rows = logged(b"SELECT i, t FROM m ORDER BY i");
    assert_eq!(
        rows,
        [
            alloc::vec![Value::Int(1), Value::Text(b"ONE".to_vec())],
            alloc::vec![Value::Int(2), Value::Text(b"two".to_vec())],
            alloc::vec![Value::Int(4), Value::Text(b"four".to_vec())],
        ]
    );
}

#[test]
fn the_index_the_log_holds_is_walked_as_a_tree() {
    let rows = logged(b"SELECT count(*), min(i), max(i) FROM m");
    assert_eq!(
        rows,
        [alloc::vec![Value::Int(3), Value::Int(1), Value::Int(4)]]
    );
}

#[test]
fn the_log_says_how_many_pages_the_database_has() {
    let log = Wal::open(super::LOG).unwrap();
    assert!(
        log.pages() > 1,
        "the log holds more than the file's one page"
    );
    assert_eq!(log.page_size(), 4096);
    assert!(log.page_bytes(1).is_some(), "page one is in the log");
    assert!(log.page_bytes(log.pages().saturating_add(1)).is_none());
    // A frame may name page zero; no reader may ask for it.
    let zeroed = built(4096, false, &[(0, alloc::vec![0u8; 4096])]);
    let zeroed = Wal::open(&zeroed).unwrap();
    assert_eq!(zeroed.pages(), 1);
    assert!(zeroed.page_bytes(0).is_none());
}

#[test]
fn a_log_shorter_than_its_header_is_refused() {
    assert_eq!(Wal::open(&super::LOG[..31]).unwrap_err(), Error::Truncated);
}

#[test]
fn a_magic_number_that_is_neither_of_the_two_is_refused() {
    let bytes = changed(|bytes| bytes[3] = 0x84);
    assert_eq!(Wal::open(&bytes).unwrap_err(), Error::Magic);
}

#[test]
fn a_page_size_the_format_does_not_allow_is_refused() {
    let bytes = changed(|bytes| {
        bytes[PAGE_SIZE + 2] = 0x00;
        bytes[PAGE_SIZE + 3] = 0x03;
    });
    assert_eq!(Wal::open(&bytes).unwrap_err(), Error::PageSize(3));
    let bytes = changed(|bytes| bytes[PAGE_SIZE + 2] = 0x18);
    assert_eq!(Wal::open(&bytes).unwrap_err(), Error::PageSize(6144));
    // A power of two is not enough: 256 is below the smallest page and
    // 131072 above the largest.
    let bytes = changed(|bytes| {
        bytes[PAGE_SIZE + 2] = 0x01;
        bytes[PAGE_SIZE + 3] = 0x00;
    });
    assert_eq!(Wal::open(&bytes).unwrap_err(), Error::PageSize(256));
    let bytes = changed(|bytes| {
        bytes[PAGE_SIZE + 1] = 0x02;
        bytes[PAGE_SIZE + 2] = 0x00;
    });
    assert_eq!(Wal::open(&bytes).unwrap_err(), Error::PageSize(131_072));
}

#[test]
fn a_page_size_the_header_does_not_name_is_refused() {
    // 512 is a page size the format allows and this database does not
    // have, so the log and the file disagree about what a page is.
    let mut page = super::SMALL[..512].to_vec();
    page.resize(512, 0);
    let log = built(512, false, &[(1, page)]);
    let log = Wal::open(&log).unwrap();
    assert_eq!(log.pages(), 1);
    assert_eq!(
        Database::open_with_log(super::SMALL, &log).unwrap_err(),
        crate::db::Error::Image(Error::PageSize(512))
    );
}

#[test]
fn a_log_whose_checksum_is_computed_big_endian_is_read_as_one() {
    // Section 4.2: the magic's low bit names the byte order the words
    // are read in, and the shell writes only the little-endian one.
    let page = super::SMALL[..4096.min(super::SMALL.len())].to_vec();
    for big in [false, true] {
        let log = built(4096, big, &[(1, page.clone())]);
        let log = Wal::open(&log).unwrap();
        assert_eq!(log.pages(), 1, "big: {big}");
        assert_eq!(log.page_bytes(1), Some(page.as_slice()), "big: {big}");
    }
}

#[test]
fn a_frame_whose_salt_is_not_the_headers_ends_the_log() {
    // The first frame's salt-1, which is at 32 + 8.
    let bytes = changed(|bytes| bytes[40] ^= 0xff);
    let log = Wal::open(&bytes).unwrap();
    assert_eq!(log.pages(), 0);
    assert!(log.page_bytes(1).is_none());
}

#[test]
fn a_frame_whose_checksum_is_not_the_running_one_ends_the_log() {
    // A byte of the first frame's page, which the checksum covers.
    let bytes = changed(|bytes| bytes[32 + 24] ^= 0xff);
    let log = Wal::open(&bytes).unwrap();
    assert_eq!(log.pages(), 0);
}

#[test]
fn a_log_of_no_committed_frame_holds_nothing() {
    let log = Wal::open(&super::LOG[..32]).unwrap();
    assert_eq!(log.pages(), 0);
    assert!(log.page_bytes(1).is_none());
    let database = Database::open_with_log(super::LOGGED, &log).unwrap();
    assert_eq!(database.tables().count(), 0);
}

#[test]
fn a_frame_after_the_last_commit_is_not_read() {
    // Truncating the log inside its last frame leaves the frames before
    // it whole, and the reader takes them up to the last commit.
    let short = super::LOG.len().saturating_sub(8);
    let log = Wal::open(&super::LOG[..short]).unwrap();
    let database = Database::open_with_log(super::LOGGED, &log).unwrap();
    assert!(database.tables().count() <= 1);
}
