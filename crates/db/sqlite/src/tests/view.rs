// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a view answers its columns as, what its column list may carry,
//! and what a `DROP` of the wrong kind is refused with.

use crate::change::Writer;
use crate::db::{Database, Error};
use crate::header::Encoding;
use crate::value::Value;

/// A connection over one table and one view of it.
fn writing() -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(x INTEGER, y)".as_slice(),
        b"CREATE VIEW v1 AS SELECT * FROM t1",
    ] {
        writer.run(sql).unwrap();
    }
    writer
}

/// `PRAGMA table_info` over a view answers the columns its statement
/// answers, under the names the column list wrote where it wrote one.
#[test]
fn what_columns_a_view_answers() {
    let mut writer = writing();
    writer
        .run(b"CREATE VIEW v2(p, q) AS SELECT x+1, y FROM t1")
        .unwrap();
    writer
        .run(b"CREATE VIEW v5(a) AS SELECT x, y FROM t1")
        .unwrap();
    writer.run(b"CREATE VIEW \"a\"\"b\" AS SELECT 1").unwrap();
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    let named = |sql: &[u8]| -> alloc::vec::Vec<alloc::vec::Vec<Value>> {
        database.query(sql).unwrap().rows
    };
    // The declared type of a column of a table reaches the view, and an
    // expression carries none.
    assert_eq!(
        named(b"PRAGMA table_info(v1)"),
        [
            alloc::vec![
                Value::Int(0),
                Value::Text(b"x".to_vec()),
                Value::Text(b"INTEGER".to_vec()),
                Value::Int(0),
                Value::Null,
                Value::Int(0)
            ],
            alloc::vec![
                Value::Int(1),
                Value::Text(b"y".to_vec()),
                Value::Text(Vec::new()),
                Value::Int(0),
                Value::Null,
                Value::Int(0)
            ]
        ]
    );
    // The names the column list wrote name the columns.
    let written: alloc::vec::Vec<Value> = named(b"PRAGMA table_info(v2)")
        .into_iter()
        .filter_map(|row| row.get(1).cloned())
        .collect();
    assert_eq!(
        written,
        [Value::Text(b"p".to_vec()), Value::Text(b"q".to_vec())]
    );
    // A name no view and no table carries answers no row.
    assert!(named(b"PRAGMA table_info(nosuch)").is_empty());
    // A view whose statement does not run answers no row either, and a
    // quote in the name of a view is written twice where the pragma
    // reads the view again.
    assert!(named(b"PRAGMA table_info(v5)").is_empty());
    assert_eq!(named(b"PRAGMA table_info('a\"b')").len(), 1);
    // `PRAGMA table_xinfo` answers the same columns with the flag of a
    // generated column after them.
    assert_eq!(named(b"PRAGMA table_xinfo(v1)").len(), 2);
}

/// A column list of another width than the statement answers is refused
/// where the view is read, and a word after a name of it where the view
/// is made.
#[test]
fn what_a_column_list_of_a_view_may_carry() {
    let mut writer = writing();
    assert_eq!(
        writer
            .run(b"CREATE VIEW v3(a, b DESC) AS SELECT x, y FROM t1")
            .unwrap_err()
            .message(),
        "syntax error after column name \"b\""
    );
    // The width is read where the view is read and not where it is made.
    writer
        .run(b"CREATE VIEW v4(a) AS SELECT x, y FROM t1")
        .unwrap();
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    assert_eq!(
        database.query(b"SELECT * FROM v4").unwrap_err().message(),
        "expected 1 columns for 'v4' but got 2"
    );
    assert_eq!(
        database.query(b"SELECT * FROM v4").unwrap_err(),
        Error::ViewWidth(b"v4".to_vec(), 1, 2)
    );
}

/// A `DROP TABLE` over a view and a `DROP VIEW` over a table each name
/// the kind the schema holds.
#[test]
fn what_a_drop_of_the_other_kind_is_refused_with() {
    let mut writer = writing();
    assert_eq!(
        writer.run(b"DROP TABLE v1").unwrap_err().message(),
        "use DROP VIEW to delete view v1"
    );
    assert_eq!(
        writer.run(b"DROP VIEW t1").unwrap_err().message(),
        "use DROP TABLE to delete table t1"
    );
    // A name the schema does not hold at all names what the statement
    // said it makes, and `IF EXISTS` writes nothing.
    assert_eq!(
        writer.run(b"DROP VIEW nosuch").unwrap_err().message(),
        "no such view: nosuch"
    );
    writer.run(b"DROP VIEW IF EXISTS nosuch").unwrap();
    // A `DROP INDEX` of a name a view carries is no view either.
    assert_eq!(
        writer.run(b"DROP INDEX v1").unwrap_err().message(),
        "no such index: v1"
    );
}

/// The body of a view and the body of a trigger each read a bare name
/// under the database that holds them, so a name no database of theirs
/// holds is refused with that database in front of it; a name the body
/// wrote a database in front of keeps the one it wrote.
#[test]
fn which_database_a_body_that_names_no_table_is_refused_under() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.opens(beside);
    for sql in [
        b"ATTACH 'two.db' AS aux".as_slice(),
        b"CREATE TABLE t1(x)",
        b"CREATE TABLE gone(y)",
        b"CREATE VIEW v1 AS SELECT * FROM gone",
        // The body of a view names no database of its own, so a view
        // that reads the attached database stands in it.
        b"CREATE VIEW aux.v2 AS SELECT * FROM gone",
        // A name the body wrote a database in front of keeps the one it
        // wrote.
        b"CREATE VIEW v3 AS SELECT * FROM main.gone",
        b"CREATE TRIGGER r1 AFTER INSERT ON t1 BEGIN INSERT INTO gone VALUES(1); END",
        b"DROP TABLE gone",
    ] {
        writer.run(sql).expect("a statement the writer takes");
    }
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    assert_eq!(
        database.query(b"SELECT * FROM v1").unwrap_err().message(),
        "no such table: main.gone"
    );
    let beside = writer
        .attached_written(b"aux")
        .expect("the attached database");
    let every = Database::open(&image)
        .expect("a database")
        .attaching(b"aux", &beside)
        .expect("the attached database");
    assert_eq!(
        every.query(b"SELECT * FROM v2").unwrap_err().message(),
        "no such table: aux.gone"
    );
    assert_eq!(
        database.query(b"SELECT * FROM v3").unwrap_err().message(),
        "no such table: main.gone"
    );
    assert_eq!(
        writer
            .run(b"INSERT INTO t1 VALUES(1)")
            .unwrap_err()
            .message(),
        "no such table: main.gone"
    );
}

/// The file the `ATTACH` of that test names.
fn beside(file: &[u8]) -> Option<alloc::vec::Vec<u8>> {
    (file == b"two.db").then(|| Writer::new(1024, 0, Encoding::Utf8).unwrap().written())
}

/// `sqlite3CreateView` refuses a statement that holds a bound parameter
/// before it reads the name, so a second statement under the same name is
/// refused the same way and no view stands.
#[test]
fn what_a_view_whose_statement_holds_a_parameter_is_refused_with() {
    let mut writer = writing();
    for sql in [
        b"CREATE VIEW v12 AS SELECT x FROM t1 WHERE y=?".as_slice(),
        b"CREATE VIEW v12(a) AS SELECT x FROM t1 WHERE y=?1",
        b"CREATE VIEW v12 AS SELECT x FROM t1 WHERE y=:one",
    ] {
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "parameters are not allowed in views",
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    assert_eq!(
        database
            .query(b"SELECT count(*) FROM sqlite_schema WHERE name='v12'")
            .unwrap()
            .rows,
        alloc::vec![alloc::vec![Value::Int(0)]]
    );
}
