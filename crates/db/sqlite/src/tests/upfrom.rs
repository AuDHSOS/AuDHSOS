// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `UPDATE ... FROM`, against what the shell answers for the same
//! statements.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// What a statement answers, as `value|` per value.
fn shown(writer: &Writer, sql: &[u8]) -> String {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answered = database.query(sql).unwrap();
    let mut out = String::new();
    for row in &answered.rows {
        for value in row {
            out.push_str(&String::from_utf8_lossy(
                &value.text().unwrap_or(b"NULL".to_vec()),
            ));
            out.push('|');
        }
    }
    out
}

/// The three counters of the connection, as `changes|total|rowid`.
fn counts(writer: &Writer) -> String {
    let image = writer.written();
    let database = Database::open(&image).unwrap().counting(writer.counts());
    let answered = database.query(b"SELECT changes()").unwrap();
    String::from_utf8_lossy(
        &answered
            .rows
            .first()
            .unwrap()
            .first()
            .unwrap()
            .text()
            .unwrap(),
    )
    .into_owned()
}

/// A connection over a table, the rows a clause names, and a second
/// table a join reaches.
fn writer(without: &[u8]) -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    let mut made = b"CREATE TABLE t1(x PRIMARY KEY, y, z) ".to_vec();
    made.extend_from_slice(without);
    writer.run(&made).unwrap();
    for sql in [
        b"INSERT INTO t1 VALUES(1,'i','one'),(2,'ii','two'),(3,'iii','three')".as_slice(),
        b"CREATE TABLE d(k,v)",
        b"INSERT INTO d VALUES(1,'ten'),(3,'thirty')",
        b"CREATE TABLE a(u,w)",
        b"INSERT INTO a VALUES(2,'q')",
    ] {
        writer.run(sql).unwrap();
    }
    writer
}

/// Each row of the table is written against the row of the clause the
/// `WHERE` holds for, and a row the clause holds more than one row for
/// takes the last of them.
#[test]
fn a_row_is_written_against_the_row_of_the_clause_the_where_holds_for() {
    for without in [b"".as_slice(), b"WITHOUT ROWID"] {
        let mut writer = writer(without);
        writer.run(b"UPDATE t1 SET z=v FROM d WHERE x=k").unwrap();
        assert_eq!(
            shown(&writer, b"SELECT x,y,z FROM t1 ORDER BY x"),
            "1|i|ten|2|ii|two|3|iii|thirty|",
            "{without:?}"
        );
        assert_eq!(counts(&writer), "2", "{without:?}");
        writer
            .run(
                b"WITH data(k,v) AS (VALUES(1,'seven'),(1,'eight')) \
                  UPDATE t1 SET z=v FROM data WHERE x=k",
            )
            .unwrap();
        assert_eq!(
            shown(&writer, b"SELECT z FROM t1 WHERE x=1"),
            "eight|",
            "{without:?}"
        );
        assert_eq!(counts(&writer), "1", "{without:?}");
        // Two tables of a clause are one join, and a column names the
        // table it came from.
        writer
            .run(b"UPDATE t1 SET y=w, z=d.v FROM d, a WHERE x=k AND d.k=1 AND a.u=2")
            .unwrap();
        assert_eq!(
            shown(&writer, b"SELECT x,y,z FROM t1 ORDER BY x"),
            "1|q|ten|2|ii|two|3|iii|thirty|",
            "{without:?}"
        );
    }
}

/// The table an `UPDATE` changes may not be named again in its `FROM`,
/// and a `WITH` before an `UPDATE` that writes no `FROM` is refused.
#[test]
fn what_an_update_from_refuses() {
    let mut writer = writer(b"");
    assert_eq!(
        writer
            .run(b"UPDATE t1 SET z='z' FROM t1")
            .unwrap_err()
            .message(),
        "target object/alias may not appear in FROM clause: t1"
    );
    assert_eq!(
        writer
            .run(b"UPDATE t1 SET z='z' FROM d AS t1")
            .unwrap_err()
            .message(),
        "target object/alias may not appear in FROM clause: t1"
    );
    // A `WITH` before an `UPDATE` names the tables of its `FROM`.
    assert!(
        writer
            .run(b"WITH data(k) AS (VALUES(1)) UPDATE t1 SET z='z' WHERE x IN (SELECT k FROM data)")
            .is_err()
    );
}

/// An `INSTEAD OF UPDATE` trigger of a view runs once per row of the
/// join, duplicates included.
#[test]
fn a_view_runs_its_trigger_once_per_row_of_the_join() {
    let mut writer = writer(b"");
    for sql in [
        b"CREATE TABLE log(t)".as_slice(),
        b"CREATE VIEW v1 AS SELECT * FROM t1",
        b"CREATE TRIGGER v1tr INSTEAD OF UPDATE ON v1 BEGIN INSERT INTO log VALUES(new.z); END",
        b"WITH data(k,v) AS (VALUES(1,'seven'),(1,'eight')) \
          UPDATE v1 SET z=v FROM data WHERE x=k",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(shown(&writer, b"SELECT t FROM log"), "seven|eight|");
}

/// A source of a clause is a table under the one schema a file holds,
/// a statement in brackets, or a name the `WITH` before the statement
/// gave; a name under another schema reaches nothing.
#[test]
fn what_a_column_of_a_clause_is_named_by() {
    let mut writer = writer(b"");
    writer
        .run(b"UPDATE t1 SET z=v FROM main.d WHERE x=k")
        .unwrap();
    assert_eq!(shown(&writer, b"SELECT z FROM t1 WHERE x=1"), "ten|");
    writer
        .run(b"UPDATE t1 SET z=s.n FROM (SELECT 2 AS m, 'sub' AS n) AS s WHERE x=s.m")
        .unwrap();
    assert_eq!(shown(&writer, b"SELECT z FROM t1 WHERE x=2"), "sub|");
    // A statement in brackets under no name of its own.
    writer
        .run(b"UPDATE t1 SET z='bare' FROM (SELECT 3 AS m) WHERE x=m")
        .unwrap();
    assert_eq!(shown(&writer, b"SELECT z FROM t1 WHERE x=3"), "bare|");
    assert_eq!(
        writer
            .run(b"UPDATE t1 SET z=other.d.v FROM d WHERE x=k")
            .unwrap_err()
            .message(),
        "no such column: other.d.v"
    );
    // A column of one table of the clause under the name of another.
    assert_eq!(
        writer
            .run(b"UPDATE t1 SET z=a.v FROM d, a WHERE x=k")
            .unwrap_err()
            .message(),
        "no such column: a.v"
    );
    // A table under another schema is another object, so the name of
    // the table the statement changes reaches it.
    assert_eq!(
        writer
            .run(b"UPDATE t1 SET z='z' FROM other.t1")
            .unwrap_err()
            .message(),
        "no such table: other.t1"
    );
}

/// An `UPDATE ... FROM` of a trigger's body reads `new` and `old`
/// through the row of the clause.
#[test]
fn a_clause_of_a_triggers_body_reads_the_row_the_trigger_stands_on() {
    let mut writer = writer(b"");
    for sql in [
        b"CREATE TABLE q(a)".as_slice(),
        b"CREATE TRIGGER tr AFTER INSERT ON q BEGIN \
          UPDATE t1 SET z=v||new.a FROM d WHERE x=k; END",
        b"INSERT INTO q VALUES('!')",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(shown(&writer, b"SELECT z FROM t1 WHERE x=1"), "ten!|");
}
