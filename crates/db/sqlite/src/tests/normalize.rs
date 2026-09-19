// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One statement written again with every literal as a `?`.

use crate::normalize::{normalized, normalized_sql};

/// The names the statements of the upper-case form may read, which are
/// the tables of a schema and their columns.
fn names() -> alloc::vec::Vec<alloc::vec::Vec<u8>> {
    ["t1", "t2", "a", "b", "c", "x", "y", "col f", "a\"b"]
        .iter()
        .map(|name| name.as_bytes().to_vec())
        .collect()
}

/// What `sqlite3_normalize` writes, over the statements
/// `research/sqlite/test/normalize.test:19` holds it to.
#[test]
fn what_a_statement_written_again_in_lower_case_holds() {
    for (sql, wanted) in [
        (
            "SELECT * FROM t1 WHERE a IN (1) AND b=51.42",
            "select*from t1 where a in(?,?,?)and b=?;",
        ),
        (
            "SELECT a, b+15, c FROM t1 WHERE d NOT IN (SELECT x FROM t2);",
            "select a,b+?,c from t1 where d not in(select x from t2);",
        ),
        (
            " SELECT NULL, b FROM t1 -- comment text\n     WHERE d IN (WITH t(a) AS (VALUES(5)) /* CTE */\n                 SELECT a FROM t)\n        OR e='hello';\n  ",
            "select?,b from t1 where d in(with t(a)as(values(?))select a from t)or e=?;",
        ),
        (
            "/* Query containing parameters */\n   SELECT x,$::abc(15),y,@abc,z,?99,w FROM t1 /* Trailing comment */",
            "select x,?,y,?,z,?,w from t1;",
        ),
        (
            "/* Long list on the RHS of IN */\n   SELECT 15 IN (1,2,3,(SELECT * FROM t1),'xyz',x'abcd',22*(x+5),null);",
            "select?in(?,?,?);",
        ),
        (
            "SELECT a,NULL,b FROM t1 WHERE c IS NOT NULL or D is null or e=5",
            "select a,?,b from t1 where c is not null or d is null or e=?;",
        ),
        (
            "/* IN list exactly 5 bytes long */\n   SELECT * FROM t1 WHERE x IN (1,2,3);",
            "select*from t1 where x in(?,?,?);",
        ),
        ("    ", ""),
        // A name that ends in the letters `in` is no `IN` at all, and a
        // right side with no closing bracket is left as it stands.
        ("SELECT * FROM tin(1)", "select*from tin(?);"),
        ("SELECT x IN (1", "select x in(?;"),
        // `NULL` after a name is a value, which `notnull` is not `NOT`
        // and `ais` is not `IS`.
        ("SELECT notnull NULL", "select notnull?;"),
        ("SELECT ais NULL", "select ais?;"),
        // A statement of nothing but `NULL` has no word before it.
        ("NULL", "?;"),
        // A space stands between two words that would run together,
        // whatever byte of a name ends the first.
        ("SELECT a_ b", "select a_ b;"),
        ("SELECT a$x b", "select a$x b;"),
        ("SELECT \u{e4}a b", "select \u{e4}a b;"),
        ("SELECT a$ b", "select a$ b;"),
        ("SELECT a\u{e4} b", "select a\u{e4} b;"),
        // A word that begins with the letters of a statement is no
        // statement, so the right side of the `IN` is written again.
        ("SELECT x IN (selectx)", "select x in(?,?,?);"),
        ("SELECT x IN (withx)", "select x in(?,?,?);"),
    ] {
        assert_eq!(
            normalized(sql.as_bytes())
                .map(|held| alloc::string::String::from_utf8_lossy(&held).into_owned()),
            Some(wanted.to_owned()),
            "{sql}"
        );
    }
    // A byte no rule accepts answers nothing, which an unfinished blob
    // is.
    assert_eq!(normalized(b"SELECT x'abc'; -- illegal token"), None);
}

/// What `sqlite3_normalized_sql` writes, over the statements
/// `research/sqlite/test/normalize.test:200` holds it to.
#[test]
fn what_a_statement_written_again_in_upper_case_holds() {
    for (sql, wanted) in [
        (
            "SELECT a, b FROM t1 WHERE b = ? ORDER BY a;",
            "SELECT a,b FROM t1 WHERE b=?ORDER BY a;",
        ),
        (
            "SELECT a, b FROM t1 WHERE b = 'a' ORDER BY a;",
            "SELECT a,b FROM t1 WHERE b=?ORDER BY a;",
        ),
        (
            "DELETE FROM t1 WHERE x IN (1, 2, 'three');",
            "DELETE FROM t1 WHERE x IN(?,?,?);",
        ),
        (
            "SELECT * FROM t1 WHERE x IN (SELECT y FROM t2);",
            "SELECT*FROM t1 WHERE x IN(SELECT y FROM t2);",
        ),
        // `NULL` after `IS` or `NOT` is a word of the language and not a
        // value, and one after anything else is a value.
        (
            "SELECT a FROM t1 WHERE b IS NULL AND c IS NOT NULL AND d=NULL",
            "SELECT a FROM t1 WHERE b IS NULL AND c IS NOT NULL AND d=?;",
        ),
        // An identifier is written in lower case with its quotes taken
        // off, and a bracket that opens no right side of an `IN` is
        // left alone.
        (
            "SELECT \"A\", [B], `C` FROM (SELECT 1)",
            "SELECT a,b,c FROM(SELECT?);",
        ),
        // A name in double quotes that the schema does not carry is a
        // text, and one it carries and no bare identifier holds is
        // written in double quotes again.
        (
            "SELECT \"col f\", \"sl1\" FROM t1",
            "SELECT\"col f\",?FROM t1;",
        ),
        // An `ATTACH` names a file, so a word in double quotes there is
        // a name whatever the schema carries.
        ("ATTACH \"some.db\" AS held", "ATTACH\"some.db\"AS held;"),
        // The right side of an `IN` holds the values it was written
        // with, each of them a `?`.
        (
            "UPDATE t1 SET x = \"sl1\" WHERE x IN (1, \"sl2\", 'i')",
            "UPDATE t1 SET x=?WHERE x IN(?,?,?);",
        ),
        // A statement of one word carries the semicolon it has none of,
        // and a text of comments alone carries one as well.
        ("VACUUM", "VACUUM;"),
        ("/* a comment */", ";"),
        // A `DETACH` names a file as an `ATTACH` does, and a name no
        // bare identifier holds is written in double quotes.
        ("DETACH \"1.db\"", "DETACH\"1.db\";"),
        // A name that carries a double quote carries it twice.
        ("SELECT \"a\"\"b\" FROM t1", "SELECT\"a\"\"b\"FROM t1;"),
        // A bracket inside the right side of an `IN` closes no list.
        (
            "SELECT * FROM t1 WHERE x IN (1,(2),3)",
            "SELECT*FROM t1 WHERE x IN(?,?,?);",
        ),
        // A byte no rule accepts is written as it stands, which
        // `sqlite3Normalize` has no arm of its own for.
        ("SELECT x'abc'", "SELECT X'ABC';"),
    ] {
        assert_eq!(
            alloc::string::String::from_utf8_lossy(&normalized_sql(sql.as_bytes(), &names()))
                .into_owned(),
            wanted.to_owned(),
            "{sql}"
        );
    }
}
