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
        // The key makes an index of the table's own, which the reader
        // holds against the table of that database and not against the
        // table of the same name in another.
        writer.run(b"CREATE TABLE u(b TEXT PRIMARY KEY)").unwrap();
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
    // A name on the right of an `IN` names the table the same way, and
    // the refusal reaches the client through `crate::eval::Row`.
    assert_eq!(
        answered(b"SELECT 1 WHERE 3 IN two.t").unwrap_err(),
        "no such table: two.t"
    );
    assert_eq!(
        answered(b"SELECT 1 WHERE 3 IN nope").unwrap_err(),
        "no such table: nope"
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
    // `main` names the database the connection writes, whatever it
    // attached, and `temp` names the temp schema, which holds no table of
    // that name.
    writer.run(b"INSERT INTO main.t VALUES(4)").unwrap();
    assert_eq!(
        refused(&mut writer, b"INSERT INTO temp.t VALUES(5)"),
        "no such table: temp.t"
    );
    let held = writer.written();
    let database = crate::db::Database::open(&held).unwrap();
    assert_eq!(database.rows_of(b"t").unwrap().len(), 2);
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
    // A trigger of the attached database is named by its bare name as
    // well, and a name `main` holds names `main`.
    writer
        .run(b"CREATE TRIGGER aux.ug AFTER INSERT ON u BEGIN SELECT 1; END")
        .unwrap();
    writer.run(b"CREATE VIEW w AS SELECT 1").unwrap();
    writer
        .run(b"CREATE TRIGGER tg AFTER INSERT ON t BEGIN SELECT 1; END")
        .unwrap();
    writer.run(b"DROP TRIGGER ug").unwrap();
    writer.run(b"DROP TRIGGER tg").unwrap();
    writer.run(b"DROP VIEW w").unwrap();
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

/// A statement that writes `temp.name` writes the database of the temp
/// schema, as one written `TEMP` does.
#[test]
fn what_a_statement_that_names_the_temp_schema_writes() {
    let mut writer = opened();
    writer.run(b"CREATE TEMP TABLE tt(b)").unwrap();
    writer.run(b"INSERT INTO temp.tt VALUES(1)").unwrap();
    writer.run(b"UPDATE temp.tt SET b = 2").unwrap();
    let temp = writer.attached_written(b"temp").expect("a temp schema");
    let held = writer.written();
    let database = crate::db::Database::open(&held)
        .unwrap()
        .attaching(b"temp", &temp)
        .unwrap();
    assert_eq!(
        database.query(b"SELECT b FROM temp.tt").unwrap().rows,
        [[Value::Int(2)]]
    );
    // The database of `main` gains no row of the temp schema's table.
    assert_eq!(
        crate::db::Database::open(&held).unwrap().rows_of(b"t"),
        Ok(Vec::new())
    );
    // A `DELETE` and a `DROP` under the name reach the same database.
    writer.run(b"DELETE FROM temp.tt").unwrap();
    writer.run(b"DROP TABLE temp.tt").unwrap();
    assert_eq!(
        refused(&mut writer, b"INSERT INTO temp.tt VALUES(1)"),
        "no such table: temp.tt"
    );
}

/// One transaction over more than one database, which every database the
/// connection holds joins.
#[test]
fn what_a_transaction_over_more_than_one_database_writes() {
    let mut writer = opened();
    writer.run(b"ATTACH ':memory:' AS aux").unwrap();
    writer.run(b"CREATE TABLE aux.u(b)").unwrap();
    let counted = |writer: &Writer, name: &[u8], table: &[u8]| {
        let image = writer.attached_written(name).expect("an image");
        let database = crate::db::Database::open(&image).unwrap();
        database.rows_of(table).unwrap().len()
    };
    let held = |writer: &Writer| {
        let image = writer.written();
        let database = crate::db::Database::open(&image).unwrap();
        database.rows_of(b"t").unwrap().len()
    };
    // A `ROLLBACK` puts every database back where the `BEGIN` found it.
    writer.run(b"BEGIN").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    writer.run(b"INSERT INTO aux.u VALUES(2)").unwrap();
    writer.run(b"ROLLBACK").unwrap();
    assert_eq!(held(&writer), 0);
    assert_eq!(counted(&writer, b"aux", b"u"), 0);
    // A `COMMIT` writes every database the transaction wrote.
    writer.run(b"BEGIN").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    writer.run(b"INSERT INTO aux.u VALUES(2)").unwrap();
    writer.run(b"COMMIT").unwrap();
    assert_eq!(held(&writer), 1);
    assert_eq!(counted(&writer, b"aux", b"u"), 1);
    // A `ROLLBACK TO` puts every database back where the `SAVEPOINT`
    // found it, and a database an `ATTACH` added inside the transaction
    // joins it.
    writer.run(b"SAVEPOINT one").unwrap();
    writer.run(b"ATTACH ':memory:' AS two").unwrap();
    writer.run(b"CREATE TABLE two.v(c)").unwrap();
    writer.run(b"INSERT INTO t VALUES(3)").unwrap();
    writer.run(b"INSERT INTO aux.u VALUES(4)").unwrap();
    writer.run(b"ROLLBACK TO one").unwrap();
    assert_eq!(held(&writer), 1);
    assert_eq!(counted(&writer, b"aux", b"u"), 1);
    writer.run(b"RELEASE one").unwrap();
    // A `COMMIT` with no transaction open and a second `BEGIN` are both
    // refused.
    assert_eq!(
        refused(&mut writer, b"COMMIT"),
        "cannot commit - no transaction is active"
    );
    assert_eq!(
        refused(&mut writer, b"ROLLBACK"),
        "cannot rollback - no transaction is active"
    );
    writer.run(b"BEGIN").unwrap();
    assert_eq!(
        refused(&mut writer, b"BEGIN"),
        "cannot start a transaction within a transaction"
    );
    writer.run(b"COMMIT").unwrap();
}

/// `e_resolve-1.*` of `test/e_resolve.test`: a statement that names
/// `main` writes the rows and the indexes of that database, whatever the
/// temp schema holds under the same name, and a trigger name stands once
/// per database.
#[test]
fn what_a_statement_that_names_main_writes_where_temp_holds_the_name() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TEMP TABLE n1(x, y)".as_slice(),
        b"CREATE INDEX temp.n4 ON n1(x, y)",
        b"CREATE TRIGGER temp.n3 AFTER INSERT ON n1 BEGIN SELECT 1; END",
        b"INSERT INTO n1 VALUES('temp', 'n1')",
        b"CREATE TABLE main.n1(x, y)",
        b"CREATE TRIGGER main.n3 BEFORE INSERT ON n1 BEGIN SELECT 1; END",
        b"INSERT INTO main.n1 VALUES('main', 'n1')",
    ] {
        writer.run(sql).unwrap();
    }
    // The rows of `main` are the ones the statement that named it wrote,
    // and the file holds every page it says it does.
    let held = writer.written();
    let database = crate::db::Database::open(&held).unwrap();
    assert_eq!(
        database.query(b"SELECT x, y FROM n1").unwrap().rows,
        [alloc::vec![
            Value::Text(b"main".to_vec()),
            Value::Text(b"n1".to_vec())
        ]]
    );
    assert_eq!(
        database.query(b"PRAGMA integrity_check").unwrap().rows,
        [alloc::vec![Value::Text(b"ok".to_vec())]]
    );
    // The temp schema holds the row the statement that named no schema
    // wrote, and the index it carries.
    let temp = writer.attached_written(b"temp").expect("the temp schema");
    let database = crate::db::Database::open(&temp).unwrap();
    assert_eq!(
        database
            .query(b"SELECT x FROM n1 WHERE y='n1'")
            .unwrap()
            .rows,
        [alloc::vec![Value::Text(b"temp".to_vec())]]
    );
    assert_eq!(
        database.query(b"PRAGMA integrity_check").unwrap().rows,
        [alloc::vec![Value::Text(b"ok".to_vec())]]
    );
    // A trigger of the temp schema and one of `main` may carry the same
    // name, and a second trigger of that name in one database is
    // refused.
    assert_eq!(
        writer
            .run(b"CREATE TRIGGER main.n3 AFTER INSERT ON n1 BEGIN SELECT 1; END")
            .unwrap_err()
            .message(),
        "trigger n3 already exists"
    );
}

/// `e_blobclose-1.*` of `test/e_blobclose.test`: `PRAGMA lock_status`
/// answers the lock every database of the connection is held under.
#[test]
fn what_lock_every_database_of_a_connection_is_held_under() {
    let mut writer = opened();
    writer.run(b"ATTACH 'one.db' AS held").unwrap();
    let text = |bytes: &[u8]| Value::Text(bytes.to_vec());
    // A database no transaction has written answers `unlocked`, and the
    // temp schema answers `closed` until a statement opens it.
    assert_eq!(
        writer.run(b"PRAGMA lock_status").unwrap(),
        [
            alloc::vec![text(b"main"), text(b"unlocked")],
            alloc::vec![text(b"temp"), text(b"closed")],
            alloc::vec![text(b"held"), text(b"unlocked")],
        ]
    );
    writer.run(b"CREATE TEMP TABLE t(a)").unwrap();
    assert_eq!(
        writer.run(b"PRAGMA lock_status").unwrap(),
        [
            alloc::vec![text(b"main"), text(b"unlocked")],
            alloc::vec![text(b"temp"), text(b"unlocked")],
            alloc::vec![text(b"held"), text(b"unlocked")],
        ]
    );
    // A transaction that has opened a page to write holds the file under
    // a reserved lock, and one that has written nothing holds no lock.
    writer.run(b"CREATE TABLE m(a)").unwrap();
    writer.run(b"BEGIN").unwrap();
    assert_eq!(
        writer.run(b"PRAGMA lock_status").unwrap().first(),
        Some(&alloc::vec![text(b"main"), text(b"unlocked")])
    );
    writer.run(b"INSERT INTO m VALUES(1)").unwrap();
    assert_eq!(
        writer.run(b"PRAGMA lock_status").unwrap().first(),
        Some(&alloc::vec![text(b"main"), text(b"reserved")])
    );
    writer.run(b"COMMIT").unwrap();
    assert_eq!(
        writer.run(b"PRAGMA lock_status").unwrap().first(),
        Some(&alloc::vec![text(b"main"), text(b"unlocked")])
    );
    // A database in write-ahead logging takes no reserved lock, because
    // the pages of a transaction go into the log.
    writer.run(b"PRAGMA journal_mode=WAL").unwrap();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"INSERT INTO m VALUES(2)").unwrap();
    assert_eq!(
        writer.run(b"PRAGMA lock_status").unwrap().first(),
        Some(&alloc::vec![text(b"main"), text(b"unlocked")])
    );
    writer.run(b"COMMIT").unwrap();
}

/// A trigger of the temp schema is fixed to no database, so a statement
/// of its body names its table the way a statement outside a trigger
/// does: the temp schema first and then the databases in turn.
#[test]
fn a_trigger_of_the_temp_schema_names_its_tables_as_a_statement_outside_one_does() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE rlog(x)".as_slice(),
        b"CREATE TEMP TABLE tbl(a, b)",
        b"CREATE TEMP TABLE clog(x)",
        b"INSERT INTO tbl VALUES(1, 2)",
        b"INSERT INTO clog VALUES(7)",
        b"CREATE TRIGGER bu BEFORE UPDATE ON tbl FOR EACH ROW BEGIN \
          INSERT INTO rlog VALUES(old.a); \
          UPDATE clog SET x = old.b; \
          DELETE FROM clog WHERE x = 99; \
          SELECT 1; END",
        b"UPDATE tbl SET a = 9",
    ] {
        writer.run(sql).unwrap();
    }
    // `rlog` stands in `main` alone, so the body of a trigger of the
    // temp schema reaches it; `clog` stands in the temp schema, which
    // is read before the databases.
    let held = writer.written();
    let temp = writer.attached_written(b"temp").unwrap();
    let database = crate::db::Database::open(&held)
        .unwrap()
        .attaching(b"temp", &temp)
        .unwrap();
    assert_eq!(
        database.query(b"SELECT x FROM main.rlog").unwrap().rows,
        [alloc::vec![Value::Int(1)]]
    );
    assert_eq!(
        database.query(b"SELECT x FROM temp.clog").unwrap().rows,
        [alloc::vec![Value::Int(2)]]
    );
    assert_eq!(
        database.query(b"SELECT a, b FROM temp.tbl").unwrap().rows,
        [alloc::vec![Value::Int(9), Value::Int(2)]]
    );
}

/// A trigger of a database other than the temp schema writes the
/// database it stands in, which a table of the temp schema under the
/// same name does not reach.
#[test]
fn a_trigger_of_main_writes_the_database_it_stands_in() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE tbl(a)".as_slice(),
        b"CREATE TABLE rlog(x)",
        b"CREATE TEMP TABLE rlog(x)",
        b"INSERT INTO tbl VALUES(1)",
        b"CREATE TRIGGER bu BEFORE UPDATE ON tbl FOR EACH ROW BEGIN \
          INSERT INTO rlog VALUES(old.a); END",
        b"UPDATE tbl SET a = 9",
    ] {
        writer.run(sql).unwrap();
    }
    let held = writer.written();
    let temp = writer.attached_written(b"temp").unwrap();
    let database = crate::db::Database::open(&held)
        .unwrap()
        .attaching(b"temp", &temp)
        .unwrap();
    assert_eq!(
        database.query(b"SELECT x FROM main.rlog").unwrap().rows,
        [alloc::vec![Value::Int(1)]]
    );
    assert!(
        database
            .query(b"SELECT x FROM temp.rlog")
            .unwrap()
            .rows
            .is_empty()
    );
}

/// The temp schema of a connection is a database of its own, which a
/// caller that shares one writer between connections takes off one
/// connection and hands to another.
#[test]
fn what_the_temp_schema_of_a_connection_holds() {
    let mut writer = opened();
    assert!(writer.temp().is_none());
    writer.run(b"CREATE TEMP TABLE tt(a)").unwrap();
    writer.run(b"INSERT INTO tt VALUES(1)").unwrap();
    let held = writer.temp().expect("a temp schema");
    // A connection given no temp schema reads none of its tables.
    writer.temps(None).unwrap();
    assert!(writer.temp().is_none());
    assert!(writer.run(b"INSERT INTO tt VALUES(2)").is_err());
    // The same schema handed back answers the rows it held.
    writer.temps(Some(&held)).unwrap();
    writer.run(b"INSERT INTO tt VALUES(2)").unwrap();
    let temp = writer.temp().expect("a temp schema");
    let image = writer.written();
    let database = crate::db::Database::open(&image)
        .unwrap()
        .attaching(b"temp", &temp)
        .unwrap();
    assert_eq!(
        database.query(b"SELECT a FROM tt").unwrap().rows,
        [[Value::Int(1)], [Value::Int(2)]]
    );
    // A schema of no bytes is no schema at all.
    writer.temps(Some(&[])).unwrap();
    assert!(writer.temp().is_none());
}

/// A `CREATE TRIGGER` writes a schema in front of the table after `ON`,
/// which names the database the trigger stands in and no other, and which
/// a trigger of the temp schema is held to no name of.
#[test]
fn what_a_schema_in_front_of_the_table_of_a_trigger_names() {
    let mut writer = opened();
    writer.run(b"ATTACH ':memory:' AS aux").unwrap();
    for sql in [
        b"CREATE TABLE aux.u(b)".as_slice(),
        b"CREATE TEMP TABLE tt(c)",
        // The schema the statement names in front of its own name is the
        // one the table stands in.
        b"CREATE TRIGGER tg AFTER INSERT ON main.t BEGIN SELECT 1; END",
        b"CREATE TRIGGER aux.ug AFTER INSERT ON aux.u BEGIN SELECT 1; END",
        b"CREATE TRIGGER tq AFTER INSERT ON temp.tt BEGIN SELECT 1; END",
        // A trigger of the temp schema stands over a table of any
        // database.
        b"CREATE TEMP TRIGGER ua AFTER INSERT ON aux.u BEGIN SELECT 1; END",
    ] {
        writer.run(sql).unwrap();
    }
    let temp = writer.attached_written(b"temp").expect("the temp schema");
    let database = crate::db::Database::open(&temp).unwrap();
    assert!(database.trigger(b"tq").is_some());
    assert!(database.trigger(b"ua").is_some());
    let held = writer.written();
    assert!(
        crate::db::Database::open(&held)
            .unwrap()
            .trigger(b"tg")
            .is_some()
    );
    for (sql, message) in [
        // A table of another database is no table of the trigger, and
        // the name is written with the quotes it was written under.
        (
            b"CREATE TRIGGER bad AFTER INSERT ON aux.u BEGIN SELECT 1; END".as_slice(),
            "trigger bad cannot reference objects in database aux",
        ),
        (
            b"CREATE TRIGGER \"q\" AFTER INSERT ON aux.u BEGIN SELECT 1; END",
            "trigger \"q\" cannot reference objects in database aux",
        ),
        (
            b"CREATE TRIGGER main.q AFTER INSERT ON temp.tt BEGIN SELECT 1; END",
            "trigger q cannot reference objects in database temp",
        ),
        // The schema the statement wrote is the one the refusal names,
        // and the schema in front of the table is read before the schema
        // in front of the name.
        (
            b"CREATE TRIGGER aux.q AFTER INSERT ON aux.nosuch BEGIN SELECT 1; END",
            "no such table: aux.nosuch",
        ),
        (
            b"CREATE TRIGGER aux.q AFTER INSERT ON nosuch BEGIN SELECT 1; END",
            "no such table: aux.nosuch",
        ),
        (
            b"CREATE TEMP TRIGGER q AFTER INSERT ON nosuch.t BEGIN SELECT 1; END",
            "no such table: nosuch.t",
        ),
        // A trigger and an index both stand over a table the temp schema
        // or the database the statement writes holds, so a table only an
        // attached database holds is no table of either.
        (
            b"CREATE TRIGGER q AFTER INSERT ON u BEGIN SELECT 1; END",
            "no such table: main.u",
        ),
        (b"CREATE INDEX ub ON u(b)", "no such table: main.u"),
    ] {
        let written = alloc::string::String::from_utf8_lossy(sql).into_owned();
        assert_eq!(refused(&mut writer, sql), message, "{written}");
    }
}

/// `exclusive-1.*` of `test/exclusive.test`: the mode each database of a
/// connection is held under, which `PRAGMA locking_mode` names one of
/// per database and keeps one default of.
#[test]
fn what_mode_each_database_of_a_connection_is_held_under() {
    let mut writer = opened();
    // One answer per statement of the text, which is how the tester
    // writes a run of pragmas.
    let shown = |writer: &mut Writer, sql: &str| -> alloc::string::String {
        let mut out = alloc::string::String::new();
        for statement in sql.split(';') {
            let text = statement.trim();
            for row in writer.run(text.as_bytes()).expect("the pragma") {
                for value in &row {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(&alloc::string::String::from_utf8_lossy(
                        &value.text().unwrap_or_default(),
                    ));
                }
            }
        }
        out
    };
    // The temp schema is a database of the connection's own, which is
    // held under an exclusive lock whatever the pragma names.
    let every = "PRAGMA locking_mode; PRAGMA main.locking_mode; PRAGMA temp.locking_mode";
    assert_eq!(shown(&mut writer, every), "normal normal exclusive");
    assert_eq!(
        shown(&mut writer, "PRAGMA locking_mode = exclusive"),
        "exclusive"
    );
    assert_eq!(shown(&mut writer, every), "exclusive exclusive exclusive");
    assert_eq!(shown(&mut writer, "PRAGMA locking_mode = normal"), "normal");
    assert_eq!(shown(&mut writer, every), "normal normal exclusive");
    // A word that names neither mode, under no schema, is a query of the
    // default and writes nothing.
    assert_eq!(
        shown(&mut writer, "PRAGMA locking_mode = invalid"),
        "normal"
    );
    assert_eq!(shown(&mut writer, every), "normal normal exclusive");
    // A database attached after the default was named takes it.
    assert_eq!(
        shown(&mut writer, "PRAGMA locking_mode = exclusive"),
        "exclusive"
    );
    writer.run(b"ATTACH 'one.db' AS aux").unwrap();
    assert_eq!(
        shown(
            &mut writer,
            "PRAGMA main.locking_mode; PRAGMA aux.locking_mode"
        ),
        "exclusive exclusive"
    );
    // A pragma that names a schema writes that database alone and leaves
    // the default where it stood.
    assert_eq!(
        shown(&mut writer, "PRAGMA main.locking_mode = normal"),
        "normal"
    );
    assert_eq!(
        shown(
            &mut writer,
            "PRAGMA main.locking_mode; PRAGMA temp.locking_mode; PRAGMA aux.locking_mode"
        ),
        "normal exclusive exclusive"
    );
    assert_eq!(shown(&mut writer, "PRAGMA locking_mode"), "exclusive");
    writer.run(b"ATTACH 'utf16.db' AS aux2").unwrap_err();
    writer.run(b"ATTACH 'one.db' AS aux2").unwrap();
    assert_eq!(
        shown(
            &mut writer,
            "PRAGMA main.locking_mode; PRAGMA aux.locking_mode; PRAGMA aux2.locking_mode"
        ),
        "normal exclusive exclusive"
    );
    assert_eq!(
        shown(&mut writer, "PRAGMA aux.locking_mode = normal"),
        "normal"
    );
    assert_eq!(
        shown(
            &mut writer,
            "PRAGMA main.locking_mode; PRAGMA aux.locking_mode; PRAGMA aux2.locking_mode"
        ),
        "normal normal exclusive"
    );
    // A pragma under no schema writes every database but the temp schema
    // and answers what `main` is held under.
    assert_eq!(shown(&mut writer, "PRAGMA locking_mode = normal"), "normal");
    assert_eq!(
        shown(
            &mut writer,
            "PRAGMA main.locking_mode; PRAGMA temp.locking_mode; \
             PRAGMA aux.locking_mode; PRAGMA aux2.locking_mode"
        ),
        "normal exclusive normal normal"
    );
    // A database of no file of its own is one the connection holds.
    writer.run(b"ATTACH ':memory:' AS held").unwrap();
    assert_eq!(shown(&mut writer, "PRAGMA held.locking_mode"), "exclusive");
    assert_eq!(
        shown(&mut writer, "PRAGMA held.locking_mode = normal"),
        "exclusive"
    );
}

#[test]
fn what_database_a_check_that_names_a_schema_reads() {
    let mut writer = opened();
    writer.run(b"CREATE TABLE m(x)").unwrap();
    writer.run(b"ATTACH ':memory:' AS aux").unwrap();
    writer.run(b"CREATE TABLE aux.a(y)").unwrap();
    // A check that names a schema reads that database alone, so a table
    // only another database holds is no table of it.
    assert_eq!(
        writer
            .run(b"PRAGMA aux.integrity_check=m")
            .unwrap_err()
            .message(),
        "no such table: aux.m"
    );
    assert_eq!(
        writer
            .run(b"PRAGMA aux.integrity_check=a")
            .unwrap()
            .first()
            .and_then(|row| row.first()),
        Some(&Value::Text(b"ok".to_vec()))
    );
    // A check that names none reads the databases in turn, so it finds
    // the table wherever it stands.
    assert_eq!(
        writer
            .run(b"PRAGMA integrity_check=a")
            .unwrap()
            .first()
            .and_then(|row| row.first()),
        Some(&Value::Text(b"ok".to_vec()))
    );
}

#[test]
fn what_a_database_an_attach_added_holds_beside_its_file() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.opens(opening);
    writer.logging((1, 2));
    writer.run(b"ATTACH 'one.db' AS a0").unwrap();
    writer.run(b"PRAGMA a0.journal_mode=WAL").unwrap();
    // A database in write-ahead logging mode holds its pages in the log,
    // so a statement that reads it afterwards reads what the log holds.
    writer.run(b"CREATE TABLE a0.t0(x)").unwrap();
    writer.run(b"INSERT INTO a0.t0 VALUES(1)").unwrap();
    assert_eq!(
        writer.run(b"PRAGMA a0.table_info(t0)").unwrap().len(),
        1,
        "the table stands in the attached database"
    );
    let held = writer.attached_logs();
    let (file, bytes) = held.first().cloned().unwrap_or_default();
    assert_eq!(file, b"one.db");
    assert!(bytes.len() > 32, "the log holds frames");
    // A truncating checkpoint leaves a file of no byte at all.
    writer.run(b"PRAGMA a0.wal_checkpoint(TRUNCATE)").unwrap();
    let held = writer.attached_logs();
    assert_eq!(
        held.first().map(|(_, bytes)| bytes.len()),
        Some(0),
        "the log holds no byte"
    );
}

#[test]
fn what_a_journal_a_database_an_attach_added_keeps() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.opens(opening);
    writer.journalling(crate::journal::Mode::Persist, 7, 512);
    writer.run(b"ATTACH 'one.db' AS a0").unwrap();
    writer.run(b"PRAGMA a0.journal_mode=PERSIST").unwrap();
    // A database under a journal mode that keeps the journal holds one
    // beside its file, and a database in no logging mode holds no log.
    writer.run(b"CREATE TABLE a0.t0(x)").unwrap();
    assert_eq!(
        writer
            .attached_journals()
            .first()
            .map(|(file, _)| file.clone()),
        Some(b"one.db".to_vec())
    );
    assert!(writer.attached_logs().is_empty());
}

#[test]
fn what_a_checkpoint_that_names_no_schema_writes_back() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.opens(opening);
    writer.logging((1, 2));
    writer.run(b"ATTACH 'one.db' AS a0").unwrap();
    writer.run(b"PRAGMA a0.journal_mode=WAL").unwrap();
    writer.run(b"CREATE TABLE t0(x)").unwrap();
    writer.run(b"CREATE TABLE a0.t1(x)").unwrap();
    // A checkpoint that names no schema writes the log of every database
    // the connection holds back into its file, and answers the values of
    // the database the connection writes.
    let answered = writer.run(b"PRAGMA wal_checkpoint").unwrap();
    let row = answered.first().cloned().unwrap_or_default();
    assert_eq!(row.first(), Some(&Value::Int(0)));
    assert_ne!(row.get(1), Some(&Value::Int(0)));
    let held = writer.attached_logs();
    let (file, bytes) = held.first().cloned().unwrap_or_default();
    assert_eq!(file, b"one.db");
    // The log of the attached database holds its frames still, which a
    // checkpoint that writes them back leaves as they are.
    assert!(bytes.len() > 32, "the log stands: {}", bytes.len());
    let image = writer.attached_written(b"a0").unwrap_or_default();
    assert!(
        crate::db::Database::open(&image)
            .unwrap()
            .tables()
            .any(|table| table.name == b"t1"),
        "the file of the attached database holds the table"
    );
}

/// A trigger of the temp schema runs for the table of the one database
/// its statement named, and a statement of its body names a table the way
/// a statement outside a trigger does.
#[test]
fn which_table_a_trigger_of_the_temp_schema_stands_over() {
    let mut writer = opened();
    writer.run(b"ATTACH ':memory:' AS aux").unwrap();
    for sql in [
        b"CREATE TABLE main.t4(a,b,c)".as_slice(),
        b"CREATE TEMP TABLE t4(a,b,c)",
        b"CREATE TABLE aux.t4(a,b,c)",
        b"CREATE TABLE log(db,a,b,c)",
        b"CREATE TEMP TRIGGER g1 AFTER INSERT ON main.t4 \
          BEGIN INSERT INTO log VALUES('main',new.a,new.b,new.c); END",
        b"CREATE TEMP TRIGGER g2 AFTER INSERT ON temp.t4 \
          BEGIN INSERT INTO log VALUES('temp',new.a,new.b,new.c); END",
        b"CREATE TEMP TRIGGER g3 AFTER INSERT ON aux.t4 \
          BEGIN INSERT INTO log VALUES('aux',new.a,new.b,new.c); END",
        b"INSERT INTO main.t4 VALUES(1,2,3)",
        b"INSERT INTO temp.t4 VALUES(4,5,6)",
        b"INSERT INTO aux.t4 VALUES(7,8,9)",
    ] {
        writer.run(sql).unwrap();
    }
    let held = writer.written();
    let database = crate::db::Database::open(&held).unwrap();
    let rows = database.query(b"SELECT db, a, b, c FROM log").unwrap().rows;
    let shown: Vec<Vec<u8>> = rows
        .iter()
        .flatten()
        .map(|value| value.text().unwrap_or_default())
        .collect();
    assert_eq!(
        shown,
        [
            b"main".to_vec(),
            b"1".to_vec(),
            b"2".to_vec(),
            b"3".to_vec(),
            b"temp".to_vec(),
            b"4".to_vec(),
            b"5".to_vec(),
            b"6".to_vec(),
            b"aux".to_vec(),
            b"7".to_vec(),
            b"8".to_vec(),
            b"9".to_vec(),
        ]
    );
}

/// A statement of a trigger's body names a table under the schema the
/// trigger stands in, so a trigger of `main` reaches no table of the temp
/// schema and a trigger of the temp schema reaches the temp table first.
#[test]
fn which_database_a_statement_of_a_body_names_its_table_under() {
    let mut writer = opened();
    for sql in [
        b"CREATE TABLE t1(a,b)".as_slice(),
        b"CREATE TEMP TABLE t2(x,y)",
        b"CREATE TRIGGER r1 AFTER INSERT ON t1 BEGIN INSERT INTO t2 VALUES(new.a,new.b); END",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(
        writer
            .run(b"INSERT INTO t1 VALUES(1,2)")
            .unwrap_err()
            .message(),
        "no such table: main.t2"
    );
    // An `UPDATE` and a `DELETE` of a body name their table the same way.
    for sql in [
        b"CREATE TRIGGER u1 AFTER UPDATE ON t1 BEGIN UPDATE t2 SET x=1; END".as_slice(),
        b"CREATE TRIGGER d1 AFTER DELETE ON t1 BEGIN DELETE FROM t2; END",
        b"DROP TRIGGER r1",
        b"INSERT INTO t1 VALUES(5,6)",
    ] {
        writer.run(sql).unwrap();
    }
    for sql in [b"UPDATE t1 SET b=3".as_slice(), b"DELETE FROM t1"] {
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "no such table: main.t2",
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
    for sql in [
        b"DROP TRIGGER u1".as_slice(),
        b"DROP TRIGGER d1",
        b"CREATE TEMP TRIGGER r1 AFTER INSERT ON t1 BEGIN INSERT INTO t2 VALUES(new.a,new.b); END",
        b"INSERT INTO t1 VALUES(3,4)",
    ] {
        writer.run(sql).unwrap();
    }
    let temp = writer.attached_written(b"temp").expect("the temp schema");
    let rows = crate::db::Database::open(&temp)
        .unwrap()
        .query(b"SELECT x, y FROM t2")
        .unwrap()
        .rows;
    assert_eq!(rows, alloc::vec![alloc::vec![Value::Int(3), Value::Int(4)]]);
}

/// A statement that names the temp schema opens it, so its own table
/// answers no row where the connection holds no temp object and
/// `PRAGMA database_list` names the temp schema from then on.
#[test]
fn what_a_statement_that_names_the_temp_schema_opens() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    // A connection that made no temp object holds no temp schema, so
    // `PRAGMA database_list` names `main` alone.
    assert!(writer.temp().is_none());
    assert_eq!(writer.run(b"PRAGMA database_list").unwrap().len(), 1);
    // A statement that names no temp schema opens none.
    writer.opens_temp(b"SELECT a FROM t").unwrap();
    assert!(writer.temp().is_none());
    writer
        .opens_temp(b"SELECT * FROM temp.sqlite_master")
        .unwrap();
    let temp = writer.temp().expect("the temp schema");
    assert_eq!(writer.run(b"PRAGMA database_list").unwrap().len(), 2);
    // The temp schema holds no object, so its own table answers no row.
    let held = writer.written();
    let database = crate::db::Database::open(&held)
        .unwrap()
        .attaching(b"temp", &temp)
        .unwrap();
    for sql in [
        b"SELECT count(*) FROM temp.sqlite_master".as_slice(),
        b"SELECT count(*) FROM sqlite_temp_master",
        b"SELECT count(*) FROM sqlite_temp_schema",
    ] {
        assert_eq!(
            database.query(sql).unwrap().rows,
            alloc::vec![alloc::vec![crate::value::Value::Int(0)]],
            "{sql:?}"
        );
    }
    // A name the temp schema holds no table of is no table of it.
    assert_eq!(
        database
            .query(b"SELECT a FROM temp.t")
            .unwrap_err()
            .message(),
        "no such table: temp.t"
    );
}

/// A view and a trigger of one database whose statement names another are
/// each refused where they are made, and one of the temp schema names
/// every database.
#[test]
fn what_a_statement_that_names_another_database_is_refused_with() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"ATTACH ':memory:' AS aux").unwrap();
    writer.run(b"CREATE TABLE aux.u(b)").unwrap();
    writer.run(b"CREATE TEMP TABLE tt(c)").unwrap();
    for (sql, message) in [
        (
            b"CREATE VIEW v AS SELECT b FROM aux.u".as_slice(),
            "view v cannot reference objects in database aux",
        ),
        (
            b"CREATE VIEW v AS SELECT (SELECT b FROM aux.u)",
            "view v cannot reference objects in database aux",
        ),
        (
            b"CREATE VIEW v AS SELECT 1 WHERE 1 IN aux.u",
            "view v cannot reference objects in database aux",
        ),
        (
            b"CREATE VIEW v AS SELECT 1 FROM aux.pragma_table_info('u')",
            "view v cannot reference objects in database aux",
        ),
        (
            b"CREATE VIEW v AS SELECT c FROM temp.tt",
            "view v cannot reference objects in database temp",
        ),
        (
            b"CREATE VIEW v AS SELECT 1 FROM nope.u",
            "view v cannot reference objects in database nope",
        ),
        (
            b"CREATE TRIGGER r AFTER INSERT ON t BEGIN SELECT b FROM aux.u; END",
            "trigger r cannot reference objects in database aux",
        ),
        (
            b"CREATE TRIGGER r AFTER INSERT ON t BEGIN \
              DELETE FROM t WHERE a<(SELECT b FROM aux.u); END",
            "trigger r cannot reference objects in database aux",
        ),
    ] {
        assert_eq!(writer.run(sql).unwrap_err().message(), message, "{sql:?}");
    }
    // A name written under the database the object stands in stands, and
    // so does every name of an object of the temp schema.
    for sql in [
        b"CREATE VIEW v AS SELECT a FROM main.t".as_slice(),
        b"CREATE VIEW aux.w AS SELECT b FROM aux.u",
        b"CREATE TEMP VIEW tv AS SELECT b FROM aux.u",
        b"CREATE TRIGGER r AFTER INSERT ON t BEGIN SELECT a FROM main.t; END",
        b"CREATE TEMP TRIGGER tr AFTER INSERT ON t BEGIN SELECT b FROM aux.u; END",
    ] {
        writer.run(sql).unwrap_or_else(|error| {
            panic!("{sql:?}: {}", error.message());
        });
    }
}
