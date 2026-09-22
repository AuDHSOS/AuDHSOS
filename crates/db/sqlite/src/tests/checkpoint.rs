// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `PRAGMA wal_checkpoint`, which writes the frames of the log into the
//! database file.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// The three numbers one checkpoint answers.
fn checkpointed(writer: &mut Writer, sql: &[u8]) -> Vec<i64> {
    writer
        .run(sql)
        .expect("an answer")
        .first()
        .map(|row| row.iter().map(Value::to_integer).collect())
        .unwrap_or_default()
}

/// A file that is not logging holds no frame, which the pragma says with
/// minus one, and the frames of a log are counted from the commit that
/// began it.
#[test]
fn what_a_checkpoint_answers_and_what_the_file_holds_after_it() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    assert_eq!(
        checkpointed(&mut writer, b"PRAGMA wal_checkpoint"),
        [0, -1, -1]
    );
    writer.run(b"PRAGMA journal_mode=wal").unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    // The file the pragma left names no table, because every page the
    // statements wrote is in the log.
    let before = writer.written();
    assert!(
        Database::open(&before)
            .unwrap()
            .query(b"SELECT a FROM t")
            .is_err()
    );
    let first = checkpointed(&mut writer, b"PRAGMA wal_checkpoint");
    assert_eq!(first.first(), Some(&0));
    assert_eq!(first.get(1), first.get(2));
    assert!(first.get(1).is_some_and(|frames| *frames > 0));
    // The file the checkpoint wrote holds the rows the log held.
    let after = writer.written();
    assert_eq!(
        Database::open(&after)
            .unwrap()
            .query(b"SELECT a FROM t")
            .unwrap()
            .rows,
        [alloc::vec![Value::Int(1)]]
    );
    // The commit after a checkpoint begins the log again, so the next
    // checkpoint counts the frames of that commit alone.
    writer.run(b"INSERT INTO t VALUES(2)").unwrap();
    let second = checkpointed(&mut writer, b"PRAGMA wal_checkpoint");
    assert!(second.get(1) < first.get(1));
    // A checkpoint that follows one with no commit between them counts
    // the same frames again.
    assert_eq!(checkpointed(&mut writer, b"PRAGMA wal_checkpoint"), second);
}

/// `TRUNCATE` leaves a log of no frame and says so with two noughts,
/// and `RESTART` begins the log again and counts the frames it moved.
#[test]
fn what_truncate_and_restart_leave_the_log_at() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"PRAGMA journal_mode=wal").unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    let restarted = checkpointed(&mut writer, b"PRAGMA wal_checkpoint(RESTART)");
    assert!(restarted.get(1).is_some_and(|frames| *frames > 0));
    assert_eq!(restarted.get(1), restarted.get(2));
    // The log of no frame is what the next checkpoint counts.
    assert_eq!(
        checkpointed(&mut writer, b"PRAGMA wal_checkpoint"),
        [0, 0, 0]
    );
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    assert_eq!(
        checkpointed(&mut writer, b"PRAGMA wal_checkpoint(TRUNCATE)"),
        [0, 0, 0]
    );
    // The rows stand in the file after the log was cut back.
    let written = writer.written();
    assert_eq!(
        Database::open(&written)
            .unwrap()
            .query(b"SELECT a FROM t")
            .unwrap()
            .rows,
        [alloc::vec![Value::Int(1)]]
    );
    assert_eq!(
        checkpointed(&mut writer, b"PRAGMA wal_checkpoint(FULL)"),
        [0, 0, 0]
    );
}

/// A close writes the pages the log holds into the file and gives the
/// log up, and the file it leaves is still in write-ahead logging; a
/// close over a file that keeps no log writes nothing.
#[test]
fn what_a_close_leaves_of_the_log() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.closing();
    assert!(writer.did().is_empty());
    writer.run(b"PRAGMA journal_mode=wal").unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    let _ = writer.did();
    writer.closing();
    assert!(writer.log().is_none());
    assert_eq!(writer.journalled(), b"wal");
    // The file holds the rows the log held, and the last thing the
    // close did was give the log up.
    let image = writer.written();
    assert_eq!(
        Database::open(&image)
            .unwrap()
            .query(b"SELECT a FROM t")
            .unwrap()
            .rows,
        [alloc::vec![Value::Int(1)]]
    );
    assert_eq!(
        writer.did().last(),
        Some(&crate::change::Does::Remove(crate::change::Onto::Log))
    );
    // The commit after the close makes a log again, because the header
    // still says version two.
    writer.run(b"INSERT INTO t VALUES(2)").unwrap();
    assert!(writer.log().is_some());
    let image = writer.written();
    assert_eq!(
        Database::open(&image)
            .unwrap()
            .query(b"SELECT count(*) FROM t")
            .unwrap()
            .rows,
        [alloc::vec![Value::Int(1)]],
        "the file holds what the log has not taken back"
    );
    // A pragma naming another mode, over a file whose log the close
    // wrote back, writes the header to version one, which the file then
    // answers with.
    writer.closing();
    writer.run(b"PRAGMA journal_mode=delete").unwrap();
    assert_eq!(writer.journalled(), b"delete");
}
