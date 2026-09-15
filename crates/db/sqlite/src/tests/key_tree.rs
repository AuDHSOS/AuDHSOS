// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A table that keeps its rows in the key's own tree: what the file
//! holds for it, and what a statement that writes it answers.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// What `sql` answers over `image`.
fn answered(image: &[u8], sql: &[u8]) -> Vec<Vec<Value>> {
    let database = Database::open(image).unwrap();
    database.query(sql).unwrap().rows
}

/// One row of one text.
fn text(word: &[u8]) -> Vec<Value> {
    alloc::vec![Value::Text(word.to_vec())]
}

#[test]
fn a_row_written_into_the_key_s_own_tree_is_read_back() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a TEXT PRIMARY KEY, b) WITHOUT ROWID")
        .unwrap();
    let empty = writer.written();
    assert_eq!(
        answered(&empty, b"SELECT a FROM t"),
        Vec::<Vec<Value>>::new()
    );
    writer.run(b"INSERT INTO t VALUES('x',1),('a',2)").unwrap();
    let image = writer.written();
    assert_eq!(
        answered(&image, b"SELECT a FROM t"),
        [text(b"a"), text(b"x")]
    );
    assert_eq!(
        answered(&image, b"SELECT b FROM t WHERE a='x'"),
        [alloc::vec![Value::Int(1)]]
    );
    assert_eq!(answered(&image, b"PRAGMA integrity_check"), [text(b"ok")]);
}

#[test]
fn every_row_of_the_key_s_own_tree_is_written_and_taken_out_again() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a TEXT, b INT, c, PRIMARY KEY(a,b)) WITHOUT ROWID".as_slice(),
        b"CREATE INDEX i ON t(c)",
        b"INSERT INTO t VALUES('x',1,'one'),('x',2,'two'),('y',1,'three')",
        b"UPDATE t SET c='four' WHERE a='x' AND b=2",
        b"UPDATE t SET a='z' WHERE a='y'",
        b"DELETE FROM t WHERE b=1 AND a='x'",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    assert_eq!(
        answered(&image, b"SELECT a,b,c FROM t"),
        [
            alloc::vec![
                Value::Text(b"x".to_vec()),
                Value::Int(2),
                Value::Text(b"four".to_vec())
            ],
            alloc::vec![
                Value::Text(b"z".to_vec()),
                Value::Int(1),
                Value::Text(b"three".to_vec())
            ]
        ]
    );
    assert_eq!(
        answered(&image, b"SELECT a FROM t WHERE c='three'"),
        [text(b"z")]
    );
    assert_eq!(answered(&image, b"PRAGMA integrity_check"), [text(b"ok")]);
}

#[test]
fn a_key_a_row_already_holds_is_refused() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a PRIMARY KEY, b) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x'),(2,'y')").unwrap();
    let refused = writer.run(b"INSERT INTO t VALUES(1,'z')").unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.a");
    let refused = writer.run(b"UPDATE t SET a=1 WHERE a=2").unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.a");
    writer
        .run(b"INSERT OR REPLACE INTO t VALUES(1,'z')")
        .unwrap();
    writer
        .run(b"INSERT OR IGNORE INTO t VALUES(2,'w')")
        .unwrap();
    let image = writer.written();
    assert_eq!(
        answered(&image, b"SELECT a,b FROM t"),
        [
            alloc::vec![Value::Int(1), Value::Text(b"z".to_vec())],
            alloc::vec![Value::Int(2), Value::Text(b"y".to_vec())]
        ]
    );
    assert_eq!(answered(&image, b"PRAGMA integrity_check"), [text(b"ok")]);
}

#[test]
fn the_file_a_key_s_own_tree_is_written_into_is_the_file_the_shell_wrote() {
    // Written by the shell:
    //   PRAGMA page_size=512;
    //   CREATE TABLE t(a TEXT, b INT, c, PRIMARY KEY(a,b)) WITHOUT ROWID;
    //   CREATE INDEX i ON t(c);
    //   INSERT INTO t VALUES('x',1,'one'),('x',2,'two'),('y',1,'three');
    let shell: &[u8] = include_bytes!("fixtures/key-tree.db");
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a TEXT, b INT, c, PRIMARY KEY(a,b)) WITHOUT ROWID".as_slice(),
        b"CREATE INDEX i ON t(c)",
        b"INSERT INTO t VALUES('x',1,'one'),('x',2,'two'),('y',1,'three')",
    ] {
        writer.run(sql).unwrap();
    }
    let mine = writer.written();
    assert_eq!(mine.len(), shell.len());
    let at = mine.iter().zip(shell).position(|(one, other)| one != other);
    assert_eq!(at, None);
}

#[test]
fn a_statement_that_names_the_rowid_of_such_a_table_is_refused() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a PRIMARY KEY, b) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x')").unwrap();
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    assert_eq!(
        database
            .query(b"SELECT rowid FROM t")
            .unwrap_err()
            .message(),
        "no such column: rowid"
    );
    assert_eq!(
        writer
            .run(b"DELETE FROM t WHERE rowid=1")
            .unwrap_err()
            .message(),
        "no such column: rowid"
    );
    assert_eq!(
        writer
            .run(b"UPDATE t SET b=2 WHERE rowid=1")
            .unwrap_err()
            .message(),
        "no such column: rowid"
    );
}

#[test]
fn what_the_conflict_clause_of_the_key_says_is_what_happens() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a, b, PRIMARY KEY(a) ON CONFLICT IGNORE) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x')").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'y')").unwrap();
    assert_eq!(
        answered(&writer.written(), b"SELECT b FROM t"),
        [text(b"x")]
    );
    // What the statement says is read before what the constraint says.
    let refused = writer
        .run(b"INSERT OR FAIL INTO t VALUES(1,'z')")
        .unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.a");
    let refused = writer
        .run(b"INSERT OR ABORT INTO t VALUES(1,'z')")
        .unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.a");
}

#[test]
fn a_unique_index_over_such_a_table_holds_its_rows() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a PRIMARY KEY, b) WITHOUT ROWID".as_slice(),
        b"CREATE UNIQUE INDEX i ON t(b)",
        b"INSERT INTO t VALUES(1,'x'),(2,'y')",
    ] {
        writer.run(sql).unwrap();
    }
    let refused = writer.run(b"INSERT INTO t VALUES(3,'x')").unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.b");
    let refused = writer.run(b"UPDATE t SET b='x' WHERE a=2").unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.b");
    writer
        .run(b"INSERT OR REPLACE INTO t VALUES(3,'x')")
        .unwrap();
    let image = writer.written();
    assert_eq!(
        answered(&image, b"SELECT a FROM t"),
        [alloc::vec![Value::Int(2)], alloc::vec![Value::Int(3)]]
    );
    assert_eq!(answered(&image, b"PRAGMA integrity_check"), [text(b"ok")]);
}

#[test]
fn a_trigger_on_such_a_table_runs_over_the_row_it_writes() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a PRIMARY KEY, b) WITHOUT ROWID".as_slice(),
        b"CREATE TABLE log(what TEXT, value)",
        b"CREATE TRIGGER put AFTER INSERT ON t BEGIN INSERT INTO log VALUES('put', new.a); END",
        b"CREATE TRIGGER set_ AFTER UPDATE ON t BEGIN INSERT INTO log VALUES('set', old.b); END",
        b"CREATE TRIGGER out_ AFTER DELETE ON t BEGIN INSERT INTO log VALUES('out', old.a); END",
        b"INSERT INTO t VALUES(1,'x')",
        b"UPDATE t SET b='y' WHERE a=1",
        b"DELETE FROM t",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(
        answered(&writer.written(), b"SELECT what,value FROM log"),
        [
            alloc::vec![Value::Text(b"put".to_vec()), Value::Int(1)],
            alloc::vec![Value::Text(b"set".to_vec()), Value::Text(b"x".to_vec())],
            alloc::vec![Value::Text(b"out".to_vec()), Value::Int(1)]
        ]
    );
}

#[test]
fn a_trigger_that_stops_a_row_of_such_a_table_stops_it() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a PRIMARY KEY, b) WITHOUT ROWID".as_slice(),
        b"INSERT INTO t VALUES(1,'x'),(2,'y')",
        b"CREATE TRIGGER put BEFORE INSERT ON t WHEN new.a=3 \
          BEGIN SELECT RAISE(IGNORE); END",
        b"CREATE TRIGGER set_ BEFORE UPDATE ON t WHEN old.a=1 \
          BEGIN SELECT RAISE(IGNORE); END",
        b"CREATE TRIGGER out_ BEFORE DELETE ON t WHEN old.a=2 \
          BEGIN SELECT RAISE(IGNORE); END",
        b"INSERT INTO t VALUES(3,'z')",
        b"UPDATE t SET b='w'",
        b"DELETE FROM t",
    ] {
        writer.run(sql).unwrap();
    }
    // The row the `INSERT` trigger stopped was not written, the row the
    // `UPDATE` trigger stopped keeps what it held, and the row the
    // `DELETE` trigger stopped is still there.
    assert_eq!(
        answered(&writer.written(), b"SELECT a,b FROM t"),
        [alloc::vec![Value::Int(2), Value::Text(b"w".to_vec())]]
    );
}

#[test]
fn a_column_of_such_a_table_that_refuses_nothing_passes_the_row_over() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a PRIMARY KEY, b NOT NULL ON CONFLICT IGNORE) WITHOUT ROWID".as_slice(),
        b"INSERT INTO t VALUES(1,'x')",
        b"INSERT INTO t VALUES(2,NULL)",
        b"UPDATE t SET b=NULL WHERE a=1",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(
        answered(&writer.written(), b"SELECT a,b FROM t"),
        [alloc::vec![Value::Int(1), Value::Text(b"x".to_vec())]]
    );
}

#[test]
fn what_a_statement_says_of_a_key_two_rows_share_is_what_happens() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a, b, PRIMARY KEY(a,b)) WITHOUT ROWID".as_slice(),
        b"CREATE UNIQUE INDEX i ON t(b)",
        b"INSERT INTO t VALUES(1,'x'),(2,'y')",
    ] {
        writer.run(sql).unwrap();
    }
    // The key of the table, named by both its columns.
    let refused = writer.run(b"INSERT INTO t VALUES(1,'x')").unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.a, t.b");
    let refused = writer
        .run(b"INSERT OR FAIL INTO t VALUES(1,'x')")
        .unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.a, t.b");
    writer
        .run(b"INSERT OR IGNORE INTO t VALUES(1,'x')")
        .unwrap();
    // The key of the index beside it.
    writer
        .run(b"INSERT OR IGNORE INTO t VALUES(3,'x')")
        .unwrap();
    let refused = writer
        .run(b"INSERT OR FAIL INTO t VALUES(3,'x')")
        .unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.b");
    assert_eq!(
        answered(&writer.written(), b"SELECT a FROM t"),
        [alloc::vec![Value::Int(1)], alloc::vec![Value::Int(2)]]
    );
}

#[test]
fn what_an_update_says_of_a_key_two_rows_share_is_what_happens() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a PRIMARY KEY, b) WITHOUT ROWID".as_slice(),
        b"INSERT INTO t VALUES(1,'x'),(2,'y')",
    ] {
        writer.run(sql).unwrap();
    }
    // The key of the table: passed over, refused, or written over.
    writer.run(b"UPDATE OR IGNORE t SET a=1 WHERE a=2").unwrap();
    let refused = writer
        .run(b"UPDATE OR FAIL t SET a=1 WHERE a=2")
        .unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.a");
    let refused = writer.run(b"UPDATE t SET a=1 WHERE a=2").unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.a");
    writer
        .run(b"UPDATE OR REPLACE t SET a=1 WHERE a=2")
        .unwrap();
    assert_eq!(
        answered(&writer.written(), b"SELECT a,b FROM t"),
        [alloc::vec![Value::Int(1), Value::Text(b"y".to_vec())]]
    );
}

#[test]
fn what_an_update_says_of_an_index_two_rows_share_is_what_happens() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a PRIMARY KEY, b) WITHOUT ROWID".as_slice(),
        b"CREATE UNIQUE INDEX i ON t(b)",
        b"INSERT INTO t VALUES(1,'x'),(2,'y')",
    ] {
        writer.run(sql).unwrap();
    }
    writer
        .run(b"UPDATE OR IGNORE t SET b='x' WHERE a=2")
        .unwrap();
    let refused = writer
        .run(b"UPDATE OR FAIL t SET b='x' WHERE a=2")
        .unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.b");
    let refused = writer.run(b"UPDATE t SET b='x' WHERE a=2").unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.b");
    writer
        .run(b"UPDATE OR REPLACE t SET b='x' WHERE a=2")
        .unwrap();
    let image = writer.written();
    assert_eq!(
        answered(&image, b"SELECT a,b FROM t"),
        [alloc::vec![Value::Int(2), Value::Text(b"x".to_vec())]]
    );
    assert_eq!(answered(&image, b"PRAGMA integrity_check"), [text(b"ok")]);
}

#[test]
fn a_unique_of_such_a_table_carries_an_index_of_its_own() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a PRIMARY KEY, b UNIQUE) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x')").unwrap();
    // The key of the table carries no index and counts all the same, so
    // the index of the `UNIQUE` is the second of the table's own.
    assert_eq!(
        answered(
            &writer.written(),
            b"SELECT name FROM sqlite_master WHERE type='index'"
        ),
        [text(b"sqlite_autoindex_t_2")]
    );
    let refused = writer.run(b"INSERT INTO t VALUES(2,'x')").unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.b");
}

#[test]
fn the_rows_of_such_a_table_are_read_by_the_walk_of_its_own_tree() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a PRIMARY KEY, b) WITHOUT ROWID")
        .unwrap();
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    // A walk that names each row by a rowid is one such a table has no
    // rowid for.
    assert!(database.rows_of(b"t").is_err());
}

#[test]
fn a_replace_runs_the_triggers_of_a_delete_only_where_it_is_told_to() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a PRIMARY KEY, b) WITHOUT ROWID".as_slice(),
        b"CREATE TABLE log(m)",
        b"INSERT INTO t VALUES(1,'x')",
        b"CREATE TRIGGER g BEFORE DELETE ON t BEGIN INSERT INTO log VALUES(old.b); END",
        b"INSERT OR REPLACE INTO t VALUES(1,'y')",
    ] {
        writer.run(sql).unwrap();
    }
    // The row is written out from inside an `INSERT`, so the triggers
    // of a `DELETE` run only where recursive triggers are on.
    assert_eq!(
        answered(&writer.written(), b"SELECT count(*) FROM log"),
        [alloc::vec![Value::Int(0)]]
    );
    writer.run(b"PRAGMA recursive_triggers=on").unwrap();
    writer
        .run(b"INSERT OR REPLACE INTO t VALUES(1,'z')")
        .unwrap();
    assert_eq!(
        answered(&writer.written(), b"SELECT m FROM log"),
        [text(b"y")]
    );
}

#[test]
fn a_row_a_trigger_keeps_holds_the_key_a_replace_wanted() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"PRAGMA recursive_triggers=on".as_slice(),
        b"CREATE TABLE t(a PRIMARY KEY, b) WITHOUT ROWID",
        b"CREATE UNIQUE INDEX i ON t(b)",
        b"INSERT INTO t VALUES(1,'x'),(2,'y')",
        b"CREATE TRIGGER g BEFORE DELETE ON t BEGIN SELECT RAISE(IGNORE); END",
    ] {
        writer.run(sql).unwrap();
    }
    // The key of the table, and the key of the index beside it.
    let refused = writer
        .run(b"INSERT OR REPLACE INTO t VALUES(1,'z')")
        .unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.a");
    let refused = writer
        .run(b"INSERT OR REPLACE INTO t VALUES(3,'y')")
        .unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.b");
    let refused = writer
        .run(b"UPDATE OR REPLACE t SET a=1 WHERE a=2")
        .unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.a");
    let refused = writer
        .run(b"UPDATE OR REPLACE t SET b='x' WHERE a=2")
        .unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.b");
    let image = writer.written();
    assert_eq!(
        answered(&image, b"SELECT a,b FROM t"),
        [
            alloc::vec![Value::Int(1), Value::Text(b"x".to_vec())],
            alloc::vec![Value::Int(2), Value::Text(b"y".to_vec())]
        ]
    );
    assert_eq!(answered(&image, b"PRAGMA integrity_check"), [text(b"ok")]);
}
