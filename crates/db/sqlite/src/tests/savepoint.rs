// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `SAVEPOINT`, `RELEASE` and `ROLLBACK TO`, against what the shell
//! writes for the same statements.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// A connection over a database of one page, written under 512-byte
/// pages, which is what the fixture was written under.
fn writer() -> Writer {
    Writer::new(512, 0, Encoding::Utf8).unwrap()
}

/// What `SELECT a FROM t` answers on the file the writer holds.
fn rows(writer: &Writer) -> Vec<i64> {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    database
        .query(b"SELECT a FROM t")
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| match row.first() {
            Some(Value::Int(number)) => Some(*number),
            _ => None,
        })
        .collect()
}

#[test]
fn the_file_a_savepoint_block_writes_is_the_file_the_shell_wrote() {
    let mut writer = writer();
    for sql in [
        b"CREATE TABLE t(a)".as_slice(),
        b"SAVEPOINT one",
        b"INSERT INTO t VALUES(1)",
        b"SAVEPOINT two",
        b"INSERT INTO t VALUES(2)",
        b"ROLLBACK TO two",
        b"INSERT INTO t VALUES(3)",
        b"RELEASE one",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(rows(&writer), alloc::vec![1, 3]);
    assert_eq!(
        writer.written(),
        include_bytes!("fixtures/savepoint.db").to_vec()
    );
}

#[test]
fn a_rollback_to_a_savepoint_leaves_that_savepoint_open() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"SAVEPOINT one").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    writer.run(b"ROLLBACK TO one").unwrap();
    assert_eq!(rows(&writer), Vec::<i64>::new());
    writer.run(b"INSERT INTO t VALUES(2)").unwrap();
    writer.run(b"ROLLBACK TO one").unwrap();
    writer.run(b"INSERT INTO t VALUES(3)").unwrap();
    writer.run(b"RELEASE one").unwrap();
    assert_eq!(rows(&writer), alloc::vec![3]);
}

#[test]
fn a_savepoint_inside_a_transaction_writes_nothing_of_its_own() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"SAVEPOINT one").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    writer.run(b"RELEASE one").unwrap();
    // The release of a savepoint the `BEGIN` stands over commits
    // nothing, so the transaction is still open and the `ROLLBACK`
    // takes the row out again.
    writer.run(b"ROLLBACK").unwrap();
    assert_eq!(rows(&writer), Vec::<i64>::new());
}

#[test]
fn a_rollback_of_the_transaction_takes_every_savepoint_with_it() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"SAVEPOINT one").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    writer.run(b"ROLLBACK").unwrap();
    assert_eq!(rows(&writer), Vec::<i64>::new());
    assert_eq!(
        writer.run(b"RELEASE one").unwrap_err().message(),
        "no such savepoint: one"
    );
}

#[test]
fn a_commit_takes_every_savepoint_with_it() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"SAVEPOINT one").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    writer.run(b"COMMIT").unwrap();
    assert_eq!(rows(&writer), alloc::vec![1]);
    assert_eq!(
        writer.run(b"ROLLBACK TO one").unwrap_err().message(),
        "no such savepoint: one"
    );
}

#[test]
fn the_innermost_savepoint_of_a_name_is_the_one_a_statement_names() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"SAVEPOINT one").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    writer.run(b"SAVEPOINT one").unwrap();
    writer.run(b"INSERT INTO t VALUES(2)").unwrap();
    writer.run(b"ROLLBACK TO one").unwrap();
    // The second `SAVEPOINT one` is the one the rollback went back to,
    // so the row written before it stands.
    writer.run(b"RELEASE one").unwrap();
    assert_eq!(rows(&writer), alloc::vec![1]);
    writer.run(b"RELEASE one").unwrap();
    assert_eq!(rows(&writer), alloc::vec![1]);
}

#[test]
fn a_release_of_a_savepoint_inside_another_keeps_what_it_wrote() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"SAVEPOINT one").unwrap();
    writer.run(b"SAVEPOINT two").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    writer.run(b"RELEASE two").unwrap();
    writer.run(b"INSERT INTO t VALUES(2)").unwrap();
    writer.run(b"RELEASE one").unwrap();
    assert_eq!(rows(&writer), alloc::vec![1, 2]);
}

#[test]
fn a_name_no_savepoint_holds_is_refused() {
    let mut writer = writer();
    assert_eq!(
        writer.run(b"RELEASE nope").unwrap_err().message(),
        "no such savepoint: nope"
    );
    assert_eq!(
        writer.run(b"ROLLBACK TO nope").unwrap_err().message(),
        "no such savepoint: nope"
    );
    // A name in quotes is the name without them, and a name is read
    // without regard to case, which is what `sqlite3StrICmp` compares
    // the open savepoints with.
    writer.run(b"SAVEPOINT \"a b\"").unwrap();
    writer.run(b"SAVEPOINT one").unwrap();
    writer.run(b"RELEASE SAVEPOINT 'A B'").unwrap();
    assert_eq!(
        writer.run(b"RELEASE one").unwrap_err().message(),
        "no such savepoint: one"
    );
}

#[test]
fn what_the_parser_refuses_of_the_three_statements() {
    for sql in [
        b"SAVEPOINT".as_slice(),
        b"RELEASE",
        b"ROLLBACK TO",
        b"ROLLBACK TO SAVEPOINT",
        b"SAVEPOINT one two",
        b"RELEASE SAVEPOINT one two",
    ] {
        assert!(crate::parse::savepoint(sql).is_err(), "{sql:?}");
    }
    assert!(crate::parse::savepoint(b"ROLLBACK TRANSACTION TO SAVEPOINT one;").is_ok());
    assert!(matches!(
        crate::parse::savepoint(b"COMMIT"),
        Err(crate::parse::Error { .. })
    ));
}

#[test]
fn a_savepoint_that_opened_the_transaction_commits_at_its_release() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"SAVEPOINT one").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    let held = writer.written();
    // The statements of the savepoint are not in the file until the
    // release, because the savepoint opened the transaction they run in.
    assert_eq!(
        Database::open(&held).unwrap().rows_of(b"t").unwrap().len(),
        1
    );
    writer.run(b"RELEASE one").unwrap();
    assert_eq!(rows(&writer), alloc::vec![1]);
    assert_eq!(
        writer.run(b"COMMIT").unwrap_err().message(),
        "cannot commit - no transaction is active"
    );
}
#[test]
fn a_statement_that_refuses_inside_a_transaction_leaves_it_where_it_stood() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a UNIQUE)").unwrap();
    writer.run(b"SAVEPOINT one").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    // The second row of the statement shares a key with the first, so
    // the whole statement is undone and the row written before it
    // stands, which is the statement journal of `OE_Abort`.
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(2),(1)")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.a"
    );
    assert_eq!(rows(&writer), alloc::vec![1]);
    writer.run(b"ROLLBACK TO one").unwrap();
    assert_eq!(rows(&writer), Vec::<i64>::new());
}

#[test]
fn the_file_a_refused_statement_of_a_transaction_leaves_is_the_shell_s() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a UNIQUE)").unwrap();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    assert!(writer.run(b"INSERT INTO t VALUES(2),(1)").is_err());
    writer.run(b"COMMIT").unwrap();
    assert_eq!(rows(&writer), alloc::vec![1]);
    assert_eq!(
        writer.written(),
        include_bytes!("fixtures/refused.db").to_vec()
    );
}

/// A statement that stops where it stands keeps the rows it wrote,
/// which is `OE_Fail`.
#[test]
fn a_statement_that_stops_where_it_stands_keeps_what_it_wrote() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a UNIQUE)").unwrap();
    writer.run(b"SAVEPOINT one").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT OR FAIL INTO t VALUES(1),(2),(2),(3)")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.a"
    );
    assert_eq!(rows(&writer), alloc::vec![1, 2]);
    writer.run(b"RELEASE one").unwrap();
    assert_eq!(rows(&writer), alloc::vec![1, 2]);
}
