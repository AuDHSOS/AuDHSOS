// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::schema`, against what the pragmas answer.
//!
//! The corpus and the golden are the ones `tests/definition.rs` reads:
//! the same statements, and what SQLite made of each. Where it made a
//! table, the golden holds the rows `pragma_table_list` and
//! `pragma_table_xinfo` then answered, and this test builds the same
//! rows out of the parsed statement.

#![allow(clippy::arithmetic_side_effects)]

use crate::ast::Definition;
use crate::parse::definition;
use crate::schema::{Generated, table};
use crate::value::Affinity;

/// The statements.
fn corpus() -> Vec<&'static str> {
    include_str!("fixtures/schema.corpus").lines().collect()
}

/// What SQLite made of each.
fn golden() -> Vec<&'static str> {
    include_str!("fixtures/schema.golden").lines().collect()
}

/// The rows the pragmas would answer for the table `sql` describes.
fn rows(sql: &str) -> Option<Vec<String>> {
    let (arena, parsed) = definition(sql.as_bytes()).ok()?;
    let Definition::Table(written) = parsed else {
        return None;
    };
    let made = table(&arena, &written, sql.as_bytes()).ok()?;
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let mut out = alloc::vec![format!(
        "T|{}|{}|{}|{}",
        text(&made.name),
        made.columns.len(),
        u8::from(made.without_rowid),
        u8::from(made.strict)
    )];
    for (at, column) in made.columns.iter().enumerate() {
        out.push(format!(
            "C|{at}|{}|{}|{}|{}|{}|{}",
            text(&column.name),
            text(&column.declared),
            u8::from(column.not_null),
            column.default.as_deref().map_or("~".to_owned(), text),
            column.key,
            match column.generated {
                Generated::Never => 0,
                Generated::Virtual => 2,
                Generated::Stored => 3,
            }
        ));
    }
    Some(out)
}

#[test]
fn every_table_is_built_the_way_the_pragmas_describe_it() {
    // What is left is what one statement cannot decide: an index over a
    // table that is not there, and a table whose columns come from a
    // statement, which a file never holds because SQLite writes the
    // columns it worked out instead.
    let mut unbuilt = 0;
    let mut overbuilt = 0;
    let cases = corpus();
    let answers = golden();
    assert_eq!(cases.len(), answers.len());
    for (sql, answer) in cases.iter().zip(answers) {
        let mine = rows(sql);
        if !answer.starts_with("table\t") {
            overbuilt += usize::from(mine.is_some());
            continue;
        }
        let Some(mine) = mine else {
            unbuilt += 1;
            continue;
        };
        let theirs: Vec<&str> = answer.split('\t').skip(1).collect();
        assert_eq!(mine, theirs, "the rows of {sql}");
    }
    assert_eq!(unbuilt, 2, "the tables one statement cannot describe");
    // What is left is a foreign key whose columns carry a collation or
    // an order, which SQLite refuses in a statement and allows while it
    // is reading a schema back off a file — which is what this is.
    assert_eq!(overbuilt, 2, "what reading a schema does not refuse");
}

#[test]
fn a_type_of_three_letters_or_more_that_is_one_of_six_is_stored_as_that_one() {
    for (sql, declared, affinity) in [
        ("CREATE TABLE t(x int)", &b"INT"[..], Affinity::Integer),
        ("CREATE TABLE t(x Integer)", b"INTEGER", Affinity::Integer),
        ("CREATE TABLE t(x 'text')", b"TEXT", Affinity::Text),
        ("CREATE TABLE t(x \"blob\")", b"BLOB", Affinity::Blob),
        ("CREATE TABLE t(x any)", b"ANY", Affinity::Numeric),
        ("CREATE TABLE t(x real)", b"REAL", Affinity::Real),
        // Anything else is kept as it was written.
        (
            "CREATE TABLE t(x VARCHAR(9))",
            b"VARCHAR(9)",
            Affinity::Text,
        ),
        (
            "CREATE TABLE t(x UNSIGNED BIG INT)",
            b"UNSIGNED BIG INT",
            Affinity::Integer,
        ),
        ("CREATE TABLE t(x)", b"", Affinity::Blob),
        // A type of two letters is too short to be one of the six.
        ("CREATE TABLE t(x xy)", b"xy", Affinity::Numeric),
        // The words the parser could not tell from a type are trimmed
        // back off here.
        ("CREATE TABLE t(x GENERATED ALWAYS)", b"", Affinity::Blob),
        (
            "CREATE TABLE t(x my generated always)",
            b"my",
            Affinity::Numeric,
        ),
        ("CREATE TABLE t(x always)", b"always", Affinity::Numeric),
    ] {
        let (arena, parsed) = definition(sql.as_bytes()).expect("a definition");
        let Definition::Table(written) = parsed else {
            panic!("not a table");
        };
        let made = table(&arena, &written, sql.as_bytes()).expect("a table");
        let column = made.columns.first().expect("a column");
        assert_eq!(column.declared, declared, "the type of {sql}");
        assert_eq!(column.affinity, affinity, "the affinity of {sql}");
    }
}

/// The table `sql` describes, or the reason it is not one.
fn build(sql: &str) -> Result<crate::schema::Table, crate::schema::Error> {
    let (arena, parsed) = definition(sql.as_bytes()).expect("a definition");
    let Definition::Table(written) = parsed else {
        panic!("not a table");
    };
    table(&arena, &written, sql.as_bytes())
}

#[test]
fn the_rowid_is_another_name_for_a_key_only_where_it_is_spelled_integer() {
    assert_eq!(
        build("CREATE TABLE t(x INTEGER PRIMARY KEY)")
            .unwrap()
            .rowid_alias,
        Some(0)
    );
    assert_eq!(
        build("CREATE TABLE t(a, x INTEGER, PRIMARY KEY(x))")
            .unwrap()
            .rowid_alias,
        Some(1)
    );
    // `INT` is not `INTEGER`, backwards is not forwards, and two columns
    // are not one.
    assert_eq!(
        build("CREATE TABLE t(x INT PRIMARY KEY)")
            .unwrap()
            .rowid_alias,
        None
    );
    assert_eq!(
        build("CREATE TABLE t(x INTEGER PRIMARY KEY DESC)")
            .unwrap()
            .rowid_alias,
        None
    );
    assert_eq!(
        build("CREATE TABLE t(x INTEGER, y, PRIMARY KEY(x, y))")
            .unwrap()
            .rowid_alias,
        None
    );
    assert_eq!(
        build("CREATE TABLE t(x PRIMARY KEY)").unwrap().rowid_alias,
        None
    );
    // Ascending is what nothing written means.
    assert_eq!(
        build("CREATE TABLE t(x INTEGER PRIMARY KEY ASC)")
            .unwrap()
            .rowid_alias,
        Some(0)
    );
}

#[test]
fn what_a_statement_says_that_makes_it_no_table() {
    use crate::schema::Error;
    assert_eq!(build("CREATE TABLE t(x, x)"), Err(Error::DuplicateColumn));
    assert_eq!(build("CREATE TABLE t(x, X)"), Err(Error::DuplicateColumn));
    assert_eq!(
        build("CREATE TABLE t(x COLLATE nosuch)"),
        Err(Error::NoCollation)
    );
    assert_eq!(
        build("CREATE TABLE t(x, y AS (1) NONSENSE)"),
        Err(Error::GeneratedWord)
    );
    assert_eq!(build("CREATE TABLE t(x AS (1))"), Err(Error::AllGenerated));
    assert_eq!(
        build("CREATE TABLE t(x, y AS (1), PRIMARY KEY(y))"),
        Err(Error::GeneratedKey)
    );
    assert_eq!(
        build("CREATE TABLE t(x PRIMARY KEY, y PRIMARY KEY)"),
        Err(Error::ManyKeys)
    );
    assert_eq!(
        build("CREATE TABLE t(x PRIMARY KEY, PRIMARY KEY(x))"),
        Err(Error::ManyKeys)
    );
    assert_eq!(
        build("CREATE TABLE t(x PRIMARY KEY AUTOINCREMENT)"),
        Err(Error::Autoincrement)
    );
    assert_eq!(
        build("CREATE TABLE t(x INTEGER PRIMARY KEY AUTOINCREMENT) WITHOUT ROWID"),
        Err(Error::AutoincrementWithoutRowid)
    );
    assert_eq!(
        build("CREATE TABLE t(x) WITHOUT ROWID"),
        Err(Error::MissingKey)
    );
    assert_eq!(build("CREATE TABLE t(x) STRICT"), Err(Error::MissingType));
    assert_eq!(
        build("CREATE TABLE t(x VARCHAR(9)) STRICT"),
        Err(Error::UnknownType)
    );
    assert_eq!(
        build("CREATE TABLE t(x, PRIMARY KEY(nosuch))"),
        Err(Error::NoSuchColumn)
    );
    assert_eq!(build("CREATE TABLE t AS SELECT 1"), Err(Error::FromSelect));
}

#[test]
fn a_key_that_is_not_the_rowid_refuses_nothing_where_the_table_says_so() {
    // `WITHOUT ROWID` and `STRICT` both make the key columns `NOT NULL`,
    // which nothing in the statement said.
    let made = build("CREATE TABLE t(a, b, PRIMARY KEY(a, b)) WITHOUT ROWID").unwrap();
    assert!(made.columns[0].not_null && made.columns[1].not_null);
    assert_eq!((made.columns[0].key, made.columns[1].key), (1, 2));
    let strict = build("CREATE TABLE t(a INT, b INT, PRIMARY KEY(a, b)) STRICT").unwrap();
    assert!(strict.columns[0].not_null && strict.columns[1].not_null);
    // The rowid itself is left alone, because it is never nothing.
    let rowid = build("CREATE TABLE t(a INTEGER PRIMARY KEY, b INT) STRICT").unwrap();
    assert!(!rowid.columns[0].not_null);
    // `ANY` holds what it is given, so a strict table converts nothing.
    let strict = build("CREATE TABLE t(a ANY) STRICT").unwrap();
    assert_eq!(strict.columns[0].affinity, Affinity::Blob);
    let loose = build("CREATE TABLE t(a ANY)").unwrap();
    assert_eq!(loose.columns[0].affinity, Affinity::Numeric);
}

#[test]
fn a_name_and_a_collation_lose_their_quotes() {
    let made = build("CREATE TABLE \"the table\"(\"a b\" COLLATE \"nocase\")").unwrap();
    assert_eq!(made.name, b"the table");
    assert_eq!(made.columns[0].name, b"a b");
    assert_eq!(made.columns[0].collation, crate::value::Collation::NoCase);
    let bracketed = build("CREATE TABLE [t](`a` COLLATE [rtrim])").unwrap();
    assert_eq!(bracketed.name, b"t");
    assert_eq!(bracketed.columns[0].name, b"a");
    assert_eq!(
        bracketed.columns[0].collation,
        crate::value::Collation::Rtrim
    );
    // A doubled quote inside one is the quote itself.
    let doubled = build("CREATE TABLE t(\"a\"\"b\")").unwrap();
    assert_eq!(doubled.columns[0].name, b"a\"b");
}
