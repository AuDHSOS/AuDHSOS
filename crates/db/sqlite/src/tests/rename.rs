// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ALTER TABLE ... RENAME TO`, against what the shell writes for the
//! same statements.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// A connection over a database of one page, written under 512-byte
/// pages, which is what the fixture was written under.
fn writer() -> Writer {
    Writer::new(512, 0, Encoding::Utf8).unwrap()
}

/// The rows of `sqlite_schema` as `type|name|tbl_name|sql` lines.
fn schema(writer: &Writer) -> alloc::string::String {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answer = database
        .query(b"SELECT type,name,tbl_name,sql FROM sqlite_schema")
        .unwrap();
    let mut shown = alloc::string::String::new();
    for row in &answer.rows {
        for value in row {
            shown.push_str(&alloc::string::String::from_utf8_lossy(
                &value.text().unwrap_or_default(),
            ));
            shown.push('|');
        }
        shown.push('\n');
    }
    shown
}

#[test]
fn every_statement_that_names_the_table_is_written_again_under_the_new_name() {
    let mut writer = writer();
    for sql in [
        b"CREATE TABLE t(a INTEGER PRIMARY KEY AUTOINCREMENT, b UNIQUE, c)".as_slice(),
        b"CREATE INDEX i ON t(c)",
        b"CREATE VIEW v AS SELECT a FROM t WHERE b>1",
        b"CREATE TRIGGER tr AFTER INSERT ON t BEGIN UPDATE t SET c=c+1; END",
        b"INSERT INTO t(b) VALUES(5)",
        b"ALTER TABLE t RENAME TO u",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(
        schema(&writer),
        concat!(
            "table|u|u|CREATE TABLE \"u\"(a INTEGER PRIMARY KEY AUTOINCREMENT, b UNIQUE, c)|\n",
            "index|sqlite_autoindex_u_1|u||\n",
            "table|sqlite_sequence|sqlite_sequence|CREATE TABLE sqlite_sequence(name,seq)|\n",
            "index|i|u|CREATE INDEX i ON \"u\"(c)|\n",
            "view|v|v|CREATE VIEW v AS SELECT a FROM \"u\" WHERE b>1|\n",
            "trigger|tr|u|CREATE TRIGGER tr AFTER INSERT ON \"u\" BEGIN UPDATE \"u\" SET c=c+1; END|\n",
        )
    );
    // The row that counts the key up is written again under the new
    // name, which is what `sqlite3AlterRenameTable` writes for a table
    // that counts.
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answer = database.query(b"SELECT name FROM sqlite_sequence").unwrap();
    assert_eq!(
        answer.rows.first().and_then(|row| row.first()),
        Some(&crate::value::Value::Text(b"u".to_vec()))
    );
    drop(database);
    assert_eq!(
        writer.written(),
        include_bytes!("fixtures/renamed.db").to_vec()
    );
}

#[test]
fn the_rows_of_the_table_are_read_under_the_new_name() {
    let mut writer = writer();
    for sql in [
        b"CREATE TABLE t(a, b)".as_slice(),
        b"CREATE INDEX i ON t(b)",
        b"INSERT INTO t VALUES(1,'x'),(2,'y')",
        b"ALTER TABLE t RENAME TO u",
        b"INSERT INTO u VALUES(3,'z')",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    assert_eq!(database.rows_of(b"u").unwrap().len(), 3);
    assert_eq!(
        database
            .query(b"SELECT a FROM u WHERE b='z'")
            .unwrap()
            .rows
            .first()
            .and_then(|row| row.first()),
        Some(&crate::value::Value::Int(3))
    );
    assert!(database.rows_of(b"t").is_err());
}

#[test]
fn a_name_that_is_not_a_table_of_its_own_is_refused() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"CREATE TABLE s(b)").unwrap();
    writer.run(b"CREATE VIEW v AS SELECT 1").unwrap();
    writer.run(b"CREATE INDEX ti ON t(a)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    writer.run(b"ANALYZE").unwrap();
    for (sql, message) in [
        (
            b"ALTER TABLE nope RENAME TO x".as_slice(),
            "no such table: nope",
        ),
        (
            b"ALTER TABLE t RENAME TO s",
            "there is already another table or index with this name: s",
        ),
        (
            b"ALTER TABLE t RENAME TO v",
            "there is already another table or index with this name: v",
        ),
        (
            b"ALTER TABLE t RENAME TO ti",
            "there is already another table or index with this name: ti",
        ),
        (b"ALTER TABLE v RENAME TO w", "view v may not be altered"),
        (
            b"ALTER TABLE sqlite_schema RENAME TO x",
            "table sqlite_master may not be altered",
        ),
        (
            b"ALTER TABLE sqlite_stat1 RENAME TO x",
            "table sqlite_stat1 may not be altered",
        ),
        (
            b"ALTER TABLE sqlite_nope RENAME TO x",
            "no such table: sqlite_nope",
        ),
    ] {
        assert_eq!(writer.run(sql).unwrap_err().message(), message, "{sql:?}");
    }
}

#[test]
fn a_name_a_statement_only_reads_like_is_left_alone() {
    let mut writer = writer();
    for sql in [
        b"CREATE TABLE t(a, t)".as_slice(),
        b"CREATE TABLE other(t)",
        b"CREATE VIEW v AS SELECT 't' AS t FROM other WHERE t='t'",
        b"CREATE VIEW w AS WITH t AS (SELECT 1 AS a) SELECT a FROM t",
        b"ALTER TABLE t RENAME TO u",
    ] {
        writer.run(sql).unwrap();
    }
    // The column named `t`, the text `'t'` and the `WITH` term named
    // `t` name no table, so the rename leaves every one of them alone.
    assert_eq!(
        schema(&writer),
        concat!(
            "table|u|u|CREATE TABLE \"u\"(a, t)|\n",
            "table|other|other|CREATE TABLE other(t)|\n",
            "view|v|v|CREATE VIEW v AS SELECT 't' AS t FROM other WHERE t='t'|\n",
            "view|w|w|CREATE VIEW w AS WITH t AS (SELECT 1 AS a) SELECT a FROM t|\n",
        )
    );
}

#[test]
fn a_new_name_is_written_into_the_statements_with_its_quotes() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"ALTER TABLE t RENAME TO \"new name\"").unwrap();
    assert_eq!(
        schema(&writer),
        "table|new name|new name|CREATE TABLE \"new name\"(a)|\n"
    );
    // A name with a quote in it carries that quote doubled, which is
    // what `%w` of the C library writes.
    writer
        .run(b"ALTER TABLE \"new name\" RENAME TO \"a\"\"b\"")
        .unwrap();
    assert_eq!(
        schema(&writer),
        "table|a\"b|a\"b|CREATE TABLE \"a\"\"b\"(a)|\n"
    );
}

#[test]
fn what_the_parser_reads_of_a_rename() {
    let (_, definition) = crate::parse::definition(b"ALTER TABLE a.t RENAME TO u").unwrap();
    let crate::ast::Definition::Rename(asked) = definition else {
        panic!("a rename was written");
    };
    let sql = b"ALTER TABLE a.t RENAME TO u";
    assert_eq!(
        asked.schema.map(|span| span.text(sql)),
        Some(b"a".as_slice())
    );
    assert_eq!(asked.table.text(sql), b"t");
    assert_eq!(asked.name.text(sql), b"u");
    for sql in [
        b"ALTER TABLE t RENAME".as_slice(),
        b"ALTER TABLE t RENAME TO",
        b"ALTER TABLE t RENAME COLUMN a TO",
        b"ALTER TABLE t RENAME a",
        b"ALTER TABLE t",
    ] {
        assert!(crate::parse::definition(sql).is_err(), "{sql:?}");
    }
}

/// Where a statement names a table, read out of the tree the parser
/// built and not out of the text.
#[test]
fn every_place_a_statement_names_the_table() {
    let sql = b"CREATE TRIGGER tr AFTER INSERT ON t BEGIN INSERT INTO t SELECT * FROM t; END";
    let places = crate::rename::places(sql, b"t");
    assert_eq!(places.len(), 3);
    assert_eq!(
        crate::rename::written(sql, &places, b"u"),
        b"CREATE TRIGGER tr AFTER INSERT ON \"u\" BEGIN INSERT INTO \"u\" SELECT * FROM \"u\"; END"
            .to_vec()
    );
    // A statement the parser refuses names nothing.
    assert_eq!(crate::rename::places(b"NOT A STATEMENT", b"t"), Vec::new());
    // A name that is not an index SQLite made for a key of the table is
    // left as it is.
    assert_eq!(crate::rename::automatic(b"i", b"t", b"u"), None);
    assert_eq!(
        crate::rename::automatic(b"sqlite_autoindex_s_1", b"t", b"u"),
        None
    );
    assert_eq!(
        crate::rename::automatic(b"sqlite_autoindex_tt", b"t", b"u"),
        None
    );
    assert_eq!(
        crate::rename::automatic(b"sqlite_autoindex_t_2", b"t", b"u"),
        Some(b"sqlite_autoindex_u_2".to_vec())
    );
}

#[test]
fn a_statement_that_names_another_table_names_nothing_of_this_one() {
    for sql in [
        b"CREATE TABLE other(a)".as_slice(),
        b"CREATE INDEX i ON other(a)",
        b"CREATE VIEW v AS SELECT a FROM other",
        b"CREATE TRIGGER tr AFTER INSERT ON other BEGIN DELETE FROM other; SELECT 1; END",
    ] {
        assert_eq!(crate::rename::places(sql, b"t"), Vec::new(), "{sql:?}");
    }
    // A statement in brackets is no name of a table, and the statement
    // inside it carries the names.
    let sql = b"CREATE VIEW v AS SELECT a FROM (SELECT a FROM t)";
    assert_eq!(crate::rename::places(sql, b"t").len(), 1);
    // A step that names the table counts, whatever statement it is, and
    // a step that names another table does not.
    let sql = b"CREATE TRIGGER tr AFTER INSERT ON t BEGIN DELETE FROM t; UPDATE other SET a=1; SELECT 1; END";
    assert_eq!(crate::rename::places(sql, b"t").len(), 2);
}

#[test]
fn a_column_dropped_goes_out_of_the_statement_and_out_of_every_row() {
    let mut writer = writer();
    for sql in [
        b"CREATE TABLE t(a int, b text DEFAULT 'x', c blob, d)".as_slice(),
        b"CREATE INDEX i ON t(d)",
        b"INSERT INTO t VALUES(1,'p',x'01',4),(2,'q',x'02',5)",
        b"ALTER TABLE t DROP COLUMN b",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(
        schema(&writer),
        concat!(
            "table|t|t|CREATE TABLE t(a int, c blob, d)|\n",
            "index|i|t|CREATE INDEX i ON t(d)|\n",
        )
    );
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answer = database
        .query(b"SELECT a||'/'||quote(c)||'/'||d FROM t ORDER BY a")
        .unwrap();
    let shown: Vec<Vec<u8>> = answer
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(crate::value::Value::text))
        .collect();
    assert_eq!(
        shown,
        alloc::vec![b"1/X'01'/4".to_vec(), b"2/X'02'/5".to_vec()]
    );
    // The index over a column that stayed answers the rows it held.
    assert_eq!(
        database
            .query(b"SELECT a FROM t WHERE d=5")
            .unwrap()
            .rows
            .first()
            .and_then(|row| row.first()),
        Some(&crate::value::Value::Int(2))
    );
    assert_eq!(
        database
            .query(b"PRAGMA integrity_check")
            .unwrap()
            .rows
            .first()
            .and_then(|row| row.first()),
        Some(&crate::value::Value::Text(b"ok".to_vec()))
    );
}

#[test]
fn the_first_column_and_the_last_take_the_comma_beside_them() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE t(a int, b text)").unwrap();
    writer.run(b"ALTER TABLE t DROP COLUMN a").unwrap();
    assert_eq!(schema(&writer), "table|t|t|CREATE TABLE t(b text)|\n");
    let mut other = writer;
    other.run(b"CREATE TABLE u(a int, b text)").unwrap();
    other.run(b"ALTER TABLE u DROP COLUMN b").unwrap();
    assert_eq!(
        schema(&other),
        concat!(
            "table|t|t|CREATE TABLE t(b text)|\n",
            "table|u|u|CREATE TABLE u(a int)|\n",
        )
    );
}

#[test]
fn a_column_a_key_is_over_or_the_one_column_of_a_table_is_not_dropped() {
    let mut writer = writer();
    for sql in [
        b"CREATE TABLE u(a PRIMARY KEY, b UNIQUE, c)".as_slice(),
        b"CREATE TABLE w(a)",
    ] {
        writer.run(sql).unwrap();
    }
    for (sql, message) in [
        (
            b"ALTER TABLE u DROP COLUMN zz".as_slice(),
            "no such column: \"zz\"",
        ),
        (
            b"ALTER TABLE u DROP COLUMN a",
            "cannot drop PRIMARY KEY column: \"a\"",
        ),
        (
            b"ALTER TABLE u DROP COLUMN b",
            "cannot drop UNIQUE column: \"b\"",
        ),
        (
            b"ALTER TABLE w DROP COLUMN a",
            "cannot drop column \"a\": no other columns exist",
        ),
        (b"ALTER TABLE nope DROP COLUMN a", "no such table: nope"),
        (
            b"ALTER TABLE sqlite_schema DROP COLUMN a",
            "table sqlite_master may not be altered",
        ),
    ] {
        assert_eq!(writer.run(sql).unwrap_err().message(), message, "{sql:?}");
    }
}

#[test]
fn a_statement_that_names_the_column_holds_it_where_it_is() {
    for (made, sql, message) in [
        (
            alloc::vec![b"CREATE TABLE x(a,b)".as_slice(), b"CREATE INDEX i ON x(a)"],
            b"ALTER TABLE x DROP COLUMN a".as_slice(),
            "error in index i after drop column: no such column: a",
        ),
        (
            alloc::vec![
                b"CREATE TABLE y(a,b)",
                b"CREATE TABLE other(x UNIQUE)",
                b"CREATE VIEW v AS SELECT 1+1, a FROM y",
            ],
            b"ALTER TABLE y DROP COLUMN a",
            "error in view v after drop column: no such column: a",
        ),
        (
            alloc::vec![b"CREATE TABLE z(a,b, CHECK(a>0))"],
            b"ALTER TABLE z DROP COLUMN a",
            "error in table z after drop column: no such column: a",
        ),
        (
            alloc::vec![
                b"CREATE TABLE q(a,b)",
                b"CREATE TABLE log(what)",
                b"CREATE TRIGGER tr AFTER INSERT ON q BEGIN INSERT INTO log VALUES(new.a); END",
            ],
            b"ALTER TABLE q DROP COLUMN a",
            "error in trigger tr after drop column: no such column: new.a",
        ),
    ] {
        let mut writer = writer();
        for statement in made {
            writer.run(statement).unwrap();
        }
        let before = writer.written();
        assert_eq!(writer.run(sql).unwrap_err().message(), message, "{sql:?}");
        // The statement is refused, so the file is what it was.
        assert_eq!(writer.written(), before, "{sql:?}");
    }
}

/// The statement of a table that writes no columns out, and one that
/// writes a column a key of its own is over, which the text a drop
/// answers is read against.
#[test]
fn what_the_text_of_a_dropped_column_is_read_against() {
    assert!(matches!(
        crate::change::without_column(b"CREATE INDEX i ON t(a)", b"a"),
        Err(crate::db::Error::NoTable(_))
    ));
    assert!(matches!(
        crate::change::without_column(b"CREATE TABLE t AS SELECT 1 AS a", b"a"),
        Err(crate::db::Error::NoTable(_))
    ));
    assert_eq!(
        crate::change::without_column(b"CREATE TABLE t(a, b)", b"a").unwrap(),
        b"CREATE TABLE t(b)".to_vec()
    );
    assert!(matches!(
        crate::change::without_column(b"CREATE TABLE t(a, b)", b"zz"),
        Err(crate::db::Error::NoSuchColumn(_))
    ));
}

#[test]
fn what_the_parser_reads_of_a_dropped_column() {
    let sql = b"ALTER TABLE a.t DROP COLUMN c";
    let (_, definition) = crate::parse::definition(sql).unwrap();
    let crate::ast::Definition::DropColumn(asked) = definition else {
        panic!("a dropped column was written");
    };
    assert_eq!(
        asked.schema.map(|span| span.text(sql)),
        Some(b"a".as_slice())
    );
    assert_eq!(asked.table.text(sql), b"t");
    assert_eq!(asked.column.text(sql), b"c");
    // The word `COLUMN` may be left out, and a name must follow.
    assert!(crate::parse::definition(b"ALTER TABLE t DROP c").is_ok());
    assert!(crate::parse::definition(b"ALTER TABLE t DROP COLUMN").is_err());
}
