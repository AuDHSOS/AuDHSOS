// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The key an `INSERT` takes where the table already holds the largest
//! key an integer holds.

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// `rowid-12.2` of `test/rowid.test`: a table whose largest key is
/// 9223372036854775807 gives the next row a key drawn at random.
#[test]
fn a_table_at_the_largest_key_draws_the_next_key_at_random() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.randomness(7);
    writer
        .run(b"CREATE TABLE t(x INTEGER PRIMARY KEY, y)")
        .unwrap();
    writer
        .run(b"INSERT INTO t VALUES(9223372036854775807,'a')")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(NULL,'b')").unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    let answer = database.query(b"SELECT x FROM t ORDER BY x").unwrap();
    // The drawn key is positive and below the largest, so the row the
    // statement wrote sorts first.
    let drawn = match answer.rows.first().and_then(|row| row.first()) {
        Some(&Value::Int(key)) => key,
        other => panic!("{other:?}"),
    };
    assert!(drawn > 0 && drawn < i64::MAX, "{drawn}");
    assert_eq!(answer.rows.len(), 2);
}

/// `rowid-12.4` of `test/rowid.test`: a hundred draws that all name a
/// key a row holds refuse the write.
#[test]
fn a_hundred_draws_that_all_name_a_key_a_row_holds_refuse_the_write() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(x INTEGER PRIMARY KEY, y)")
        .unwrap();
    writer
        .run(b"INSERT INTO t VALUES(9223372036854775807,'a')")
        .unwrap();
    // Every statement draws from the same state, so the statement after
    // the first needs one draw more than the statement before it: the
    // keys the earlier draws name are taken.
    for _ in 0..100 {
        writer.randomness(7);
        writer.run(b"INSERT INTO t VALUES(NULL,'x')").unwrap();
    }
    writer.randomness(7);
    assert_eq!(
        writer.run(b"INSERT INTO t VALUES(NULL,'x')"),
        Err(crate::db::Error::Image(crate::error::Error::Full))
    );
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    let counted = database
        .query(b"SELECT count(*) FROM t WHERE y=='x'")
        .unwrap();
    assert_eq!(counted.rows, [alloc::vec![Value::Int(100)]]);
}

/// `autoIncBegin`: a table whose key counts up draws no key, so a table
/// that reached the largest key an integer holds refuses the write.
#[test]
fn a_key_that_counts_up_refuses_at_the_largest_key_an_integer_holds() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(x INTEGER PRIMARY KEY AUTOINCREMENT, y)")
        .unwrap();
    writer
        .run(b"INSERT INTO t VALUES(9223372036854775807,'a')")
        .unwrap();
    assert_eq!(
        writer.run(b"INSERT INTO t VALUES(NULL,'b')"),
        Err(crate::db::Error::Image(crate::error::Error::Full))
    );
}

/// `save_prng_state`: the state a writer answers draws the same words
/// again.
#[test]
fn the_state_a_writer_answers_draws_the_same_words_again() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.randomness(11);
    writer.run(b"CREATE TABLE t(a)").unwrap();
    let held = writer.randomness_held();
    writer.run(b"INSERT INTO t VALUES(randomblob(8))").unwrap();
    writer.randomness(held);
    writer.run(b"INSERT INTO t VALUES(randomblob(8))").unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    let rows = database.query(b"SELECT a FROM t").unwrap().rows;
    assert_eq!(rows.first(), rows.get(1));
}
