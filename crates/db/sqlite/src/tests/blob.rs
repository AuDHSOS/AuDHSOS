// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The bytes of one value, read and written where they lie, which
//! `sqlite3_blob_open` of `research/sqlite/src/vdbeblob.c:74` opens.

use crate::change::{Blob, Writer};
use crate::db::{Database, Error};
use crate::header::Encoding;
use crate::value::Value;

/// A connection that ran the statements.
fn ran(page: u32, statements: &[&str]) -> Writer {
    let mut writer = Writer::new(page, 0, Encoding::Utf8).unwrap();
    for sql in statements {
        writer.run(sql.as_bytes()).expect("the statement");
    }
    writer
}

/// One handle over the table `t`, column `b`, row one.
fn asked(writing: bool) -> Blob<'static> {
    Blob {
        schema: None,
        table: b"t",
        column: b"b",
        rowid: 1,
        writing,
    }
}

/// What one statement answers over the file the writer holds.
fn answered(writer: &Writer, sql: &str) -> alloc::vec::Vec<alloc::vec::Vec<Value>> {
    let bytes = writer.written();
    Database::open(&bytes)
        .unwrap()
        .query(sql.as_bytes())
        .unwrap()
        .rows
}

/// `e_blobbytes-1.*` and `incrblob-1.2.*` of `test/incrblob.test`: the
/// bytes of a value, read and written where they lie.
#[test]
fn what_a_blob_handle_reads_and_writes() {
    let mut writer = ran(
        1024,
        &[
            "CREATE TABLE t(a, b, c)",
            "INSERT INTO t VALUES(1, 'hello world', x'0102030405')",
        ],
    );
    // The five a handle names are compared and shown as the values of
    // every other command are.
    assert_eq!(asked(true), asked(true));
    assert!(alloc::format!("{:?}", asked(true)).contains("Blob"));
    assert_eq!(writer.blob_bytes(&asked(false)).unwrap(), 11);
    assert_eq!(writer.blob_read(&asked(false), 6, 5).unwrap(), b"world");
    writer.blob_write(&asked(true), 0, b"HELLO").unwrap();
    assert_eq!(
        answered(&writer, "SELECT b FROM t"),
        [alloc::vec![Value::Text(b"HELLO world".to_vec())]]
    );
    // A value of bytes is read and written as one of text is, and the
    // length it carries stands.
    let held = Blob {
        column: b"c",
        ..asked(true)
    };
    assert_eq!(writer.blob_bytes(&held).unwrap(), 5);
    writer.blob_write(&held, 1, b"\xff\xfe").unwrap();
    assert_eq!(
        writer.blob_read(&held, 0, 5).unwrap(),
        b"\x01\xff\xfe\x04\x05"
    );
    assert_eq!(
        answered(&writer, "PRAGMA integrity_check"),
        [alloc::vec![Value::Text(b"ok".to_vec())]]
    );
    // A row past the first is reached by its key.
    writer
        .run(b"INSERT INTO t VALUES(2, 'second row', NULL)")
        .unwrap();
    let second = Blob {
        rowid: 2,
        ..asked(true)
    };
    assert_eq!(writer.blob_read(&second, 7, 3).unwrap(), b"row");
    writer.blob_write(&second, 0, b"SECOND").unwrap();
    assert_eq!(
        answered(&writer, "SELECT b FROM t WHERE rowid=2"),
        [alloc::vec![Value::Text(b"SECOND row".to_vec())]]
    );
    // A read that reaches past the value carries the words the C
    // library answers for a code with no message of its own.
    assert_eq!(
        writer.blob_read(&second, 8, 8).unwrap_err().message(),
        "SQL logic error"
    );
}

/// `incrblob-1.3.*`: a value that runs onto a chain of overflow pages is
/// read and written page by page.
#[test]
fn what_a_blob_handle_reads_over_a_chain() {
    let text = alloc::string::String::from_utf8(alloc::vec![b'.'; 2000]).unwrap();
    let mut writer = ran(512, &["CREATE TABLE t(a, b)"]);
    writer
        .run(alloc::format!("INSERT INTO t VALUES(1, '{text}')").as_bytes())
        .unwrap();
    let held = asked(true);
    assert_eq!(writer.blob_bytes(&held).unwrap(), 2000);
    // The first bytes lie on the page of the row, the last on the last
    // page of the chain, and a write that begins past a page of the
    // chain steps over it.
    writer.blob_write(&held, 0, b"ABC").unwrap();
    writer.blob_write(&held, 1990, b"0123456789").unwrap();
    writer.blob_write(&held, 1000, b"middle").unwrap();
    assert_eq!(writer.blob_read(&held, 0, 4).unwrap(), b"ABC.");
    assert_eq!(writer.blob_read(&held, 1000, 6).unwrap(), b"middle");
    assert_eq!(writer.blob_read(&held, 1990, 10).unwrap(), b"0123456789");
    assert_eq!(
        answered(&writer, "SELECT length(b) FROM t"),
        [alloc::vec![Value::Int(2000)]]
    );
    assert_eq!(
        answered(&writer, "PRAGMA integrity_check"),
        [alloc::vec![Value::Text(b"ok".to_vec())]]
    );
    // A read or a write that reaches past the value is refused.
    assert_eq!(writer.blob_read(&held, 1995, 6), Err(Error::BlobRange));
    assert_eq!(writer.blob_write(&held, 1999, b"xx"), Err(Error::BlobRange));
}

/// `e_blobopen-2.*`: what a blob handle is refused for.
#[test]
fn what_a_blob_handle_is_refused_for() {
    let mut writer = ran(
        1024,
        &[
            "CREATE TABLE t(a, b, c REFERENCES p(x))",
            "CREATE TABLE p(x PRIMARY KEY)",
            "CREATE TABLE k(a TEXT PRIMARY KEY, b) WITHOUT ROWID",
            "CREATE TABLE i(a, b)",
            "CREATE INDEX ib ON i(b)",
            "CREATE VIEW v AS SELECT a, b FROM t",
            "INSERT INTO t VALUES(1, 'text', NULL)",
            "INSERT INTO i VALUES(1, 'held')",
        ],
    );
    let refused = |writer: &mut Writer, held: &Blob<'_>| writer.blob_bytes(held).unwrap_err();
    // A table no database holds, a view and a table without a rowid.
    assert_eq!(
        refused(
            &mut writer,
            &Blob {
                table: b"nosuch",
                ..asked(false)
            }
        )
        .message(),
        "no such table: main.nosuch"
    );
    assert_eq!(
        refused(
            &mut writer,
            &Blob {
                table: b"v",
                ..asked(false)
            }
        )
        .message(),
        "cannot open view: v"
    );
    assert_eq!(
        refused(
            &mut writer,
            &Blob {
                table: b"k",
                ..asked(false)
            }
        )
        .message(),
        "cannot open table without rowid: k"
    );
    // A column the table does not hold, and a key no row carries.
    assert_eq!(
        refused(
            &mut writer,
            &Blob {
                column: b"nosuch",
                ..asked(false)
            }
        )
        .message(),
        "no such column: \"nosuch\""
    );
    assert_eq!(
        refused(
            &mut writer,
            &Blob {
                rowid: 7,
                ..asked(false)
            }
        )
        .message(),
        "no such rowid: 7"
    );
}

/// `e_blobopen-2.*`: a value that is neither text nor bytes carries no
/// handle.
#[test]
fn what_value_a_blob_handle_may_not_open() {
    let mut writer = ran(
        1024,
        &[
            "CREATE TABLE t(a, b, c)",
            "INSERT INTO t VALUES(1, 'text', NULL)",
        ],
    );
    let refused = |writer: &mut Writer, held: &Blob<'_>| writer.blob_bytes(held).unwrap_err();
    assert_eq!(
        refused(
            &mut writer,
            &Blob {
                column: b"a",
                ..asked(false)
            }
        )
        .message(),
        "cannot open value of type integer"
    );
    assert_eq!(
        refused(
            &mut writer,
            &Blob {
                column: b"c",
                ..asked(false)
            }
        )
        .message(),
        "cannot open value of type null"
    );
    // A row written before a column was added holds no value for it.
    writer.run(b"ALTER TABLE t ADD COLUMN d").unwrap();
    assert_eq!(
        refused(
            &mut writer,
            &Blob {
                column: b"d",
                ..asked(false)
            }
        )
        .message(),
        "cannot open value of type null"
    );
    writer.run(b"UPDATE t SET a=1.5").unwrap();
    assert_eq!(
        refused(
            &mut writer,
            &Blob {
                column: b"a",
                ..asked(false)
            }
        )
        .message(),
        "cannot open value of type real"
    );
}

/// `e_blobopen-2.*`: a column an index, the primary key or a foreign key
/// holds is read and not written.
#[test]
fn what_column_a_blob_handle_may_not_write() {
    let mut writer = ran(
        1024,
        &[
            "CREATE TABLE t(a, b, c REFERENCES p(x))",
            "CREATE TABLE p(x PRIMARY KEY)",
            "CREATE TABLE i(a, b)",
            "CREATE INDEX ib ON i(b)",
            "INSERT INTO p VALUES('x')",
            "INSERT INTO t VALUES(1, 'text', 'x')",
            "INSERT INTO i VALUES(1, 'held')",
            "PRAGMA foreign_keys=ON",
        ],
    );
    let refused = |writer: &mut Writer, held: &Blob<'_>| writer.blob_bytes(held).unwrap_err();
    let indexed = Blob {
        table: b"i",
        column: b"b",
        rowid: 1,
        writing: true,
        schema: None,
    };
    assert_eq!(
        refused(&mut writer, &indexed).message(),
        "cannot open indexed column for writing"
    );
    assert_eq!(
        writer
            .blob_bytes(&Blob {
                writing: false,
                ..indexed
            })
            .unwrap(),
        4
    );
    assert_eq!(
        refused(
            &mut writer,
            &Blob {
                column: b"c",
                ..asked(true)
            }
        )
        .message(),
        "cannot open foreign key column for writing"
    );
    // A column of an index over an expression is one the handle may not
    // write either.
    writer.run(b"CREATE INDEX it ON t(b || 'x')").unwrap();
    assert_eq!(
        refused(&mut writer, &asked(true)).message(),
        "cannot open indexed column for writing"
    );
}

/// The database a blob handle names is the one it reads, which the temp
/// schema and a database an `ATTACH` added are each one of.
#[test]
fn what_database_a_blob_handle_names() {
    let mut writer = ran(1024, &["CREATE TABLE t(a, b)"]);
    writer.run(b"INSERT INTO t VALUES(1, 'main')").unwrap();
    writer.run(b"CREATE TEMP TABLE t(a, b)").unwrap();
    writer.run(b"INSERT INTO temp.t VALUES(1, 'temp')").unwrap();
    assert_eq!(
        writer
            .blob_read(
                &Blob {
                    schema: Some(b"temp"),
                    ..asked(false)
                },
                0,
                4
            )
            .unwrap(),
        b"temp"
    );
    assert_eq!(
        writer
            .blob_read(
                &Blob {
                    schema: Some(b"main"),
                    ..asked(false)
                },
                0,
                4
            )
            .unwrap(),
        b"main"
    );
    // A name the connection holds no database under names no table of
    // that database either.
    assert_eq!(
        writer
            .blob_bytes(&Blob {
                schema: Some(b"nosuch"),
                ..asked(false)
            })
            .unwrap_err()
            .message(),
        "no such table: nosuch.t"
    );
    // A write reaches the database the handle names and leaves the
    // other as it stands.
    writer
        .blob_write(
            &Blob {
                schema: Some(b"temp"),
                ..asked(true)
            },
            0,
            b"TEMP",
        )
        .unwrap();
    assert_eq!(
        answered(&writer, "SELECT b FROM main.t"),
        [alloc::vec![Value::Text(b"main".to_vec())]]
    );
    let temp = writer.attached_written(b"temp").expect("the temp schema");
    assert_eq!(
        Database::open(&temp)
            .unwrap()
            .query(b"SELECT b FROM t")
            .unwrap()
            .rows,
        [alloc::vec![Value::Text(b"TEMP".to_vec())]]
    );
}

/// What a blob handle over a file that says what it does not hold is
/// refused with.
#[test]
fn what_a_blob_handle_over_a_broken_file_is_refused_with() {
    let held = || {
        let mut writer = ran(1024, &["CREATE TABLE t(a, b)"]);
        writer.run(b"INSERT INTO t VALUES(1, 'hello')").unwrap();
        writer.written()
    };
    // The page the table begins on carries a type no b-tree page has,
    // so the walk to the row is refused.
    let mut image = held();
    image[1024] = 0x7f;
    let mut writer = Writer::opened(&image).unwrap();
    assert!(writer.blob_bytes(&asked(false)).is_err());
    // The record says the value holds more bytes than the row carries,
    // so the write reaches past the payload.
    let mut image = held();
    // The row lies at the end of the page it is on: the header of the
    // record is three bytes and the type of the second value stands at
    // its end.
    let at = image
        .windows(5)
        .position(|held| held == b"hello")
        .expect("the row");
    image[at - 1] = 12 + 2 * 40 + 1;
    let mut writer = Writer::opened(&image).unwrap();
    assert_eq!(writer.blob_bytes(&asked(true)).unwrap(), 40);
    assert!(writer.blob_write(&asked(true), 0, &[b'x'; 40]).is_err());
}
