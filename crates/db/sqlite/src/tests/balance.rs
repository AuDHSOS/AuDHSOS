// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a page that has no room for a cell does, against the walk of
//! `PRAGMA integrity_check`.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// What `PRAGMA integrity_check` answers for the file the writer holds.
fn checked(writer: &Writer) -> Vec<Vec<u8>> {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    database
        .query(b"PRAGMA integrity_check")
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(crate::value::Value::text))
        .collect()
}

/// The file of `btree01.test`: thirty rows that fill a page of 64 KiB,
/// written again shorter, and one of them written again longer than a
/// page holds room for.
///
/// A row in the middle of a leaf that the parent's right pointer does
/// not name is where the quick balance answered wrongly: it wrote the
/// page it made into the parent's right pointer, which named a page of
/// the tree twice and left another with nothing naming it.
#[test]
fn a_row_written_longer_than_the_page_holds_leaves_the_tree_whole() {
    let mut writer = Writer::new(65536, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t1(a INTEGER PRIMARY KEY, b BLOB)")
        .unwrap();
    for size in [3000_usize, 2000] {
        for at in [1_usize, 10, 15, 30] {
            for sql in [
                b"DELETE FROM t1".to_vec(),
                b"WITH RECURSIVE c(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM c WHERE i<30) \
                  INSERT INTO t1(a,b) SELECT i, zeroblob(6500) FROM c"
                    .to_vec(),
                alloc::format!("UPDATE t1 SET b=zeroblob({size})").into_bytes(),
                alloc::format!("UPDATE t1 SET b=zeroblob(64000) WHERE a={at}").into_bytes(),
            ] {
                writer.run(&sql).unwrap();
            }
            assert_eq!(checked(&writer), alloc::vec![b"ok".to_vec()], "{size}/{at}");
            let image = writer.written();
            let database = Database::open(&image).unwrap();
            assert_eq!(
                database
                    .query(b"SELECT count(*), sum(length(b)) FROM t1")
                    .unwrap()
                    .rows
                    .first(),
                Some(&alloc::vec![
                    Value::Int(30),
                    Value::Int(
                        i64::try_from(size.saturating_mul(29).saturating_add(64000)).unwrap()
                    )
                ]),
                "{size}/{at}"
            );
        }
    }
}

/// The quick balance itself: a row written at the end of the right-most
/// leaf, which is the one place it holds.
#[test]
fn a_row_written_past_the_last_of_them_goes_on_a_page_of_its_own() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER PRIMARY KEY, b)")
        .unwrap();
    for i in 1..=60 {
        writer
            .run(alloc::format!("INSERT INTO t VALUES({i}, zeroblob(200))").as_bytes())
            .unwrap();
    }
    assert_eq!(checked(&writer), alloc::vec![b"ok".to_vec()]);
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    assert_eq!(
        database
            .query(b"SELECT count(*) FROM t")
            .unwrap()
            .rows
            .first()
            .and_then(|row| row.first()),
        Some(&Value::Int(60))
    );
}
