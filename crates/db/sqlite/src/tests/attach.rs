// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ATTACH` and `DETACH`, which say what databases one connection holds.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::header::Encoding;
use crate::value::Value;

/// A connection over one table, with the opening function of these tests
/// told to it.
fn opened() -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.opens(opening);
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer
}

/// The opening function of these tests, which answers by file name:
/// `one.db` a database of one table, `empty.db` no bytes at all,
/// `broken.db` bytes no header reads, `utf16.db` a database whose text is
/// UTF-16, and every other name nothing.
fn opening(file: &[u8]) -> Option<Vec<u8>> {
    if file == b"one.db" {
        let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
        writer.run(b"CREATE TABLE u(b)").unwrap();
        return Some(writer.written());
    }
    if file == b"empty.db" {
        return Some(Vec::new());
    }
    if file == b"broken.db" {
        return Some(alloc::vec![0; 512]);
    }
    if file == b"utf16.db" {
        let mut writer = Writer::new(1024, 0, Encoding::Utf16Le).unwrap();
        writer.run(b"CREATE TABLE u(b)").unwrap();
        return Some(writer.written());
    }
    None
}

/// The names and file names one statement of the connection answers.
fn shown(writer: &mut Writer) -> alloc::string::String {
    let rows = writer.run(b"PRAGMA database_list").unwrap();
    let mut out = alloc::string::String::new();
    for row in rows {
        for value in row {
            out.push_str(&alloc::string::String::from_utf8_lossy(
                &value.text().unwrap_or_default(),
            ));
            out.push('|');
        }
    }
    out
}

/// The message one statement of the connection is refused with.
fn refused(writer: &mut Writer, sql: &[u8]) -> alloc::string::String {
    writer.run(sql).expect_err("a refusal").message()
}

/// `ATTACH` adds a database the connection holds under the name the
/// statement gave, and `PRAGMA database_list` answers every one of them.
#[test]
fn what_databases_one_connection_holds() {
    let mut writer = opened();
    assert_eq!(shown(&mut writer), "0|main||");
    // A bare name is the text of that name, which
    // `resolveAttachExpr` reads it as, and so is a name in quotes and a
    // name an expression answers.
    writer.run(b"ATTACH ':memory:' AS aux").unwrap();
    writer.run(b"ATTACH DATABASE ':memory:' AS 'two'").unwrap();
    writer.run(b"ATTACH ':'||'memory:' AS three").unwrap();
    assert_eq!(shown(&mut writer), "0|main||2|aux||3|two||4|three||");
    assert_eq!(
        writer.attached_names(),
        [b"aux".to_vec(), b"two".to_vec(), b"three".to_vec()]
    );
    // A file name the opening function answers an image for carries that
    // image, and its name is what `PRAGMA database_list` writes.
    writer.run(b"ATTACH 'one.db' AS four KEY ''").unwrap();
    assert_eq!(
        shown(&mut writer),
        "0|main||2|aux||3|two||4|three||5|four|one.db|"
    );
    let image = writer.attached_written(b"four").expect("an image");
    let database = crate::db::Database::open(&image).unwrap();
    assert!(database.table(b"u").is_some());
    assert!(writer.attached_written(b"five").is_none());
    // A file name of no bytes and a file the function answers no bytes
    // for are both a database of one page.
    writer.run(b"ATTACH NULL AS five").unwrap();
    writer.run(b"ATTACH 'empty.db' AS six").unwrap();
    assert_eq!(writer.attached_names().len(), 6);
    // A `DETACH` takes one away, and the places of the ones after it
    // count on from where it stood.
    writer.run(b"DETACH aux").unwrap();
    assert_eq!(
        shown(&mut writer),
        "0|main||2|two||3|three||4|four|one.db|5|five||6|six|empty.db|"
    );
}

/// What an `ATTACH` and a `DETACH` are refused for.
#[test]
fn what_an_attach_and_a_detach_are_refused_for() {
    let mut writer = opened();
    writer.run(b"ATTACH ':memory:' AS aux").unwrap();
    // A name the connection already holds a database under, which the
    // two names every connection holds a place for are.
    for name in [b"aux".as_slice(), b"main", b"temp"] {
        let mut sql = b"ATTACH ':memory:' AS ".to_vec();
        sql.extend_from_slice(name);
        let want = alloc::format!(
            "database {} is already in use",
            alloc::string::String::from_utf8_lossy(name)
        );
        assert_eq!(refused(&mut writer, &sql), want);
    }
    // A file name the opening function answers nothing for, and one
    // whose bytes no header reads.
    assert_eq!(
        refused(&mut writer, b"ATTACH 'nosuch.db' AS q"),
        "unable to open database: nosuch.db"
    );
    assert_eq!(
        refused(&mut writer, b"ATTACH 'broken.db' AS q"),
        "file is not a database"
    );
    // A file whose encoding is not the one of `main`.
    assert_eq!(
        refused(&mut writer, b"ATTACH 'utf16.db' AS q"),
        "attached databases must use the same text encoding as main database"
    );
    // The eleventh database of a connection.
    for at in 1..crate::db::ATTACHED {
        let mut sql = b"ATTACH ':memory:' AS held".to_vec();
        sql.extend_from_slice(alloc::format!("{at}").as_bytes());
        writer.run(&sql).unwrap();
    }
    assert_eq!(
        refused(&mut writer, b"ATTACH ':memory:' AS last"),
        "too many attached databases - max 10"
    );
    // `main` is never taken away, and a name the connection holds no
    // database under is no database at all, which `temp` is until the
    // connection makes one.
    assert_eq!(
        refused(&mut writer, b"DETACH main"),
        "cannot detach database main"
    );
    assert_eq!(
        refused(&mut writer, b"DETACH temp"),
        "no such database: temp"
    );
    assert_eq!(
        refused(&mut writer, b"DETACH nope"),
        "no such database: nope"
    );
}

/// A connection told no opening function refuses every `ATTACH` of a
/// file name, and one of a database of its own runs.
#[test]
fn a_connection_told_no_opening_function_reads_no_file() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    assert_eq!(
        refused(&mut writer, b"ATTACH 'one.db' AS aux"),
        "unable to open database: one.db"
    );
    writer.run(b"ATTACH ':memory:' AS aux").unwrap();
    assert_eq!(writer.attached_names(), [b"aux".to_vec()]);
    assert_eq!(
        writer.run(b"PRAGMA database_list").unwrap().first(),
        Some(&alloc::vec![
            Value::Int(0),
            Value::Text(b"main".to_vec()),
            Value::Text(Vec::new())
        ])
    );
}

/// What the parser reads an `ATTACH` and a `DETACH` as.
#[test]
fn what_an_attach_and_a_detach_are_read_as() {
    use crate::ast::Definition;
    for sql in [
        b"ATTACH 'f' AS a".as_slice(),
        b"ATTACH DATABASE 'f' AS a",
        b"ATTACH DATABASE 'f' AS a KEY 'k'",
    ] {
        let (_, definition) = crate::parse::definition(sql).expect("an attach");
        assert!(matches!(definition, Definition::Attach(_)), "{sql:?}");
    }
    for sql in [b"DETACH a".as_slice(), b"DETACH DATABASE a"] {
        let (_, definition) = crate::parse::definition(sql).expect("a detach");
        assert!(matches!(definition, Definition::Detach(_)), "{sql:?}");
    }
    // A statement that names no database and one that names no file are
    // both refused.
    for sql in [
        b"ATTACH 'f'".as_slice(),
        b"ATTACH 'f' AS",
        b"ATTACH AS a",
        b"DETACH",
    ] {
        assert!(crate::parse::definition(sql).is_err(), "{sql:?}");
    }
}

/// The rows a statement answers out of a database an `ATTACH` added.
#[test]
fn what_a_statement_reads_out_of_an_attached_database() {
    let mut main = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    main.run(b"CREATE TABLE t(a)").unwrap();
    main.run(b"INSERT INTO t VALUES(1),(2)").unwrap();
    let mut aux = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    aux.run(b"CREATE TABLE t(a)").unwrap();
    aux.run(b"INSERT INTO t VALUES(3)").unwrap();
    aux.run(b"CREATE TABLE u(b PRIMARY KEY) WITHOUT ROWID")
        .unwrap();
    aux.run(b"INSERT INTO u VALUES(4)").unwrap();
    aux.run(b"CREATE INDEX ua ON u(b)").unwrap();
    aux.run(b"CREATE VIEW v AS SELECT b FROM u").unwrap();
    let held = main.written();
    let beside = aux.written();
    let database = crate::db::Database::open(&held)
        .unwrap()
        .attaching(b"aux", &beside)
        .unwrap();
    let answered = |sql: &[u8]| {
        database
            .query(sql)
            .map(|answered| {
                let mut out = alloc::string::String::new();
                for row in answered.rows {
                    for value in row {
                        out.push_str(&alloc::string::String::from_utf8_lossy(
                            &value.text().unwrap_or_default(),
                        ));
                        out.push(',');
                    }
                }
                out
            })
            .map_err(|error| error.message())
    };
    // A bare name is answered out of `main` first, which
    // `sqlite3FindTable` reads before the attached databases.
    assert_eq!(answered(b"SELECT a FROM t ORDER BY a").unwrap(), "1,2,");
    assert_eq!(
        answered(b"SELECT a FROM main.t ORDER BY a").unwrap(),
        "1,2,"
    );
    assert_eq!(answered(b"SELECT a FROM aux.t").unwrap(), "3,");
    // A table only the attached database holds is answered by its bare
    // name as well, and so are its index, its view and its own schema.
    assert_eq!(answered(b"SELECT b FROM u").unwrap(), "4,");
    assert_eq!(answered(b"SELECT b FROM aux.u WHERE b=4").unwrap(), "4,");
    assert_eq!(answered(b"SELECT b FROM aux.v").unwrap(), "4,");
    assert_eq!(answered(b"SELECT b FROM v").unwrap(), "4,");
    assert_eq!(
        answered(b"SELECT count(*) FROM aux.sqlite_master").unwrap(),
        "4,"
    );
    // A schema in front of a column names the database the side reads.
    assert_eq!(answered(b"SELECT aux.t.a FROM aux.t").unwrap(), "3,");
    assert_eq!(
        answered(b"SELECT main.t.a FROM t ORDER BY a").unwrap(),
        "1,2,"
    );
    assert_eq!(
        answered(b"SELECT a FROM main.t JOIN aux.u ON b>a ORDER BY a").unwrap(),
        "1,2,"
    );
    assert_eq!(answered(b"SELECT 1 WHERE 3 IN aux.t").unwrap(), "1,");
    // A schema the connection holds no database under names no table,
    // and so does a name that database does not hold.
    assert_eq!(
        answered(b"SELECT a FROM two.t").unwrap_err(),
        "no such table: two.t"
    );
    assert_eq!(
        answered(b"SELECT a FROM aux.nope").unwrap_err(),
        "no such table: aux.nope"
    );
    assert_eq!(
        answered(b"SELECT a FROM main.u").unwrap_err(),
        "no such table: main.u"
    );
    // A name on the right of an `IN` that no database holds is refused
    // where the statement is read, and `crate::eval::Row::answered`
    // carries a value or nothing rather than the refusal, so the message
    // is the one that stands for nothing.
    assert_eq!(
        answered(b"SELECT 1 WHERE 3 IN two.t").unwrap_err(),
        "Unsupported"
    );
    assert_eq!(
        answered(b"SELECT 1 WHERE 3 IN nope").unwrap_err(),
        "Unsupported"
    );
    // A column named under a schema the side does not read is no column
    // of it.
    assert_eq!(
        answered(b"SELECT two.t.a FROM aux.t").unwrap_err(),
        "no such column: two.t.a"
    );
}

/// What a statement writes into a database an `ATTACH` added.
#[test]
fn what_a_statement_writes_into_an_attached_database() {
    let mut writer = opened();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    writer.run(b"ATTACH 'one.db' AS aux").unwrap();
    // A statement that names the database writes that one, and the one
    // the connection was opened over is left as it was.
    writer.run(b"CREATE TABLE aux.made(b)").unwrap();
    writer.run(b"INSERT INTO aux.made VALUES(2)").unwrap();
    writer.run(b"INSERT INTO aux.made SELECT a FROM t").unwrap();
    writer.run(b"UPDATE aux.made SET b=b+10 WHERE b=1").unwrap();
    writer.run(b"DELETE FROM aux.made WHERE b=2").unwrap();
    let image = writer.attached_written(b"aux").expect("an image");
    let database = crate::db::Database::open(&image).unwrap();
    assert_eq!(
        database.rows_of(b"made").unwrap(),
        [(2, alloc::vec![Value::Int(11)])]
    );
    assert!(
        crate::db::Database::open(&writer.written())
            .unwrap()
            .table(b"made")
            .is_none()
    );
    // A bare name only the attached database holds names that database,
    // which `sqlite3LocateTable` reads the databases in turn for.
    writer.run(b"INSERT INTO u VALUES(3)").unwrap();
    writer.run(b"DROP TABLE made").unwrap();
    let image = writer.attached_written(b"aux").expect("an image");
    let database = crate::db::Database::open(&image).unwrap();
    assert!(database.table(b"made").is_none());
    assert_eq!(database.rows_of(b"u").unwrap().len(), 1);
    // A `CREATE` of a bare name makes the table in the database the
    // connection writes, whatever the attached one holds.
    writer.run(b"CREATE TABLE u(c)").unwrap();
    assert!(
        crate::db::Database::open(&writer.written())
            .unwrap()
            .table(b"u")
            .is_some()
    );
    // The file name of every attached database that names one, which the
    // client writes the bytes back to.
    let files = writer.attached_files();
    assert_eq!(files.len(), 1);
    assert_eq!(
        files.first().map(|(file, _)| file.clone()),
        Some(b"one.db".to_vec())
    );
    writer.run(b"ATTACH ':memory:' AS held").unwrap();
    assert_eq!(writer.attached_files().len(), 1);
    // A schema the connection holds no database under is refused: the
    // statements that name a table by `no such table`, and the ones that
    // name the database alone by `unknown database`.
    assert_eq!(
        refused(&mut writer, b"INSERT INTO two.t VALUES(1)"),
        "no such table: two.t"
    );
    assert_eq!(
        refused(&mut writer, b"UPDATE two.t SET a=1"),
        "no such table: two.t"
    );
    assert_eq!(
        refused(&mut writer, b"DELETE FROM two.t"),
        "no such table: two.t"
    );
    assert_eq!(
        refused(&mut writer, b"DROP TABLE two.t"),
        "no such table: two.t"
    );
    assert_eq!(
        refused(&mut writer, b"ALTER TABLE two.t RENAME TO x"),
        "no such table: two.t"
    );
    assert_eq!(
        refused(&mut writer, b"CREATE TABLE two.x(a)"),
        "unknown database two"
    );
    assert_eq!(
        refused(&mut writer, b"CREATE INDEX two.i ON t(a)"),
        "unknown database two"
    );
    assert_eq!(
        refused(&mut writer, b"CREATE VIEW two.v AS SELECT 1"),
        "unknown database two"
    );
    assert_eq!(
        refused(
            &mut writer,
            b"CREATE TRIGGER two.g AFTER INSERT ON t BEGIN SELECT 1; END"
        ),
        "unknown database two"
    );
    for sql in [
        b"ALTER TABLE two.t ADD COLUMN x".as_slice(),
        b"ALTER TABLE two.t DROP COLUMN a",
        b"ALTER TABLE two.t RENAME COLUMN a TO b",
        b"ALTER TABLE two.t DROP CONSTRAINT c",
    ] {
        assert_eq!(refused(&mut writer, sql), "no such table: two.t", "{sql:?}");
    }
    // `main` and `temp` both name the database the connection writes,
    // whatever it attached.
    writer.run(b"INSERT INTO main.t VALUES(4)").unwrap();
    writer.run(b"INSERT INTO temp.t VALUES(5)").unwrap();
    let held = writer.written();
    let database = crate::db::Database::open(&held).unwrap();
    assert_eq!(database.rows_of(b"t").unwrap().len(), 3);
}

/// A view and an index only an attached database holds are both named by
/// their bare names, which `sqlite3LocateTable` and `sqlite3FindIndex`
/// read the databases in turn for.
#[test]
fn what_a_bare_name_of_a_view_and_of_an_index_names() {
    let mut writer = opened();
    writer.run(b"ATTACH ':memory:' AS aux").unwrap();
    writer.run(b"CREATE TABLE aux.u(b)").unwrap();
    writer.run(b"CREATE INDEX aux.ub ON u(b)").unwrap();
    writer.run(b"CREATE VIEW aux.v AS SELECT b FROM u").unwrap();
    writer.run(b"DROP VIEW v").unwrap();
    writer.run(b"DROP INDEX ub").unwrap();
    let image = writer.attached_written(b"aux").expect("an image");
    let database = crate::db::Database::open(&image).unwrap();
    assert!(database.view(b"v").is_none());
    assert!(database.index(b"ub").is_none());
    assert!(database.table(b"u").is_some());
}

/// A name that holds the letters `temp` and is no word of its own opens
/// no temp schema, which is what `sqlite3OpenTempDatabase` is reached for.
#[test]
fn what_names_holding_the_letters_of_temp_open() {
    let mut writer = opened();
    for sql in [
        b"CREATE TABLE temperature(x)".as_slice(),
        b"CREATE TABLE xtemp(x)",
        b"CREATE TABLE \"temp$x\"(x)",
        "CREATE TABLE \"tempé\"(x)".as_bytes(),
    ] {
        writer.run(sql).unwrap();
        assert_eq!(shown(&mut writer), "0|main||", "{sql:?}");
    }
    // The word itself opens one, and its own table is read under the
    // schema as well as by its bare name.
    writer.run(b"CREATE TEMP TABLE tt(a)").unwrap();
    assert_eq!(shown(&mut writer), "0|main||1|temp||");
    let temp = writer.attached_written(b"temp").expect("a temp schema");
    let held = writer.written();
    let database = crate::db::Database::open(&held)
        .unwrap()
        .attaching(b"temp", &temp)
        .unwrap();
    for name in [
        b"temp.sqlite_temp_master".as_slice(),
        b"temp.sqlite_temp_schema",
        b"temp.sqlite_master",
        b"temp.sqlite_schema",
    ] {
        let mut sql = b"SELECT count(*) FROM ".to_vec();
        sql.extend_from_slice(name);
        assert_eq!(
            database.query(&sql).unwrap().rows,
            [[Value::Int(1)]],
            "{name:?}"
        );
    }
}

/// The temp schema, which a statement written `TEMP` writes and which
/// stands in front of `main` for a bare name.
#[test]
fn what_the_temp_schema_holds() {
    let mut writer = opened();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    writer.run(b"CREATE TEMP TABLE tt(b)").unwrap();
    writer.run(b"INSERT INTO tt VALUES(2)").unwrap();
    // The temp schema stands at schema place one, which no file of the
    // client holds.
    assert_eq!(shown(&mut writer), "0|main||1|temp||");
    assert_eq!(writer.attached_names(), [b"temp".to_vec()]);
    assert!(writer.attached_files().is_empty());
    let temp = writer.attached_written(b"temp").expect("a temp schema");
    let held = writer.written();
    let database = crate::db::Database::open(&held)
        .unwrap()
        .attaching(b"temp", &temp)
        .unwrap();
    // A bare name both hold is answered out of the temp schema, and the
    // two names of its own table read it.
    writer.run(b"CREATE TEMP TABLE t(c)").unwrap();
    writer.run(b"INSERT INTO t VALUES(3)").unwrap();
    let answered = |sql: &[u8], database: &crate::db::Database<'_>| {
        database
            .query(sql)
            .map(|answered| answered.rows)
            .map_err(|error| error.message())
    };
    for name in [b"sqlite_temp_master".as_slice(), b"sqlite_temp_schema"] {
        let mut sql = b"SELECT count(*) FROM ".to_vec();
        sql.extend_from_slice(name);
        assert_eq!(answered(&sql, &database).unwrap(), [[Value::Int(1)]]);
    }
    assert_eq!(
        answered(b"SELECT count(*) FROM sqlite_master", &database).unwrap(),
        [[Value::Int(1)]]
    );
    assert_eq!(
        answered(b"SELECT b FROM temp.tt", &database).unwrap(),
        [[Value::Int(2)]]
    );
    // A `DROP` of a bare name both hold takes the temp one away, and the
    // one of `main` stands.
    writer.run(b"DROP TABLE t").unwrap();
    let held = writer.written();
    let database = crate::db::Database::open(&held).unwrap();
    assert_eq!(database.rows_of(b"t").unwrap().len(), 1);
    // `main` and `temp` are both refused a `DETACH`, and the temp schema
    // holds a place no `ATTACH` counts against the ten.
    assert_eq!(
        refused(&mut writer, b"DETACH temp"),
        "cannot detach database temp"
    );
    for at in 0..crate::db::ATTACHED {
        let mut sql = b"ATTACH ':memory:' AS held".to_vec();
        sql.extend_from_slice(alloc::format!("{at}").as_bytes());
        writer.run(&sql).unwrap();
    }
    assert_eq!(
        refused(&mut writer, b"ATTACH ':memory:' AS last"),
        "too many attached databases - max 10"
    );
    // A `DETACH` leaves the databases after the one it took away one
    // place lower, which `sqlite3DetachDatabase` moves them down by.
    writer.run(b"DETACH held0").unwrap();
    assert_eq!(
        shown(&mut writer),
        "0|main||1|temp||2|held1||3|held2||4|held3||5|held4||6|held5||7|held6||8|held7||9|held8||10|held9||"
    );
}
