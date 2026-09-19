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
