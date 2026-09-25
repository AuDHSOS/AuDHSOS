// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The functions an application defines on a connection, and the count
//! of pages it holds the file to.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::func::{Defined, Safety};
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
        safety: Safety::Innocuous,
    },
    Defined {
        name: b"drawn",
        count: Some(0),
        answer: drawn,
        safety: Safety::Innocuous,
    },
    Defined {
        name: b"joined",
        count: None,
        answer: joined,
        safety: Safety::Innocuous,
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

/// `REGEXP` and `MATCH` reach the function the application defined under
/// that name, which the library holds none of.
#[test]
fn what_regexp_and_match_reach_of_the_functions_the_application_defined() {
    /// Whether the value holds the pattern, which is the first argument,
    /// and nothing where either is null.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the shape every function the application defines answers in"
    )]
    fn holds(
        _name: &'static [u8],
        args: &[Value],
        _random: Option<&Source>,
    ) -> Result<Value, crate::eval::Error> {
        let (Some(pattern), Some(value)) = (
            args.first().and_then(Value::text),
            args.get(1).and_then(Value::text),
        ) else {
            return Ok(Value::Null);
        };
        Ok(Value::Int(i64::from(
            value
                .windows(pattern.len().max(1))
                .any(|held| held == pattern),
        )))
    }
    static MATCHING: &[Defined] = &[
        Defined {
            name: b"regexp",
            count: Some(2),
            answer: holds,
            safety: Safety::Innocuous,
        },
        Defined {
            name: b"match",
            count: Some(2),
            answer: holds,
            safety: Safety::Innocuous,
        },
    ];
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(x)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES('abcd'),('efgh'),(NULL)")
        .unwrap();
    let bytes = writer.written();
    // A connection told neither name is refused both, which is the
    // refusal the library answers for them.
    let plain = Database::open(&bytes).unwrap();
    assert_eq!(
        plain
            .query(b"SELECT x FROM t WHERE x REGEXP 'bc'")
            .unwrap_err()
            .message(),
        "no such function: REGEXP"
    );
    assert_eq!(
        plain
            .query(b"SELECT x FROM t WHERE x MATCH 'bc'")
            .unwrap_err()
            .message(),
        "no such function: MATCH"
    );
    // `x REGEXP y` is `regexp(y, x)`, so the pattern is the first
    // argument.
    let database = Database::open(&bytes).unwrap().defining(MATCHING);
    for sql in [
        b"SELECT x FROM t WHERE x REGEXP 'bc'".as_slice(),
        b"SELECT x FROM t WHERE x MATCH 'bc'",
    ] {
        assert_eq!(
            database.query(sql).unwrap().rows,
            alloc::vec![alloc::vec![Value::Text(b"abcd".to_vec())]]
        );
    }
    // `NOT REGEXP` answers the other rows, and a row of nothing answers
    // nothing either way.
    assert_eq!(
        database
            .query(b"SELECT x FROM t WHERE x NOT REGEXP 'bc'")
            .unwrap()
            .rows,
        alloc::vec![alloc::vec![Value::Text(b"efgh".to_vec())]]
    );
    assert_eq!(
        database
            .query(b"SELECT x REGEXP 'bc' FROM t WHERE x IS NULL")
            .unwrap()
            .rows,
        alloc::vec![alloc::vec![Value::Null]]
    );
    // A statement that writes reaches the same function.
    let mut told = Writer::opened(&bytes).unwrap();
    told.defines(MATCHING);
    told.run(b"DELETE FROM t WHERE x REGEXP 'bc'").unwrap();
    let left = told.written();
    assert_eq!(
        Database::open(&left)
            .unwrap()
            .query(b"SELECT count(*) FROM t")
            .unwrap()
            .rows,
        alloc::vec![alloc::vec![Value::Int(2)]]
    );
}

/// A column a table computes reaches the functions the application
/// defined, both where the value stands in the row and where the row is
/// read.
#[test]
fn what_a_computed_column_reaches_of_the_functions_the_application_defined() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.defines(DEFINED);
    writer
        .run(b"CREATE TABLE t(a, b AS (twice(a)), c AS (twice(a)+1) STORED)")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(21)").unwrap();
    let image = writer.written();
    let database = Database::open(&image)
        .expect("a database")
        .defining(DEFINED);
    assert_eq!(
        database.query(b"SELECT a, b, c FROM t").unwrap().rows,
        alloc::vec![alloc::vec![Value::Int(21), Value::Int(42), Value::Int(43)]]
    );
    // A connection that was told no such function reads no such column,
    // the value of a column the row holds standing as it is.
    assert_eq!(
        Database::open(&image)
            .expect("a database")
            .query(b"SELECT b FROM t")
            .map(|answered| answered.rows)
            .map_err(|error| error.message())
            .unwrap_err(),
        "no such function: twice"
    );
    // The column is read the same way where an index over it names the
    // rows, which holds the value the column computed.
    writer.run(b"CREATE INDEX tb ON t(b)").unwrap();
    let image = writer.written();
    let database = Database::open(&image)
        .expect("a database")
        .defining(DEFINED);
    assert_eq!(
        database.query(b"SELECT a FROM t WHERE b=42").unwrap().rows,
        alloc::vec![alloc::vec![Value::Int(21)]]
    );
}
/// Three functions, one of each safety the application may mark.
static EDGY: &[Defined] = &[
    Defined {
        name: b"innocuous",
        count: Some(1),
        answer: twice,
        safety: Safety::Innocuous,
    },
    Defined {
        name: b"unsafely",
        count: Some(1),
        answer: twice,
        safety: Safety::Unsafe,
    },
    Defined {
        name: b"directly",
        count: Some(1),
        answer: twice,
        safety: Safety::Direct,
    },
];

/// The functions an expression of a schema object may name: one the
/// application marked `SQLITE_DIRECTONLY` is named by none, and one it
/// marked neither way only where the connection trusts the schema.
#[test]
fn what_a_function_an_expression_of_the_schema_names_is_refused_with() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.defines(EDGY);
    writer.run(b"CREATE TABLE t(a)").unwrap();
    // A `CHECK`, a computed column, a term of an index and the `WHERE` of
    // a partial index are each resolved where the object is made, so each
    // is refused there.
    for sql in [
        b"CREATE TABLE u(a, CHECK(directly(a)>0))".as_slice(),
        b"CREATE TABLE u(a CHECK(directly(a)>0))",
        b"CREATE TABLE u(a, b AS (directly(a)))",
        b"CREATE INDEX ta ON t(directly(a))",
        b"CREATE INDEX ta ON t(a) WHERE directly(a)",
    ] {
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "unsafe use of directly()",
            "{sql:?}"
        );
    }
    // A function the application marked neither way stands while the
    // connection trusts the schema, which it does at open.
    writer
        .run(b"CREATE TABLE u(a, CHECK(unsafely(a)>0))")
        .unwrap();
    writer.run(b"PRAGMA trusted_schema=OFF").unwrap();
    assert_eq!(
        writer
            .run(b"CREATE TABLE w(a, CHECK(unsafely(a)>0))")
            .unwrap_err()
            .message(),
        "unsafe use of unsafely()"
    );
    // One the application marked innocuous stands whatever the connection
    // says, and so does every expression of an object of the temp schema.
    writer
        .run(b"CREATE TABLE w(a, CHECK(innocuous(a)>0))")
        .unwrap();
    writer
        .run(b"CREATE TEMP TABLE tw(a, CHECK(directly(a)>0))")
        .unwrap();
}

/// The statement of a view and the expression of a computed column are
/// each resolved where a statement reads them, so the object is made and
/// the read is refused.
#[test]
fn what_a_function_a_view_and_a_computed_column_name_is_refused_with() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.defines(EDGY);
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"INSERT INTO t VALUES(21)").unwrap();
    for sql in [
        b"CREATE VIEW v AS SELECT directly(a) FROM t".as_slice(),
        b"CREATE VIEW x AS SELECT unsafely(a) FROM t",
        b"CREATE TABLE y(a, b AS (unsafely(a)))",
        b"INSERT INTO y VALUES(21)",
        // A `CREATE TABLE ... AS SELECT` holds no expression of its own.
        b"CREATE TABLE z AS SELECT directly(a) FROM t",
    ] {
        writer.run(sql).unwrap_or_else(|error| {
            panic!("{sql:?}: {}", error.message());
        });
    }
    let image = writer.written();
    let reading = |trusted: bool, sql: &[u8]| {
        Database::open(&image)
            .expect("a database")
            .defining(EDGY)
            .trusting(trusted)
            .query(sql)
            .map(|answered| answered.rows)
            .map_err(|error| error.message())
    };
    assert_eq!(
        reading(true, b"SELECT * FROM v").unwrap_err(),
        "unsafe use of directly()"
    );
    assert_eq!(
        reading(true, b"SELECT * FROM x").unwrap(),
        alloc::vec![alloc::vec![Value::Int(42)]]
    );
    assert_eq!(
        reading(false, b"SELECT * FROM x").unwrap_err(),
        "unsafe use of unsafely()"
    );
    // A statement a client wrote names every function the connection
    // holds, whatever the connection says about the schema.
    assert_eq!(
        reading(false, b"SELECT directly(a) FROM t").unwrap(),
        alloc::vec![alloc::vec![Value::Int(42)]]
    );
    assert_eq!(
        reading(true, b"SELECT b FROM y").unwrap(),
        alloc::vec![alloc::vec![Value::Int(42)]]
    );
    assert_eq!(
        reading(false, b"SELECT b FROM y").unwrap_err(),
        "unsafe use of unsafely()"
    );
}
/// A `CHECK`, a `DEFAULT` and the body of a trigger are each expressions
/// of the schema, so each names the functions the application defined and
/// is held to the ones a schema may name.
#[test]
fn what_a_function_a_constraint_and_a_trigger_name_is_refused_with() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.defines(EDGY);
    for sql in [
        b"CREATE TABLE t(a, b DEFAULT (unsafely(21)))".as_slice(),
        b"CREATE TABLE u(a, CHECK(unsafely(a)>0))",
        b"CREATE TABLE seen(x)",
        b"CREATE TRIGGER r AFTER INSERT ON t BEGIN \
          INSERT INTO seen(x) SELECT unsafely(new.a); END",
    ] {
        writer.run(sql).unwrap_or_else(|error| {
            panic!("{sql:?}: {}", error.message());
        });
    }
    // While the connection trusts the schema each of the three answers.
    writer.run(b"INSERT INTO t(a) VALUES(1)").unwrap();
    writer.run(b"INSERT INTO u VALUES(1)").unwrap();
    assert_eq!(
        writer
            .run(b"SELECT b FROM t")
            .map_err(|error| error.message()),
        Err(alloc::string::String::from("near \"SELECT\": syntax error"))
    );
    let image = writer.written();
    let database = Database::open(&image).expect("a database").defining(EDGY);
    assert_eq!(
        database.query(b"SELECT b FROM t").unwrap().rows,
        alloc::vec![alloc::vec![Value::Int(42)]]
    );
    assert_eq!(
        database.query(b"SELECT x FROM seen").unwrap().rows,
        alloc::vec![alloc::vec![Value::Int(2)]]
    );
    // A connection that does not trust the schema is refused each of
    // them where the expression is read.
    writer.run(b"PRAGMA trusted_schema=OFF").unwrap();
    for (sql, message) in [
        (
            b"INSERT INTO t(a) VALUES(2)".as_slice(),
            "unsafe use of unsafely()",
        ),
        (b"INSERT INTO u VALUES(2)", "unsafe use of unsafely()"),
    ] {
        assert_eq!(writer.run(sql).unwrap_err().message(), message, "{sql:?}");
    }
    // The body of the trigger is read the same way, which the statement
    // that fires it carries out.
    writer.run(b"PRAGMA trusted_schema=ON").unwrap();
    writer.run(b"DROP TABLE t").unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer
        .run(
            b"CREATE TRIGGER r2 AFTER INSERT ON t BEGIN \
              INSERT INTO seen(x) SELECT unsafely(new.a); END",
        )
        .unwrap();
    writer.run(b"PRAGMA trusted_schema=OFF").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(3)")
            .unwrap_err()
            .message(),
        "unsafe use of unsafely()"
    );
}
