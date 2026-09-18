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
fn twice(
    _name: &'static [u8],
    args: &[Value],
    _random: Option<&Source>,
) -> Result<Value, crate::eval::Error> {
    let held = args.first().map_or(0, Value::to_integer);
    Ok(Value::Int(held.saturating_mul(2)))
}

/// `drawn()`, which answers as many bytes as the connection draws for
/// it, so a connection that was given no source answers nothing.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the shape every function the application defines answers in"
)]
fn drawn(
    _name: &'static [u8],
    _args: &[Value],
    random: Option<&Source>,
) -> Result<Value, crate::eval::Error> {
    Ok(Value::Blob(
        random.map_or_else(Vec::new, |source| source.bytes(4)),
    ))
}

/// `joined(...)`, which takes any number of arguments and answers
/// their text run together, which is `nArg` at -1.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the shape every function the application defines answers in"
)]
fn joined(
    name: &'static [u8],
    args: &[Value],
    _random: Option<&Source>,
) -> Result<Value, crate::eval::Error> {
    let mut out = name.to_vec();
    for value in args {
        out.push(b':');
        out.extend_from_slice(&value.text().unwrap_or_default());
    }
    Ok(Value::Text(out))
}

/// The three, as a connection holds them.
static DEFINED: &[Defined] = &[
    Defined {
        name: b"twice",
        count: Some(1),
        answer: twice,
    },
    Defined {
        name: b"drawn",
        count: Some(0),
        answer: drawn,
    },
    Defined {
        name: b"joined",
        count: None,
        answer: joined,
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
    // A function the application defined for any number of arguments
    // answers for every count, and is given the name it was called
    // under.
    for (sql, text) in [
        (b"SELECT joined()".as_slice(), "joined"),
        (b"SELECT joined('x')", "joined:x"),
        (b"SELECT joined('x','y','z')", "joined:x:y:z"),
    ] {
        assert_eq!(
            database.query(sql).unwrap().rows,
            alloc::vec![alloc::vec![Value::Text(text.as_bytes().to_vec())]]
        );
    }
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

/// `md5sum` of `test_md5.c`: an aggregate the application defined reads
/// the rows of its group at once.
#[test]
fn an_aggregate_the_application_defined_reads_the_rows_of_its_group() {
    use crate::func::Grouped;
    use crate::value::Value as V;
    /// The arguments of every row, in order, as one text.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the shape every aggregate the application defines answers in"
    )]
    fn joined(name: &'static [u8], rows: &[alloc::vec::Vec<V>]) -> Result<V, crate::eval::Error> {
        let mut out = name.to_vec();
        for row in rows {
            out.push(b':');
            for value in row {
                out.extend_from_slice(&value.text().unwrap_or_default());
            }
        }
        Ok(V::Text(out))
    }
    static GROUPED: &[Grouped] = &[
        Grouped {
            name: b"joined",
            count: None,
            answer: joined,
        },
        // One that takes one argument and nothing else, which a call of
        // two arguments is no call of.
        Grouped {
            name: b"single",
            count: Some(1),
            answer: joined,
        },
    ];
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.groups(GROUPED);
    writer.run(b"CREATE TABLE t(a, b)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES(1,'x'),(2,'y'),(3,NULL)")
        .unwrap();
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap().grouping(GROUPED);
    // Every row of the group is stepped, in the order the rows are read.
    assert_eq!(
        database.query(b"SELECT joined(a,b) FROM t").unwrap().rows,
        [alloc::vec![Value::Text(b"joined:1x:2y:3".to_vec())]]
    );
    // One group per key, and a group of no row answers the aggregate
    // over no row at all.
    assert_eq!(
        database
            .query(b"SELECT joined(a) FROM t GROUP BY a%2 ORDER BY a%2")
            .unwrap()
            .rows,
        [
            alloc::vec![Value::Text(b"joined:2".to_vec())],
            alloc::vec![Value::Text(b"joined:1:3".to_vec())],
        ]
    );
    assert_eq!(
        database
            .query(b"SELECT joined(a) FROM t WHERE a>9")
            .unwrap()
            .rows,
        [alloc::vec![Value::Text(b"joined".to_vec())]]
    );
    // `DISTINCT` puts the rows through one column, and an `OVER` reads
    // the aggregate over the frame.
    assert_eq!(
        database
            .query(b"SELECT joined(DISTINCT a%2) FROM t")
            .unwrap()
            .rows,
        [alloc::vec![Value::Text(b"joined:1:0".to_vec())]]
    );
    assert_eq!(
        database
            .query(b"SELECT joined(a) OVER (ORDER BY a) FROM t ORDER BY a")
            .unwrap()
            .rows,
        [
            alloc::vec![Value::Text(b"joined:1".to_vec())],
            alloc::vec![Value::Text(b"joined:1:2".to_vec())],
            alloc::vec![Value::Text(b"joined:1:2:3".to_vec())],
        ]
    );
    // An aggregate that takes one argument answers a call of one and
    // is no call of two.
    assert_eq!(
        database.query(b"SELECT single(a) FROM t").unwrap().rows,
        [alloc::vec![Value::Text(b"single:1:2:3".to_vec())]]
    );
    assert!(database.query(b"SELECT single(a,b) FROM t").is_err());
    // A name the connection was not told of is no function at all.
    let plain = Database::open(&bytes).unwrap();
    assert!(plain.query(b"SELECT joined(a) FROM t").is_err());
}
