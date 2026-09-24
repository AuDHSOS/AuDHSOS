// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `INSERT ... ON CONFLICT`, against what the shell answers for the
//! same statements.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// A connection over a table of a key, a unique column and a value.
fn writer() -> Writer {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER PRIMARY KEY, b UNIQUE, c)")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x',10)").unwrap();
    writer
}

/// The rows of the table, as `rowid|a|b|c` lines.
fn rows(writer: &Writer) -> alloc::string::String {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let mut shown = alloc::string::String::new();
    for (rowid, values) in database.rows_of(b"t").unwrap() {
        shown.push_str(&alloc::string::ToString::to_string(&rowid));
        for value in &values {
            shown.push('|');
            shown.push_str(&alloc::string::String::from_utf8_lossy(
                &value.text().unwrap_or(b"NULL".to_vec()),
            ));
        }
        shown.push('\n');
    }
    shown
}

#[test]
fn a_clause_that_does_nothing_passes_the_row_over() {
    let mut writer = writer();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT DO NOTHING")
            .unwrap(),
        Vec::<Vec<Value>>::new()
    );
    assert_eq!(rows(&writer), "1|1|x|10\n");
    // A row that shares no key is written.
    writer
        .run(b"INSERT INTO t VALUES(2,'y',20) ON CONFLICT DO NOTHING")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|10\n2|2|y|20\n");
}

#[test]
fn a_clause_that_writes_writes_the_row_the_conflict_found() {
    let mut writer = writer();
    // The key the table is written under is the one the clause names.
    writer
        .run(b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a) DO UPDATE SET c=excluded.c")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|20\n");
    // The unique column is the other key, and the row the conflict
    // found is the one the clause reads its columns from.
    writer
        .run(b"INSERT INTO t VALUES(2,'x',30) ON CONFLICT(b) DO UPDATE SET c=c+1")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|21\n");
    // A `WHERE` the row does not hold for passes it over.
    writer
        .run(b"INSERT INTO t VALUES(2,'x',30) ON CONFLICT(b) DO UPDATE SET c=99 WHERE c<0")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|21\n");
}

#[test]
fn the_clause_a_conflict_reaches_is_the_one_over_the_key_it_shares() {
    let mut writer = writer();
    writer.run(b"INSERT INTO t VALUES(2,'y',20)").unwrap();
    // The row shares the key of the table with one row and the unique
    // column with another, and each clause writes its own.
    writer
        .run(
            b"INSERT INTO t VALUES(1,'y',30) ON CONFLICT(a) DO UPDATE SET c=1 \
              ON CONFLICT(b) DO UPDATE SET c=2",
        )
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|1\n2|2|y|20\n");
    // A clause over another key leaves the conflict to the constraint.
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,'z',40) ON CONFLICT(b) DO NOTHING")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.a"
    );
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(9,'x',40) ON CONFLICT(a) DO NOTHING")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.b"
    );
}

#[test]
fn a_clause_that_names_the_columns_of_no_key_is_refused() {
    let mut writer = writer();
    for sql in [
        b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(c) DO NOTHING".as_slice(),
        b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a,b) DO NOTHING",
        b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a) DO UPDATE SET c=1 ON CONFLICT(c) DO NOTHING",
    ] {
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint",
            "{sql:?}"
        );
    }
    // The row is left as it was, because the statement is refused
    // before it writes.
    assert_eq!(rows(&writer), "1|1|x|10\n");
}

#[test]
fn a_column_the_row_of_the_conflict_does_not_hold_is_refused() {
    let mut writer = writer();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a) DO UPDATE SET c=excluded.zz")
            .unwrap_err()
            .message(),
        "no such column: excluded.zz"
    );
}

#[test]
fn a_clause_that_writes_holds_the_row_to_the_constraints_of_the_table() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER PRIMARY KEY, b UNIQUE, c NOT NULL)")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x',10)").unwrap();
    writer.run(b"INSERT INTO t VALUES(2,'y',20)").unwrap();
    // The row the clause writes carries the unique column of another
    // row, which the constraint refuses.
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,'z',30) ON CONFLICT(a) DO UPDATE SET b='y'")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.b"
    );
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,'z',30) ON CONFLICT(a) DO UPDATE SET c=NULL")
            .unwrap_err()
            .message(),
        "NOT NULL constraint failed: t.c"
    );
}

#[test]
fn what_the_parser_reads_of_a_clause() {
    let sql = b"INSERT INTO t(a) VALUES(1) ON CONFLICT(a COLLATE NOCASE DESC) WHERE a>0 \
                DO UPDATE SET b=excluded.b WHERE b<5 ON CONFLICT DO NOTHING";
    let (arena, change) = crate::parse::change(sql).unwrap();
    let crate::ast::Change::Insert(statement) = change else {
        panic!("an insert was written");
    };
    let clauses = arena.upserts(statement.upserts);
    assert_eq!(clauses.len(), 2);
    assert_eq!(arena.names(clauses[0].targets).len(), 1);
    assert!(clauses[0].over.is_some());
    assert!(clauses[0].writes);
    assert!(clauses[0].filter.is_some());
    assert!(arena.names(clauses[1].targets).is_empty());
    assert!(!clauses[1].writes);
    for sql in [
        b"INSERT INTO t VALUES(1) ON CONFLICT".as_slice(),
        b"INSERT INTO t VALUES(1) ON CONFLICT DO",
        b"INSERT INTO t VALUES(1) ON CONFLICT(a) DO UPDATE",
        b"INSERT INTO t VALUES(1) ON CONFLICT(a) DO UPDATE SET",
        b"INSERT INTO t VALUES(1) ON CONFLICT() DO NOTHING",
    ] {
        assert!(crate::parse::change(sql).is_err(), "{sql:?}");
    }
}

/// A clause over a table that keeps its rows in the key's own tree
/// names the `PRIMARY KEY` of the table or an index over it, and the
/// row it writes may write the key itself.
#[test]
fn a_clause_writes_the_row_of_a_table_that_keeps_its_rows_in_the_key_s_own_tree() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE k(a TEXT PRIMARY KEY, b, c UNIQUE) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO k VALUES('x',1,'p')").unwrap();
    let shown = |writer: &Writer| -> alloc::string::String {
        let image = writer.written();
        let database = Database::open(&image).unwrap();
        let answered = database.query(b"SELECT a,b,c FROM k").unwrap();
        let mut out = alloc::string::String::new();
        for row in &answered.rows {
            for value in row {
                out.push_str(&alloc::string::String::from_utf8_lossy(
                    &value.text().unwrap_or(b"NULL".to_vec()),
                ));
                out.push('|');
            }
        }
        out
    };
    // `DO NOTHING` over the key of the table passes the row over.
    writer
        .run(b"INSERT INTO k VALUES('x',2,'q') ON CONFLICT(a) DO NOTHING")
        .unwrap();
    assert_eq!(shown(&writer), "x|1|p|");
    // `DO UPDATE` reads the row the conflict found under the name of
    // the table and the row that was not written under `excluded`.
    writer
        .run(b"INSERT INTO k VALUES('x',3,'r') ON CONFLICT(a) DO UPDATE SET b=excluded.b, c=k.c||'!'")
        .unwrap();
    assert_eq!(shown(&writer), "x|3|p!|");
    // A clause that names an index over the table reaches the row that
    // index found.
    writer
        .run(b"INSERT INTO k VALUES('y',4,'p!') ON CONFLICT(c) DO UPDATE SET b=b+100")
        .unwrap();
    assert_eq!(shown(&writer), "x|103|p!|");
    // A clause that writes the key writes the entry under the new key.
    writer
        .run(b"INSERT INTO k VALUES('x',9,'z') ON CONFLICT(a) DO UPDATE SET a='w'")
        .unwrap();
    assert_eq!(shown(&writer), "w|103|p!|");
}

#[test]
fn a_clause_writes_the_row_of_a_table_that_has_no_key_of_its_own() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE u(b UNIQUE, c)").unwrap();
    writer.run(b"INSERT INTO u VALUES('x',1)").unwrap();
    // The row the conflict found answers under the name of the table
    // and under `rowid`, and the row that was not written answers under
    // `excluded`.
    writer
        .run(b"INSERT INTO u VALUES('x',2) ON CONFLICT(b) DO UPDATE SET c=hex(excluded.c)||u.c||rowid")
        .unwrap();
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let held = database.rows_of(b"u").unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(
        held.first().and_then(|(_, values)| values.get(1)),
        Some(&Value::Text(b"3211".to_vec()))
    );
    // A name of another table is no column of either row.
    assert_eq!(
        writer
            .run(b"INSERT INTO u VALUES('x',2) ON CONFLICT(b) DO UPDATE SET c=q.c")
            .unwrap_err()
            .message(),
        "no such column: q.c"
    );
    // A column the table does not hold is no column to write either.
    assert_eq!(
        writer
            .run(b"INSERT INTO u VALUES('x',2) ON CONFLICT(b) DO UPDATE SET zz=1")
            .unwrap_err()
            .message(),
        "no such column: zz"
    );
}

#[test]
fn a_clause_that_writes_the_key_moves_the_row() {
    let mut writer = writer();
    writer
        .run(b"INSERT INTO t VALUES(1,'z',5) ON CONFLICT(a) DO UPDATE SET a=7")
        .unwrap();
    assert_eq!(rows(&writer), "7|7|x|10\n");
    // The key another row holds is refused.
    writer.run(b"INSERT INTO t VALUES(2,'y',20)").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(7,'q',30) ON CONFLICT(a) DO UPDATE SET a=2")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.a"
    );
}

#[test]
fn a_trigger_that_says_the_row_stays_leaves_the_clause_nothing_to_write() {
    let mut writer = writer();
    writer
        .run(b"CREATE TRIGGER tr BEFORE UPDATE ON t BEGIN SELECT RAISE(IGNORE); END")
        .unwrap();
    writer
        .run(b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a) DO UPDATE SET c=99")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|10\n");
    // A trigger that says nothing leaves the row written, and the
    // trigger after it runs over what was written.
    writer.run(b"DROP TRIGGER tr").unwrap();
    writer.run(b"CREATE TABLE log(what)").unwrap();
    writer
        .run(b"CREATE TRIGGER tr AFTER UPDATE ON t BEGIN INSERT INTO log VALUES(new.c); END")
        .unwrap();
    writer
        .run(b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a) DO UPDATE SET c=99")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|99\n");
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    assert_eq!(
        database
            .rows_of(b"log")
            .unwrap()
            .first()
            .and_then(|(_, values)| values.first()),
        Some(&Value::Int(99))
    );
}

#[test]
fn the_row_the_statement_would_have_written_carries_the_key_it_was_given() {
    let mut writer = writer();
    writer
        .run(b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a ASC) DO UPDATE SET b='z', c=excluded.rowid WHERE c>0")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|z|1\n");
    // A key that is no whole number is refused, which is what the key
    // of a table answers for a value of another type.
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,'q',30) ON CONFLICT(a) DO UPDATE SET a='abc'")
            .unwrap_err()
            .message(),
        "datatype mismatch"
    );
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES('abc',2,3)")
            .unwrap_err()
            .message(),
        "datatype mismatch"
    );
    assert_eq!(
        writer.run(b"UPDATE t SET a='abc'").unwrap_err().message(),
        "datatype mismatch"
    );
}

/// A statement written inside an expression of a statement that writes,
/// which is what `sqlite3ExprCodeSubselect` answers there.
#[test]
fn a_statement_that_writes_reads_the_statements_written_inside_it() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(a PRIMARY KEY, b)".as_slice(),
        b"CREATE TABLE chng(a PRIMARY KEY, b)",
        b"INSERT INTO t1 VALUES(1,1),(2,2),(3,3),(4,4)",
        b"INSERT INTO chng VALUES(2,3),(4,5)",
    ] {
        writer.run(sql).unwrap();
    }
    // A row that takes the key of a row the statement has not reached
    // is refused, and the statement leaves the rows as they were.
    assert_eq!(
        writer
            .run(b"UPDATE t1 SET a=(SELECT b FROM chng WHERE a=t1.a)")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t1.a"
    );
    writer
        .run(b"UPDATE t1 SET b=7 WHERE a IN (SELECT a FROM chng)")
        .unwrap();
    // Under `OR REPLACE` the row that held the key is taken out, and
    // the statement passes it over when it reaches it, which is ticket
    // #2832.
    writer
        .run(b"UPDATE OR REPLACE t1 SET a=(SELECT b FROM chng WHERE a=t1.a)")
        .unwrap();
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answer = database
        .query(b"SELECT quote(a)||'/'||b FROM t1 ORDER BY a")
        .unwrap();
    let shown: Vec<Vec<u8>> = answer
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(crate::value::Value::text))
        .collect();
    assert_eq!(
        shown,
        alloc::vec![b"NULL/1".to_vec(), b"3/7".to_vec(), b"5/7".to_vec()]
    );
    // The index of the key holds one entry per row, which is what the
    // walk of the file answers.
    assert_eq!(
        database
            .query(b"PRAGMA integrity_check")
            .unwrap()
            .rows
            .first()
            .and_then(|row| row.first()),
        Some(&Value::Text(b"ok".to_vec()))
    );
}

#[test]
fn a_table_that_keeps_its_rows_in_the_key_s_own_tree_reads_them_the_same_way() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE k(a PRIMARY KEY, b) WITHOUT ROWID".as_slice(),
        b"CREATE TABLE chng(a PRIMARY KEY, b)",
        b"INSERT INTO k VALUES(1,1),(2,2),(3,3),(4,4)",
        b"INSERT INTO chng VALUES(2,3),(4,5)",
    ] {
        writer.run(sql).unwrap();
    }
    // The key of such a table refuses nothing, so a row whose key the
    // statement written inside it answers nothing for is refused.
    assert_eq!(
        writer
            .run(b"UPDATE OR REPLACE k SET a=(SELECT b FROM chng WHERE a=k.a)")
            .unwrap_err()
            .message(),
        "NOT NULL constraint failed: k.a"
    );
    writer
        .run(b"UPDATE k SET b=9 WHERE a IN (SELECT a FROM chng)")
        .unwrap();
    writer
        .run(b"DELETE FROM k WHERE a IN (SELECT a FROM chng WHERE b=3)")
        .unwrap();
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answer = database
        .query(b"SELECT quote(a)||'/'||quote(b) FROM k ORDER BY a")
        .unwrap();
    let shown: Vec<Vec<u8>> = answer
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(crate::value::Value::text))
        .collect();
    assert_eq!(
        shown,
        alloc::vec![b"1/1".to_vec(), b"3/3".to_vec(), b"4/9".to_vec()]
    );
    assert_eq!(
        database
            .query(b"PRAGMA integrity_check")
            .unwrap()
            .rows
            .first()
            .and_then(|row| row.first()),
        Some(&Value::Text(b"ok".to_vec()))
    );
}

/// A row a statement took out on the way, in a table that keeps its
/// rows in the key's own tree.
///
/// The C library walks the tree itself and reaches a row again where
/// the write moved it past the walk, so `UPDATE OR REPLACE k SET a=a+1`
/// leaves it one row of the key 5; this crate reads the rows once, so
/// each row is written once and the rows it took out are passed over.
/// The file holds what it says either way, which is what the walk of
/// `PRAGMA integrity_check` answers.
#[test]
fn a_row_the_statement_took_out_of_the_key_s_own_tree_is_passed_over() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE k(a PRIMARY KEY, b) WITHOUT ROWID".as_slice(),
        b"INSERT INTO k VALUES(1,1),(2,2),(3,3),(4,4)",
        b"UPDATE OR REPLACE k SET a=a+1",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answer = database
        .query(b"SELECT a||'/'||b FROM k ORDER BY a")
        .unwrap();
    let shown: Vec<Vec<u8>> = answer
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(crate::value::Value::text))
        .collect();
    assert_eq!(shown, alloc::vec![b"2/1".to_vec(), b"4/3".to_vec()]);
    assert_eq!(
        database
            .query(b"PRAGMA integrity_check")
            .unwrap()
            .rows
            .first()
            .and_then(|row| row.first()),
        Some(&Value::Text(b"ok".to_vec()))
    );
}

#[test]
fn the_key_a_clause_names_is_held_to_its_columns_and_their_collations() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE xyz(a INTEGER PRIMARY KEY,b,c,d)")
        .unwrap();
    writer
        .run(b"CREATE UNIQUE INDEX xyz1 ON xyz(d,c,b COLLATE nocase)")
        .unwrap();
    writer.run(b"INSERT INTO xyz VALUES(10,1,1,'one')").unwrap();
    // A term that writes the collation the index holds the column in,
    // and a term that writes none, both name the key.
    for clause in [
        b"(b COLLATE nocase, c, d)".as_slice(),
        b"(b, c, d)",
        b"",
        b"(b, c, d) WHERE a!=0",
    ] {
        let mut sql = b"INSERT INTO xyz VALUES(11,1,1,'one') ON CONFLICT ".to_vec();
        sql.extend_from_slice(clause);
        sql.extend_from_slice(b" DO NOTHING");
        assert!(writer.run(&sql).is_ok(), "{clause:?}");
    }
    // A term that writes another collation, and a list that names one
    // column twice and another not at all, name no key.
    for clause in [
        b"(b, c COLLATE nocase, d)".as_slice(),
        b"(d, c, c)",
        b"(b COLLATE nocase, c COLLATE nocase, d)",
    ] {
        let mut sql = b"INSERT INTO xyz VALUES(11,1,1,'one') ON CONFLICT ".to_vec();
        sql.extend_from_slice(clause);
        sql.extend_from_slice(b" DO NOTHING");
        assert_eq!(
            writer.run(&sql).unwrap_err().message(),
            "ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint",
            "{clause:?}"
        );
    }
    // The key the index holds is the one the message names.
    assert_eq!(
        writer
            .run(b"INSERT INTO xyz VALUES(11,1,1,'one') ON CONFLICT (a) DO NOTHING")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: xyz.d, xyz.c, xyz.b"
    );
}

#[test]
fn a_row_reaches_the_clause_whose_key_the_statement_names_first() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t1(a INTEGER PRIMARY KEY, b, c UNIQUE, d UNIQUE, e UNIQUE)")
        .unwrap();
    let held = |writer: &Writer| {
        let image = writer.written();
        let database = Database::open(&image).unwrap();
        database.query(b"SELECT a,b,c,d,e FROM t1").unwrap().rows
    };
    // The key of the table stands in the place of the clause that names
    // it, and the indexes no clause names come after the named ones.
    for (clauses, want) in [
        // Every key shares, so the first clause reaches the row.
        (
            b"(1,NULL,3,4,5) ON CONFLICT(a) DO UPDATE SET b='a' \
              ON CONFLICT(c) DO UPDATE SET b='c'"
                .as_slice(),
            b"a".as_slice(),
        ),
        // The key of the table shares as well, and the clause that
        // names `c` stands before the one that names it.
        (
            b"(1,NULL,3,94,95) ON CONFLICT(c) DO UPDATE SET b='c' \
              ON CONFLICT(a) DO UPDATE SET b='a'",
            b"c",
        ),
        // `c` shares nothing, so the row reaches the clause that names
        // `d`, which stands before the key of the table.
        (
            b"(1,NULL,93,4,95) ON CONFLICT(c) DO UPDATE SET b='c' \
              ON CONFLICT(d) DO UPDATE SET b='d' ON CONFLICT(a) DO UPDATE SET b='a'",
            b"d",
        ),
        // The key of the table stands after the named keys where no
        // clause names it, and before the clause that names none.
        (
            b"(1,NULL,93,4,5) ON CONFLICT(c) DO UPDATE SET b='c' \
              ON CONFLICT(d) DO UPDATE SET b='d' ON CONFLICT DO UPDATE SET b='x'",
            b"d",
        ),
        // A clause that names a key an earlier clause named is one no
        // row reaches.
        (
            b"(1,NULL,3,4,5) ON CONFLICT(c) DO UPDATE SET b='c' \
              ON CONFLICT(c) DO UPDATE SET b='z'",
            b"c",
        ),
    ] {
        writer.run(b"DELETE FROM t1").unwrap();
        writer
            .run(b"INSERT INTO t1(a,b,c,d,e) VALUES(1,2,3,4,5)")
            .unwrap();
        let mut sql = b"INSERT INTO t1(a,b,c,d,e) VALUES".to_vec();
        sql.extend_from_slice(clauses);
        writer.run(&sql).unwrap();
        assert_eq!(
            held(&writer).first().and_then(|row| row.get(1).cloned()),
            Some(Value::Text(want.to_vec())),
            "{}",
            alloc::string::String::from_utf8_lossy(clauses)
        );
    }
    // A clause whose term is not a column names no key.
    assert_eq!(
        writer
            .run(b"INSERT INTO t1(a) VALUES(9) ON CONFLICT(a+1) DO NOTHING")
            .unwrap_err()
            .message(),
        "ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint"
    );
}

/// A clause over a table that keeps its rows in the key's own tree
/// writes under `OE_Abort`, which is `sqlite3UpsertDoUpdate` building
/// the `UPDATE` that way, so a key the row it writes shares with
/// another refuses the statement whatever the key's own clause says.
#[test]
fn what_a_clause_over_a_table_with_no_rowid_refuses() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE k(a TEXT PRIMARY KEY ON CONFLICT IGNORE, b) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO k VALUES('x',1),('y',2)").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO k VALUES('x',3) ON CONFLICT(a) DO UPDATE SET a='y'")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: k.a"
    );
    // A column the table does not hold.
    assert_eq!(
        writer
            .run(b"INSERT INTO k VALUES('x',3) ON CONFLICT(a) DO UPDATE SET zz=1")
            .unwrap_err()
            .message(),
        "no such column: zz"
    );
}

/// The `WHERE` of a clause over a table that keeps its rows in the
/// key's own tree passes the row over, and the `UPDATE` triggers of the
/// table run over the row the clause writes.
#[test]
fn a_clause_over_a_table_with_no_rowid_runs_the_update_triggers() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE k(a TEXT PRIMARY KEY, b) WITHOUT ROWID".as_slice(),
        b"CREATE TABLE log(t)",
        b"INSERT INTO k VALUES('x',1)",
        b"CREATE TRIGGER kb BEFORE UPDATE ON k BEGIN INSERT INTO log VALUES('b'||old.b||new.b); END",
        b"CREATE TRIGGER ka AFTER UPDATE ON k BEGIN INSERT INTO log VALUES('a'||old.b||new.b); END",
    ] {
        writer.run(sql).unwrap();
    }
    let shown = |writer: &Writer, sql: &[u8]| -> alloc::string::String {
        let image = writer.written();
        let database = Database::open(&image).unwrap();
        let answered = database.query(sql).unwrap();
        let mut out = alloc::string::String::new();
        for row in &answered.rows {
            for value in row {
                out.push_str(&alloc::string::String::from_utf8_lossy(
                    &value.text().unwrap_or(b"NULL".to_vec()),
                ));
                out.push('|');
            }
        }
        out
    };
    // A `WHERE` the row does not hold for passes it over, so no trigger
    // runs.
    writer
        .run(b"INSERT INTO k VALUES('x',2) ON CONFLICT(a) DO UPDATE SET b=99 WHERE b<0")
        .unwrap();
    assert_eq!(shown(&writer, b"SELECT a,b FROM k"), "x|1|");
    assert_eq!(shown(&writer, b"SELECT t FROM log"), "");
    writer
        .run(b"INSERT INTO k VALUES('x',3) ON CONFLICT(a) DO UPDATE SET b=excluded.b")
        .unwrap();
    assert_eq!(shown(&writer, b"SELECT a,b FROM k"), "x|3|");
    assert_eq!(shown(&writer, b"SELECT t FROM log"), "b13|a13|");
}

/// A table that carries an `AFTER UPDATE` trigger and no `BEFORE` one
/// runs that trigger over the row a clause writes.
#[test]
fn a_clause_over_a_table_with_no_rowid_runs_an_after_trigger_of_its_own() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE k(a TEXT PRIMARY KEY, b) WITHOUT ROWID".as_slice(),
        b"CREATE TABLE log(t)",
        b"INSERT INTO k VALUES('x',1)",
        b"CREATE TRIGGER ka AFTER UPDATE ON k BEGIN INSERT INTO log VALUES(new.b); END",
        b"INSERT INTO k VALUES('x',7) ON CONFLICT(a) DO UPDATE SET b=excluded.b",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answered = database.query(b"SELECT t FROM log").unwrap();
    assert_eq!(answered.rows, alloc::vec![alloc::vec![Value::Int(7)]]);
}

/// A `BEFORE UPDATE` trigger that raises `IGNORE` leaves the row the
/// clause would have written as it stands.
#[test]
fn a_trigger_that_ignores_the_row_a_clause_writes_leaves_it_as_it_stands() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE k(a TEXT PRIMARY KEY, b) WITHOUT ROWID".as_slice(),
        b"INSERT INTO k VALUES('x',1)",
        b"CREATE TRIGGER kb BEFORE UPDATE ON k BEGIN SELECT RAISE(IGNORE); END",
        b"INSERT INTO k VALUES('x',5) ON CONFLICT(a) DO UPDATE SET b=excluded.b",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answered = database.query(b"SELECT b FROM k").unwrap();
    assert_eq!(answered.rows, alloc::vec![alloc::vec![Value::Int(1)]]);
}

/// A clause over a table that keeps its rows in the key's own tree
/// writes where its `WHERE` holds, and its `RETURNING` answers the row
/// it wrote.
#[test]
fn a_clause_over_a_table_with_no_rowid_answers_what_it_wrote() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE k(a TEXT PRIMARY KEY, b) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO k VALUES('x',1)").unwrap();
    assert_eq!(
        writer
            .run(
                b"INSERT INTO k VALUES('x',2) ON CONFLICT(a) DO UPDATE SET b=excluded.b \
                  WHERE b>0 RETURNING a,b"
            )
            .unwrap(),
        alloc::vec![alloc::vec![Value::Text(b"x".to_vec()), Value::Int(2)]]
    );
}

/// How two expressions written as `sql` compare, each parsed into a tree
/// of its own so that the comparison reads two trees.
fn compared(one: &[u8], other: &[u8]) -> crate::ast::Alike {
    let (held, id) = crate::parse::expression(one).unwrap();
    let (beside, at) = crate::parse::expression(other).unwrap();
    crate::ast::alike((&held, id, one), (&beside, at, other))
}

#[test]
fn what_two_expressions_of_two_trees_compare_as() {
    use crate::ast::Alike;
    // The same expression of every kind a place of an index may hold.
    for text in [
        "x",
        "t.x",
        "main.t.x",
        "17",
        "1.5",
        "'one'",
        "x'00'",
        "NULL",
        "CURRENT_DATE",
        "CURRENT_TIMESTAMP",
        "?1",
        "-x",
        "x + 1",
        "x BETWEEN 1 AND 2",
        "x NOT BETWEEN 1 AND 2",
        "x IN (1, 2)",
        "x NOT IN (1, 2)",
        "x LIKE 'a%'",
        "x GLOB 'a*' ",
        "x NOT LIKE 'a%' ESCAPE '\\'",
        "CAST(x AS INTEGER)",
        "x COLLATE nocase",
        "abs(x)",
        "count(DISTINCT x)",
        "count(*)",
        "CASE x WHEN 1 THEN 2 ELSE 3 END",
        "CASE WHEN x THEN 2 END",
        "(x, 1)",
        "max(x) FILTER (WHERE x > 0)",
    ] {
        assert_eq!(
            compared(text.as_bytes(), text.as_bytes()),
            Alike::Same,
            "{text}"
        );
    }
    // A name is read without its quotes and without its case.
    assert_eq!(compared(b"\"X\"", b"x"), Alike::Same);
    // Another expression of the same kind.
    for (one, other) in [
        ("x", "y"),
        ("t.x", "u.x"),
        ("17", "18"),
        ("1.5", "1.6"),
        ("'one'", "'two'"),
        ("x'00'", "x'01'"),
        ("NULL", "CURRENT_DATE"),
        ("CURRENT_DATE", "CURRENT_TIME"),
        ("?1", "?2"),
        ("-x", "+x"),
        ("x + 1", "x - 1"),
        ("x BETWEEN 1 AND 2", "x NOT BETWEEN 1 AND 2"),
        ("x IN (1, 2)", "x NOT IN (1, 2)"),
        ("x LIKE 'a%'", "x GLOB 'a%'"),
        ("CAST(x AS INTEGER)", "CAST(x AS TEXT)"),
        ("x COLLATE nocase", "x COLLATE rtrim"),
        ("abs(x)", "length(x)"),
        ("count(DISTINCT x)", "count(x)"),
        ("count(*)", "count(x)"),
        ("abs(x)", "abs(x, 1)"),
        ("CASE x WHEN 1 THEN 2 END", "CASE WHEN 1 THEN 2 END"),
        ("(x, 1)", "x"),
        ("x", "(SELECT 1)"),
        ("(SELECT 1)", "x"),
        ("(SELECT 1)", "(SELECT 1)"),
        ("EXISTS(SELECT 1)", "EXISTS(SELECT 1)"),
        ("x IN (SELECT 1)", "x IN (SELECT 1)"),
        ("x IN t", "x IN t"),
        ("max(x) OVER ()", "max(x) OVER ()"),
    ] {
        assert_eq!(
            compared(one.as_bytes(), other.as_bytes()),
            Alike::Other,
            "{one} against {other}"
        );
    }
    // A `COLLATE` one side writes and the other does not leaves the
    // expressions the same but for that collation, whichever side writes
    // it.
    assert_eq!(compared(b"x COLLATE nocase", b"x"), Alike::Collated);
    assert_eq!(compared(b"x", b"x COLLATE nocase"), Alike::Collated);
    assert_eq!(compared(b"x COLLATE nocase", b"y"), Alike::Other);
    assert_eq!(compared(b"y", b"x COLLATE nocase"), Alike::Other);
    // A collation one side writes under the node answers the same.
    assert_eq!(
        compared(b"x + (y COLLATE nocase)", b"x + y"),
        Alike::Collated
    );
    assert_eq!(compared(b"x + (y COLLATE nocase)", b"x + z"), Alike::Other);
}

#[test]
fn what_an_on_conflict_clause_that_names_an_expression_or_a_partial_index_writes() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE abc(a INTEGER PRIMARY KEY, x, y)")
        .unwrap();
    writer
        .run(b"CREATE UNIQUE INDEX abc1 ON abc(('x' || x) COLLATE nocase)")
        .unwrap();
    writer
        .run(b"INSERT INTO abc VALUES(1, 'one', 'two')")
        .unwrap();
    // A clause names a place over an expression by writing that
    // expression again, under the collation the place is held in or
    // under none.
    for clause in [
        "ON CONFLICT ('x' || x) DO NOTHING",
        "ON CONFLICT (('x' || x) COLLATE nocase) DO NOTHING",
    ] {
        let mut sql = alloc::string::String::from("INSERT INTO abc VALUES(2, 'one', NULL) ");
        sql.push_str(clause);
        writer.run(sql.as_bytes()).unwrap();
    }
    // A clause that writes another collation, or another expression,
    // names no index of the table.
    for clause in [
        "ON CONFLICT (('x' || x) COLLATE binary) DO NOTHING",
        "ON CONFLICT (x || 'x') DO NOTHING",
        "ON CONFLICT (x) DO NOTHING",
    ] {
        let mut sql = alloc::string::String::from("INSERT INTO abc VALUES(2, 'one', NULL) ");
        sql.push_str(clause);
        assert_eq!(
            writer.run(sql.as_bytes()).map_err(|error| error.message()),
            Err(alloc::string::String::from(
                "ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint"
            )),
            "{clause}"
        );
    }
    let rows = Database::open(&writer.written())
        .unwrap()
        .query(b"SELECT count(*) FROM abc")
        .unwrap();
    assert_eq!(rows.rows, [[Value::Int(1)]]);
    // A partial index is named by a clause that writes its `WHERE`
    // again, and a clause that writes none names no partial index.
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER PRIMARY KEY, x, y)")
        .unwrap();
    writer
        .run(b"CREATE UNIQUE INDEX t1 ON t(x) WHERE y>0")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(1, 'one', 1)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES(2, 'one', 10) ON CONFLICT(x) WHERE y>0 DO NOTHING")
        .unwrap();
    for clause in [
        "ON CONFLICT(x) DO NOTHING",
        "ON CONFLICT(x) WHERE y>=0 DO NOTHING",
    ] {
        let mut sql = alloc::string::String::from("INSERT INTO t VALUES(2, 'one', 10) ");
        sql.push_str(clause);
        assert_eq!(
            writer.run(sql.as_bytes()).map_err(|error| error.message()),
            Err(alloc::string::String::from(
                "ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint"
            )),
            "{clause}"
        );
    }
    // An index that holds more than one key per row is named by no
    // clause at all.
    writer.run(b"CREATE INDEX t2 ON t(y)").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(3, 'two', 1) ON CONFLICT(y) DO NOTHING")
            .map_err(|error| error.message()),
        Err(alloc::string::String::from(
            "ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint"
        ))
    );
}

#[test]
fn what_an_on_conflict_clause_that_writes_a_collation_on_the_key_names() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a TEXT COLLATE nocase, b, PRIMARY KEY(a)) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES('x', 1)").unwrap();
    // A clause that writes the collation the key is held in names that
    // key, and one that writes another names no key of the table.
    writer
        .run(b"INSERT INTO t VALUES('X', 2) ON CONFLICT(a COLLATE nocase) DO NOTHING")
        .unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES('X', 2) ON CONFLICT(a COLLATE binary) DO NOTHING")
            .map_err(|error| error.message()),
        Err(alloc::string::String::from(
            "ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint"
        ))
    );
    let rows = Database::open(&writer.written())
        .unwrap()
        .query(b"SELECT b FROM t")
        .unwrap();
    assert_eq!(rows.rows, [[Value::Int(1)]]);
}
