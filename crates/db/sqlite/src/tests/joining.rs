// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The words a join is written with, what an `ON` may name, and what
//! a trigger may not carry.

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// A database of three tables, one column each.
fn three() -> alloc::vec::Vec<u8> {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE aa(a)".as_slice(),
        b"CREATE TABLE bb(b)",
        b"CREATE TABLE cc(c)",
        b"INSERT INTO aa VALUES('one')",
        b"INSERT INTO bb VALUES('one')",
        b"INSERT INTO cc VALUES('one')",
    ] {
        writer.run(sql).unwrap();
    }
    writer.written()
}

/// A `NATURAL` join with a condition on it, a `USING` naming a column one
/// side does not hold, and a `WITH` term writing another number of column
/// names than its statement answers.
#[test]
fn what_a_join_the_two_sides_cannot_be_made_into_is_refused_with() {
    let image = three();
    let database = Database::open(&image).expect("a database");
    for (sql, message) in [
        (
            "SELECT * FROM aa NATURAL JOIN bb ON 1",
            "a NATURAL join may not have an ON or USING clause",
        ),
        (
            "SELECT * FROM aa NATURAL JOIN bb USING(a)",
            "a NATURAL join may not have an ON or USING clause",
        ),
        (
            "SELECT * FROM aa JOIN bb USING(nosuch)",
            "cannot join using column nosuch - column not present in both tables",
        ),
        (
            "WITH i(x,y) AS (SELECT 1) SELECT * FROM i",
            "table i has 1 values for 2 columns",
        ),
        (
            "WITH i(x) AS (SELECT 1, 2) SELECT * FROM i",
            "table i has 2 values for 1 columns",
        ),
    ] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            message,
            "{sql}"
        );
    }
}

/// A combination of words no join is written with is refused naming
/// the words, which `sqlite3JoinType` does: a word no join carries, and
/// `INNER` beside `OUTER`.
#[test]
fn what_words_a_join_is_written_with() {
    let image = three();
    let database = Database::open(&image).expect("a database");
    for (sql, message) in [
        (
            "SELECT * FROM aa INNER OUTER JOIN bb",
            "unknown join type: INNER OUTER",
        ),
        (
            "SELECT * FROM aa INNER OUTER CROSS JOIN bb",
            "unknown join type: INNER OUTER CROSS",
        ),
        (
            "SELECT * FROM aa OUTER NATURAL INNER JOIN bb",
            "unknown join type: OUTER NATURAL INNER",
        ),
        (
            "SELECT * FROM aa LEFT BOGUS JOIN bb",
            "unknown join type: LEFT BOGUS",
        ),
        (
            "SELECT * FROM aa INNER BOGUS CROSS JOIN bb",
            "unknown join type: INNER BOGUS CROSS",
        ),
    ] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            message,
            "{sql}"
        );
    }
    // A word that is neither a name nor one of SQL's own ends the
    // reading, and the statement is refused where `JOIN` should stand.
    assert_eq!(
        database
            .query(b"SELECT * FROM aa LEFT 5 JOIN bb")
            .unwrap_err()
            .message(),
        "near \"5\": syntax error"
    );
    // The combinations a join is written with stand.
    for sql in [
        "SELECT * FROM aa NATURAL LEFT OUTER JOIN bb",
        "SELECT * FROM aa CROSS JOIN bb",
        "SELECT * FROM aa LEFT JOIN bb ON a=b",
        "SELECT * FROM aa FULL OUTER JOIN bb ON a=b",
        "SELECT * FROM aa INNER JOIN bb ON a=b",
    ] {
        database.query(sql.as_bytes()).unwrap_or_else(|error| {
            panic!("{sql}: {}", error.message());
        });
    }
}

/// Which value each column of a `FROM` inside brackets answers: its own,
/// and the one whichever side filled for a column the join inside the
/// brackets matched.
#[test]
fn which_value_each_column_of_a_from_inside_brackets_answers() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t2(a,b)".as_slice(),
        b"CREATE TABLE t3(a,b)",
        b"INSERT INTO t2 VALUES(222,'x2')",
        b"INSERT INTO t3 VALUES(222,'x3')",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    let shown = |sql: &[u8]| {
        let mut out = alloc::string::String::new();
        for row in database.query(sql).expect("rows").rows {
            for value in row {
                out.push_str(&alloc::string::String::from_utf8_lossy(
                    &value.text().unwrap_or_default(),
                ));
                out.push('|');
            }
        }
        out
    };
    // Two columns of one name stand for two columns where no `USING`
    // matched them, so each answers the value of its own table.
    assert_eq!(
        shown(b"SELECT * FROM (t2 JOIN t3 ON t2.a=t3.a)"),
        "222|x2|222|x3|"
    );
    assert_eq!(shown(b"SELECT * FROM (t2), (t3)"), "222|x2|222|x3|");
    // A column the `USING` matched is answered once, by whichever side
    // filled it.
    assert_eq!(shown(b"SELECT * FROM (t2 JOIN t3 USING(a))"), "222|x2|x3|");
    assert_eq!(
        shown(b"SELECT * FROM (t2 LEFT JOIN t3 USING(a))"),
        "222|x2|x3|"
    );
    assert_eq!(shown(b"SELECT a FROM (t2 JOIN t3 USING(a))"), "222|");
    // A column the `USING` matched that no side filled answers nothing.
    let mut writing = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE u2(a,b)".as_slice(),
        b"CREATE TABLE u3(a,b)",
        b"INSERT INTO u2 VALUES(NULL,'y2')",
    ] {
        writing.run(sql).unwrap();
    }
    let held = writing.written();
    let reader = Database::open(&held).expect("a database");
    let rows = reader
        .query(b"SELECT * FROM (u2 LEFT JOIN u3 USING(a))")
        .expect("rows")
        .rows;
    assert_eq!(
        rows,
        [alloc::vec![
            crate::value::Value::Null,
            crate::value::Value::Text(b"y2".to_vec()),
            crate::value::Value::Null
        ]]
    );
}

/// Which columns a `*` written after a name and a dot answers over a
/// `FROM` inside brackets: the columns of the table of that name.
#[test]
fn which_columns_a_star_after_a_name_answers_over_a_from_inside_brackets() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(a,b)".as_slice(),
        b"CREATE TABLE t2(a,b)",
        b"CREATE TABLE t3(a,b)",
        b"CREATE TABLE t4(a,b)",
        b"INSERT INTO t1 VALUES(111,'x1')",
        b"INSERT INTO t2 VALUES(222,'x2')",
        b"INSERT INTO t3 VALUES(333,'x3')",
        b"INSERT INTO t4 VALUES(444,'x4')",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    let shown = |sql: &[u8]| {
        let mut out = alloc::string::String::new();
        for row in database.query(sql).expect("rows").rows {
            for value in row {
                out.push_str(&alloc::string::String::from_utf8_lossy(
                    &value.text().unwrap_or_default(),
                ));
                out.push('|');
            }
        }
        out
    };
    let inside = b" FROM (t2 JOIN t3 ON t3.a=t2.a+111)";
    let held = |columns: &[u8]| {
        let mut sql = b"SELECT ".to_vec();
        sql.extend_from_slice(columns);
        sql.extend_from_slice(inside);
        sql
    };
    assert_eq!(shown(&held(b"t3.*")), "333|x3|");
    assert_eq!(shown(&held(b"t2.*")), "222|x2|");
    assert_eq!(shown(&held(b"t3.*, t2.*")), "333|x3|222|x2|");
    assert_eq!(shown(&held(b"t2.*, t3.*")), "222|x2|333|x3|");
    // The name reaches a table however many brackets stand around it.
    assert_eq!(
        shown(
            b"SELECT t3.* FROM t1 JOIN (t2 JOIN (t3 JOIN t4 ON t4.a=t3.a+111) \
              ON t3.a=t2.a+111) ON t2.a=t1.a+111"
        ),
        "333|x3|"
    );
    // A number of a `GROUP BY` counts the columns such a `*` answers.
    assert_eq!(
        shown(b"SELECT t3.* FROM (t2 JOIN t3 ON t3.a=t2.a+111) GROUP BY 1"),
        "333|x3|"
    );
    // A name no table inside the brackets carries is refused.
    assert_eq!(
        database.query(&held(b"nosuch.*")).unwrap_err().message(),
        "no such table: nosuch"
    );
}

/// An `ON` or a `USING` on the first source of a `FROM` is refused
/// naming the word, and a second constraint on one join is a syntax
/// error, which the grammar of `research/sqlite/src/parse.y:892` makes
/// of it: `on_using` takes one `ON` or one `USING`.
#[test]
fn what_an_on_or_a_using_without_a_join_is_refused_with() {
    let image = three();
    let database = Database::open(&image).expect("a database");
    for (sql, message) in [
        (
            "SELECT * FROM aa AS t ON b",
            "a JOIN clause is required before ON",
        ),
        (
            "SELECT * FROM aa AS t USING(a)",
            "a JOIN clause is required before USING",
        ),
        (
            "SELECT * FROM (aa) AS t ON b",
            "a JOIN clause is required before ON",
        ),
        (
            "SELECT * FROM (SELECT * FROM aa) AS t ON b",
            "a JOIN clause is required before ON",
        ),
        (
            "SELECT * FROM aa AS t ON b USING(a)",
            "near \"USING\": syntax error",
        ),
        (
            "SELECT * FROM aa JOIN bb ON a=b USING(a)",
            "near \"USING\": syntax error",
        ),
    ] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            message,
            "{sql}"
        );
    }
}

/// An `ON` of an outer join that names a table read after it is
/// refused, which the walk of `select.c` does; an inner join carries no
/// such mark, its `ON` being read as a `WHERE`.
#[test]
fn what_an_on_of_an_outer_join_may_name() {
    let image = three();
    let database = Database::open(&image).expect("a database");
    for sql in [
        "SELECT * FROM aa LEFT JOIN cc ON (a=b) JOIN bb ON (b=coalesce(c,1))",
        "SELECT * FROM aa RIGHT JOIN cc ON (a=b) JOIN bb ON (b=coalesce(c,1))",
        "SELECT * FROM aa LEFT JOIN cc ON (a=bb.b) JOIN bb ON (b=c)",
        // The name of a side read after this one, with a name of a side
        // read before it after that.
        "SELECT * FROM aa LEFT JOIN cc ON (b=a) JOIN bb ON (b=c)",
        // A name with the schema in front of it names no side a join
        // reads, so the walk passes over it and the name beside it is
        // the one the refusal names.
        "SELECT * FROM aa LEFT JOIN cc ON (main.aa.a=b) JOIN bb ON (b=c)",
    ] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            "ON clause references tables to its right",
            "{sql}"
        );
    }
    // An `ON` that names the sides read up to it stands.
    for sql in [
        "SELECT * FROM aa LEFT JOIN cc ON (a=c)",
        "SELECT * FROM aa LEFT JOIN cc ON (a=c) LEFT JOIN bb ON (b=c)",
    ] {
        database.query(sql.as_bytes()).unwrap_or_else(|error| {
            panic!("{sql}: {}", error.message());
        });
    }
}

/// A trigger runs with no statement of its own to bind against, so a
/// variable anywhere in it is refused, and a write of its body takes
/// the name of a table alone.
#[test]
fn what_a_trigger_may_not_carry() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(a,b)").unwrap();
    writer.run(b"CREATE TABLE t2(c,d)").unwrap();
    for body in [
        "AFTER INSERT ON t1 WHEN new.a = ? BEGIN SELECT 1; END",
        "BEFORE DELETE ON t1 BEGIN SELECT ?; END",
        "BEFORE DELETE ON t1 BEGIN SELECT * FROM (SELECT * FROM (SELECT ?)); END",
        "BEFORE DELETE ON t1 BEGIN SELECT * FROM t2 GROUP BY ?; END",
        "BEFORE DELETE ON t1 BEGIN SELECT * FROM t2 LIMIT ?; END",
        "BEFORE DELETE ON t1 BEGIN SELECT * FROM t2 ORDER BY ?; END",
        "BEFORE UPDATE ON t1 BEGIN UPDATE t2 SET c = ?; END",
        "BEFORE UPDATE ON t1 BEGIN UPDATE t2 SET c = 1 WHERE d = ?; END",
        "BEFORE INSERT ON t1 BEGIN INSERT INTO t2 SELECT $1, 1 FROM t1; END",
    ] {
        let sql = alloc::format!("CREATE TRIGGER tr1 {body}");
        assert_eq!(
            writer.run(sql.as_bytes()).unwrap_err().message(),
            "trigger cannot use variables",
            "{body}"
        );
    }
    // A variable inside a text is the text and not a variable.
    writer
        .run(b"CREATE TRIGGER tr1 AFTER INSERT ON t1 BEGIN SELECT '?'; END")
        .expect("a trigger");
    for body in [
        "AFTER UPDATE ON t1 BEGIN INSERT INTO main.t2 VALUES(new.a, new.b); END",
        "AFTER UPDATE ON t1 BEGIN UPDATE main.t2 SET c = 1; END",
        "AFTER UPDATE ON t1 BEGIN DELETE FROM main.t2; END",
    ] {
        let sql = alloc::format!("CREATE TRIGGER tr2 {body}");
        assert_eq!(
            writer.run(sql.as_bytes()).unwrap_err().message(),
            concat!(
                "qualified table names are not allowed on ",
                "INSERT, UPDATE, and DELETE statements within triggers"
            ),
            "{body}"
        );
    }
    // A write that names the table alone stands.
    writer
        .run(b"CREATE TRIGGER tr2 AFTER UPDATE ON t1 BEGIN INSERT INTO t2 VALUES(1,2); END")
        .expect("a trigger");
}

/// `likelihood(X,Y)` takes a real written as one for `Y`, between
/// nought and one, and nothing else.
#[test]
fn what_the_second_argument_of_likelihood_may_be() {
    let image = three();
    let database = Database::open(&image).expect("a database");
    for sql in [
        "SELECT likelihood(123, 1.000001)",
        "SELECT likelihood(123, -0.000001)",
        "SELECT likelihood(123, 0.5+0.3)",
        "SELECT likelihood(123, 1)",
        "SELECT likelihood(123, '0.5')",
        "SELECT likelihood(123, NULL)",
    ] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            "second argument to likelihood() must be a constant between 0.0 and 1.0",
            "{sql}"
        );
    }
    // A call of another number of arguments is refused for that.
    for sql in ["SELECT likelihood(123)", "SELECT likelihood(1,2,3)"] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            "wrong number of arguments to function likelihood()",
            "{sql}"
        );
    }
    for sql in [
        "SELECT likelihood(123, 1.0)",
        "SELECT likelihood(456, 0.0)",
        "SELECT likelihood(NULL, 0.5)",
        "SELECT likelihood('test-string', 0.5)",
    ] {
        database.query(sql.as_bytes()).unwrap_or_else(|error| {
            panic!("{sql}: {}", error.message());
        });
    }
}

/// `#1` is the register of a routine the C library writes and no
/// statement carries one, so the token is read and then refused.
#[test]
fn what_a_register_of_the_routine_is_refused_with() {
    let image = three();
    let database = Database::open(&image).expect("a database");
    for (sql, message) in [
        ("SELECT #1", "near \"#1\": syntax error"),
        ("SELECT 1 WHERE #0=1", "near \"#0\": syntax error"),
        ("SELECT a FROM aa ORDER BY #2", "near \"#2\": syntax error"),
    ] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            message,
            "{sql}"
        );
    }
}

/// A column added to a table that points at a row of another falls
/// back to nothing, the rows the table already holds gaining no value
/// of their own.
#[test]
fn what_a_column_that_points_may_fall_back_to() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(a,b)").unwrap();
    writer.run(b"PRAGMA foreign_keys=on").unwrap();
    writer
        .run(b"ALTER TABLE t1 ADD COLUMN f REFERENCES t1")
        .expect("a column");
    writer
        .run(b"ALTER TABLE t1 ADD COLUMN h REFERENCES t1 DEFAULT NULL")
        .expect("a column");
    // `sqlite3ErrorIfNotEmpty` reads the table first, so a table that
    // holds no row takes the column whatever it falls back to.
    writer
        .run(b"ALTER TABLE t1 ADD COLUMN j REFERENCES t1 DEFAULT 4")
        .expect("a column");
    writer.run(b"CREATE TABLE t2(a)").unwrap();
    writer.run(b"INSERT INTO t2 VALUES(1)").unwrap();
    // A column that points at no row, and one that points and falls back
    // to nothing, are both taken by a table that holds a row.
    writer
        .run(b"ALTER TABLE t2 ADD COLUMN k DEFAULT 4")
        .expect("a column");
    writer
        .run(b"ALTER TABLE t2 ADD COLUMN l REFERENCES t1 DEFAULT NULL")
        .expect("a column");
    assert_eq!(
        writer
            .run(b"ALTER TABLE t2 ADD COLUMN g REFERENCES t1 DEFAULT 4")
            .unwrap_err()
            .message(),
        "Cannot add a REFERENCES column with non-NULL default value"
    );
    // A connection that holds its rows to no foreign key takes the
    // column, which `db->flags&SQLITE_ForeignKeys` reads.
    writer.run(b"PRAGMA foreign_keys=off").unwrap();
    writer
        .run(b"ALTER TABLE t2 ADD COLUMN g REFERENCES t1 DEFAULT 4")
        .expect("a column");
    // A column that points at no row takes the value it falls back to.
    writer
        .run(b"ALTER TABLE t1 ADD COLUMN i DEFAULT 4")
        .expect("a column");
}

/// A `WITH` term reads the terms beside it, whichever was written
/// first, and terms that read each other are refused naming the one
/// the statement reads.
#[test]
fn what_the_terms_of_a_with_may_read() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(x)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1),(2)").unwrap();
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    // A term written after the one that reads it is answered first.
    assert_eq!(
        database
            .query(
                b"WITH tmp2(x) AS (SELECT * FROM tmp1), tmp1(a) AS (SELECT * FROM t1) \
                  SELECT * FROM tmp2"
            )
            .unwrap()
            .rows
            .len(),
        2
    );
    for (sql, message) in [
        (
            "WITH tmp2(x) AS (SELECT * FROM tmp1), tmp1(a) AS (SELECT * FROM tmp2) \
             SELECT * FROM tmp1",
            "circular reference: tmp1",
        ),
        (
            "WITH i(x) AS (SELECT * FROM j), j(x) AS (SELECT * FROM k), \
             k(x) AS (SELECT * FROM i) SELECT * FROM i",
            "circular reference: i",
        ),
        (
            "WITH i(x) AS (SELECT * FROM (SELECT * FROM j)), \
             j(x) AS (SELECT * FROM (SELECT * FROM i)) SELECT * FROM i",
            "circular reference: i",
        ),
        (
            "WITH tmp(a) AS (SELECT * FROM t1), tmp(a) AS (SELECT * FROM t1) \
             SELECT * FROM tmp",
            "duplicate WITH table name: tmp",
        ),
    ] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            message,
            "{sql}"
        );
    }
    // A circle the statement never reads is left unanswered, which is
    // what the C library does by answering a term where it is read.
    assert_eq!(
        database
            .query(
                b"WITH i(x) AS (SELECT * FROM j), j(x) AS (SELECT * FROM i) \
                  SELECT * FROM t1"
            )
            .unwrap()
            .rows
            .len(),
        2
    );
    assert_eq!(
        database
            .query(
                b"WITH a(x) AS (SELECT 1), b(x) AS (SELECT * FROM c), \
                  c(x) AS (SELECT * FROM b) SELECT * FROM a"
            )
            .unwrap()
            .rows
            .len(),
        1
    );
    // A term that reads a table the database does not hold is refused
    // for that table and not for a circle.
    assert_eq!(
        database
            .query(b"WITH i(x) AS (SELECT * FROM nowhere) SELECT * FROM i")
            .unwrap_err()
            .message(),
        "no such table: nowhere"
    );
}

/// A walk stops once the rows a `LIMIT` takes are all there, whichever
/// loop answers the last of them, and a `LIMIT` that names a column is
/// refused before the walk begins.
#[test]
fn what_a_limit_stops_the_walk_at() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE aa(a)".as_slice(),
        b"CREATE TABLE bb(b)",
        b"INSERT INTO aa VALUES(1),(2),(3)",
        b"INSERT INTO bb VALUES(2),(3),(4)",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let rows = |sql: &[u8]| database.query(sql).map(|answer| answer.rows);
    // The rows a `RIGHT` join keeps that nothing matched come after
    // every row the nest answered, so a `LIMIT` those rows fill stops
    // the walk of that pass.
    assert_eq!(
        rows(b"SELECT a, b FROM aa RIGHT JOIN bb ON a=b LIMIT 3")
            .unwrap()
            .len(),
        3
    );
    assert_eq!(rows(b"SELECT a FROM aa LIMIT 2").unwrap().len(), 2);
    assert_eq!(rows(b"SELECT a FROM aa LIMIT 1 OFFSET 2").unwrap().len(), 1);
    // A `LIMIT` reads no row, so one that names a column is refused.
    assert_eq!(
        rows(b"SELECT a FROM aa LIMIT a").unwrap_err().message(),
        "no such column: a"
    );
}

/// The sides of one statement are held to the bits of the mask the walk
/// of them counts each side in, which is 64.
#[test]
fn how_many_sides_the_from_of_one_statement_holds() {
    let image = three();
    let database = Database::open(&image).expect("a database");
    let sides = |count: usize| -> alloc::vec::Vec<u8> {
        let mut sql = b"SELECT 1 FROM aa".to_vec();
        for _ in 1..count {
            sql.extend_from_slice(b", aa");
        }
        sql
    };
    for count in [1, 30, 63, 64] {
        let sql = sides(count);
        database
            .query(&sql)
            .unwrap_or_else(|error| panic!("{count} sides: {error:?}"));
    }
    for count in [65, 100] {
        assert_eq!(
            database.query(&sides(count)).unwrap_err().message(),
            "at most 64 tables in a join",
            "{count} sides"
        );
    }
    // A statement written inside another one carries its own sides, so
    // two of 64 stand together.
    let mut sql = sides(64);
    sql.extend_from_slice(b" WHERE EXISTS(");
    sql.extend_from_slice(&sides(64));
    sql.extend_from_slice(b")");
    database
        .query(&sql)
        .unwrap_or_else(|error| panic!("two statements of 64 sides: {error:?}"));
}

/// A term of the `WHERE` that names the side a `LEFT JOIN` attaches is
/// read for the row that holds nothing of that side as it is read for a
/// row that holds one, so such a row stands where the term holds of nulls
/// and nowhere else.
#[test]
fn what_a_term_that_names_the_side_a_left_join_attaches_answers() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(a INT, b INT)".as_slice(),
        b"INSERT INTO t1 VALUES(1,2)",
        b"INSERT INTO t1 VALUES(1,3)",
        b"INSERT INTO t1 VALUES(1,4)",
        b"CREATE TABLE t2(c INT, d INT)",
        b"INSERT INTO t2 VALUES(3,33)",
        b"INSERT INTO t2 VALUES(4,44)",
        b"INSERT INTO t2 VALUES(5,55)",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let rows = |sql: &[u8]| {
        database
            .query(sql)
            .unwrap()
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|value| match value {
                        crate::value::Value::Int(held) => alloc::format!("{held}"),
                        _ => alloc::string::String::from("NULL"),
                    })
                    .collect::<alloc::vec::Vec<_>>()
                    .join("|")
            })
            .collect::<alloc::vec::Vec<_>>()
            .join(" ")
    };
    assert_eq!(
        rows(b"SELECT * FROM t1 LEFT JOIN t2 ON b=c WHERE c IS NULL"),
        "1|2|NULL|NULL"
    );
    assert_eq!(
        rows(b"SELECT * FROM t1 LEFT JOIN t2 ON b=c WHERE d>40"),
        "1|4|4|44"
    );
    assert_eq!(
        rows(b"SELECT * FROM t1 LEFT JOIN t2 ON b=c WHERE c IS NOT NULL"),
        "1|3|3|33 1|4|4|44"
    );
    // A term that names the side on the left holds of every row of it.
    assert_eq!(
        rows(b"SELECT * FROM t1 LEFT JOIN t2 ON b=c WHERE a=1"),
        "1|2|NULL|NULL 1|3|3|33 1|4|4|44"
    );
}
