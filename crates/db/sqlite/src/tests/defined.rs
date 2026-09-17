// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The functions an application defines on a connection, and the count
//! of pages it holds the file to.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::func::Defined;
use crate::header::Encoding;
use crate::random::Source;
use crate::value::Value;

/// `twice(x)`, which answers the number it is given doubled.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the shape every function the application defines answers in"
)]
fn twice(args: &[Value], _random: Option<&Source>) -> Result<Value, crate::eval::Error> {
    let held = args.first().map_or(0, Value::to_integer);
    Ok(Value::Int(held.saturating_mul(2)))
}

/// `drawn()`, which answers as many bytes as the connection draws for
/// it, so a connection that was given no source answers nothing.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the shape every function the application defines answers in"
)]
fn drawn(_args: &[Value], random: Option<&Source>) -> Result<Value, crate::eval::Error> {
    Ok(Value::Blob(
        random.map_or_else(Vec::new, |source| source.bytes(4)),
    ))
}

/// The two, as a connection holds them.
static DEFINED: &[Defined] = &[
    Defined {
        name: b"twice",
        count: 1,
        answer: twice,
    },
    Defined {
        name: b"drawn",
        count: 0,
        answer: drawn,
    },
];

/// A statement that writes and one that reads both reach the functions
/// the application defined, and a name it defined nothing for is the
/// refusal it always was.
#[test]
fn what_a_statement_reaches_of_the_functions_the_application_defined() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.defines(DEFINED);
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES(twice(21), drawn())")
        .unwrap();
    writer
        .run(b"UPDATE t SET a=twice(a) WHERE a=twice(21)")
        .unwrap();
    let image = writer.written();
    let database = Database::open(&image)
        .expect("a database")
        .defining(DEFINED);
    let answered = database
        .query(b"SELECT twice(a), length(b) FROM t")
        .unwrap();
    assert_eq!(
        answered.rows,
        alloc::vec![alloc::vec![Value::Int(168), Value::Int(4)]]
    );
    // A name the application defined for another number of arguments
    // is the refusal the library answers for it.
    assert_eq!(
        database
            .query(b"SELECT twice(1,2) FROM t")
            .unwrap_err()
            .message(),
        "no such function: twice"
    );
    // A connection told of no function reads the name as one the
    // library holds, which it does not.
    let plain = Database::open(&image).expect("a database");
    assert_eq!(
        plain
            .query(b"SELECT twice(a) FROM t")
            .unwrap_err()
            .message(),
        "no such function: twice"
    );
}

/// `PRAGMA max_page_count` holds the file to a count of pages, which a
/// statement that would grow it past is refused for.
#[test]
fn what_a_file_held_to_a_count_of_pages_refuses() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"PRAGMA max_page_count=20").unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    let mut written = 0;
    let refused = loop {
        written += 1;
        assert!(written < 500, "the file never filled");
        if let Err(error) = writer.run(b"INSERT INTO t VALUES(zeroblob(900))") {
            break error.message();
        }
    };
    assert_eq!(refused, "database or disk is full");
    // The pages the statement wrote before it was refused are not
    // there, so the file stands where the statement found it.
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    assert_eq!(
        database.query(b"SELECT count(*) FROM t").unwrap().rows,
        alloc::vec![alloc::vec![Value::Int(written - 1)]]
    );
    // A count the file already passed leaves it as it is and refuses
    // the next page all the same.
    writer.run(b"PRAGMA max_page_count=1").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(zeroblob(900))")
            .unwrap_err()
            .message(),
        "database or disk is full"
    );
}
