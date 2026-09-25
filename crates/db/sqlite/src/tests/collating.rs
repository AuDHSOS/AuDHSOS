// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The collations an application defines on a connection.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::{Collating, Value};
use crate::wal::Wal;

/// `BACKWARDS` of `test/collate2.test`: the bytes of each value are
/// read back to front before the two are compared, which costs O(n)
/// over the longer value.
fn backwards(_name: &'static [u8], left: &[u8], right: &[u8]) -> core::cmp::Ordering {
    let one: Vec<u8> = left.iter().rev().copied().collect();
    let another: Vec<u8> = right.iter().rev().copied().collect();
    one.cmp(&another)
}

/// `CASELESS` of `test/collate3.test`, which compares letters without
/// their case.
fn caseless(_name: &'static [u8], left: &[u8], right: &[u8]) -> core::cmp::Ordering {
    left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
}

/// The two, as a connection holds them.
static COLLATING: &[Collating] = &[
    Collating {
        name: b"BACKWARDS",
        by: backwards,
    },
    Collating {
        name: b"CASELESS",
        by: caseless,
    },
];

/// The first value of every row one statement answers, as text.
fn texts(database: &Database, sql: &[u8]) -> Vec<Vec<u8>> {
    database
        .query(sql)
        .expect("an answer")
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(Value::text))
        .collect()
}

/// A column declared `COLLATE BACKWARDS` orders under the collation
/// the connection defines, a `COLLATE` in the statement names another,
/// and an index over that column holds the same order.
#[test]
fn a_collation_the_connection_defines_orders_a_column() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.collates(COLLATING);
    writer
        .run(b"CREATE TABLE t1(a COLLATE BACKWARDS, b)")
        .unwrap();
    for value in [b"aa".as_slice(), b"ab", b"ba", b"bb"] {
        let mut sql = b"INSERT INTO t1 VALUES('".to_vec();
        sql.extend_from_slice(value);
        sql.extend_from_slice(b"', 1)");
        writer.run(&sql).unwrap();
    }
    writer.run(b"CREATE INDEX i1 ON t1(a)").unwrap();
    // A reader is given the collations of the connection that wrote,
    // and reads no schema where it is given none.
    let image = writer.written();
    assert_eq!(
        Database::open(&image).unwrap_err().message(),
        "no such collation sequence: BACKWARDS"
    );
    let database = Database::open_collating(&image, COLLATING).expect("a database");
    assert_eq!(
        texts(&database, b"SELECT a FROM t1 ORDER BY a"),
        [b"aa", b"ba", b"ab", b"bb"]
    );
    assert_eq!(
        texts(&database, b"SELECT a FROM t1 ORDER BY a COLLATE BINARY"),
        [b"aa", b"ab", b"ba", b"bb"]
    );
    assert_eq!(
        texts(&database, b"SELECT a FROM t1 WHERE a > 'ba' ORDER BY a"),
        [b"ab", b"bb"]
    );
    // The name is read without its case, which is how
    // `sqlite3FindCollSeq` looks one up.
    assert_eq!(
        texts(&database, b"SELECT a FROM t1 ORDER BY a COLLATE backwards"),
        [b"aa", b"ba", b"ab", b"bb"]
    );
}

/// A statement that names a collation the connection does not define
/// is refused, whether the name stands in a column or in a statement.
#[test]
fn a_collation_the_connection_does_not_define_is_refused() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.collates(COLLATING);
    writer.run(b"CREATE TABLE t2(a)").unwrap();
    writer
        .run(b"INSERT INTO t2 VALUES('AB'),('ab'),('Ba')")
        .unwrap();
    assert_eq!(
        writer
            .run(b"CREATE TABLE t3(a COLLATE NOSUCH)")
            .unwrap_err()
            .message(),
        "no such collation sequence: NOSUCH"
    );
    let image = writer.written();
    let database = Database::open_collating(&image, COLLATING).expect("a database");
    assert_eq!(
        database
            .query(b"SELECT a FROM t2 ORDER BY a COLLATE NOSUCH")
            .unwrap_err()
            .message(),
        "no such collation sequence: NOSUCH"
    );
    assert_eq!(
        texts(
            &database,
            b"SELECT a FROM t2 ORDER BY a COLLATE CASELESS, a"
        ),
        [b"AB", b"ab", b"Ba"]
    );
}

/// A statement that writes rows compares under a collation the
/// connection defines, which the writer's own row answers, and a
/// reader that follows a write-ahead log is given the same collations.
#[test]
fn a_statement_that_writes_compares_under_a_defined_collation() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.collates(COLLATING);
    writer.run(b"PRAGMA journal_mode=wal").unwrap();
    writer.run(b"CREATE TABLE t4(a)").unwrap();
    writer
        .run(b"INSERT INTO t4 VALUES('AB'),('ab'),('Ba')")
        .unwrap();
    // `DELETE` compares the column against a value under the collation
    // the statement writes on it, which the writer's own row answers.
    assert!(
        writer
            .run(b"DELETE FROM t4 WHERE a = 'ab' COLLATE CASELESS")
            .expect("a statement")
            .is_empty()
    );
    let image = writer.written();
    let bytes = writer.log().expect("a log").to_vec();
    let log = Wal::open(&bytes).expect("a log");
    let database = Database::open_log_collating(&image, &log, COLLATING).expect("a database");
    assert_eq!(texts(&database, b"SELECT a FROM t4 ORDER BY a"), [b"Ba"]);
}

/// A `COLLATE` on a whole number of an `ORDER BY` or a `GROUP BY`
/// leaves the number counting the answered columns, and names the
/// collation the sort uses.
#[test]
fn a_collate_on_a_number_leaves_it_counting_the_columns() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.collates(COLLATING);
    writer.run(b"CREATE TABLE t5(a)").unwrap();
    writer
        .run(b"INSERT INTO t5 VALUES('aa'),('ab'),('ba'),('bb')")
        .unwrap();
    let image = writer.written();
    let database = Database::open_collating(&image, COLLATING).expect("a database");
    assert_eq!(
        texts(&database, b"SELECT a FROM t5 ORDER BY 1 COLLATE BACKWARDS"),
        [b"aa", b"ba", b"ab", b"bb"]
    );
    assert_eq!(
        texts(
            &database,
            b"SELECT a FROM t5 ORDER BY 1 COLLATE BACKWARDS DESC"
        ),
        [b"bb", b"ab", b"ba", b"aa"]
    );
    assert_eq!(
        texts(
            &database,
            b"SELECT a FROM t5 GROUP BY 1 COLLATE BACKWARDS ORDER BY 1"
        ),
        [b"aa", b"ab", b"ba", b"bb"]
    );
    // A number past the columns is refused with the `COLLATE` on it
    // taken off, which is what `resolveOrderGroupBy` counts against.
    assert_eq!(
        database
            .query(b"SELECT a FROM t5 ORDER BY 2 COLLATE BACKWARDS")
            .unwrap_err()
            .message(),
        "1st ORDER BY term out of range - should be between 1 and 1"
    );
}

/// Compares by the length of the value and then by its bytes.
fn by_length(_name: &'static [u8], left: &[u8], right: &[u8]) -> core::cmp::Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

/// Compares by the bytes of the value alone.
fn by_value(_name: &'static [u8], left: &[u8], right: &[u8]) -> core::cmp::Ordering {
    left.cmp(right)
}

/// One name under the first comparison.
static LENGTHWISE: &[Collating] = &[Collating {
    name: b"SORTER",
    by: by_length,
}];

/// The same name under the second.
static VALUEWISE: &[Collating] = &[Collating {
    name: b"SORTER",
    by: by_value,
}];

/// Answers that two values are the same whatever they hold.
fn as_one(_name: &'static [u8], _left: &[u8], _right: &[u8]) -> core::cmp::Ordering {
    core::cmp::Ordering::Equal
}

/// The same name under a comparison every value is one under.
static AS_ONE: &[Collating] = &[Collating {
    name: b"SORTER",
    by: as_one,
}];

/// The file the `ATTACH` of the reindex test names.
fn attaching(file: &[u8]) -> Option<Vec<u8>> {
    (file == b"two.db").then(|| Writer::new(1024, 0, Encoding::Utf8).unwrap().written())
}

/// The order the index of one database holds, as a reader told the
/// second comparison answers it out of that index.
fn ordered(writer: &Writer, name: Option<&[u8]>) -> Vec<Vec<u8>> {
    let image = match name {
        None => writer.written(),
        Some(name) => writer.attached_written(name).expect("a database beside"),
    };
    let database = Database::open_collating(&image, VALUEWISE).expect("a database");
    texts(&database, b"SELECT x FROM t1 ORDER BY x COLLATE SORTER")
}

/// `REINDEX` with no name written after it writes the indexes of every
/// database the connection holds again, a name under a schema writes the
/// indexes of that database alone, and a collation writes every index
/// held in it whatever database holds the index.
#[test]
fn which_databases_a_reindex_writes_the_indexes_of() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.opens(attaching);
    writer.collates(LENGTHWISE);
    for sql in [
        b"ATTACH 'two.db' AS aux".as_slice(),
        b"CREATE TABLE t1(x)",
        b"CREATE INDEX i1 ON t1(x COLLATE SORTER)",
        b"INSERT INTO t1 VALUES('aaa'),('bb'),('c')",
        b"CREATE TABLE aux.t1(x)",
        b"CREATE INDEX aux.i1 ON t1(x COLLATE SORTER)",
        b"INSERT INTO aux.t1 VALUES('aaa'),('bb'),('c')",
    ] {
        writer.run(sql).expect("a statement the writer takes");
    }
    let held = None;
    let beside = Some(b"aux".as_slice());
    let length: Vec<Vec<u8>> = alloc::vec![b"c".to_vec(), b"bb".to_vec(), b"aaa".to_vec()];
    let value: Vec<Vec<u8>> = alloc::vec![b"aaa".to_vec(), b"bb".to_vec(), b"c".to_vec()];
    assert_eq!(ordered(&writer, held), length);
    assert_eq!(ordered(&writer, beside), length);
    // The connection is told the second comparison under the same name,
    // and the entries of an index stand in the order the first wrote
    // them until a `REINDEX` writes them again.
    writer.collates(VALUEWISE);
    writer.run(b"REINDEX aux.t1").unwrap();
    assert_eq!(ordered(&writer, held), length);
    assert_eq!(ordered(&writer, beside), value);
    writer.run(b"REINDEX").unwrap();
    assert_eq!(ordered(&writer, held), value);
    assert_eq!(ordered(&writer, beside), value);
    writer.collates(LENGTHWISE);
    writer.run(b"REINDEX SORTER").unwrap();
    assert_eq!(ordered(&writer, held), length);
    assert_eq!(ordered(&writer, beside), length);
}

/// A `REINDEX` of a unique index whose collation the connection has been
/// told again is refused where two rows share the columns of the index
/// under the new comparison, which no row shared under the old one.
#[test]
fn what_a_reindex_of_a_unique_index_over_rows_that_share_a_key_is_refused_with() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.collates(LENGTHWISE);
    for sql in [
        b"CREATE TABLE t2(x)".as_slice(),
        b"INSERT INTO t2 VALUES('aa'),('b')",
        b"CREATE UNIQUE INDEX u ON t2(x COLLATE SORTER)",
    ] {
        writer.run(sql).expect("a statement the writer takes");
    }
    writer.collates(AS_ONE);
    assert_eq!(
        writer.run(b"REINDEX").unwrap_err().message(),
        "UNIQUE constraint failed: t2.x"
    );
}

/// A `COLLATE` on the argument of an aggregate or of a window function
/// is the collation the answer of the call is compared under, which is
/// the path `sqlite3ExprCollSeq` walks into the arguments of a call.
#[test]
fn what_a_collate_on_the_argument_of_an_aggregate_compares_under() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t6(a INTEGER PRIMARY KEY, b)")
        .unwrap();
    writer
        .run(b"INSERT INTO t6 VALUES(1,'abcd'),(2,'BCDE'),(3,'cdef'),(4,'DEFG')")
        .unwrap();
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    // The sort of a term that is no column and no number reads the
    // collation off the answer, which the call carries out of its
    // argument and the concatenation out of its left side.
    assert_eq!(
        texts(
            &database,
            b"SELECT max(b COLLATE nocase)||'' FROM t6 GROUP BY a \
              ORDER BY max(b COLLATE nocase)||''"
        ),
        [b"abcd", b"BCDE", b"cdef", b"DEFG"]
    );
    for (sql, answer) in [
        (
            b"SELECT max(b COLLATE nocase) = 'ABCD' FROM t6 WHERE a=1".as_slice(),
            1,
        ),
        (b"SELECT max(b) = 'ABCD' FROM t6 WHERE a=1", 0),
        (
            b"SELECT max(b COLLATE nocase) OVER () = 'ABCD' FROM t6 WHERE a=1",
            1,
        ),
    ] {
        assert_eq!(
            database.query(sql).unwrap().rows,
            alloc::vec![alloc::vec![crate::value::Value::Int(answer)]],
            "{sql:?}"
        );
    }
}
