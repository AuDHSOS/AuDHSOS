// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Reads arbitrary bytes as a rollback journal beside a database.
//!
//! A journal is a file a reader plays back, so every field of it is a
//! number the file chooses: the sector size, the page size, how many
//! pages the database had, how many records follow, the nonce each
//! checksum starts from, and the page number of every record. None of
//! them may panic and none may make the playback run forever.
#![forbid(unsafe_code)]

use db_sqlite::db::Database;
use db_sqlite::image::Image;
use db_sqlite::journal::Journal;

/// The database the journal is read beside, which is one caught with a
/// transaction half written.
const ROLLBACK: &[u8] = include_bytes!("../../../crates/db/sqlite/src/tests/fixtures/rollback.db");

/// The journal of it, which the fuzzer mutates.
const JOURNAL: &[u8] =
    include_bytes!("../../../crates/db/sqlite/src/tests/fixtures/rollback.db-journal");

/// The most pages a lookup asks for. A journal can claim more; asking
/// for them adds no coverage and costs the fuzzer its time.
const MAX_PAGES: u32 = 4096;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    // The bytes as a journal on their own: whatever they say, the reader
    // answers a journal or a refusal and nothing else.
    let Ok(journal) = Journal::open(bytes) else {
        return;
    };
    assert_eq!(
        journal.hot(),
        journal.pages() != 0,
        "a journal is hot exactly where it restores pages"
    );
    if journal.hot() {
        assert!(
            journal.page_size().is_power_of_two() && (512..=65536).contains(&journal.page_size()),
            "a page size the format does not allow"
        );
    }
    for number in 1..=journal.pages().min(MAX_PAGES) {
        if let Some(page) = journal.page_bytes(number) {
            assert_eq!(
                u32::try_from(page.len()).unwrap_or(0),
                journal.page_size(),
                "a record shorter than a page"
            );
        }
    }
    // Section 1.2 numbers pages from one, and the playback truncates to
    // the page count the first header names.
    assert!(journal.page_bytes(0).is_none(), "page zero is in no journal");
    assert!(
        journal.page_bytes(journal.pages().saturating_add(1)).is_none(),
        "a page past the database's own size is restored"
    );

    // The journal beside the database it belongs to: a schema read out of
    // whatever the playback left, and a walk of every tree it names.
    let Ok(database) = Database::open_with_journal(ROLLBACK, &journal) else {
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
    if let Ok(image) = Image::open_with_journal(ROLLBACK, &journal) {
        for number in 1..=image.pages().min(MAX_PAGES) {
            let _ = image.page(number);
        }
    }

    // A journal mutated from the committed one, so that a record which
    // wins over the file is played back and not only one that fills a
    // gap in it.
    let mut mixed = JOURNAL.to_vec();
    let head = bytes.len().min(mixed.len());
    mixed
        .get_mut(..head)
        .unwrap_or_default()
        .copy_from_slice(bytes.get(..head).unwrap_or_default());
    if let Ok(journal) = Journal::open(&mixed)
        && let Ok(database) = Database::open_with_journal(ROLLBACK, &journal)
    {
        drop(database.query(b"SELECT count(*), max(b) FROM t"));
    }
});
