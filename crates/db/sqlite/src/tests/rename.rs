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
    let places = crate::rename::places(sql, b"t", crate::rename::Marking::Every);
    assert_eq!(places.len(), 3);
    assert_eq!(
        crate::rename::written(sql, &places, b"u"),
        b"CREATE TRIGGER tr AFTER INSERT ON \"u\" BEGIN INSERT INTO \"u\" SELECT * FROM \"u\"; END"
            .to_vec()
    );
    // The table a column reference names is one of those places, and a
    // name a source carries as an alias or a `WITH` term carries is
    // not.
    for (sql, want) in [
        (
            b"CREATE VIEW v AS SELECT main.t.a, t.b FROM t".as_slice(),
            b"CREATE VIEW v AS SELECT main.\"u\".a, \"u\".b FROM \"u\"".as_slice(),
        ),
        // A qualifier that names another table is left alone.
        (
            b"CREATE VIEW v AS SELECT t.a, s.b FROM t, s",
            b"CREATE VIEW v AS SELECT \"u\".a, s.b FROM \"u\", s",
        ),
        (
            b"CREATE VIEW v AS SELECT t.a FROM s AS t",
            b"CREATE VIEW v AS SELECT t.a FROM s AS t",
        ),
        (
            b"CREATE VIEW v AS WITH t(a) AS (SELECT 1) SELECT t.a FROM t",
            b"CREATE VIEW v AS WITH t(a) AS (SELECT 1) SELECT t.a FROM t",
        ),
        (
            b"CREATE INDEX i ON t(a) WHERE t.b>0",
            b"CREATE INDEX i ON \"u\"(a) WHERE \"u\".b>0",
        ),
    ] {
        let places = crate::rename::places(sql, b"t", crate::rename::Marking::Every);
        assert_eq!(
            alloc::string::String::from_utf8_lossy(&crate::rename::written(sql, &places, b"u")),
            alloc::string::String::from_utf8_lossy(want),
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
    // A statement the parser refuses names nothing.
    assert_eq!(
        crate::rename::places(b"NOT A STATEMENT", b"t", crate::rename::Marking::Every),
        Vec::new()
    );
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

/// Where a `CREATE TABLE` names the table as the parent of a foreign
/// key, which `ALTER TABLE ... RENAME TO` writes the new name at.
#[test]
fn every_place_a_foreign_key_names_the_parent() {
    // A column's own `REFERENCES`, the table's own name beside it, and
    // a `FOREIGN KEY` clause of the table.
    let sql = b"CREATE TABLE t(a PRIMARY KEY, b REFERENCES t, c, FOREIGN KEY(c) REFERENCES t(a))";
    let places = crate::rename::places(sql, b"t", crate::rename::Marking::Every);
    assert_eq!(places.len(), 3);
    assert_eq!(
        crate::rename::written(sql, &places, b"u"),
        b"CREATE TABLE \"u\"(a PRIMARY KEY, b REFERENCES \"u\", c, FOREIGN KEY(c) REFERENCES \"u\"(a))"
            .to_vec()
    );
    // A key that names another table is left as it is, and so is a
    // table whose columns come out of a statement.
    let sql = b"CREATE TABLE other(a REFERENCES third, UNIQUE(a), FOREIGN KEY(a) REFERENCES third)";
    assert_eq!(
        crate::rename::places(sql, b"t", crate::rename::Marking::Every),
        Vec::new()
    );
    assert_eq!(
        crate::rename::places(
            b"CREATE TABLE other AS SELECT 1",
            b"t",
            crate::rename::Marking::Every
        ),
        Vec::new()
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
        assert_eq!(
            crate::rename::places(sql, b"t", crate::rename::Marking::Every),
            Vec::new(),
            "{sql:?}"
        );
    }
    // A statement in brackets is no name of a table, and the statement
    // inside it carries the names.
    let sql = b"CREATE VIEW v AS SELECT a FROM (SELECT a FROM t)";
    assert_eq!(
        crate::rename::places(sql, b"t", crate::rename::Marking::Every).len(),
        1
    );
    // A step that names the table counts, whatever statement it is, and
    // a step that names another table does not.
    let sql = b"CREATE TRIGGER tr AFTER INSERT ON t BEGIN DELETE FROM t; UPDATE other SET a=1; SELECT 1; END";
    assert_eq!(
        crate::rename::places(sql, b"t", crate::rename::Marking::Every).len(),
        2
    );
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

/// The rows of the temp schema as `type|name|tbl_name|sql` lines.
fn temped(writer: &Writer) -> alloc::string::String {
    let image = writer.temp().unwrap_or_default();
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

/// The statements a connection runs in turn, each of which must be
/// taken.
fn ran(writer: &mut Writer, statements: &[&[u8]]) {
    for sql in statements {
        writer.run(sql).unwrap();
    }
}

#[test]
fn a_view_and_a_trigger_of_the_temp_schema_are_written_again_under_the_new_name() {
    let mut writer = writer();
    ran(
        &mut writer,
        &[
            b"CREATE TABLE t1(a,b)",
            b"CREATE TABLE other(a)",
            b"CREATE TEMP TABLE x(y)",
            b"CREATE TEMP VIEW v1 AS SELECT a FROM t1",
            b"CREATE TEMP VIEW v2 AS SELECT a FROM other",
            b"CREATE TEMP TRIGGER tr1 AFTER INSERT ON t1 BEGIN INSERT INTO t1(a) VALUES(new.a); END",
            b"CREATE TEMP TRIGGER tr2 AFTER UPDATE ON main.t1 BEGIN UPDATE t1 SET b=1; \
               DELETE FROM t1 WHERE a=2; SELECT 1; END",
            b"ALTER TABLE t1 RENAME TO t2",
        ],
    );
    assert_eq!(
        temped(&writer),
        alloc::string::String::from(concat!(
            "table|x|x|CREATE TABLE x(y)|\n",
            "view|v1|v1|CREATE VIEW v1 AS SELECT a FROM \"t2\"|\n",
            "view|v2|v2|CREATE VIEW v2 AS SELECT a FROM other|\n",
            "trigger|tr1|t2|CREATE TRIGGER tr1 AFTER INSERT ON \"t2\" ",
            "BEGIN INSERT INTO \"t2\"(a) VALUES(new.a); END|\n",
            "trigger|tr2|t2|CREATE TRIGGER tr2 AFTER UPDATE ON main.\"t2\" ",
            "BEGIN UPDATE \"t2\" SET b=1; DELETE FROM \"t2\" WHERE a=2; ",
            "SELECT 1; END|\n",
        ))
    );
    ran(&mut writer, &[b"ALTER TABLE t2 RENAME COLUMN a TO c"]);
    assert_eq!(
        temped(&writer),
        alloc::string::String::from(concat!(
            "table|x|x|CREATE TABLE x(y)|\n",
            "view|v1|v1|CREATE VIEW v1 AS SELECT c FROM \"t2\"|\n",
            "view|v2|v2|CREATE VIEW v2 AS SELECT a FROM other|\n",
            "trigger|tr1|t2|CREATE TRIGGER tr1 AFTER INSERT ON \"t2\" ",
            "BEGIN INSERT INTO \"t2\"(c) VALUES(new.c); END|\n",
            "trigger|tr2|t2|CREATE TRIGGER tr2 AFTER UPDATE ON main.\"t2\" ",
            "BEGIN UPDATE \"t2\" SET b=1; DELETE FROM \"t2\" WHERE c=2; ",
            "SELECT 1; END|\n",
        ))
    );
}

#[test]
fn a_temp_table_of_the_name_leaves_the_temp_schema_as_it_stands() {
    let mut writer = writer();
    ran(
        &mut writer,
        &[
            b"CREATE TABLE t1(a,b)",
            b"CREATE TEMP TABLE t1(a,b)",
            b"CREATE TEMP TRIGGER tr1 AFTER INSERT ON t1 BEGIN SELECT new.a; END",
            b"CREATE TEMP VIEW v1 AS SELECT a FROM t1",
            b"ALTER TABLE main.t1 RENAME COLUMN a TO c",
            b"ALTER TABLE main.t1 RENAME TO t2",
        ],
    );
    assert_eq!(
        temped(&writer),
        alloc::string::String::from(concat!(
            "table|t1|t1|CREATE TABLE t1(a,b)|\n",
            "trigger|tr1|t1|CREATE TRIGGER tr1 AFTER INSERT ON t1 BEGIN SELECT new.a; END|\n",
            "view|v1|v1|CREATE VIEW v1 AS SELECT a FROM t1|\n",
        ))
    );
}

#[test]
fn a_rename_of_a_temp_table_writes_the_temp_schema_once() {
    let mut writer = writer();
    ran(
        &mut writer,
        &[
            b"CREATE TEMP TABLE t1(a,b)",
            b"CREATE TEMP VIEW v1 AS SELECT a FROM t1",
            b"ALTER TABLE t1 RENAME TO t2",
            b"CREATE TEMP TABLE u(a,b)",
            b"ALTER TABLE u RENAME COLUMN a TO c",
        ],
    );
    assert_eq!(
        temped(&writer),
        alloc::string::String::from(concat!(
            "table|t2|t2|CREATE TABLE \"t2\"(a,b)|\n",
            "view|v1|v1|CREATE VIEW v1 AS SELECT a FROM \"t2\"|\n",
            "table|u|u|CREATE TABLE u(c,b)|\n",
        ))
    );
}

#[test]
fn every_table_a_statement_names() {
    let shown = |sql: &[u8]| {
        let mut out = alloc::string::String::new();
        for (schema, name) in crate::rename::named_tables(sql) {
            if let Some(schema) = schema {
                out.push_str(&alloc::string::String::from_utf8_lossy(schema.text(sql)));
                out.push('.');
            }
            out.push_str(&alloc::string::String::from_utf8_lossy(name.text(sql)));
            out.push(' ');
        }
        out
    };
    // A statement the parser refuses names nothing.
    assert_eq!(shown(b"CREATE TABLE"), "");
    assert_eq!(
        shown(b"CREATE VIEW v AS SELECT a FROM aux.t, u"),
        "aux.t u "
    );
    // A statement in brackets is no table, so it names none.
    assert_eq!(shown(b"CREATE VIEW v AS SELECT a FROM (SELECT 1 AS a)"), "");
    let stepped = |sql: &[u8]| {
        let mut out = alloc::string::String::new();
        for name in crate::rename::named_steps(sql) {
            out.push_str(&alloc::string::String::from_utf8_lossy(name.text(sql)));
            out.push(' ');
        }
        out
    };
    assert_eq!(stepped(b"CREATE TABLE"), "");
    assert_eq!(
        stepped(
            b"CREATE TRIGGER tr AFTER INSERT ON t BEGIN INSERT INTO a VALUES(1); \
              UPDATE b SET c=1; DELETE FROM d; SELECT 1 FROM e; END"
        ),
        "a b d "
    );
    assert_eq!(
        shown(
            b"CREATE TRIGGER tr AFTER INSERT ON main.t BEGIN INSERT INTO one.a VALUES(1); \
              UPDATE two.b SET c=1; DELETE FROM three.d; SELECT 1 FROM four.e; END"
        ),
        "main.t four.e one.a two.b three.d "
    );
}

#[test]
fn a_trigger_that_names_a_table_no_database_holds_refuses_a_rename() {
    let mut writer = writer();
    ran(
        &mut writer,
        &[
            b"CREATE TABLE t1(a,b)",
            b"CREATE TABLE t3(e,f)",
            b"CREATE TRIGGER tr1 AFTER INSERT ON t1 BEGIN INSERT INTO t2 VALUES(new.a, new.b); END",
        ],
    );
    // The rename is over `t3`, which the trigger names nothing of, and
    // the trigger of `t1` refuses it all the same.
    for sql in [
        b"ALTER TABLE t3 RENAME TO t4".as_slice(),
        b"ALTER TABLE t3 RENAME e TO eee",
    ] {
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "error in trigger tr1: no such table: main.t2",
            "{sql:?}"
        );
    }
    // The refusal leaves the statement that made the table as it stands.
    assert!(schema(&writer).contains("CREATE TABLE t3(e,f)"));
    // A trigger every step of which names a table takes the rename.
    ran(
        &mut writer,
        &[b"CREATE TABLE t2(c)", b"ALTER TABLE t3 RENAME TO t4"],
    );
    assert!(schema(&writer).contains("CREATE TABLE \"t4\"(e,f)"));
}

#[test]
fn a_trigger_of_the_temp_schema_names_the_table_of_every_database() {
    let mut writer = writer();
    ran(
        &mut writer,
        &[
            b"CREATE TABLE t1(a)",
            b"CREATE TEMP TABLE u7(x,y)",
            b"CREATE TEMP TABLE u9(z)",
            b"CREATE TEMP TRIGGER u9t AFTER INSERT ON u9 BEGIN INSERT INTO t1 VALUES(new.z); END",
            b"ALTER TABLE u7 RENAME x TO xxx",
        ],
    );
    // The step names `t1`, which `main` holds, so the rename of the temp
    // table is taken and the temp schema carries the new name.
    assert!(temped(&writer).contains("CREATE TABLE u7(xxx,y)"));
    // A step that names a table no database holds refuses the rename,
    // and the refusal names the table under no schema.
    ran(
        &mut writer,
        &[b"CREATE TEMP TRIGGER u8t AFTER INSERT ON u9 BEGIN INSERT INTO u8 VALUES(new.z); END"],
    );
    assert_eq!(
        writer
            .run(b"ALTER TABLE u7 RENAME y TO yyy")
            .unwrap_err()
            .message(),
        "error in trigger u8t: no such table: u8"
    );
}

#[test]
fn what_a_rename_writes_under_legacy_alter_table() {
    let mut writer = writer();
    ran(
        &mut writer,
        &[
            b"PRAGMA legacy_alter_table=1",
            b"CREATE TABLE p(a PRIMARY KEY)",
            b"CREATE TABLE t(a, b REFERENCES p(a))",
            b"CREATE INDEX i ON t(a) WHERE a>0",
            b"CREATE VIEW v AS SELECT a FROM t",
            b"CREATE TRIGGER tr AFTER INSERT ON t BEGIN INSERT INTO t VALUES(1,2); END",
            b"ALTER TABLE t RENAME TO u",
            b"ALTER TABLE p RENAME TO q",
        ],
    );
    // The table's own name, the table an index is over and the table a
    // trigger stands on are written again; every reference is left as it
    // stands, and a `REFERENCES` of the renamed parent too, because the
    // keys are not held.
    assert_eq!(
        schema(&writer),
        concat!(
            "table|q|q|CREATE TABLE \"q\"(a PRIMARY KEY)|\n",
            "index|sqlite_autoindex_q_1|q||\n",
            "table|u|u|CREATE TABLE \"u\"(a, b REFERENCES p(a))|\n",
            "index|i|u|CREATE INDEX i ON \"u\"(a) WHERE a>0|\n",
            "view|v|v|CREATE VIEW v AS SELECT a FROM t|\n",
            "trigger|tr|u|CREATE TRIGGER tr AFTER INSERT ON \"u\" ",
            "BEGIN INSERT INTO t VALUES(1,2); END|\n",
        )
    );
    assert_eq!(
        writer.run(b"PRAGMA legacy_alter_table").unwrap(),
        alloc::vec![alloc::vec![crate::value::Value::Int(1)]]
    );
}

#[test]
fn a_legacy_rename_writes_a_reference_of_the_parent_where_the_keys_are_held() {
    let mut writer = writer();
    ran(
        &mut writer,
        &[
            b"PRAGMA legacy_alter_table=1",
            b"PRAGMA foreign_keys=1",
            b"CREATE TABLE p(a PRIMARY KEY)",
            b"CREATE TABLE c(b REFERENCES p(a))",
            b"ALTER TABLE p RENAME TO q",
        ],
    );
    assert!(schema(&writer).contains("CREATE TABLE c(b REFERENCES \"q\"(a))"));
}

#[test]
fn a_legacy_rename_that_leaves_a_statement_unreadable_is_refused() {
    let mut writer = writer();
    ran(
        &mut writer,
        &[
            b"PRAGMA legacy_alter_table=1",
            b"CREATE TABLE t(a, b, CHECK(t.a>0))",
        ],
    );
    // The `CHECK` names the table, which the rename leaves as it stands,
    // so the statement no longer reads and the rename is refused. A
    // trigger that names a table no database holds is resolved by no
    // legacy rename, so the schema below it stands.
    assert_eq!(
        writer
            .run(b"ALTER TABLE t RENAME TO u")
            .unwrap_err()
            .message(),
        "error in table u after rename: no such column: t.a"
    );
    assert_eq!(
        schema(&writer),
        "table|t|t|CREATE TABLE t(a, b, CHECK(t.a>0))|\n"
    );
    ran(
        &mut writer,
        &[
            b"CREATE TABLE w(c)",
            b"CREATE TRIGGER tr AFTER INSERT ON w BEGIN INSERT INTO gone VALUES(1); END",
            b"ALTER TABLE w RENAME TO x",
        ],
    );
    assert!(schema(&writer).contains("CREATE TABLE \"x\"(c)"));
}
