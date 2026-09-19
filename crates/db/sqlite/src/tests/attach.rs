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
