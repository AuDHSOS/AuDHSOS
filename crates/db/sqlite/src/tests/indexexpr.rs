// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! An index over an expression and an index over fewer rows than the
//! table has, against the answers of the C library's shell.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// The rows a statement answers against the file the writer holds.
fn query(writer: &Writer, sql: &[u8]) -> Vec<Vec<Value>> {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    database.query(sql).unwrap().rows
}

/// What `PRAGMA integrity_check` answers for the file the writer holds.
fn checked(writer: &Writer) -> Vec<Vec<u8>> {
    query(writer, b"PRAGMA integrity_check")
        .iter()
        .filter_map(|row| row.first().and_then(crate::value::Value::text))
        .collect()
}

/// The rows the file holds for the tree of `index`, each as one value.
fn entries(writer: &Writer, index: &[u8]) -> usize {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let (_, root) = database.index(index).unwrap();
    database.image().entries(root).count()
}

#[test]
fn an_index_over_an_expression_holds_what_the_expression_answers() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b,c)").unwrap();
    writer
        .run(
            b"INSERT INTO t VALUES('In_the_beginning',1,1),('and_the_Word',1,2),\
              ('and_God',3,1),(NULL,2,5)",
        )
        .unwrap();
    writer
        .run(b"CREATE INDEX t1a1 ON t(substr(a,1,7))")
        .unwrap();
    assert_eq!(entries(&writer, b"t1a1"), 4);
    assert_eq!(checked(&writer), [b"ok".to_vec()]);
    // The statement reads the rows rather than the index, so it answers
    // what the expression answers for each row.
    assert_eq!(
        query(
            &writer,
            b"SELECT b,c FROM t WHERE substr(a,1,7)=='and_the' ORDER BY b,c"
        ),
        [[Value::Int(1), Value::Int(2)]]
    );
    // A row written, changed and taken out keeps the index whole.
    writer
        .run(b"UPDATE t SET a='and_the_rest' WHERE c=1")
        .unwrap();
    assert_eq!(checked(&writer), [b"ok".to_vec()]);
    writer.run(b"DELETE FROM t WHERE c=5").unwrap();
    assert_eq!(entries(&writer, b"t1a1"), 3);
    assert_eq!(checked(&writer), [b"ok".to_vec()]);
    // `REINDEX` writes the entries again out of the rows, which a
    // partial index beside the first holds fewer of.
    writer.run(b"CREATE INDEX t1p ON t(b) WHERE c>1").unwrap();
    writer.run(b"REINDEX").unwrap();
    assert_eq!(entries(&writer, b"t1a1"), 3);
    assert_eq!(entries(&writer, b"t1p"), 1);
    assert_eq!(checked(&writer), [b"ok".to_vec()]);
    // `ANALYZE` counts the entries each index holds and not the rows
    // the table has.
    writer.run(b"ANALYZE").unwrap();
    assert_eq!(
        query(&writer, b"SELECT idx,stat FROM sqlite_stat1 ORDER BY idx"),
        [
            [Value::Text(b"t1a1".to_vec()), Value::Text(b"3 3".to_vec())],
            [Value::Text(b"t1p".to_vec()), Value::Text(b"1 1".to_vec())],
        ]
    );
}

#[test]
fn a_statement_is_not_planned_against_an_index_that_holds_fewer_entries() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"CREATE TABLE u(c,d)").unwrap();
    writer.run(b"CREATE INDEX tp ON t(a) WHERE b>1").unwrap();
    writer.run(b"CREATE INDEX ta ON t(abs(a))").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,1),(2,2)").unwrap();
    writer.run(b"INSERT INTO u VALUES(1,9),(2,8)").unwrap();
    // The row the index holds no entry for is one the statement reads
    // all the same, which a walk of the table answers and a lookup in
    // the index would not.
    assert_eq!(
        query(&writer, b"SELECT b FROM t WHERE a=1"),
        [[Value::Int(1)]]
    );
    assert_eq!(
        query(&writer, b"SELECT u.d FROM u JOIN t ON t.a=u.c WHERE u.c=1"),
        [[Value::Int(9)]]
    );
}

#[test]
fn an_index_term_written_as_text_names_a_column() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,2)").unwrap();
    // `sqlite3StringToId` reads the term as the column `b`, so the
    // index holds that column and not the text.
    writer.run(b"CREATE UNIQUE INDEX tb ON t('b')").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(3,2)")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.b"
    );
    assert!(writer.run(b"CREATE INDEX tz ON t('zz')").is_err());
    assert_eq!(checked(&writer), [b"ok".to_vec()]);
    // The column is one the index reads, so dropping it refuses the
    // whole statement, which `renameTestSchema` names the index in.
    assert_eq!(
        writer
            .run(b"ALTER TABLE t DROP COLUMN b")
            .unwrap_err()
            .message(),
        "error in index tb after drop column: no such column: b"
    );
}

#[test]
fn an_index_term_that_names_a_column_under_a_table_is_refused() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,2)").unwrap();
    assert_eq!(
        writer
            .run(b"CREATE INDEX ta ON t(abs(t.a))")
            .unwrap_err()
            .message(),
        "the \".\" operator prohibited in index expressions"
    );
    // The `WHERE` of a partial index names a column under its table,
    // and under the schema, which `NC_PartIdx` leaves alone.
    writer
        .run(b"CREATE INDEX tp ON t(a) WHERE main.t.b>1")
        .unwrap();
    assert_eq!(entries(&writer, b"tp"), 1);
    // A column under a table the statement is not over answers
    // nothing, so the index is refused as it is filled.
    assert!(writer.run(b"CREATE INDEX tq ON t(a) WHERE u.b>1").is_err());
    assert!(
        writer
            .run(b"CREATE INDEX tr ON t(a) WHERE other.t.b>1")
            .is_err()
    );
}

#[test]
fn an_index_with_a_where_holds_the_rows_that_clause_answers_true_for() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b,c)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES(1,1,1),(2,1,2),(3,3,1),(4,2,5)")
        .unwrap();
    writer.run(b"CREATE INDEX t1p ON t(b) WHERE c>1").unwrap();
    assert_eq!(entries(&writer, b"t1p"), 2);
    assert_eq!(checked(&writer), [b"ok".to_vec()]);
    // A row the clause stops answering true for loses its entry, and a
    // row it begins answering true for gains one.
    writer.run(b"UPDATE t SET c=0 WHERE a=2").unwrap();
    assert_eq!(entries(&writer, b"t1p"), 1);
    assert_eq!(checked(&writer), [b"ok".to_vec()]);
    writer.run(b"UPDATE t SET c=9 WHERE a=1").unwrap();
    assert_eq!(entries(&writer, b"t1p"), 2);
    assert_eq!(checked(&writer), [b"ok".to_vec()]);
    writer.run(b"DELETE FROM t WHERE a=4").unwrap();
    assert_eq!(entries(&writer, b"t1p"), 1);
    assert_eq!(checked(&writer), [b"ok".to_vec()]);
}

#[test]
fn a_unique_index_over_an_expression_names_itself_in_the_message() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,1)").unwrap();
    writer.run(b"CREATE UNIQUE INDEX tu ON t(abs(a))").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(-1,2)")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: index 'tu'"
    );
    // A row the key holds no place for is one the index refuses
    // nothing over.
    writer.run(b"INSERT INTO t VALUES(2,3)").unwrap();
    assert_eq!(checked(&writer), [b"ok".to_vec()]);
}

#[test]
fn a_unique_index_over_fewer_rows_holds_only_those_rows_to_the_key() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer
        .run(b"CREATE UNIQUE INDEX tu ON t(a) WHERE b=1")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(1,1)").unwrap();
    // The clause answers false for the second row, so the key it shares
    // with the first refuses nothing.
    writer.run(b"INSERT INTO t VALUES(1,2)").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,1)")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.a"
    );
    assert_eq!(entries(&writer, b"tu"), 1);
    assert_eq!(checked(&writer), [b"ok".to_vec()]);
}

#[test]
fn an_index_over_an_expression_is_read_under_the_collation_it_writes() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"INSERT INTO t VALUES('AB')").unwrap();
    writer
        .run(b"CREATE UNIQUE INDEX tu ON t(substr(a,1,2) COLLATE NOCASE)")
        .unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES('ab')")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: index 'tu'"
    );
    assert_eq!(checked(&writer), [b"ok".to_vec()]);
}
