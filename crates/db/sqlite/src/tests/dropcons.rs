// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The text of a `CREATE TABLE` with one constraint taken out of it,
//! against the pairs `altercons.test` writes.

use crate::constraint::{Dropped, Error, without};

/// The statement `sql` without the constraint named `abc`.
fn dropped(sql: &[u8]) -> alloc::string::String {
    let text = without(sql, Dropped::Named(b"abc")).unwrap();
    alloc::string::String::from_utf8_lossy(&text).into_owned()
}

#[test]
fn a_constraint_the_statement_named_is_cut_out_of_it() {
    for (before, after) in [
        (
            "CREATE TABLE t1(a, b CONSTRAINT abc CHECK(t1.a != t1.b))",
            "CREATE TABLE t1(a, b)",
        ),
        (
            "CREATE TABLE t1(a, b CONSTRAINT abc CHECK(t1.a != t1.b) NOT NULL)",
            "CREATE TABLE t1(a, b NOT NULL)",
        ),
        (
            "CREATE TABLE t1(a, b CONSTRAINT abc CHECK(t1.a != t1.b)NOT NULL)",
            "CREATE TABLE t1(a, b NOT NULL)",
        ),
        (
            "CREATE TABLE t1(a, b NOT NULL CONSTRAINT abc CHECK(t1.a != t1.b))",
            "CREATE TABLE t1(a, b NOT NULL)",
        ),
        (
            "CREATE TABLE t1(a, b, CONSTRAINT abc CHECK(t1.a != t1.b))",
            "CREATE TABLE t1(a, b)",
        ),
        (
            "CREATE TABLE t1(a, b, CONSTRAINT abc CHECK(t1.a != t1.b), PRIMARY KEY(a))",
            "CREATE TABLE t1(a, b, PRIMARY KEY(a))",
        ),
        // A name before a clause that opens another constraint is a
        // constraint of no body, so the name alone is cut.
        (
            "CREATE TABLE t1(a, b, c CONSTRAINT abc GENERATED ALWAYS AS (b+1) STORED)",
            "CREATE TABLE t1(a, b, c GENERATED ALWAYS AS (b+1) STORED)",
        ),
        (
            "CREATE TABLE t1(a, b CONSTRAINT abc NOT NULL)",
            "CREATE TABLE t1(a, b)",
        ),
    ] {
        assert_eq!(dropped(before.as_bytes()), after, "{before}");
    }
}

#[test]
fn a_constraint_that_is_neither_a_check_nor_a_not_null_stands() {
    assert_eq!(
        without(
            b"CREATE TABLE t2(x, y CONSTRAINT ccc UNIQUE)",
            Dropped::Named(b"ccc")
        ),
        Err(Error::Kept(b"ccc".to_vec()))
    );
    assert_eq!(
        without(
            b"CREATE TABLE t2(x, y CONSTRAINT ccc UNIQUE)",
            Dropped::Named(b"ddd")
        ),
        Err(Error::NoSuch(b"ddd".to_vec()))
    );
}

#[test]
fn the_not_null_of_one_column_is_cut_and_the_rest_stand() {
    let text = |sql: &[u8], at: usize| {
        let out = without(sql, Dropped::NotNull(at)).unwrap();
        alloc::string::String::from_utf8_lossy(&out).into_owned()
    };
    assert_eq!(
        text(b"CREATE TABLE t1(a NOT NULL, b)", 0),
        "CREATE TABLE t1(a, b)"
    );
    assert_eq!(
        text(b"CREATE TABLE t1(a NOT NULL ON CONFLICT FAIL, b)", 0),
        "CREATE TABLE t1(a, b)"
    );
    assert_eq!(
        text(b"CREATE TABLE t1(a, b NOT NULL)", 1),
        "CREATE TABLE t1(a, b)"
    );
    assert_eq!(
        text(b"CREATE TABLE t1(a CONSTRAINT nn NOT NULL, b)", 0),
        "CREATE TABLE t1(a, b)"
    );
    // A column that holds no `NOT NULL` is no error, and the statement
    // stands as it was.
    assert_eq!(
        text(b"CREATE TABLE t1(a, b NOT NULL)", 0),
        "CREATE TABLE t1(a, b NOT NULL)"
    );
}

#[test]
fn the_statement_of_the_table_is_written_again_without_the_constraint() {
    use crate::change::Writer;
    use crate::db::Database;
    use crate::header::Encoding;
    let schema = |writer: &Writer| {
        let image = writer.written();
        let database = Database::open(&image).unwrap();
        database
            .query(b"SELECT sql FROM sqlite_schema WHERE name='t1'")
            .unwrap()
            .rows
            .first()
            .and_then(|row| row.first().and_then(crate::value::Value::text))
            .map(|text| alloc::string::String::from_utf8_lossy(&text).into_owned())
    };
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t1(a, b CONSTRAINT abc CHECK(t1.a != t1.b) NOT NULL)")
        .unwrap();
    writer.run(b"ALTER TABLE t1 DROP CONSTRAINT abc").unwrap();
    assert_eq!(
        schema(&writer).as_deref(),
        Some("CREATE TABLE t1(a, b NOT NULL)")
    );
    // The column no longer refuses nothing once its `NOT NULL` is gone.
    writer.run(b"INSERT INTO t1(a) VALUES(1)").unwrap_err();
    writer
        .run(b"ALTER TABLE t1 ALTER COLUMN b DROP NOT NULL")
        .unwrap();
    assert_eq!(schema(&writer).as_deref(), Some("CREATE TABLE t1(a, b)"));
    writer.run(b"INSERT INTO t1(a) VALUES(1)").unwrap();
    // A name the statement does not hold, and one it holds over a
    // constraint of another kind.
    writer
        .run(b"CREATE TABLE t2(x, y CONSTRAINT ccc UNIQUE)")
        .unwrap();
    assert_eq!(
        writer
            .run(b"ALTER TABLE t2 DROP CONSTRAINT ccc")
            .unwrap_err()
            .message(),
        "constraint may not be dropped: ccc"
    );
    assert_eq!(
        writer
            .run(b"ALTER TABLE t2 DROP CONSTRAINT ddd")
            .unwrap_err()
            .message(),
        "no such constraint: ddd"
    );
    assert_eq!(
        writer
            .run(b"ALTER TABLE t2 ALTER COLUMN zz DROP NOT NULL")
            .unwrap_err()
            .message(),
        "no such column: zz"
    );
    assert_eq!(
        writer
            .run(b"ALTER TABLE sqlite_master DROP CONSTRAINT abc")
            .unwrap_err()
            .message(),
        "table sqlite_master may not be altered"
    );
}

#[test]
fn a_constraint_is_written_into_the_column_or_after_the_columns() {
    let text = |sql: &[u8], at: Option<usize>, held: &[u8]| {
        let out = crate::constraint::with(sql, at, held);
        alloc::string::String::from_utf8_lossy(&out).into_owned()
    };
    assert_eq!(
        text(b"CREATE TABLE t1(a, b)", Some(0), b"NOT NULL"),
        "CREATE TABLE t1(a NOT NULL, b)"
    );
    assert_eq!(
        text(
            b"CREATE TABLE t1(a, b)",
            Some(0),
            b"NOT NULL ON CONFLICT FAIL"
        ),
        "CREATE TABLE t1(a NOT NULL ON CONFLICT FAIL, b)"
    );
    // The words are written as the statement wrote them.
    assert_eq!(
        text(
            b"CREATE TABLE t1(a, b)",
            Some(1),
            b"NOT   NULL ON CONFLICT IGNORE"
        ),
        "CREATE TABLE t1(a, b NOT   NULL ON CONFLICT IGNORE)"
    );
    assert_eq!(
        text(
            b"CREATE TABLE t1(a, 'a b c' VARCHAR(10), UNIQUE(a))",
            Some(1),
            b"NOT NULL"
        ),
        "CREATE TABLE t1(a, 'a b c' VARCHAR(10) NOT NULL, UNIQUE(a))"
    );
    // A constraint of the table's own stands before the bracket that
    // closes the columns, and the space before it stands.
    assert_eq!(
        text(
            b"CREATE TABLE t1(a, b)",
            None,
            b"CONSTRAINT nn CHECK (a>=0)"
        ),
        "CREATE TABLE t1(a, b, CONSTRAINT nn CHECK (a>=0))"
    );
    assert_eq!(
        text(
            b"CREATE TABLE t1(a, b  )",
            None,
            b"CONSTRAINT nn CHECK (a>=0)"
        ),
        "CREATE TABLE t1(a, b  , CONSTRAINT nn CHECK (a>=0))"
    );
    assert_eq!(
        text(b"CREATE TABLE t1(a, b  )", None, b"CHECK (a>=0)"),
        "CREATE TABLE t1(a, b  , CHECK (a>=0))"
    );
}

#[test]
fn a_constraint_a_statement_writes_is_held_by_the_rows_it_stands_over() {
    use crate::change::Writer;
    use crate::db::Database;
    use crate::header::Encoding;
    let schema = |writer: &Writer, name: &[u8]| {
        let image = writer.written();
        let database = Database::open(&image).unwrap();
        let mut sql = b"SELECT sql FROM sqlite_schema WHERE name='".to_vec();
        sql.extend_from_slice(name);
        sql.push(b'\'');
        database
            .query(&sql)
            .unwrap()
            .rows
            .first()
            .and_then(|row| row.first().and_then(crate::value::Value::text))
            .map(|text| alloc::string::String::from_utf8_lossy(&text).into_owned())
    };
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t3(a INTEGER PRIMARY KEY, b)")
        .unwrap();
    writer.run(b"INSERT INTO t3 VALUES(1000, NULL)").unwrap();
    // A row that holds nothing in the column refuses the constraint.
    assert_eq!(
        writer
            .run(b"ALTER TABLE t3 ALTER b SET NOT NULL")
            .unwrap_err()
            .message(),
        "constraint failed"
    );
    writer.run(b"UPDATE t3 SET b=1").unwrap();
    writer.run(b"ALTER TABLE t3 ALTER b SET NOT NULL").unwrap();
    assert_eq!(
        schema(&writer, b"t3").as_deref(),
        Some("CREATE TABLE t3(a INTEGER PRIMARY KEY, b NOT NULL)")
    );
    writer.run(b"CREATE TABLE t1(a, b, c)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1,2,3),(4,5,6)").unwrap();
    // A row the clause does not hold refuses the constraint, and the
    // statement stands as it was.
    assert_eq!(
        writer
            .run(b"ALTER TABLE t1 ADD CONSTRAINT nn CHECK (c!=6)")
            .unwrap_err()
            .message(),
        "constraint failed"
    );
    assert_eq!(
        schema(&writer, b"t1").as_deref(),
        Some("CREATE TABLE t1(a, b, c)")
    );
    writer.run(b"DELETE FROM t1 WHERE c=6").unwrap();
    writer
        .run(b"ALTER TABLE t1 ADD CONSTRAINT nn CHECK (c!=6)")
        .unwrap();
    assert_eq!(
        schema(&writer, b"t1").as_deref(),
        Some("CREATE TABLE t1(a, b, c, CONSTRAINT nn CHECK (c!=6))")
    );
    // The constraint holds the rows written after it.
    assert_eq!(
        writer
            .run(b"INSERT INTO t1 VALUES(4,5,6)")
            .unwrap_err()
            .message(),
        "CHECK constraint failed: nn"
    );
    // A name the statement already holds is refused.
    writer
        .run(b"CREATE TABLE b1(a, b, CONSTRAINT abc CHECK (a!=2))")
        .unwrap();
    assert_eq!(
        writer
            .run(b"ALTER TABLE b1 ADD CONSTRAINT abc CHECK (a!=3)")
            .unwrap_err()
            .message(),
        "constraint abc already exists"
    );
    assert_eq!(
        schema(&writer, b"b1").as_deref(),
        Some("CREATE TABLE b1(a, b, CONSTRAINT abc CHECK (a!=2))")
    );
    // A `CHECK` with no name is written as it was.
    writer.run(b"ALTER TABLE b1 ADD CHECK (a>=0)").unwrap();
    assert_eq!(
        schema(&writer, b"b1").as_deref(),
        Some("CREATE TABLE b1(a, b, CONSTRAINT abc CHECK (a!=2), CHECK (a>=0))")
    );
}

#[test]
fn a_statement_the_tokens_run_out_of_names_no_constraint() {
    // A statement that holds no bracket names nothing, and one whose
    // text ends inside the columns does too.
    for sql in [
        b"CREATE TABLE t1".as_slice(),
        b"CREATE TABLE t1(",
        b"CREATE TABLE t1(a CHECK(",
        b"CREATE TABLE t1(a CHECK((a)) ",
    ] {
        assert_eq!(
            without(sql, Dropped::Named(b"abc")),
            Err(Error::NoSuch(b"abc".to_vec())),
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
        assert_eq!(
            without(sql, Dropped::NotNull(0)).unwrap(),
            sql.to_vec(),
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
        assert!(crate::constraint::with(sql, None, b"CHECK (1)").len() >= sql.len());
    }
    // A name a constraint of no body carries, where another name is
    // being dropped.
    assert_eq!(
        dropped(b"CREATE TABLE t1(a CONSTRAINT zz DEFAULT 1, b CONSTRAINT abc NOT NULL)"),
        "CREATE TABLE t1(a CONSTRAINT zz DEFAULT 1, b)"
    );
    // A byte no rule accepts ends the walk, inside the columns and
    // before them.
    for sql in [
        b"CREATE TABLE t1(a CHECK(#))".as_slice(),
        b"CREATE # TABLE t1(a)",
    ] {
        assert_eq!(
            without(sql, Dropped::Named(b"abc")),
            Err(Error::NoSuch(b"abc".to_vec())),
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
    // A named `NOT NULL` of another column is passed over.
    assert_eq!(
        without(
            b"CREATE TABLE t1(a CONSTRAINT nn NOT NULL, b NOT NULL)",
            Dropped::NotNull(1)
        )
        .unwrap(),
        b"CREATE TABLE t1(a CONSTRAINT nn NOT NULL, b)".to_vec()
    );
    // A `DROP NOT NULL` passes over a named constraint of another kind.
    assert_eq!(
        without(
            b"CREATE TABLE t1(a CONSTRAINT zz UNIQUE NOT NULL, b)",
            Dropped::NotNull(0)
        )
        .unwrap(),
        b"CREATE TABLE t1(a CONSTRAINT zz UNIQUE, b)".to_vec()
    );
}
