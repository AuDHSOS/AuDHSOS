// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::db`, against the answers SQLite gives for the same
//! statements over the same files.
//!
//! `fixtures/query.corpus` is a fixture and a statement per line;
//! `fixtures/query.golden` is the columns SQLite named and the rows it
//! answered, each value quoted, written by `tools/sqlite-oracle.c`. The
//! comparison is the name of every column and the value of every field.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use crate::db::Database;
use crate::func::{Function, call};
use crate::value::{Collation, Value};

/// The cases: which file, and what to ask it.
fn corpus() -> Vec<(&'static str, &'static str)> {
    include_str!("fixtures/query.corpus")
        .lines()
        .map(|line| line.split_once('|').unwrap_or((line, "")))
        .collect()
}

/// What SQLite answered.
fn golden() -> Vec<&'static str> {
    include_str!("fixtures/query.golden").lines().collect()
}

/// The bytes of a fixture, by name.
fn fixture(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "small.db" => super::SMALL,
        "page512.db" => super::PAGE512,
        "utf16.db" => super::UTF16,
        "overflow.db" => super::OVERFLOW,
        "indexed.db" => super::INDEXED,
        "keys.db" => super::KEYS,
        "generated.db" => super::GENERATED,
        "wide16.db" => super::WIDE16,
        "joins.db" => super::JOINS,
        "format1.db" => super::FORMAT1,
        "format3.db" => super::FORMAT3,
        "format4.db" => super::FORMAT4,
        "defaults.db" => super::DEFAULTS,
        "affinity.db" => super::AFFINITY,
        _ => return super::matrix::fixture(name),
    })
}

/// One value as `quote` writes it.
fn quoted(value: &Value) -> String {
    let answer = call(
        Function::Quote,
        core::slice::from_ref(value),
        Collation::Binary,
        crate::header::Encoding::Utf8,
        None,
    )
    .expect("a function that always answers");
    String::from_utf8_lossy(&answer.text().unwrap_or_default()).into_owned()
}

/// What this engine answers, as the oracle would have written it.
fn answer(bytes: &[u8], sql: &str) -> Option<String> {
    let database = Database::open(bytes).ok()?;
    let answered = database.query(sql.as_bytes()).ok()?;
    let mut out = String::from("N");
    for name in &answered.names {
        out.push('|');
        out.push_str(&String::from_utf8_lossy(name));
    }
    for row in &answered.rows {
        out.push_str("\tR");
        for value in row {
            out.push('|');
            out.push_str(&quoted(value));
        }
    }
    Some(out)
}

#[test]
fn every_statement_answers_what_the_c_library_answers() {
    let cases = corpus();
    let answers = golden();
    assert_eq!(cases.len(), answers.len());
    for ((name, sql), theirs) in cases.iter().zip(answers) {
        let bytes = fixture(name).unwrap_or_else(|| panic!("no fixture {name}"));
        let mine = answer(bytes, sql);
        // A refusal before the first row is a line of its own; one
        // after some rows is a field at the end of the line. Either way
        // this engine answers the whole statement or none of it.
        if theirs.starts_with('!') || theirs.contains("\t!") {
            assert!(mine.is_none(), "{sql} over {name} is refused by SQLite");
            continue;
        }
        let mine = mine.unwrap_or_else(|| panic!("{sql} over {name} is refused"));
        assert_eq!(mine, theirs, "{sql} over {name}");
    }
}

#[test]
fn a_file_that_is_not_a_database_is_refused_as_one() {
    use crate::db::Error;
    use crate::error;
    assert_eq!(
        Database::open(b"not a database at all").unwrap_err(),
        Error::Image(error::Error::Truncated)
    );
    let mut bytes = super::SMALL.to_vec();
    bytes[0] = b'X';
    assert_eq!(
        Database::open(&bytes).unwrap_err(),
        Error::Image(error::Error::Magic)
    );
}

#[test]
fn a_statement_that_is_not_one_is_refused_as_one() {
    use crate::db::Error;
    let database = Database::open(super::SMALL).expect("a database");
    assert!(matches!(
        database.query(b"SELECT FROM").unwrap_err(),
        Error::Parse(_)
    ));
    assert_eq!(
        database.query(b"SELECT * FROM nosuch"),
        Err(Error::NoTable(b"nosuch".to_vec()))
    );
    assert_eq!(
        database.query(b"SELECT a FROM t ORDER BY 9"),
        Err(Error::OrderRange)
    );
}

#[test]
fn a_schema_this_crate_cannot_read_is_refused() {
    use crate::db::Error;
    use crate::schema;
    // The schema of a file is text, and a file is not obliged to hold
    // text this crate can make a table of. One byte makes the small
    // fixture's two columns one name.
    let mut bytes = super::SMALL.to_vec();
    let at = bytes
        .windows(4)
        .position(|window| window == b"b TE")
        .expect("the column in the schema");
    bytes[at] = b'a';
    assert_eq!(
        Database::open(&bytes).unwrap_err(),
        Error::Schema(schema::Error::DuplicateColumn)
    );
}

#[test]
fn the_tables_of_a_file_are_the_ones_its_schema_names() {
    let database = Database::open(super::INDEXED).expect("a database");
    let names: Vec<String> = database
        .tables()
        .map(|table| String::from_utf8_lossy(&table.name).into_owned())
        .collect();
    // The indexes of the fixture are not tables, and are skipped.
    assert_eq!(names, ["k", "m", "e", "o", "u", "f"]);
    let database = Database::open(super::KEYS).expect("a database");
    let names: Vec<String> = database
        .tables()
        .map(|table| String::from_utf8_lossy(&table.name).into_owned())
        .collect();
    assert_eq!(names, ["r", "w", "d", "u", "p", "q"]);
}

/// A database whose schema table holds exactly `records`, built by hand
/// so that a row the shell would never write can be put in front of the
/// reader.
fn schema_file(records: &[Vec<u8>]) -> Vec<u8> {
    const PAGE: usize = 4096;
    let mut bytes = super::SMALL[..100].to_vec();
    bytes.resize(PAGE, 0);
    bytes[28..32].copy_from_slice(&1u32.to_be_bytes());
    let mut cells = Vec::new();
    let mut offset = PAGE;
    for (at, record) in records.iter().enumerate() {
        let (len, len_bytes) = super::varint(record.len() as u64);
        let (rowid, rowid_bytes) = super::varint(at as u64 + 1);
        let mut cell = Vec::new();
        cell.extend_from_slice(&len[..len_bytes]);
        cell.extend_from_slice(&rowid[..rowid_bytes]);
        cell.extend_from_slice(record);
        offset -= cell.len();
        bytes[offset..offset + cell.len()].copy_from_slice(&cell);
        cells.push(offset);
    }
    bytes[100] = 13;
    bytes[103..105].copy_from_slice(&(cells.len() as u16).to_be_bytes());
    bytes[105..107].copy_from_slice(&(offset as u16).to_be_bytes());
    for (at, start) in cells.iter().enumerate() {
        let slot = 108 + at * 2;
        bytes[slot..slot + 2].copy_from_slice(&(*start as u16).to_be_bytes());
    }
    bytes
}

/// One record of five values, each either text or a small integer.
fn record(values: [Result<&str, i64>; 5]) -> Vec<u8> {
    let mut serials = Vec::new();
    let mut body = Vec::new();
    for value in values {
        match value {
            Ok(text) => {
                let (code, len) = super::varint(text.len() as u64 * 2 + 13);
                serials.extend_from_slice(&code[..len]);
                body.extend_from_slice(text.as_bytes());
            }
            Err(number) => {
                serials.push(1);
                body.push(number as u8);
            }
        }
    }
    let mut out = Vec::new();
    let (len, bytes) = super::varint(serials.len() as u64 + 1);
    out.extend_from_slice(&len[..bytes]);
    out.extend_from_slice(&serials);
    out.extend_from_slice(&body);
    out
}

#[test]
fn a_computed_column_that_cannot_be_computed_is_refused() {
    use crate::db::Error;
    // The table reads the schema page as its own rows, so that a row
    // exists to compute a column of. Its last column names itself.
    for text in [
        "CREATE TABLE a(p,q,r,s,t,u AS (u+1))",
        "CREATE TABLE a(p,q,r,s,t,u AS (nosuch))",
    ] {
        let file = schema_file(&[record([Ok("table"), Ok("a"), Ok("a"), Err(1), Ok(text)])]);
        let database = Database::open(&file).unwrap();
        assert_eq!(
            database.query(b"SELECT * FROM a").unwrap_err(),
            Error::Computed,
            "{text}"
        );
    }
}

#[test]
fn a_schema_row_that_is_not_a_table_is_passed_over() {
    // A file is not obliged to hold the schema the shell would write.
    // Five rows the reader must walk past rather than trip on: one whose
    // type is not text at all, one that is not a table, one whose root
    // page is not a number, one with no statement, and one that calls
    // itself a table and holds an index.
    let file = schema_file(&[
        record([Err(1), Ok("a"), Ok("a"), Err(2), Ok("CREATE TABLE a(x)")]),
        record([
            Ok("index"),
            Ok("i"),
            Ok("t"),
            Err(3),
            Ok("CREATE INDEX i ON t(x)"),
        ]),
        record([
            Ok("table"),
            Ok("b"),
            Ok("b"),
            Ok("two"),
            Ok("CREATE TABLE b(x)"),
        ]),
        record([Ok("table"), Ok("c"), Ok("c"), Err(4), Ok("")]),
        record([
            Ok("table"),
            Ok("e"),
            Ok("e"),
            Err(6),
            Ok("CREATE INDEX i ON t(x)"),
        ]),
        record([
            Ok("table"),
            Ok("d"),
            Ok("d"),
            Err(5),
            Ok("CREATE TABLE d(x)"),
        ]),
        // Three index rows the reader must walk past as well: one whose
        // root page is not a number, one that calls itself an index and
        // holds a table, and one over a table the file does not have.
        record([
            Ok("index"),
            Ok("j"),
            Ok("d"),
            Ok("two"),
            Ok("CREATE INDEX j ON d(x)"),
        ]),
        record([
            Ok("index"),
            Ok("l"),
            Ok("d"),
            Err(7),
            Ok("CREATE TABLE l(x)"),
        ]),
        record([
            Ok("index"),
            Ok("n"),
            Ok("nosuch"),
            Err(8),
            Ok("CREATE INDEX n ON nosuch(x)"),
        ]),
        // Two view rows the reader must walk past as well: one whose
        // statement it cannot read, and one that calls itself a view
        // and holds a table.
        record([Ok("view"), Ok("p"), Ok("p"), Err(0), Ok("CREATE VIEW p AS")]),
        record([
            Ok("view"),
            Ok("q"),
            Ok("q"),
            Err(0),
            Ok("CREATE TABLE q(x)"),
        ]),
        record([
            Ok("view"),
            Ok("r"),
            Ok("r"),
            Err(0),
            Ok("CREATE VIEW r AS SELECT x FROM d"),
        ]),
    ]);
    let database = Database::open(&file).expect("a database");
    let names: Vec<String> = database
        .tables()
        .map(|table| String::from_utf8_lossy(&table.name).into_owned())
        .collect();
    assert_eq!(names, ["d"]);
    // The view the reader could read is the only one it holds, and a
    // statement that names either of the others answers no table.
    assert!(database.view(b"r").is_some());
    assert!(database.view(b"p").is_none());
    assert!(database.view(b"q").is_none());
    assert!(database.query(b"SELECT x FROM p").is_err());
}

#[test]
fn a_schema_row_that_is_not_a_trigger_is_passed_over() {
    // Two trigger rows the reader must walk past: one whose statement
    // it cannot read, and one that calls itself a trigger and holds a
    // table.
    let file = schema_file(&[
        record([
            Ok("table"),
            Ok("d"),
            Ok("d"),
            Err(2),
            Ok("CREATE TABLE d(x)"),
        ]),
        record([
            Ok("trigger"),
            Ok("s"),
            Ok("d"),
            Err(0),
            Ok("CREATE TRIGGER s AFTER"),
        ]),
        record([
            Ok("trigger"),
            Ok("u"),
            Ok("d"),
            Err(0),
            Ok("CREATE TABLE u(x)"),
        ]),
        record([
            Ok("trigger"),
            Ok("w"),
            Ok("d"),
            Err(0),
            Ok("CREATE TRIGGER w AFTER INSERT ON d BEGIN SELECT 1; END"),
        ]),
    ]);
    let database = Database::open(&file).expect("a database");
    // The trigger the reader could read is the only one it holds.
    assert!(database.trigger(b"w").is_some());
    assert!(database.trigger(b"s").is_none());
    assert!(database.trigger(b"u").is_none());
}

#[test]
fn a_payload_longer_than_the_file_is_refused_before_room_is_made_for_it() {
    use crate::db::Error;
    use crate::error;
    // The length of a row is a number the file gives, and a file may
    // give one no machine holds. The reader refuses it rather than
    // asking for the room: a cell that is whole on its page but claims a
    // million million bytes on overflow pages.
    const PAGE: usize = 4096;
    const TOTAL: u64 = 1 << 40;
    // What the format leaves on the page for a payload that long.
    const LOCAL: usize = 1024;
    let mut file = super::SMALL[..100].to_vec();
    file.resize(PAGE, 0);
    file[28..32].copy_from_slice(&1u32.to_be_bytes());
    let (length, length_bytes) = super::varint(TOTAL);
    let mut cell = Vec::new();
    cell.extend_from_slice(&length[..length_bytes]);
    cell.push(1);
    cell.extend(core::iter::repeat_n(b'a', LOCAL));
    // The overflow page, which is never read because the length is
    // refused first.
    cell.extend_from_slice(&2u32.to_be_bytes());
    let at = PAGE - cell.len();
    file[at..].copy_from_slice(&cell);
    file[100] = 13;
    file[103..105].copy_from_slice(&1u16.to_be_bytes());
    file[105..107].copy_from_slice(&(at as u16).to_be_bytes());
    file[108..110].copy_from_slice(&(at as u16).to_be_bytes());
    assert_eq!(
        Database::open(&file).unwrap_err(),
        Error::Image(error::Error::Overrun)
    );
}

#[test]
fn a_text_of_comments_alone_holds_no_statement() {
    use crate::change::Writer;
    use crate::header::Encoding;
    // `sqlite3_prepare_v2` answers no statement for a text of comments,
    // whitespace and semicolons, so the connection that reads answers
    // no row and the one that writes leaves the file as it found it.
    assert!(crate::parse::blank(b"-- nothing"));
    assert!(crate::parse::blank(b"/* nothing */ ;; "));
    assert!(!crate::parse::blank(b"-- nothing\nSELECT 1"));

    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    let before = writer.written();
    assert_eq!(writer.run(b"/* nothing */").unwrap(), Vec::<Vec<_>>::new());
    assert_eq!(writer.written(), before);

    let database = Database::open(&before).unwrap();
    assert_eq!(
        database.query(b"-- nothing").unwrap(),
        crate::db::Answer::default()
    );
}

/// A database of two tables and three indexes, which the tests of the
/// plan and of the terms a level reads both ask.
fn joined_database() -> Vec<u8> {
    use crate::change::Writer;
    use crate::header::Encoding;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a, k)".as_slice(),
        b"CREATE TABLE u(b, c, d TEXT COLLATE NOCASE)",
        b"CREATE INDEX i ON u(b)",
        b"CREATE INDEX j ON u(c DESC)",
        b"CREATE INDEX n ON u(d COLLATE BINARY)",
        b"INSERT INTO t VALUES(1,'p'),(2,'q'),(NULL,'r')",
        b"INSERT INTO u VALUES(1,'x','X'),(1,'y','Y'),(2,'z','Z'),(NULL,'n','N')",
    ] {
        writer.run(sql).unwrap();
    }
    writer.written()
}

/// What `sql` answers over [`joined_database`], as one text per row.
fn joined_answer(sql: &[u8]) -> Vec<String> {
    let bytes = joined_database();
    let database = Database::open(&bytes).expect("a database");
    let answer = database.query(sql).unwrap_or_else(|error| {
        panic!("{} is refused with {error:?}", String::from_utf8_lossy(sql))
    });
    answer
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|value| alloc::format!("{value:?}"))
                .collect::<Vec<String>>()
                .join(",")
        })
        .collect()
}

/// A database of three tables a `USING` names one column of, which the
/// tests of what a `RIGHT JOIN` answers for that column ask.
fn kept_database() -> Vec<u8> {
    use crate::change::Writer;
    use crate::header::Encoding;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(a INT, c INT)".as_slice(),
        b"CREATE TABLE t4(a INT, f INT)",
        b"CREATE TABLE t5(a INT, g INT)",
        b"INSERT INTO t1 VALUES(11,31),(15,35)",
        b"INSERT INTO t4 VALUES(11,41),(15,45),(19,49),(NULL,40)",
        b"INSERT INTO t5 VALUES(15,55),(19,59),(NULL,50)",
    ] {
        writer.run(sql).unwrap();
    }
    writer.written()
}

#[test]
fn the_tables_inside_brackets_are_the_statement_the_join_is_against() {
    // `seltablist ::= stl_prefix LP seltablist RP` of `parse.y`: the
    // tables inside brackets stand for a statement that answers every
    // column of them, and the join written after the brackets is
    // against what that statement answers.
    use crate::change::Writer;
    use crate::header::Encoding;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a, k)".as_slice(),
        b"CREATE TABLE u(b, c, d)",
        b"CREATE TABLE v(b, e)",
        b"INSERT INTO t VALUES(1,'p'),(2,'q'),(NULL,'r')",
        b"INSERT INTO u VALUES(1,'x','X'),(1,'y','Y'),(2,'z','Z'),(NULL,'n','N')",
        b"INSERT INTO v VALUES(1,'E1'),(3,'E3')",
    ] {
        writer.run(sql).unwrap();
    }
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    let answered = |sql: &[u8]| {
        database
            .query(sql)
            .unwrap()
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|value| alloc::format!("{value:?}"))
                    .collect::<Vec<String>>()
                    .join(",")
            })
            .collect::<Vec<String>>()
    };
    assert_eq!(
        answered(b"SELECT count(*) FROM (t JOIN u ON t.a=u.b)"),
        ["Int(3)"]
    );
    assert_eq!(
        answered(b"SELECT a, c, e FROM t JOIN (u LEFT JOIN v USING(b)) ON t.a=b ORDER BY a, c"),
        [
            "Int(1),Text([120]),Text([69, 49])",
            "Int(1),Text([121]),Text([69, 49])",
            "Int(2),Text([122]),Null"
        ]
    );
    assert_eq!(
        answered(b"SELECT count(*) FROM t JOIN (u JOIN v USING(b)) ON t.a=b"),
        ["Int(2)"]
    );
}

#[test]
fn a_table_inside_brackets_answers_under_its_own_name() {
    // `SF_NestedFrom`: a column written with the name of a table
    // inside the brackets reaches that table, and the name the
    // brackets carry reaches the same columns.
    use crate::change::Writer;
    use crate::header::Encoding;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(a,b)".as_slice(),
        b"CREATE TABLE t2(a,c)",
        b"CREATE TABLE t3(a,d)",
        b"INSERT INTO t1 VALUES(1,'B1'),(2,'B2')",
        b"INSERT INTO t2 VALUES(1,'C1'),(3,'C3')",
        b"INSERT INTO t3 VALUES(1,'D1'),(3,'D3')",
    ] {
        writer.run(sql).unwrap();
    }
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    let answered = |sql: &[u8]| {
        database
            .query(sql)
            .unwrap()
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|value| alloc::format!("{value:?}"))
                    .collect::<Vec<String>>()
                    .join(",")
            })
            .collect::<Vec<String>>()
    };
    assert_eq!(
        answered(b"SELECT t2.c, t3.d FROM t1 JOIN (t2 JOIN t3 USING(a)) USING(a)"),
        ["Text([67, 49]),Text([68, 49])"]
    );
    assert_eq!(
        answered(b"SELECT t2.a, t3.a FROM t1 JOIN (t2 JOIN t3 USING(a)) USING(a)"),
        ["Int(1),Int(1)"]
    );
    // The name the brackets carry reaches them as well, and a table
    // inside them that carries a name of its own answers under that
    // name alone.
    assert_eq!(
        answered(b"SELECT x.c FROM t1 JOIN (t2 JOIN t3 USING(a)) AS x USING(a)"),
        ["Text([67, 49])"]
    );
    assert_eq!(
        answered(b"SELECT y.c FROM t1 JOIN (t2 AS y JOIN t3 USING(a)) USING(a)"),
        ["Text([67, 49])"]
    );
    assert_eq!(
        database.query(b"SELECT t2.c FROM t1 JOIN (t2 AS y JOIN t3 USING(a)) USING(a)"),
        Err(crate::db::Error::Eval(crate::eval::Error::NoColumn(
            b"t2.c".to_vec()
        )))
    );
    // A `*` answers the column a `USING` matched once, whatever
    // brackets stand around the tables, and the brackets carrying a
    // name of their own answer the same columns.
    assert_eq!(
        answered(b"SELECT * FROM (t2 JOIN t3 USING(a))"),
        [
            "Int(1),Text([67, 49]),Text([68, 49])",
            "Int(3),Text([67, 51]),Text([68, 51])"
        ]
    );
    assert_eq!(
        answered(b"SELECT * FROM t1 JOIN (t2 JOIN t3 USING(a)) AS x USING(a)"),
        ["Int(1),Text([66, 49]),Text([67, 49]),Text([68, 49])"]
    );
    assert_eq!(
        answered(b"SELECT * FROM ((t2 JOIN t3 USING(a)) JOIN t1 USING(a))"),
        ["Int(1),Text([67, 49]),Text([68, 49]),Text([66, 49])"]
    );
    assert_eq!(
        answered(b"SELECT t2.a, t3.a FROM ((t2 JOIN t3 USING(a)) JOIN t1 USING(a))"),
        ["Int(1),Int(1)"]
    );
    // A `GROUP BY` that counts to a column counts the ones a `*`
    // answers.
    assert_eq!(
        answered(b"SELECT a, count(*) FROM (t2 JOIN t3 USING(a)) GROUP BY 1 ORDER BY 1"),
        ["Int(1),Int(1)", "Int(3),Int(1)"]
    );
    assert_eq!(
        answered(b"SELECT a, count(*) FROM t2 JOIN t3 USING(a) GROUP BY 1 ORDER BY 1"),
        ["Int(1),Int(1)", "Int(3),Int(1)"]
    );
    // A statement written inside the `FROM` answers under its own name
    // and the tables it reads are not reachable through it.
    assert_eq!(
        database.query(b"SELECT t2.c FROM t1 JOIN (SELECT * FROM t2) USING(a)"),
        Err(crate::db::Error::Eval(crate::eval::Error::NoColumn(
            b"t2.c".to_vec()
        )))
    );
}

#[test]
fn the_column_a_using_names_is_the_one_the_join_filled_it_from() {
    // A `RIGHT JOIN` keeps a row of the side on the right with the
    // sides on its left empty, so the column the `USING` names stands
    // for the side that filled it, which is what the join after it
    // compares against.
    let bytes = kept_database();
    let database = Database::open(&bytes).unwrap();
    let answered = |sql: &[u8]| {
        database
            .query(sql)
            .unwrap()
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|value| alloc::format!("{value:?}"))
                    .collect::<Vec<String>>()
                    .join(",")
            })
            .collect::<Vec<String>>()
    };
    assert_eq!(
        answered(b"SELECT a, t1.a, t4.a, t5.a FROM t1 RIGHT JOIN t4 USING(a) JOIN t5 USING(a) ORDER BY a"),
        ["Int(15),Int(15),Int(15),Int(15)", "Int(19),Null,Int(19),Int(19)"]
    );
    // The row a `RIGHT JOIN` keeps whose own column is nothing matches
    // no row of the side after it, nothing included.
    assert_eq!(
        answered(b"SELECT count(*) FROM t1 RIGHT JOIN t4 USING(a) LEFT JOIN t5 USING(a)"),
        ["Int(4)"]
    );
    assert_eq!(
        answered(b"SELECT a, t1.a, t4.a FROM t1 RIGHT JOIN t4 USING(a) ORDER BY a"),
        [
            "Null,Null,Null",
            "Int(11),Int(11),Int(11)",
            "Int(15),Int(15),Int(15)",
            "Int(19),Null,Int(19)"
        ]
    );
}

#[test]
fn a_join_whose_condition_names_an_index_answers_the_rows_a_scan_answers() {
    // `sqlite3WhereLoopAddBtree` reads the side of a join by an index
    // its `ON` names the key of, and the `ON` is read again per row, so
    // the rows are the rows a scan of the whole table answers.
    assert_eq!(
        joined_answer(b"SELECT t.a, u.c FROM t JOIN u ON t.a=u.b ORDER BY t.a, u.c"),
        [
            "Int(1),Text([120])",
            "Int(1),Text([121])",
            "Int(2),Text([122])"
        ]
    );
    // The key the other way round, and a second term of the `AND`
    // spine the index does not carry.
    assert_eq!(
        joined_answer(b"SELECT t.a FROM t JOIN u ON u.b=t.a AND u.c='y'"),
        ["Int(1)"]
    );
    // A key of nothing reaches no entry, so the side is scanned and the
    // `LEFT` join answers the row with nothing beside it.
    assert_eq!(
        joined_answer(b"SELECT count(*) FROM t LEFT JOIN u ON t.a=u.b"),
        ["Int(4)"]
    );
    // A `RIGHT` join is walked twice and tells the rows it matched by
    // where they stand, so it is read out of its own tree.
    assert_eq!(
        joined_answer(b"SELECT count(*) FROM t RIGHT JOIN u ON t.a=u.b"),
        ["Int(4)"]
    );
    // An index held in another order, one held under another collation
    // than its column compares under, a term that is not an equality,
    // a term of two columns of the side itself, and a term over a
    // column no index of the side is over: none names a key.
    for sql in [
        b"SELECT count(*) FROM t JOIN u ON t.a=u.c".as_slice(),
        b"SELECT count(*) FROM t JOIN u ON t.k=u.d",
        b"SELECT count(*) FROM t JOIN u ON t.a<u.b",
        b"SELECT count(*) FROM t JOIN u ON u.b=u.c",
        b"SELECT count(*) FROM t JOIN u ON t.a=u.b+0",
        b"SELECT count(*) FROM t JOIN u ON u.b",
        b"SELECT count(*) FROM t JOIN u ON u.b=1",
        b"SELECT count(*) FROM t JOIN u ON t.rowid=u.b",
    ] {
        assert_eq!(
            joined_answer(sql).len(),
            1,
            "{}",
            String::from_utf8_lossy(sql)
        );
    }
}

#[test]
fn a_term_of_a_where_is_read_on_the_level_that_answers_it() {
    // `sqlite3WhereSplit` puts a term on the level where the sides it
    // names are read, and the term is read again where the statement is
    // answered, so the rows are the rows the whole `WHERE` leaves.
    assert_eq!(
        joined_answer(b"SELECT t.a, u.c FROM t, u WHERE t.a=u.b AND u.c='y'"),
        ["Int(1),Text([121])"]
    );
    // A term of every shape the walk answers: a `BETWEEN`, an `IN` over
    // a list, a `LIKE`, a `CAST`, a `COLLATE`, a `CASE` and a row.
    for sql in [
        b"SELECT t.a FROM t, u WHERE t.a BETWEEN 1 AND 1 AND u.b=1 AND u.c='x'".as_slice(),
        b"SELECT t.a FROM t, u WHERE u.b IN (1,3) AND t.a=1 AND u.c='x'",
        b"SELECT t.a FROM t, u WHERE u.c LIKE 'x%' AND t.a=1 AND u.b=1",
        b"SELECT t.a FROM t, u WHERE CAST(t.a AS TEXT)='1' AND u.b=1 AND u.c='x'",
        b"SELECT t.a FROM t, u WHERE u.d='x' COLLATE NOCASE AND t.a=1 AND u.b=1",
        b"SELECT t.a FROM t, u WHERE CASE WHEN t.a=1 THEN 1 ELSE 0 END AND u.b=1 AND u.c='x'",
        b"SELECT t.a FROM t, u WHERE -t.a=-1 AND u.b=1 AND u.c='x'",
    ] {
        assert_eq!(
            joined_answer(sql),
            ["Int(1)"],
            "{}",
            String::from_utf8_lossy(sql)
        );
    }
    // A term this walk does not answer stays where the statement is
    // answered: a function, a statement of its own, an `EXISTS`, an
    // `IN` over a statement, an `IN` over a table, and a name no side
    // answers.
    for (sql, rows) in [
        (b"SELECT count(*) FROM t, u WHERE abs(t.a)=1".as_slice(), 1),
        (b"SELECT count(*) FROM t, u WHERE (SELECT 1)=1", 1),
        (
            b"SELECT count(*) FROM t, u WHERE EXISTS(SELECT 1 FROM u)",
            1,
        ),
        (
            b"SELECT count(*) FROM t, u WHERE t.a IN (SELECT b FROM u)",
            1,
        ),
        (b"SELECT count(*) FROM t, u WHERE true", 1),
    ] {
        assert_eq!(
            joined_answer(sql).len(),
            rows,
            "{}",
            String::from_utf8_lossy(sql)
        );
    }
    // A name two sides answer is ambiguous, so no level is given it.
    assert!(
        Database::open(&joined_database())
            .unwrap()
            .query(b"SELECT count(*) FROM u, u AS v WHERE b=1")
            .is_err()
    );
    // A term above a `RIGHT` join is read on the level of that join,
    // because a row is marked matched where the levels under it are
    // read.
    assert_eq!(
        joined_answer(b"SELECT count(*) FROM t RIGHT JOIN u ON t.a=u.b WHERE t.a IS NULL"),
        ["Int(1)"]
    );
}

#[test]
fn the_order_by_of_a_compound_compares_under_what_the_term_was_written_with() {
    use crate::change::Writer;
    use crate::header::Encoding;
    // `multiSelectOrderBy` reads the collation off the term and not off
    // the column it counts to, so a `COLLATE` on the term is what the
    // whole compound is ordered under.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(b TEXT COLLATE NOCASE)".as_slice(),
        b"INSERT INTO t1 VALUES('a'),('B')",
        b"CREATE TABLE t2(y TEXT COLLATE NOCASE)",
        b"INSERT INTO t2 VALUES('C'),('d')",
    ] {
        writer.run(sql).unwrap();
    }
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    let ordered = |sql: &[u8]| -> Vec<Vec<u8>> {
        database
            .query(sql)
            .unwrap()
            .rows
            .iter()
            .filter_map(|row| match row.first() {
                Some(Value::Text(text)) => Some(text.clone()),
                _ => None,
            })
            .collect()
    };
    let text = |held: &[&str]| -> Vec<Vec<u8>> {
        held.iter().map(|one| one.as_bytes().to_vec()).collect()
    };
    let core = b"SELECT b FROM t1 UNION ALL SELECT y FROM t2 ORDER BY ".as_slice();
    let ask = |rest: &str| -> Vec<u8> {
        let mut sql = core.to_vec();
        sql.extend_from_slice(rest.as_bytes());
        sql
    };
    assert_eq!(
        ordered(&ask("b COLLATE BINARY")),
        text(&["B", "C", "a", "d"])
    );
    assert_eq!(ordered(&ask("b")), text(&["a", "B", "C", "d"]));
    assert_eq!(ordered(&ask("1 DESC")), text(&["d", "C", "B", "a"]));
}

#[test]
fn the_order_by_of_a_recursive_term_says_which_row_is_taken_next() {
    use crate::change::Writer;
    use crate::header::Encoding;
    // `generateWithRecursiveQuery` keeps the rows of the term in a
    // queue: the `ORDER BY` says which is taken next and the `LIMIT`
    // how many are taken at all.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE link(aa,bb)").unwrap();
    writer
        .run(b"INSERT INTO link VALUES(1,3),(3,5),(5,7),(7,9),(9,11)")
        .unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    let held = |sql: &[u8]| -> Vec<i64> {
        database
            .query(sql)
            .unwrap()
            .rows
            .iter()
            .filter_map(|row| match row.first() {
                Some(Value::Int(number)) => Some(*number),
                _ => None,
            })
            .collect()
    };
    let closure = b"WITH RECURSIVE closure(x) AS (SELECT 1 AS x UNION \
                    SELECT bb FROM link JOIN closure ON aa=x "
        .as_slice();
    let ask = |rest: &str| -> Vec<u8> {
        let mut sql = closure.to_vec();
        sql.extend_from_slice(rest.as_bytes());
        sql.extend_from_slice(b") SELECT * FROM closure");
        sql
    };
    assert_eq!(held(&ask("")), [1, 3, 5, 7, 9, 11]);
    assert_eq!(held(&ask("ORDER BY x LIMIT 4")), [1, 3, 5, 7]);
    assert_eq!(held(&ask("ORDER BY x DESC LIMIT 3")), [1, 3, 5]);
    assert_eq!(held(&ask("ORDER BY x LIMIT 3 OFFSET 2")), [5, 7, 9]);
    assert_eq!(held(&ask("LIMIT -1")), [1, 3, 5, 7, 9, 11]);

    // A row that reaches two takes the smaller of them first, so the
    // queue holds more than one row waiting and the walk reads it for
    // the smallest.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE link(aa,bb)").unwrap();
    writer
        .run(b"INSERT INTO link VALUES(1,5),(1,3),(3,9),(5,7)")
        .unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    let rows = database
        .query(
            b"WITH RECURSIVE closure(x) AS (SELECT 1 AS x UNION               SELECT bb FROM link JOIN closure ON aa=x ORDER BY x LIMIT 3)               SELECT * FROM closure",
        )
        .unwrap()
        .rows;
    assert_eq!(rows, [[Value::Int(1)], [Value::Int(3)], [Value::Int(5)]]);
}

#[test]
fn the_schema_is_a_table_that_is_read_and_not_written() {
    use crate::change::Writer;
    use crate::header::Encoding;
    // `sqlite3InitOne` builds the schema's own table in memory out of
    // the five columns every row of it is written with, and
    // `sqlite3SchemaMayNotBeModified` keeps a statement from writing
    // it.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"CREATE INDEX i ON t(a)").unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    let rows = database
        .query(b"SELECT type, name, tbl_name, rootpage FROM sqlite_master ORDER BY rootpage")
        .unwrap();
    assert_eq!(
        rows.names,
        [
            b"type".to_vec(),
            b"name".to_vec(),
            b"tbl_name".to_vec(),
            b"rootpage".to_vec()
        ]
    );
    assert_eq!(
        rows.rows,
        [
            [
                Value::Text(b"table".to_vec()),
                Value::Text(b"t".to_vec()),
                Value::Text(b"t".to_vec()),
                Value::Int(2)
            ],
            [
                Value::Text(b"index".to_vec()),
                Value::Text(b"i".to_vec()),
                Value::Text(b"t".to_vec()),
                Value::Int(3)
            ]
        ]
    );
    // The three names it answers to, and the statement it holds.
    for name in [
        b"sqlite_master".as_slice(),
        b"sqlite_schema",
        b"sqlite_temp_master",
        b"sqlite_temp_schema",
    ] {
        let mut sql = b"SELECT count(*) FROM ".to_vec();
        sql.extend_from_slice(name);
        assert_eq!(database.query(&sql).unwrap().rows, [[Value::Int(2)]]);
    }
    assert_eq!(
        database
            .query(b"SELECT sql FROM sqlite_master WHERE name='i'")
            .unwrap()
            .rows,
        [[Value::Text(b"CREATE INDEX i ON t(a)".to_vec())]]
    );
    // It is not one of the tables the file holds, and no statement
    // writes it.
    assert!(
        !database
            .tables()
            .any(|table| table.name == b"sqlite_master")
    );
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    for sql in [
        b"INSERT INTO sqlite_master VALUES('x','y','z',1,'w')".as_slice(),
        b"UPDATE sqlite_schema SET name='q'",
        b"DELETE FROM sqlite_master",
        b"DROP TABLE sqlite_master",
    ] {
        assert_eq!(
            writer.run(sql).err(),
            Some(crate::db::Error::Unsupported),
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
}

#[test]
fn a_row_of_the_schema_holds_the_statement_from_the_name_on() {
    use crate::change::Writer;
    use crate::header::Encoding;
    // `sqlite3EndTable` and `sqlite3CreateIndex` write the words
    // `CREATE` and the kind and then the text from the name to the end,
    // so a row holds neither the `TEMP` nor the `IF NOT EXISTS` that
    // stand before the name.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE IF NOT EXISTS a(x)".as_slice(),
        b"CREATE UNIQUE INDEX IF NOT EXISTS i ON a(x)",
        b"CREATE VIEW IF NOT EXISTS v AS SELECT 1",
        b"CREATE TEMP TABLE abc(a, b, c)",
        b"CREATE INDEX j ON abc(a)",
    ] {
        writer.run(sql).unwrap();
    }
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    let held: Vec<Vec<u8>> = database
        .query(b"SELECT sql FROM sqlite_master")
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| match row.first() {
            Some(Value::Text(text)) => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        held,
        [
            b"CREATE TABLE a(x)".to_vec(),
            b"CREATE UNIQUE INDEX i ON a(x)".to_vec(),
            b"CREATE VIEW v AS SELECT 1".to_vec(),
            b"CREATE TABLE abc(a, b, c)".to_vec(),
            b"CREATE INDEX j ON abc(a)".to_vec(),
        ]
    );
}

#[test]
fn a_key_that_counts_up_is_counted_in_the_table_of_sequences() {
    use crate::change::Writer;
    use crate::header::Encoding;
    // `sqlite3StartTable` makes `sqlite_sequence` with the first table
    // that counts its keys up, and `autoIncrementEnd` writes the
    // largest key the table ever held into it.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(x INTEGER PRIMARY KEY AUTOINCREMENT, y)")
        .unwrap();
    let held = |writer: &Writer| -> Vec<Vec<Value>> {
        let written = writer.written();
        let database = Database::open(&written).unwrap();
        database
            .query(b"SELECT name, seq FROM sqlite_sequence")
            .unwrap()
            .rows
    };
    // The table is there and holds no row until one is written.
    assert_eq!(held(&writer), Vec::<Vec<Value>>::new());
    writer
        .run(b"INSERT INTO t VALUES(NULL,'a'),(NULL,'b')")
        .unwrap();
    assert_eq!(held(&writer), [[Value::Text(b"t".to_vec()), Value::Int(2)]]);
    // A key given back is not given out again.
    writer.run(b"DELETE FROM t").unwrap();
    assert_eq!(held(&writer), [[Value::Text(b"t".to_vec()), Value::Int(2)]]);
    writer.run(b"INSERT INTO t VALUES(NULL,'c')").unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"SELECT x FROM t").unwrap().rows,
        [[Value::Int(3)]]
    );
    assert_eq!(held(&writer), [[Value::Text(b"t".to_vec()), Value::Int(3)]]);
    // A second such table gains a row of its own, and a key the
    // statement names that is smaller than the count leaves it.
    writer
        .run(b"CREATE TABLE u(x INTEGER PRIMARY KEY AUTOINCREMENT)")
        .unwrap();
    writer.run(b"INSERT INTO u VALUES(9)").unwrap();
    writer.run(b"INSERT INTO u VALUES(4)").unwrap();
    assert_eq!(
        held(&writer),
        [
            [Value::Text(b"t".to_vec()), Value::Int(3)],
            [Value::Text(b"u".to_vec()), Value::Int(9)]
        ]
    );
    // A table taken away takes its count with it, and a table that
    // counts nothing takes none.
    writer.run(b"CREATE TABLE plain(a)").unwrap();
    writer.run(b"DROP TABLE plain").unwrap();
    writer.run(b"DROP TABLE t").unwrap();
    assert_eq!(held(&writer), [[Value::Text(b"u".to_vec()), Value::Int(9)]]);

    // A statement that writes no row leaves the count as it stands.
    writer
        .run(b"INSERT INTO u SELECT x FROM u WHERE 0")
        .unwrap();
    assert_eq!(held(&writer), [[Value::Text(b"u".to_vec()), Value::Int(9)]]);

    // A database with no table that counts its keys up has no table of
    // counts, so a `DROP` there takes no row out of one.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"DROP TABLE t").unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    assert_eq!(database.tables().count(), 0);
}

#[test]
fn a_column_of_a_compound_converts_what_every_core_of_it_leaves_it_converting() {
    use crate::change::Writer;
    use crate::header::Encoding;
    // `sqlite3SubqueryColumnTypes`: the affinity is the first core's,
    // or the first core after it that has one, and a core beyond that
    // one which answers a class the affinity would convert takes the
    // affinity away.
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE whole(id INT, n)".as_slice(),
        b"CREATE TABLE more(id INT, n)",
        b"CREATE TABLE words(id TEXT, n)",
        b"CREATE TABLE plain(id, n)",
        b"INSERT INTO whole VALUES(1,'a')",
        b"INSERT INTO more VALUES(2,'b')",
        b"INSERT INTO words VALUES('4','e')",
        b"INSERT INTO plain VALUES(7,'g')",
        // A compound of two cores that count in numbers keeps the
        // type; one of a number and a text keeps none; one whose first
        // core converts nothing keeps that, because a column with no
        // type still has the affinity `BLOB`.
        b"CREATE TABLE agree AS SELECT * FROM whole UNION SELECT * FROM more",
        b"CREATE TABLE differ AS SELECT * FROM whole UNION SELECT * FROM words",
        b"CREATE TABLE later AS SELECT * FROM plain UNION SELECT * FROM whole",
        b"CREATE TABLE textual AS SELECT * FROM words UNION SELECT * FROM words",
        b"CREATE TABLE spoilt AS SELECT id+0, n FROM words UNION SELECT id, n FROM words",
        b"CREATE TABLE said AS SELECT 'x', 1 UNION SELECT id, n FROM words",
        // The shapes of expression a class is read off: a `+`, a null,
        // a blob and a `CASE`.
        b"CREATE TABLE shapes AS SELECT +id, NULL, x'00', CASE WHEN n THEN 'a' ELSE 2 END           FROM words UNION SELECT id, n, n, n FROM words",
        b"CREATE TABLE elseless AS SELECT CASE WHEN n THEN 'a' END FROM words UNION SELECT n FROM words",
    ] {
        writer.run(sql).unwrap();
    }
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    let statements: Vec<Vec<u8>> = database
        .query(b"SELECT sql FROM sqlite_master WHERE type='table'")
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| match row.first() {
            Some(Value::Text(text)) => Some(text.clone()),
            _ => None,
        })
        .skip(4)
        .collect();
    assert_eq!(
        statements,
        [
            b"CREATE TABLE agree(id INT,n)".to_vec(),
            b"CREATE TABLE differ(id,n)".to_vec(),
            b"CREATE TABLE later(id,n)".to_vec(),
            b"CREATE TABLE textual(id TEXT,n)".to_vec(),
            b"CREATE TABLE spoilt(\"id+0\",n)".to_vec(),
            b"CREATE TABLE said(\"'x'\" TEXT,\"1\")".to_vec(),
            b"CREATE TABLE shapes(\n  \"+id\" TEXT,\n  \"NULL\",\n  \"x'00'\",\n  \"CASE WHEN n THEN 'a' ELSE 2 END\"\n)".to_vec(),
            b"CREATE TABLE elseless(\"CASE WHEN n THEN 'a' END\")".to_vec(),
        ]
    );
    // A column that converts nothing compares a number against text as
    // the two classes sort and not as numbers, which is what the join
    // of `affinity3.test` reads.
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE mi(id INT, name)".as_slice(),
        b"CREATE TABLE mt(id TEXT, name)",
        b"CREATE TABLE data(id TEXT, name)",
        b"INSERT INTO mi VALUES(1,'a')",
        b"INSERT INTO mt VALUES('4','e')",
        b"INSERT INTO data VALUES(1,'abc'),('4','xyz')",
        b"CREATE VIEW both AS SELECT * FROM mi UNION SELECT * FROM mt",
        b"CREATE VIEW one AS SELECT * FROM mi",
    ] {
        writer.run(sql).unwrap();
    }
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    let rows = database
        .query(b"SELECT data.name FROM data JOIN both USING(id)")
        .unwrap()
        .rows;
    assert_eq!(rows, [[Value::Text(b"xyz".to_vec())]]);
    // A view over one table keeps that table's affinity, so the same
    // join there does convert.
    let database = Database::open(&written).unwrap();
    let rows = database
        .query(b"SELECT data.name FROM data JOIN one USING(id)")
        .unwrap()
        .rows;
    assert_eq!(rows, [[Value::Text(b"abc".to_vec())]]);
}
