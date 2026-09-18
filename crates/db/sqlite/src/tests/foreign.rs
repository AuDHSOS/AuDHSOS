// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the foreign keys a table's rows are held to, against what
//! the C library answers for the same statements over the same rows.

use alloc::string::String;

use crate::change::Writer;
use crate::db::{Database, Error};
use crate::header::Encoding;
use crate::value::Value;

/// A connection that has run the statements, and what the last of them
/// answered.
fn ran(statements: &[&str]) -> Result<(Writer, alloc::vec::Vec<String>), Error> {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    let mut out = alloc::vec::Vec::new();
    for sql in statements {
        out.clear();
        for row in writer.run(sql.as_bytes())? {
            for value in &row {
                out.push(shown(value));
            }
        }
    }
    Ok((writer, out))
}

/// What one value looks like as a list element.
fn shown(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Int(number) => alloc::format!("{number}"),
        Value::Real(number) => String::from_utf8_lossy(&crate::fp::text(*number, 15)).into_owned(),
        Value::Text(bytes) | Value::Blob(bytes) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// What a statement answers over a connection that has run the ones
/// before it.
fn answered(writer: &Writer, sql: &str) -> alloc::vec::Vec<String> {
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    let mut out = alloc::vec::Vec::new();
    for row in &database.query(sql.as_bytes()).unwrap().rows {
        for value in row {
            out.push(shown(value));
        }
    }
    out
}

/// What a pragma answers, which the connection that writes answers out
/// of what it holds.
fn pragma(writer: &mut Writer, sql: &str) -> alloc::vec::Vec<String> {
    let mut out = alloc::vec::Vec::new();
    for row in writer.run(sql.as_bytes()).unwrap() {
        for value in &row {
            out.push(shown(value));
        }
    }
    out
}

/// The tables the tests of the parent side are written over.
const PARENTS: &[&str] = &[
    "PRAGMA foreign_keys=ON",
    "CREATE TABLE p(x INTEGER PRIMARY KEY, y)",
    "CREATE TABLE gone(a, b REFERENCES p ON DELETE CASCADE)",
    "CREATE TABLE emptied(a, b REFERENCES p ON DELETE SET NULL)",
    "CREATE TABLE fallen(a, b DEFAULT 7 REFERENCES p ON DELETE SET DEFAULT)",
    "CREATE TABLE held(a, b REFERENCES p)",
    "INSERT INTO p VALUES(1,'one'),(2,'two'),(3,'three'),(4,'four')",
    "INSERT INTO gone VALUES('a',1)",
    "INSERT INTO emptied VALUES('b',2)",
    "INSERT INTO fallen VALUES('c',3)",
    "INSERT INTO held VALUES('d',4)",
];

#[test]
fn a_row_whose_foreign_key_points_at_no_row_is_refused() {
    // `I.1` of `src/fkey.c`, which `PRAGMA foreign_keys` turns on and
    // which a connection told nothing leaves off.
    let (writer, _) = ran(&[
        "CREATE TABLE p(x PRIMARY KEY, y)",
        "CREATE TABLE c(a, b REFERENCES p(x))",
        "INSERT INTO p VALUES(1,'one')",
        "INSERT INTO c VALUES('a',9)",
    ])
    .unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM c"), ["1"]);
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE p(x PRIMARY KEY, y)",
        "CREATE TABLE c(a, b REFERENCES p(x))",
        "INSERT INTO p VALUES(1,'one')",
    ])
    .unwrap();
    assert_eq!(
        writer.run(b"INSERT INTO c VALUES('a',9)"),
        Err(Error::Foreign)
    );
    // A key with a null in it points at nothing, which `MATCH SIMPLE`
    // holds to nothing.
    writer.run(b"INSERT INTO c VALUES('a',NULL)").unwrap();
    writer.run(b"INSERT INTO c VALUES('b',1)").unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM c"), ["2"]);
    // An `UPDATE` is held to the same rule.
    assert_eq!(
        writer.run(b"UPDATE c SET b=9 WHERE a='b'"),
        Err(Error::Foreign)
    );
    writer.run(b"UPDATE c SET b=1 WHERE a='a'").unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM c WHERE b=1"), ["2"]);
}

#[test]
fn a_foreign_key_that_points_at_no_unique_columns_is_a_mismatch() {
    // `sqlite3FkLocateIndex`: the columns pointed at are the primary
    // key of that table or carry a unique index of their own.
    for sql in [
        // No unique columns at all.
        "INSERT INTO loose VALUES(1)",
        // A name the table pointed at does not carry.
        "INSERT INTO astray VALUES(1)",
        // More columns than the key it points at.
        "INSERT INTO wide VALUES(1,2)",
        // A table with no primary key at all.
        "INSERT INTO keyless VALUES(1)",
        // As many columns as the primary key carries, under another
        // name, which no unique index covers.
        "INSERT INTO sideways VALUES(1)",
    ] {
        let (mut writer, _) = ran(&[
            "PRAGMA foreign_keys=ON",
            "CREATE TABLE plain(x, y)",
            "CREATE TABLE keyed(x PRIMARY KEY, y)",
            "CREATE TABLE loose(a REFERENCES plain(x))",
            "CREATE TABLE astray(a REFERENCES keyed(z))",
            "CREATE TABLE wide(a, b, FOREIGN KEY(a,b) REFERENCES keyed)",
            "CREATE TABLE nowhere(a REFERENCES missing(x))",
            "CREATE TABLE keyless(a REFERENCES plain)",
            "CREATE TABLE sideways(a REFERENCES keyed(y))",
        ])
        .unwrap();
        assert!(
            matches!(
                writer.run(sql.as_bytes()),
                Err(Error::ForeignMismatch(_, _))
            ),
            "{sql}"
        );
    }
    // `sqlite3CreateForeignKey` reads the two lists of names against
    // each other, so a key of one width pointing at another is refused
    // where the `CREATE TABLE` is read.
    let (mut writer, _) = ran(&["CREATE TABLE keyed(x PRIMARY KEY, y)"]).unwrap();
    assert_eq!(
        writer
            .run(b"CREATE TABLE uneven(a, FOREIGN KEY(a) REFERENCES keyed(x, y))")
            .unwrap_err()
            .message(),
        "number of columns in foreign key does not match the number of columns in the referenced table"
    );
    assert_eq!(
        writer
            .run(b"CREATE TABLE unknown(a, b, FOREIGN KEY(a, c) REFERENCES keyed(x, y))")
            .unwrap_err()
            .message(),
        "unknown column \"c\" in foreign key definition"
    );
    // A key that points at a table the schema does not hold names that
    // table under the schema it would stand in.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE nowhere(a REFERENCES missing(x))",
    ])
    .unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO nowhere VALUES(1)")
            .unwrap_err()
            .message(),
        "no such table: main.missing"
    );
    // A `UNIQUE` over the columns pointed at is enough, and so is a
    // table whose primary key is the rowid.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE u(x, y, UNIQUE(x,y))",
        "CREATE TABLE r(x INTEGER PRIMARY KEY)",
        "CREATE TABLE two(a, b, FOREIGN KEY(a,b) REFERENCES u(x,y))",
        "CREATE TABLE one(a REFERENCES r)",
        "INSERT INTO u VALUES(1,2)",
        "INSERT INTO r VALUES(5)",
        "INSERT INTO two VALUES(1,2)",
        "INSERT INTO one VALUES(5)",
    ])
    .unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM two"), ["1"]);
    assert_eq!(
        writer.run(b"INSERT INTO two VALUES(1,3)"),
        Err(Error::Foreign)
    );
    assert_eq!(
        writer.run(b"INSERT INTO one VALUES(6)"),
        Err(Error::Foreign)
    );
    // A key that names no columns points at the primary key in the
    // order the primary key was written, not the order of the table.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE both(x, y, PRIMARY KEY(y,x))",
        "CREATE TABLE pair(a, b, FOREIGN KEY(a,b) REFERENCES both)",
        "INSERT INTO both VALUES(1,2)",
        "INSERT INTO pair VALUES(2,1)",
    ])
    .unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM pair"), ["1"]);
    assert_eq!(
        writer.run(b"INSERT INTO pair VALUES(1,2)"),
        Err(Error::Foreign)
    );
}

#[test]
fn what_happens_to_the_rows_that_point_at_a_row_that_goes() {
    // `D.2` of `src/fkey.c`, one action per table.
    let (mut writer, _) = ran(PARENTS).unwrap();
    writer.run(b"DELETE FROM p WHERE x=1").unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM gone"), ["0"]);
    writer.run(b"DELETE FROM p WHERE x=2").unwrap();
    assert_eq!(answered(&writer, "SELECT a, b FROM emptied"), ["b", ""]);
    writer.run(b"DELETE FROM p WHERE x=3").unwrap();
    assert_eq!(answered(&writer, "SELECT a, b FROM fallen"), ["c", "7"]);
    assert_eq!(writer.run(b"DELETE FROM p WHERE x=4"), Err(Error::Foreign));
    assert_eq!(answered(&writer, "SELECT count(*) FROM p"), ["1"]);
    // A row nothing points at goes, and so does a row whose key is
    // null wherever it is read.
    writer.run(b"INSERT INTO p VALUES(5,'five')").unwrap();
    writer.run(b"DELETE FROM p WHERE x=5").unwrap();
    writer.run(b"INSERT INTO held VALUES('e',NULL)").unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM held"), ["2"]);
}

#[test]
fn what_happens_to_the_rows_that_point_at_a_row_that_changes() {
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE p(x PRIMARY KEY, y)",
        "CREATE TABLE moved(a, b REFERENCES p ON UPDATE CASCADE)",
        "CREATE TABLE emptied(a, b REFERENCES p ON UPDATE SET NULL)",
        "CREATE TABLE held(a, b REFERENCES p)",
        "INSERT INTO p VALUES(1,'one'),(2,'two'),(3,'three')",
        "INSERT INTO moved VALUES('a',1)",
        "INSERT INTO emptied VALUES('b',2)",
        "INSERT INTO held VALUES('c',3)",
    ])
    .unwrap();
    writer.run(b"UPDATE p SET x=11 WHERE x=1").unwrap();
    assert_eq!(answered(&writer, "SELECT b FROM moved"), ["11"]);
    writer.run(b"UPDATE p SET x=12 WHERE x=2").unwrap();
    assert_eq!(answered(&writer, "SELECT b FROM emptied"), [""]);
    assert_eq!(
        writer.run(b"UPDATE p SET x=13 WHERE x=3"),
        Err(Error::Foreign)
    );
    // An `UPDATE` that leaves the columns pointed at alone leaves the
    // rows that point alone.
    writer.run(b"UPDATE p SET y='III' WHERE x=3").unwrap();
    assert_eq!(answered(&writer, "SELECT b FROM held"), ["3"]);
}

#[test]
fn a_row_the_key_of_which_is_null_is_pointed_at_by_nothing() {
    // The row of the table pointed at has nothing in the column that is
    // pointed at, so no row points at it and it goes alone.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE u(x UNIQUE, y)",
        "CREATE TABLE c(a, b REFERENCES u(x) ON DELETE CASCADE)",
        "INSERT INTO u VALUES(NULL,'n'),(1,'one')",
        "INSERT INTO c VALUES('a',1)",
    ])
    .unwrap();
    writer.run(b"DELETE FROM u WHERE y='n'").unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM c"), ["1"]);
}

#[test]
fn a_row_that_points_and_keeps_its_rows_in_the_key_of_its_own_is_written_again() {
    // The column the key of the table that points is another name for
    // takes no place in the record, so the row is written again with
    // the key left out of it.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE p(x PRIMARY KEY)",
        "CREATE TABLE k(id INTEGER PRIMARY KEY, b REFERENCES p(x) ON UPDATE CASCADE)",
        "INSERT INTO p VALUES(1)",
        "INSERT INTO k VALUES(7,1)",
    ])
    .unwrap();
    writer.run(b"UPDATE p SET x=2").unwrap();
    assert_eq!(answered(&writer, "SELECT id, b FROM k"), ["7", "2"]);
}

#[test]
fn a_cascade_reaches_the_rows_that_point_at_the_rows_it_takes() {
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE a(x PRIMARY KEY)",
        "CREATE TABLE b(x PRIMARY KEY, y REFERENCES a ON DELETE CASCADE)",
        "CREATE TABLE c(x, y REFERENCES b ON DELETE CASCADE)",
        "INSERT INTO a VALUES(1)",
        "INSERT INTO b VALUES(2,1)",
        "INSERT INTO c VALUES(3,2)",
    ])
    .unwrap();
    writer.run(b"DELETE FROM a").unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM b"), ["0"]);
    assert_eq!(answered(&writer, "SELECT count(*) FROM c"), ["0"]);
}

#[test]
fn the_foreign_keys_a_table_carries_are_answered_newest_first() {
    let (mut writer, _) = ran(&[
        "CREATE TABLE p(x PRIMARY KEY, y)",
        "CREATE TABLE two(u, v, PRIMARY KEY(u,v))",
        "CREATE TABLE c(a, b REFERENCES p(x) ON DELETE CASCADE, d REFERENCES p)",
        "CREATE TABLE wide(a, b, FOREIGN KEY(a,b) REFERENCES two)",
    ])
    .unwrap();
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_list(c)"),
        [
            "0",
            "0",
            "p",
            "d",
            "",
            "NO ACTION",
            "NO ACTION",
            "NONE",
            "1",
            "0",
            "p",
            "b",
            "x",
            "NO ACTION",
            "CASCADE",
            "NONE"
        ]
    );
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_list(wide)"),
        [
            "0",
            "0",
            "two",
            "a",
            "",
            "NO ACTION",
            "NO ACTION",
            "NONE",
            "0",
            "1",
            "two",
            "b",
            "",
            "NO ACTION",
            "NO ACTION",
            "NONE"
        ]
    );
    // The three actions the list writes out beside the two it writes
    // as `NO ACTION`.
    let (mut writer, _) = ran(&[
        "CREATE TABLE p(x PRIMARY KEY)",
        "CREATE TABLE ways(a REFERENCES p ON DELETE SET NULL ON UPDATE SET DEFAULT, \
         b REFERENCES p ON DELETE RESTRICT)",
    ])
    .unwrap();
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_list(ways)"),
        [
            "0",
            "0",
            "p",
            "b",
            "",
            "NO ACTION",
            "RESTRICT",
            "NONE",
            "1",
            "0",
            "p",
            "a",
            "",
            "SET DEFAULT",
            "SET NULL",
            "NONE"
        ]
    );
    let (mut writer, _) = ran(&[
        "CREATE TABLE p(x PRIMARY KEY, y)",
        "CREATE TABLE two(u, v, PRIMARY KEY(u,v))",
        "CREATE TABLE c(a, b REFERENCES p(x) ON DELETE CASCADE, d REFERENCES p)",
    ])
    .unwrap();
    // A table with no foreign key, and a name no table carries, answer
    // no row.
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_list(p)"),
        Vec::<String>::new()
    );
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_list(nowhere)"),
        Vec::<String>::new()
    );
}

#[test]
fn the_rows_that_point_at_no_row_are_answered_whatever_the_pragma_says() {
    let (mut writer, _) = ran(&[
        "CREATE TABLE p(x PRIMARY KEY, y)",
        "CREATE TABLE c(a, b REFERENCES p(x))",
        "CREATE TABLE d(a, b REFERENCES missing(x))",
        "INSERT INTO p VALUES(1,'one')",
        "INSERT INTO c VALUES('a',1),('b',9),('c',NULL)",
    ])
    .unwrap();
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_check"),
        ["c", "2", "p", "0"]
    );
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_check(c)"),
        ["c", "2", "p", "0"]
    );
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_check(p)"),
        Vec::<String>::new()
    );
}

#[test]
fn the_text_a_refusal_is_written_as() {
    assert_eq!(Error::Foreign.message(), "FOREIGN KEY constraint failed");
    assert_eq!(
        Error::ForeignMismatch(b"child".to_vec(), b"parent".to_vec()).message(),
        "foreign key mismatch - \"child\" referencing \"parent\""
    );
    assert_eq!(
        Error::Unique(b"t1.a, t1.b".to_vec()).message(),
        "UNIQUE constraint failed: t1.a, t1.b"
    );
    assert_eq!(
        Error::NotNull(b"t4.a".to_vec()).message(),
        "NOT NULL constraint failed: t4.a"
    );
    assert_eq!(
        Error::Check(b"a>0".to_vec()).message(),
        "CHECK constraint failed: a>0"
    );
    assert_eq!(
        Error::Nested.message(),
        "cannot start a transaction within a transaction"
    );
    assert_eq!(
        Error::NoTransaction.message(),
        "cannot commit - no transaction is active"
    );
    assert_eq!(
        Error::Recursion.message(),
        "recursive aggregate queries not supported"
    );
    assert_eq!(
        Error::NoTable(b"t9".to_vec()).message(),
        "no such table: t9"
    );
    // A statement that names a table the schema does not hold says
    // which name it named, whatever the statement does with it.
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    for sql in [
        "UPDATE nosuch SET a=1",
        "DELETE FROM nosuch",
        "INSERT INTO nosuch VALUES(1)",
        "ALTER TABLE nosuch ADD COLUMN c",
    ] {
        assert_eq!(
            writer
                .run(sql.as_bytes())
                .err()
                .map(|error| error.message()),
            Some(alloc::string::String::from("no such table: nosuch")),
            "{sql}"
        );
    }
    assert_eq!(
        Error::Eval(crate::eval::Error::NoFunction(b"xyzzy".to_vec())).message(),
        "no such function: xyzzy"
    );
    assert_eq!(
        Error::Eval(crate::eval::Error::WrongArguments(b"abs".to_vec())).message(),
        "wrong number of arguments to function abs()"
    );
    assert_eq!(
        Error::Eval(crate::eval::Error::NoColumn(b"t.a".to_vec())).message(),
        "no such column: t.a"
    );
    assert_eq!(
        Error::Eval(crate::eval::Error::NoCollation(b"zz".to_vec())).message(),
        "no such collation sequence: zz"
    );
    // A refusal this crate has no text for is written as its name.
    assert_eq!(Error::Having.message(), "Having");
    assert_eq!(
        Error::Eval(crate::eval::Error::Unsupported).message(),
        "Unsupported"
    );
}

#[test]
fn the_column_of_the_parent_says_how_the_keys_are_compared() {
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys = ON",
        "CREATE TABLE t1(a COLLATE nocase PRIMARY KEY)",
        "CREATE TABLE t2(b REFERENCES t1)",
        "INSERT INTO t1 VALUES('ONE')",
        "INSERT INTO t2 VALUES('OnE')",
    ])
    .unwrap();
    // The child column compares under `BINARY` and the parent's under
    // `NOCASE`, so the row that points is found under the parent's.
    assert_eq!(
        writer
            .run(b"DELETE FROM t1 WHERE rowid=1")
            .unwrap_err()
            .message(),
        "FOREIGN KEY constraint failed"
    );
}

#[test]
fn an_index_of_the_parents_own_says_the_columns_are_a_key() {
    // A unique index over the columns pointed at is enough.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys = ON",
        "CREATE TABLE t1(x)",
        "CREATE UNIQUE INDEX t1i ON t1(x)",
        "CREATE TABLE t2(a REFERENCES t1(x))",
    ])
    .unwrap();
    assert!(writer.run(b"INSERT INTO t2 VALUES(NULL)").is_ok());
    // An index held in another collation than the column compares
    // under is not, and neither is a table with no key at all; the
    // refusal is read where the statement is read, so a row that holds
    // nothing reaches it.
    for made in [
        "CREATE UNIQUE INDEX u1i ON u1(x COLLATE nocase)",
        "CREATE INDEX u1i ON u1(x)",
    ] {
        let (mut writer, _) = ran(&[
            "PRAGMA foreign_keys = ON",
            "CREATE TABLE u1(x)",
            made,
            "CREATE TABLE u2(a REFERENCES u1(x))",
        ])
        .unwrap();
        assert_eq!(
            writer
                .run(b"INSERT INTO u2 VALUES(NULL)")
                .unwrap_err()
                .message(),
            "foreign key mismatch - \"u2\" referencing \"u1\"",
            "{made}"
        );
    }
}

#[test]
fn the_foreign_keys_pragma_stands_while_a_transaction_is_open() {
    let (mut writer, _) = ran(&["PRAGMA foreign_keys = ON", "BEGIN"]).unwrap();
    // `PragTyp_FLAG` takes the flag out of the mask while the
    // connection has a transaction open, so the statement changes
    // nothing.
    writer.run(b"PRAGMA foreign_keys = OFF").unwrap();
    assert_eq!(
        writer.run(b"PRAGMA foreign_keys").unwrap(),
        [[Value::Int(1)]]
    );
    // Every other pragma the connection keeps is set there all the
    // same, because the mask holds only that one flag back.
    writer.run(b"PRAGMA cache_size = 100").unwrap();
    assert_eq!(
        writer.run(b"PRAGMA cache_size").unwrap(),
        [[Value::Int(100)]]
    );
    writer.run(b"COMMIT").unwrap();
    writer.run(b"PRAGMA foreign_keys = OFF").unwrap();
    assert_eq!(
        writer.run(b"PRAGMA foreign_keys").unwrap(),
        [[Value::Int(0)]]
    );
}

#[test]
fn a_row_written_into_the_parent_is_one_the_child_may_point_at() {
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys = on",
        "CREATE TABLE t1(a PRIMARY KEY, b)",
        "CREATE TABLE t2(c REFERENCES t1(a), d)",
        "CREATE TABLE t3(a PRIMARY KEY, b)",
        "CREATE TABLE t4(c REFERENCES t3, d)",
        "CREATE TABLE t7(a, b INTEGER PRIMARY KEY)",
        "CREATE TABLE t8(c REFERENCES t7, d)",
        "CREATE TABLE t9(a REFERENCES nosuchtable, b)",
        "CREATE TABLE t10(a REFERENCES t9(c), b)",
    ])
    .unwrap();
    for (sql, refused) in [
        ("INSERT INTO t2 VALUES(1, 3)", true),
        ("INSERT INTO t1 VALUES(1, 2)", false),
        ("INSERT INTO t2 VALUES(1, 3)", false),
        ("INSERT INTO t2 VALUES(2, 4)", true),
        ("INSERT INTO t2 VALUES(NULL, 4)", false),
        ("UPDATE t2 SET c=2 WHERE d=4", true),
        ("UPDATE t2 SET c=1 WHERE d=4", false),
        ("UPDATE t2 SET c=NULL WHERE d=4", false),
        ("DELETE FROM t1 WHERE a=1", true),
        ("UPDATE t1 SET a = 2", true),
        ("UPDATE t1 SET a = 1", false),
        ("INSERT INTO t4 VALUES(1, 3)", true),
        ("INSERT INTO t3 VALUES(1, 2)", false),
        ("INSERT INTO t4 VALUES(1, 3)", false),
        ("INSERT INTO t8 VALUES(1, 3)", true),
        ("INSERT INTO t7 VALUES(2, 1)", false),
        ("INSERT INTO t8 VALUES(1, 3)", false),
    ] {
        let answer = writer.run(sql.as_bytes());
        let shown = answer.as_ref().err().map(crate::db::Error::message);
        assert_eq!(answer.is_err(), refused, "{sql}: {shown:?}");
    }
}

#[test]
fn a_table_that_keeps_its_rows_in_the_keys_own_tree_holds_a_foreign_key() {
    // The rows a key points at, and the rows that point, are read by
    // the key of each row and not by a rowid, so a table that keeps its
    // rows in the key's own tree is held to the key like any other.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE p(a PRIMARY KEY, b) WITHOUT ROWID",
        "CREATE TABLE c(x REFERENCES p(a) ON DELETE CASCADE, y)",
    ])
    .unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO c VALUES(9,'z')")
            .unwrap_err()
            .message(),
        "FOREIGN KEY constraint failed"
    );
    let rows = |writer: &Writer, sql: &[u8]| {
        let image = writer.written();
        let database = Database::open(&image).unwrap();
        database.query(sql).unwrap().rows
    };
    // `ON DELETE CASCADE` takes the rows that point away with the row
    // they pointed at.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE p(a PRIMARY KEY, b) WITHOUT ROWID",
        "CREATE TABLE c(x PRIMARY KEY REFERENCES p(a) ON DELETE CASCADE, y) WITHOUT ROWID",
        "INSERT INTO p VALUES(1,'one'),(2,'two')",
        "INSERT INTO c VALUES(1,'a'),(2,'b')",
    ])
    .unwrap();
    writer.run(b"DELETE FROM p WHERE a=1").unwrap();
    assert_eq!(
        rows(&writer, b"SELECT x,y FROM c"),
        [[Value::Int(2), Value::Text(b"b".to_vec())]]
    );
    // `ON DELETE SET NULL` writes nothing into the columns that point.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE p(a PRIMARY KEY, b) WITHOUT ROWID",
        "CREATE TABLE c(k PRIMARY KEY, x REFERENCES p(a) ON DELETE SET NULL) WITHOUT ROWID",
        "INSERT INTO p VALUES(1,'one')",
        "INSERT INTO c VALUES(7,1)",
    ])
    .unwrap();
    writer.run(b"DELETE FROM p WHERE a=1").unwrap();
    assert_eq!(
        rows(&writer, b"SELECT k,x FROM c"),
        [[Value::Int(7), Value::Null]]
    );
    assert_eq!(
        rows(&writer, b"PRAGMA integrity_check")
            .first()
            .and_then(|row| row.first().and_then(crate::value::Value::text)),
        Some(b"ok".to_vec())
    );
}

/// A key that points at no key of the table it names is refused where
/// the statement is read, so a statement that reaches no row is refused
/// all the same.
#[test]
fn a_key_that_points_at_no_key_is_refused_before_a_row_is_read() {
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE p2(a, b, UNIQUE(a, b))",
        "CREATE TABLE c2(c, d, FOREIGN KEY(c, d) REFERENCES p2(a, x))",
    ])
    .unwrap();
    for sql in [
        b"UPDATE c2 SET c = 1, d = 2".as_slice(),
        b"DELETE FROM c2",
        b"DELETE FROM p2",
        b"UPDATE p2 SET a = 1, b = 2",
        b"INSERT INTO p2 SELECT 1, 2",
    ] {
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "foreign key mismatch - \"c2\" referencing \"p2\"",
            "{sql:?}"
        );
    }
}

/// A statement over a view, and one over a table the schema does not
/// hold, reads no key of its own and is refused by what it names.
#[test]
fn what_a_statement_over_no_table_reads_of_the_keys() {
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE t(a)",
        "CREATE VIEW v AS SELECT a FROM t",
    ])
    .unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO v VALUES(1)")
            .unwrap_err()
            .message(),
        "cannot modify v because it is a view"
    );
    assert_eq!(
        writer.run(b"DELETE FROM nosuch").unwrap_err().message(),
        "no such table: nosuch"
    );
}

/// The columns a key points at are a key of the table it names in
/// whatever order they were written in, which is what a `UNIQUE` over
/// them the other way round is.
#[test]
fn what_order_the_columns_a_key_points_at_stand_in() {
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE parent(x, y, UNIQUE(y, x))",
        "CREATE TABLE c1(a, b, FOREIGN KEY(a, b) REFERENCES parent(x, y))",
    ])
    .unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO c1 VALUES(1, 2)")
            .unwrap_err()
            .message(),
        "FOREIGN KEY constraint failed"
    );
    writer.run(b"INSERT INTO parent VALUES(1, 2)").unwrap();
    writer.run(b"INSERT INTO c1 VALUES(1, 2)").unwrap();
}

/// `fkey2-3.1.3` of `test/fkey2.test`: a row an `ON UPDATE CASCADE`
/// writes is held to the `CHECK` of its table and carries the chain of
/// keys past the table it wrote.
#[test]
fn a_row_a_cascade_writes_is_held_to_the_constraints_of_its_table() {
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE ab(a PRIMARY KEY, b)",
        "CREATE TABLE cd(c PRIMARY KEY REFERENCES ab ON UPDATE CASCADE ON DELETE CASCADE, d)",
        "CREATE TABLE ef(e REFERENCES cd ON UPDATE CASCADE, f, CHECK (e!=5))",
        "INSERT INTO ab VALUES(1, 'b')",
        "INSERT INTO cd VALUES(1, 'd')",
        "INSERT INTO ef VALUES(1, 'e')",
    ])
    .unwrap();
    // The chain reaches `ef`, whose `CHECK` refuses the value.
    assert!(writer.run(b"UPDATE ab SET a = 5").is_err());
    assert_eq!(answered(&writer, "SELECT a FROM ab"), ["1"]);
    // A value the `CHECK` holds carries the whole chain.
    writer.run(b"UPDATE ab SET a = 6").unwrap();
    assert_eq!(answered(&writer, "SELECT c FROM cd"), ["6"]);
    assert_eq!(answered(&writer, "SELECT e FROM ef"), ["6"]);
    // `fkey2-3.2.1`: a row `ef` points at goes with the row `cd`
    // points at, and `ef` names no action, so the delete is refused.
    assert_eq!(writer.run(b"DELETE FROM ab"), Err(Error::Foreign));
}

/// `SQLITE_MAX_TRIGGER_DEPTH`: a chain of keys that reaches deeper than
/// a trigger's body may is refused.
#[test]
fn a_chain_of_cascades_deeper_than_a_trigger_may_reach_is_refused() {
    let mut statements = alloc::vec![
        String::from("PRAGMA foreign_keys=ON"),
        String::from("CREATE TABLE t0(a PRIMARY KEY)"),
        String::from("INSERT INTO t0 VALUES(1)"),
    ];
    for at in 1..40 {
        statements.push(alloc::format!(
            "CREATE TABLE t{at}(a PRIMARY KEY REFERENCES t{} ON UPDATE CASCADE)",
            at - 1
        ));
        statements.push(alloc::format!("INSERT INTO t{at} VALUES(1)"));
    }
    let borrowed: alloc::vec::Vec<&str> = statements.iter().map(String::as_str).collect();
    let (mut writer, _) = ran(&borrowed).unwrap();
    assert_eq!(writer.run(b"UPDATE t0 SET a = 2"), Err(Error::Unsupported));
}
