// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The rollback journal a reader must play back, `src/pager.c`.

use alloc::vec::Vec;

use crate::db::Database;
use crate::error::Error;
use crate::journal::Journal;
use crate::value::Value;

/// Where the header's sector size stands.
const SECTOR: usize = 20;

/// Where the header's page size stands.
const PAGE_SIZE: usize = 24;

/// The journal of the fixture, with `change` applied to its bytes.
fn changed(change: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut bytes = super::JOURNAL.to_vec();
    change(&mut bytes);
    bytes
}

/// A journal written by hand: a header naming `sector`, `page_size` and
/// `pages`, padded to the sector, then one record per page given.
///
/// `count` is what the header claims; `None` writes the count a process
/// working without sync writes, which says the rest of the file is
/// records and no header follows them.
fn built(
    sector: u32,
    page_size: u32,
    pages: u32,
    count: Option<u32>,
    records: &[(u32, Vec<u8>)],
) -> Vec<u8> {
    let nonce = 0x5eed_1234u32;
    let mut out = alloc::vec![0u8; usize::try_from(sector).unwrap()];
    out[..8].copy_from_slice(&[0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7]);
    for (at, word) in [
        (8, count.unwrap_or(0xffff_ffff)),
        (12, nonce),
        (16, pages),
        (20, sector),
        (24, page_size),
    ] {
        out[at..at + 4].copy_from_slice(&word.to_be_bytes());
    }
    for (page, content) in records {
        out.extend_from_slice(&page.to_be_bytes());
        out.extend_from_slice(content);
        out.extend_from_slice(&sum(nonce, content).to_be_bytes());
    }
    out
}

/// The checksum of one record, which the test computes for itself so
/// that the reader is held to `pager_cksum` and not to its own
/// arithmetic.
fn sum(nonce: u32, page: &[u8]) -> u32 {
    let mut cksum = nonce;
    let mut at = page.len();
    while at > 200 {
        at -= 200;
        cksum = cksum.wrapping_add(u32::from(page[at]));
    }
    cksum
}

/// What the fixture answers for `sql`, read with its journal.
fn rolled(sql: &[u8]) -> Vec<Vec<Value>> {
    let journal = Journal::open(super::JOURNAL);
    let database = Database::open_with_journal(super::ROLLBACK, &journal).unwrap();
    database.query(sql).unwrap().rows
}

/// What the fixture answers for `sql`, read without one.
fn unrolled(sql: &[u8]) -> Vec<Vec<Value>> {
    let database = Database::open(super::ROLLBACK).unwrap();
    database.query(sql).unwrap().rows
}

#[test]
fn the_file_alone_holds_the_transaction_the_journal_takes_back() {
    // The file was caught with the update written and the commit not.
    assert_eq!(
        unrolled(b"SELECT count(*), max(b) FROM t"),
        [alloc::vec![Value::Int(4), Value::Text(b"four".to_vec())]]
    );
    assert_eq!(
        rolled(b"SELECT count(*), max(b) FROM t"),
        [alloc::vec![Value::Int(3), Value::Text(b"two".to_vec())]]
    );
}

#[test]
fn every_row_reads_back_as_the_transaction_found_it() {
    assert_eq!(
        rolled(b"SELECT a, b FROM t ORDER BY a"),
        [
            alloc::vec![Value::Int(1), Value::Text(b"one".to_vec())],
            alloc::vec![Value::Int(2), Value::Text(b"two".to_vec())],
            alloc::vec![Value::Int(3), Value::Text(b"three".to_vec())],
        ]
    );
}

#[test]
fn the_journal_says_what_it_restores() {
    let journal = Journal::open(super::JOURNAL);
    assert!(journal.hot());
    assert_eq!(journal.page_size(), 4096);
    assert_eq!(journal.pages(), 2);
    assert!(journal.page_bytes(1).is_some());
    assert!(journal.page_bytes(2).is_some());
    assert!(journal.page_bytes(3).is_none());
    // Section 1.2 numbers pages from one.
    assert!(journal.page_bytes(0).is_none());
}

#[test]
fn a_journal_too_short_to_hold_a_header_is_not_hot() {
    // What a journal mode of `truncate` leaves beside a database it
    // committed is a file of no bytes, and `readJournalHdr` answers
    // `SQLITE_DONE` for one rather than a refusal.
    for short in [0usize, 7, 31] {
        let journal = Journal::open(&super::JOURNAL[..short]);
        assert!(!journal.hot(), "{short} bytes");
        assert!(journal.page_bytes(1).is_none());
    }
}

#[test]
fn a_journal_whose_magic_is_not_the_magic_is_not_hot() {
    // SQLite writes zeros there until the records are on disk, so such a
    // journal sits beside a database that was never changed.
    let bytes = changed(|bytes| bytes[0] = 0);
    let journal = Journal::open(&bytes);
    assert!(!journal.hot());
    assert_eq!(journal.pages(), 0);
    assert_eq!(journal.page_size(), 0);
    assert!(journal.page_bytes(1).is_none());
    let database = Database::open_with_journal(super::ROLLBACK, &journal).unwrap();
    assert_eq!(
        database.query(b"SELECT count(*) FROM t").unwrap().rows,
        [alloc::vec![Value::Int(4)]]
    );
}

#[test]
fn a_header_whose_sizes_are_out_of_range_stops_the_playback() {
    // `readJournalHdr` refuses each of the four on its own, so each is
    // reached with the other three left as the fixture has them.
    for (at, byte, why) in [
        (SECTOR + 2, 0x00u8, "a sector of 0, below the smallest"),
        (SECTOR + 2, 0x03, "a sector of 768, not a power of two"),
        (
            PAGE_SIZE + 2,
            0x01,
            "a page size of 256, below the smallest",
        ),
        (
            PAGE_SIZE + 2,
            0x18,
            "a page size of 6144, not a power of two",
        ),
    ] {
        let bytes = changed(|bytes| bytes[at] = byte);
        assert!(!Journal::open(&bytes).hot(), "{why}");
    }
    // A sector past the largest, which is 0x01000000.
    let bytes = changed(|bytes| bytes[SECTOR] = 0x02);
    assert!(!Journal::open(&bytes).hot());
    // A page size past the largest, which is 65536.
    let bytes = changed(|bytes| bytes[PAGE_SIZE + 1] = 0x02);
    assert!(!Journal::open(&bytes).hot());
}

#[test]
fn a_count_that_names_no_number_reads_to_the_end_of_the_file() {
    // A process working without sync writes 0xffffffff, and the records
    // are then however many the file holds.
    let bytes = built(
        512,
        512,
        4,
        None,
        &[(1, alloc::vec![7u8; 512]), (2, alloc::vec![9u8; 512])],
    );
    let journal = Journal::open(&bytes);
    assert!(journal.hot());
    assert_eq!(journal.page_bytes(1), Some([7u8; 512].as_slice()));
    assert_eq!(journal.page_bytes(2), Some([9u8; 512].as_slice()));
}

#[test]
fn records_that_end_on_a_sector_are_followed_by_no_padding() {
    // A sector of 32 and a page of 512 make a record 520 bytes long, so
    // four of them end where the next header would begin.
    let records: Vec<(u32, Vec<u8>)> = (1..=4)
        .map(|at| (at, alloc::vec![u8::try_from(at).unwrap(); 512]))
        .collect();
    let bytes = built(32, 512, 4, Some(4), &records);
    assert_eq!(bytes.len() % 32, 0, "the records end on a sector");
    let journal = Journal::open(&bytes);
    assert_eq!(journal.page_bytes(4), Some([4u8; 512].as_slice()));
}

#[test]
fn what_follows_the_records_and_is_not_a_header_ends_the_playback() {
    // Far enough past the records for a header to be read there, so the
    // walk reads one and finds bytes that are not a magic.
    let mut bytes = super::JOURNAL.to_vec();
    bytes.resize(512 + 2 * 4104 + 1024, 0);
    let journal = Journal::open(&bytes);
    assert!(journal.page_bytes(1).is_some(), "the records still read");
    assert!(journal.page_bytes(2).is_some());
}

#[test]
fn a_second_header_carries_records_of_its_own() {
    // A transaction that fills one header's records writes another, at
    // the next sector. The first header is the one that says how large
    // the database was, and a page the first run holds is not taken
    // again by the second.
    let first = built(512, 512, 4, Some(1), &[(1, alloc::vec![7u8; 512])]);
    let second = built(
        512,
        512,
        99,
        Some(2),
        &[(2, alloc::vec![8u8; 512]), (1, alloc::vec![9u8; 512])],
    );
    let mut bytes = first;
    let over = bytes.len() % 512;
    bytes.resize(bytes.len() + 512 - over, 0);
    bytes.extend_from_slice(&second);
    let journal = Journal::open(&bytes);
    assert_eq!(journal.pages(), 4, "the first header says how large it was");
    assert_eq!(journal.page_bytes(1), Some([7u8; 512].as_slice()));
    assert_eq!(journal.page_bytes(2), Some([8u8; 512].as_slice()));
}

#[test]
fn a_record_whose_page_number_is_cut_off_stops_the_playback() {
    let journal = Journal::open(&super::JOURNAL[..512 + 4104 + 2]);
    assert!(journal.page_bytes(1).is_some());
    assert!(journal.page_bytes(2).is_none());
}

#[test]
fn a_record_whose_checksum_is_cut_off_stops_the_playback() {
    let journal = Journal::open(&super::JOURNAL[..512 + 4104 + 4 + 4096]);
    assert!(journal.page_bytes(1).is_some());
    assert!(journal.page_bytes(2).is_none());
}

#[test]
fn a_page_the_database_did_not_have_is_passed_over() {
    // The playback truncates to the header's page count, so a record
    // naming a page past it restores nothing.
    let bytes = built(
        512,
        512,
        1,
        Some(2),
        &[(1, alloc::vec![7u8; 512]), (9, alloc::vec![9u8; 512])],
    );
    let journal = Journal::open(&bytes);
    assert_eq!(journal.pages(), 1);
    assert!(journal.page_bytes(1).is_some());
    assert!(journal.page_bytes(9).is_none());
}

#[test]
fn a_page_journaled_twice_keeps_what_it_began_with() {
    // A page written twice in one transaction is journaled once with the
    // content the transaction started from, and `pDone` keeps the first.
    let bytes = built(
        512,
        512,
        1,
        Some(2),
        &[(1, alloc::vec![7u8; 512]), (1, alloc::vec![9u8; 512])],
    );
    let journal = Journal::open(&bytes);
    assert_eq!(journal.page_bytes(1), Some([7u8; 512].as_slice()));
}

#[test]
fn a_record_whose_checksum_is_not_its_own_stops_the_playback() {
    // A byte of the first record's page, which the checksum covers: the
    // checksum reads every two-hundredth byte back from the end.
    let at = 512 + 4 + 4096 - 200;
    let bytes = changed(|bytes| bytes[at] ^= 0xff);
    let journal = Journal::open(&bytes);
    assert!(journal.hot(), "the header is still a header");
    assert!(journal.page_bytes(1).is_none(), "no record was taken");
}

#[test]
fn a_record_naming_page_zero_stops_the_playback() {
    let bytes = changed(|bytes| {
        bytes[512] = 0;
        bytes[513] = 0;
        bytes[514] = 0;
        bytes[515] = 0;
    });
    let journal = Journal::open(&bytes);
    assert!(journal.page_bytes(1).is_none());
    assert!(journal.page_bytes(2).is_none());
}

#[test]
fn a_record_cut_short_stops_the_playback() {
    // The second record, whose page the file no longer reaches the end
    // of.
    let bytes = super::JOURNAL[..512 + 4104 + 8].to_vec();
    let journal = Journal::open(&bytes);
    assert!(journal.page_bytes(1).is_some(), "the first record is whole");
    assert!(journal.page_bytes(2).is_none(), "the second is not");
}

#[test]
fn a_page_size_the_header_does_not_name_is_refused() {
    // The database header names 4096, so a journal of 512 is a pair that
    // does not belong together.
    let bytes = changed(|bytes| {
        bytes[PAGE_SIZE + 1] = 0x00;
        bytes[PAGE_SIZE + 2] = 0x02;
        bytes[PAGE_SIZE + 3] = 0x00;
    });
    let journal = Journal::open(&bytes);
    assert_eq!(
        Database::open_with_journal(super::ROLLBACK, &journal).unwrap_err(),
        crate::db::Error::Image(Error::PageSize(512))
    );
}

#[test]
fn the_journal_a_committed_transaction_leaves_changes_nothing() {
    // Two of the six journal modes leave the file behind: `persist`
    // zeroes its header and `truncate` cuts it to no bytes. Neither is
    // hot, so the database beside it reads as it stands.
    for (bytes, journal, name) in [
        (super::PERSIST, super::PERSIST_JOURNAL, "persist"),
        (super::TRUNCATE, super::TRUNCATE_JOURNAL, "truncate"),
    ] {
        let journal = Journal::open(journal);
        assert!(!journal.hot(), "{name}");
        let database = Database::open_with_journal(bytes, &journal).unwrap();
        assert_eq!(
            database
                .query(b"SELECT count(*), max(t) FROM m")
                .unwrap()
                .rows,
            [alloc::vec![Value::Int(3), Value::Text(b"two".to_vec())]],
            "{name}"
        );
    }
}

#[test]
fn a_journal_played_back_gives_the_database_the_transaction_found() {
    // `rollback.db` is a database caught between the sync of its
    // journal and the sync of its own pages. Playing the journal back
    // over it gives `rolled.db`, which is the file the transaction
    // began with, byte for byte.
    let journal = Journal::open(super::JOURNAL);
    let rolled = journal.rolled_back(super::ROLLBACK);
    assert_eq!(rolled.len(), super::ROLLED.len());
    assert_eq!(rolled, super::ROLLED);
    // A journal that is not hot restores nothing, so the database is
    // the one the caller handed in.
    let cold = Journal::open(&[]);
    assert_eq!(cold.rolled_back(super::ROLLBACK), super::ROLLBACK);
}

#[test]
fn a_playback_shortens_a_database_the_transaction_had_grown() {
    // The journal says the database had two pages, so a file that grew
    // to three is cut back to the two the transaction found.
    let journal = Journal::open(super::JOURNAL);
    let page = usize::try_from(journal.page_size()).unwrap();
    let mut grown = super::ROLLBACK.to_vec();
    grown.resize(page * 3, 7);
    assert_eq!(journal.rolled_back(&grown), super::ROLLED);
    // And one cut short is filled back out to them.
    let short = super::ROLLBACK.get(..page).unwrap().to_vec();
    let back = journal.rolled_back(&short);
    assert_eq!(back.len(), super::ROLLED.len());
    assert_eq!(back.get(..page), super::ROLLED.get(..page));
}
