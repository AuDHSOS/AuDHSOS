// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Reads arbitrary bytes as a write-ahead log beside a database.
//!
//! A log is a file a reader must follow, so every field of it is a
//! number the file chooses: the page size, the salts, the checksums, the
//! page number of a frame and the size a commit frame claims. None of
//! them may panic and none may make the walk run forever.
#![forbid(unsafe_code)]

use db_sqlite::db::Database;
use db_sqlite::image::Image;
use db_sqlite::wal::Wal;

/// The database the log is read beside, which is the one whose own
/// content is in a log.
const LOGGED: &[u8] = include_bytes!("../../../crates/db/sqlite/src/tests/fixtures/logged.db");

/// The log of it, which the fuzzer mutates.
const LOG: &[u8] = include_bytes!("../../../crates/db/sqlite/src/tests/fixtures/logged.db-wal");

/// The most pages a lookup asks for. A log can claim more; asking for
/// them adds no coverage and costs the fuzzer its time.
const MAX_PAGES: u32 = 4096;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    // The bytes as a log on their own: whatever they say, the reader
    // answers a log or a refusal and nothing else.
    let Ok(log) = Wal::open(bytes) else {
        return;
    };
    assert!(log.page_size().is_power_of_two(), "a page size that is not");
    assert!(
        (512..=65536).contains(&log.page_size()),
        "a page size out of range"
    );
    for number in 1..=log.pages().min(MAX_PAGES) {
        if let Some(page) = log.page_bytes(number) {
            assert_eq!(
                u32::try_from(page.len()).unwrap_or(0),
                log.page_size(),
                "a frame shorter than a page"
            );
        }
    }
    // A page the log does not hold answers nothing rather than the
    // bytes of another page.
    assert!(log.page_bytes(0).is_none(), "page zero is in no log");

    // The log beside the database it belongs to, which is a schema and
    // then a walk of whatever tree the log's pages describe.
    let Ok(database) = Database::open_with_log(LOGGED, &log) else {
        return;
    };
    let names: Vec<Vec<u8>> = database
        .tables()
        .map(|table| table.name.clone())
        .take(4)
        .collect();
    for name in &names {
        let mut sql = b"SELECT count(*), max(rowid) FROM \"".to_vec();
        for byte in name {
            if *byte == b'"' {
                sql.push(b'"');
            }
            sql.push(*byte);
        }
        sql.extend_from_slice(b"\" LIMIT 64");
        drop(database.query(&sql));
    }

    // The same log beside the database the bytes came from, which is the
    // pair the fixture is: the header then lies in the log.
    if let Ok(image) = Image::open_with_log(LOGGED, &log) {
        for number in 1..=image.pages().min(MAX_PAGES) {
            let _ = image.page(number);
        }
    }

    // A log mutated from the committed one, read beside a database whose
    // own file holds the rows, so that a frame that wins over the file
    // is read and not only one that fills a gap in it.
    let mut mixed = LOG.to_vec();
    let head = bytes.len().min(mixed.len());
    mixed.get_mut(..head).unwrap_or_default().copy_from_slice(
        bytes.get(..head).unwrap_or_default(),
    );
    if let Ok(log) = Wal::open(&mixed)
        && let Ok(database) = Database::open_with_log(LOGGED, &log)
    {
        drop(database.query(b"SELECT count(*) FROM m"));
    }
});
