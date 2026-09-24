// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The authorizer of a connection: what a statement is asked for, what a
//! denial refuses, and what an ignored action leaves undone.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::auth::{Action, Answer, Asked};
use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// Every action, in the order the words of `tclsqlite.c` stand.
static ACTIONS: &[Action] = &[
    Action::CreateIndex,
    Action::CreateTable,
    Action::CreateTempIndex,
    Action::CreateTempTable,
    Action::CreateTempTrigger,
    Action::CreateTempView,
    Action::CreateTrigger,
    Action::CreateView,
    Action::Delete,
    Action::DropIndex,
    Action::DropTable,
    Action::DropTempIndex,
    Action::DropTempTable,
    Action::DropTempTrigger,
    Action::DropTempView,
    Action::DropTrigger,
    Action::DropView,
    Action::Insert,
    Action::Pragma,
    Action::Read,
    Action::Select,
    Action::Transaction,
    Action::Update,
    Action::AlterTable,
    Action::Reindex,
    Action::Analyze,
    Action::Function,
    Action::Savepoint,
    Action::Recursive,
    Action::Attach,
    Action::Detach,
];

/// What the function under test refuses: the place of the action in
/// [`ACTIONS`] plus one, the answer it gives for that action, and the
/// text the fourth argument must carry for the answer to apply.
static RULE: AtomicU32 = AtomicU32::new(0);

/// What the fourth argument must carry, as its first byte, or nought for
/// any fourth argument at all.
static SECOND: AtomicU32 = AtomicU32::new(0);

/// How many actions the function was asked since the last rule was set.
static COUNT: AtomicU32 = AtomicU32::new(0);

/// The function every test tells the connection: it counts the actions
/// it is asked and answers what [`RULE`] names for the one action that
/// rule carries.
fn asking(asked: &Asked<'_>) -> Answer {
    COUNT.fetch_add(1, Ordering::Relaxed);
    let rule = RULE.load(Ordering::Relaxed);
    let (place, answer) = (rule >> 2, rule & 3);
    if place == 0 {
        return Answer::Ok;
    }
    let wanted = ACTIONS
        .get(usize::try_from(place.saturating_sub(1)).unwrap_or(0))
        .copied();
    if wanted != Some(asked.action) {
        return Answer::Ok;
    }
    let byte = SECOND.load(Ordering::Relaxed);
    if byte != 0 && asked.second.first().copied() != u8::try_from(byte).ok() {
        return Answer::Ok;
    }
    match answer {
        1 => Answer::Deny,
        2 => Answer::Ignore,
        _ => Answer::Ok,
    }
}

/// The function answers `answer` for `action` and `Ok` for every other.
fn rule(action: Action, answer: Answer) {
    let place = ACTIONS
        .iter()
        .position(|held| *held == action)
        .unwrap_or_default();
    let code = match answer {
        Answer::Ok => 0,
        Answer::Deny => 1,
        Answer::Ignore => 2,
    };
    let place = u32::try_from(place.saturating_add(1)).unwrap_or(0);
    RULE.store((place << 2) | code, Ordering::Relaxed);
    SECOND.store(0, Ordering::Relaxed);
    COUNT.store(0, Ordering::Relaxed);
}

/// The same, for the one call whose fourth argument begins with `byte`.
fn rule_of(action: Action, answer: Answer, byte: u8) {
    rule(action, answer);
    SECOND.store(u32::from(byte), Ordering::Relaxed);
}

/// The function answers `Ok` for every action.
fn allows() {
    RULE.store(0, Ordering::Relaxed);
    SECOND.store(0, Ordering::Relaxed);
    COUNT.store(0, Ordering::Relaxed);
}

/// A connection over two tables, told the function.
fn told() -> Writer {
    allows();
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t1(a INTEGER PRIMARY KEY, b, c)")
        .unwrap();
    writer.run(b"CREATE TABLE t2(x, y)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1,2,3)").unwrap();
    writer.run(b"INSERT INTO t2 VALUES(4,5)").unwrap();
    writer.asks(asking);
    writer
}

/// What a statement the connection reads answers, against the database
/// the caller opens as the harness of the suite opens one.
fn read(writer: &Writer, sql: &[u8]) -> Result<Vec<Vec<Value>>, alloc::string::String> {
    let bytes = writer.written();
    let database = match writer.asking() {
        Some(asking) => Database::open(&bytes).unwrap().asked(asking),
        None => Database::open(&bytes).unwrap(),
    };
    database
        .query(sql)
        .map(|answered| answered.rows)
        .map_err(|error| error.message())
}

/// Every action carries the word `tclsqlite.c` writes it as, and no two
/// actions carry one word.
#[test]
fn what_word_every_action_is_written_as() {
    assert_eq!(ACTIONS.len(), 31);
    for (at, action) in ACTIONS.iter().enumerate() {
        let word = action.word();
        assert!(word.starts_with(b"SQLITE_"), "a word of its own");
        for other in ACTIONS.iter().skip(at.saturating_add(1)) {
            assert_ne!(word, other.word());
        }
    }
    assert_eq!(Action::Read.word(), b"SQLITE_READ");
    assert_eq!(Action::CreateTempView.word(), b"SQLITE_CREATE_TEMP_VIEW");
}

/// The three refusals carry the messages `sqlite3ErrorMsg` writes.
#[test]
fn what_message_a_refusal_carries() {
    assert_eq!(crate::auth::Error::Denied.message(), "not authorized");
    assert_eq!(
        crate::auth::Error::Prohibited(b"t1.b".to_vec()).message(),
        "access to t1.b is prohibited"
    );
    assert_eq!(
        crate::auth::Error::Function(b"abs".to_vec()).message(),
        "not authorized to use function: abs"
    );
}

/// A connection told no function is asked nothing, and one told a
/// function that allows everything answers what it did.
fn what_a_connection_told_no_function_is_asked() {
    let mut writer = told();
    writer.asks_nothing();
    assert!(writer.asking().is_none());
    COUNT.store(0, Ordering::Relaxed);
    writer.run(b"INSERT INTO t2 VALUES(6,7)").unwrap();
    assert_eq!(read(&writer, b"SELECT count(*) FROM t2").unwrap().len(), 1);
    assert_eq!(COUNT.load(Ordering::Relaxed), 0);
    // The same connection told the function is asked for every action.
    writer.asks(asking);
    allows();
    writer.run(b"INSERT INTO t2 VALUES(8,9)").unwrap();
    assert!(COUNT.load(Ordering::Relaxed) > 0);
}

/// `SQLITE_SELECT` denied refuses the statement and ignored answers no
/// row.
fn what_a_denied_select_answers() {
    let writer = told();
    rule(Action::Select, Answer::Deny);
    assert_eq!(
        read(&writer, b"SELECT a FROM t1").unwrap_err(),
        "not authorized"
    );
    rule(Action::Select, Answer::Ignore);
    assert!(read(&writer, b"SELECT a FROM t1").unwrap().is_empty());
    allows();
    assert_eq!(
        read(&writer, b"SELECT a FROM t1").unwrap(),
        alloc::vec![alloc::vec![Value::Int(1)]]
    );
}

/// `SQLITE_READ` denied names the column, and ignored answers a null for
/// it wherever the statement reads it.
fn what_a_read_the_function_refuses_answers() {
    let writer = told();
    rule_of(Action::Read, Answer::Deny, b'b');
    assert_eq!(
        read(&writer, b"SELECT b FROM t1").unwrap_err(),
        "access to t1.b is prohibited"
    );
    // A `*` reads every column, so the one the function ignores answers
    // a null and the others stand.
    rule_of(Action::Read, Answer::Ignore, b'b');
    assert_eq!(
        read(&writer, b"SELECT * FROM t1").unwrap(),
        alloc::vec![alloc::vec![Value::Int(1), Value::Null, Value::Int(3)]]
    );
    // The `WHERE` reads the null as well, so a row it held for no
    // longer is.
    assert!(
        read(&writer, b"SELECT * FROM t1 WHERE b=2")
            .unwrap()
            .is_empty()
    );
    // A column written with the name of its table, and one written with
    // the name the statement knows the table by.
    assert_eq!(
        read(&writer, b"SELECT t1.b FROM t1").unwrap(),
        alloc::vec![alloc::vec![Value::Null]]
    );
    assert_eq!(
        read(&writer, b"SELECT k.b FROM t1 AS k").unwrap(),
        alloc::vec![alloc::vec![Value::Null]]
    );
    assert_eq!(
        read(&writer, b"SELECT main.t1.b FROM main.t1").unwrap(),
        alloc::vec![alloc::vec![Value::Null]]
    );
    // A `t.*` reads the columns of that table alone.
    assert_eq!(
        read(&writer, b"SELECT t1.* FROM t1, t2").unwrap(),
        alloc::vec![alloc::vec![Value::Int(1), Value::Null, Value::Int(3)]]
    );
}

/// The name a bare `rowid` is asked under is the column the key is
/// another name for, and `ROWID` where the table has none.
fn what_name_a_rowid_is_asked_under() {
    let writer = told();
    // `t1.a` is the column the key is another name for, so `rowid` is
    // asked under `a`.
    rule_of(Action::Read, Answer::Deny, b'a');
    assert_eq!(
        read(&writer, b"SELECT rowid FROM t1").unwrap_err(),
        "access to t1.a is prohibited"
    );
    // `t2` has no such column, so `rowid` is asked under `ROWID`.
    rule_of(Action::Read, Answer::Deny, b'R');
    assert_eq!(
        read(&writer, b"SELECT rowid FROM t2").unwrap_err(),
        "access to t2.ROWID is prohibited"
    );
    rule_of(Action::Read, Answer::Ignore, b'R');
    assert_eq!(
        read(&writer, b"SELECT oid FROM t2").unwrap(),
        alloc::vec![alloc::vec![Value::Null]]
    );
}

/// A table no column of is read is asked for under the empty name, which
/// `SELECT count(*) FROM t1` is.
fn what_a_table_no_column_of_is_read_is_asked_for() {
    let writer = told();
    rule(Action::Read, Answer::Deny);
    SECOND.store(0, Ordering::Relaxed);
    // `sqlite3Select` asks for that read through `sqlite3AuthCheck` and
    // not through `sqlite3AuthReadCol`, so a denial is the refusal every
    // other action carries.
    assert_eq!(
        read(&writer, b"SELECT count(*) FROM t1").unwrap_err(),
        "not authorized"
    );
    // A name no table of the statement answers is asked for nothing at
    // all, and a statement over no table reads nothing.
    allows();
    COUNT.store(0, Ordering::Relaxed);
    assert_eq!(
        read(&writer, b"SELECT 1+1").unwrap(),
        alloc::vec![alloc::vec![Value::Int(2)]]
    );
    assert_eq!(COUNT.load(Ordering::Relaxed), 1);
}

/// Every clause of a statement is read, and a statement written inside
/// one is read as a statement of its own.
fn what_clauses_of_a_statement_are_read() {
    let writer = told();
    for sql in [
        b"SELECT 1 FROM t1 WHERE b=2".as_slice(),
        b"SELECT 1 FROM t1 GROUP BY b",
        b"SELECT 1 FROM t1 GROUP BY 1 HAVING max(b)>0",
        b"SELECT 1 FROM t1 ORDER BY b",
        b"SELECT 1 FROM t1 LIMIT b",
        b"SELECT 1 FROM t1 LIMIT b OFFSET b",
        b"SELECT 1 FROM t1 JOIN t2 ON b=x",
        b"SELECT 1 FROM t2 WHERE x IN (SELECT b FROM t1)",
        b"WITH k AS (SELECT b FROM t1) SELECT 1 FROM k",
        b"SELECT b FROM t1 UNION SELECT x FROM t2",
        b"VALUES((SELECT b FROM t1))",
    ] {
        rule_of(Action::Read, Answer::Deny, b'b');
        let answered = read(&writer, sql);
        assert!(
            answered.is_err(),
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
        assert_eq!(
            answered.unwrap_err(),
            "access to t1.b is prohibited",
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
}

/// A name no table of the statement answers is asked for nothing, and a
/// statement written inside the `FROM` is read as a statement of its
/// own.
fn what_a_name_no_table_answers_is_asked_for() {
    let mut writer = told();
    // The statement inside the `FROM` reads the table; the side it
    // stands for is no table of the schema, so no column of it is asked
    // for twice.
    rule_of(Action::Read, Answer::Deny, b'b');
    assert_eq!(
        read(&writer, b"SELECT * FROM (SELECT b FROM t1)").unwrap_err(),
        "access to t1.b is prohibited"
    );
    // A `t.*` that names no side of the statement, a column written
    // under a schema no side of the statement is in, and a column the
    // table it is written under does not hold are each asked for
    // nothing.
    allows();
    COUNT.store(0, Ordering::Relaxed);
    for sql in [
        b"SELECT t2.* FROM t1".as_slice(),
        b"SELECT temp.t1.b FROM main.t1",
        b"SELECT t1.z FROM t1",
        b"SELECT zz FROM t1",
        b"SELECT 1 FROM nosuch(1)",
    ] {
        rule_of(Action::Read, Answer::Deny, b'b');
        assert!(
            read(&writer, sql).is_err(),
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
    // A term of a `WITH` that reads a statement inside its own `FROM`
    // and one that reads a table-valued function are each read for the
    // name they carry.
    allows();
    assert_eq!(
        read(
            &writer,
            b"WITH k AS (SELECT * FROM (SELECT 1)) SELECT * FROM k"
        )
        .unwrap(),
        alloc::vec![alloc::vec![Value::Int(1)]]
    );
    assert!(
        read(
            &writer,
            b"WITH k AS (SELECT * FROM nosuch(1)) SELECT * FROM k"
        )
        .is_err()
    );
    // The three names a rowid answers to are each asked for under the
    // name of the key.
    rule_of(Action::Read, Answer::Deny, b'R');
    assert_eq!(
        read(&writer, b"SELECT _rowid_ FROM t2").unwrap_err(),
        "access to t2.ROWID is prohibited"
    );
    // A `rowid` of one side answers a null and the `rowid` of the other
    // stands, because the function was asked for one side alone.
    rule_of(Action::Read, Answer::Ignore, b'a');
    assert_eq!(
        read(&writer, b"SELECT t1.rowid, t2.rowid FROM t1, t2").unwrap(),
        alloc::vec![alloc::vec![Value::Null, Value::Int(1)]]
    );
    // A statement that writes a table the schema does not hold reads
    // nothing of its `WHERE`.
    allows();
    assert!(writer.run(b"DELETE FROM nosuch WHERE a=1").is_err());
    // An `INSERT` that reads a statement asks for `SQLITE_SELECT`, which
    // one that reads a `VALUES` does not.
    rule(Action::Select, Answer::Deny);
    assert_eq!(
        writer
            .run(b"INSERT INTO t2 SELECT b, c FROM t1")
            .unwrap_err()
            .message(),
        "not authorized"
    );
    writer.run(b"INSERT INTO t2 VALUES(1,2)").unwrap();
    allows();
    writer.run(b"DELETE FROM t2 WHERE x=1").unwrap();
}

/// `SQLITE_FUNCTION` denied names the function, whatever the statement
/// calls it under.
fn what_a_denied_function_refuses() {
    let mut writer = told();
    rule(Action::Function, Answer::Deny);
    assert_eq!(
        read(&writer, b"SELECT abs(-1)").unwrap_err(),
        "not authorized to use function: abs"
    );
    assert_eq!(
        writer
            .run(b"INSERT INTO t2 VALUES(abs(-1),2)")
            .unwrap_err()
            .message(),
        "not authorized to use function: abs"
    );
    rule(Action::Function, Answer::Ignore);
    assert_eq!(
        read(&writer, b"SELECT abs(-1)").unwrap(),
        alloc::vec![alloc::vec![Value::Int(1)]]
    );
}

/// `CREATE TABLE` writes the schema's own table, so the function is
/// asked for the write and then for the table.
fn what_a_create_is_asked_for() {
    let mut writer = told();
    for (action, sql) in [
        (Action::Insert, b"CREATE TABLE t3(a)".as_slice()),
        (Action::CreateTable, b"CREATE TABLE t3(a)"),
        (Action::CreateTempTable, b"CREATE TEMP TABLE t3(a)"),
        (Action::CreateView, b"CREATE VIEW v3 AS SELECT 1"),
        (Action::CreateTempView, b"CREATE TEMP VIEW v3 AS SELECT 1"),
        (Action::CreateIndex, b"CREATE INDEX i3 ON t1(b)"),
        (
            Action::CreateTrigger,
            b"CREATE TRIGGER g3 AFTER INSERT ON t1 BEGIN SELECT 1; END",
        ),
        (
            Action::CreateTempTrigger,
            b"CREATE TEMP TRIGGER g3 AFTER INSERT ON t1 BEGIN SELECT 1; END",
        ),
    ] {
        rule(action, Answer::Deny);
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "not authorized",
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
        // The same action ignored leaves the statement undone, and the
        // schema holds nothing new.
        rule(action, Answer::Ignore);
        writer.run(sql).unwrap();
    }
    allows();
    assert_eq!(
        read(
            &writer,
            b"SELECT count(*) FROM sqlite_master WHERE name LIKE '%3'"
        )
        .unwrap(),
        alloc::vec![alloc::vec![Value::Int(0)]]
    );
    // An index over a table of the temp schema is asked for as an index
    // of that schema, and one over a table no database holds names no
    // table.
    writer.run(b"CREATE TEMP TABLE tt(a)").unwrap();
    rule(Action::CreateTempIndex, Answer::Deny);
    assert_eq!(
        writer
            .run(b"CREATE INDEX i4 ON tt(a)")
            .unwrap_err()
            .message(),
        "not authorized"
    );
    allows();
    assert_eq!(
        writer
            .run(b"CREATE INDEX i5 ON nosuch(a)")
            .unwrap_err()
            .message(),
        "no such table: main.nosuch"
    );
    // `CREATE TABLE ... AS` reads the rows it writes, so the statement
    // it reads is read as well.
    rule_of(Action::Read, Answer::Deny, b'b');
    assert_eq!(
        writer
            .run(b"CREATE TABLE t5 AS SELECT b FROM t1")
            .unwrap_err()
            .message(),
        "access to t1.b is prohibited"
    );
}

/// `DROP` takes a row out of the schema's own table, so the function is
/// asked for that and then for what is taken away.
fn what_a_drop_is_asked_for() {
    let mut writer = told();
    writer.run(b"CREATE INDEX i1 ON t1(b)").unwrap();
    writer
        .run(b"CREATE TRIGGER g1 AFTER INSERT ON t1 BEGIN SELECT 1; END")
        .unwrap();
    writer.run(b"CREATE VIEW v1 AS SELECT 1").unwrap();
    // The temp schema holds the table the `DROP TABLE` names, because a
    // statement over a table locates it in the schema it wrote alone.
    writer.run(b"CREATE TEMP TABLE t3(x, y)").unwrap();
    for (action, sql) in [
        (Action::Delete, b"DROP INDEX i1".as_slice()),
        (Action::DropIndex, b"DROP INDEX i1"),
        (Action::DropTrigger, b"DROP TRIGGER g1"),
        (Action::DropView, b"DROP VIEW v1"),
        (Action::DropTable, b"DROP TABLE t2"),
        (Action::DropTempIndex, b"DROP INDEX temp.i1"),
        (Action::DropTempTrigger, b"DROP TRIGGER temp.g1"),
        (Action::DropTempView, b"DROP VIEW temp.v1"),
        (Action::DropTempTable, b"DROP TABLE temp.t3"),
    ] {
        rule(action, Answer::Deny);
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "not authorized",
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
        rule(action, Answer::Ignore);
        writer.run(sql).unwrap();
    }
    // Everything stands, because every drop was refused or left undone.
    allows();
    assert_eq!(
        read(&writer, b"SELECT count(*) FROM sqlite_master").unwrap(),
        alloc::vec![alloc::vec![Value::Int(5)]]
    );
    // A `DROP INDEX` of a name the schema holds no index under is asked
    // for with no table at all.
    writer.run(b"DROP INDEX IF EXISTS nosuch").unwrap();
    writer.run(b"DROP TRIGGER IF EXISTS nosuch").unwrap();
}

/// The four `ALTER TABLE` forms are asked for under the schema, the
/// table and, where one is named, the column.
fn what_an_alter_is_asked_for() {
    let mut writer = told();
    for sql in [
        b"ALTER TABLE t2 RENAME TO t9".as_slice(),
        b"ALTER TABLE t2 ADD COLUMN z",
        b"ALTER TABLE t2 DROP COLUMN y",
        b"ALTER TABLE t2 RENAME COLUMN y TO z",
        b"ALTER TABLE t2 ALTER y DROP NOT NULL",
    ] {
        rule(Action::AlterTable, Answer::Deny);
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "not authorized",
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
        rule(Action::AlterTable, Answer::Ignore);
        writer.run(sql).unwrap();
    }
    allows();
    assert_eq!(
        read(&writer, b"SELECT count(*) FROM t2").unwrap(),
        alloc::vec![alloc::vec![Value::Int(1)]]
    );
}

/// `INSERT`, `DELETE` and `UPDATE` are asked for against the table they
/// write, and an `UPDATE` once per column.
fn what_a_statement_that_writes_is_asked_for() {
    let mut writer = told();
    rule(Action::Insert, Answer::Deny);
    assert_eq!(
        writer
            .run(b"INSERT INTO t2 VALUES(6,7)")
            .unwrap_err()
            .message(),
        "not authorized"
    );
    rule(Action::Insert, Answer::Ignore);
    writer.run(b"INSERT INTO t2 VALUES(6,7)").unwrap();
    writer.run(b"INSERT INTO t2 DEFAULT VALUES").unwrap();
    allows();
    assert_eq!(
        read(&writer, b"SELECT count(*) FROM t2").unwrap(),
        alloc::vec![alloc::vec![Value::Int(1)]]
    );
    // A `DELETE` the function ignores writes the rows all the same,
    // because the answer only leaves the whole-table shortcut, which
    // this crate has none of.
    rule(Action::Delete, Answer::Deny);
    assert_eq!(
        writer
            .run(b"DELETE FROM t2 WHERE x=4")
            .unwrap_err()
            .message(),
        "not authorized"
    );
    rule(Action::Delete, Answer::Ignore);
    writer.run(b"DELETE FROM t2 WHERE x=4").unwrap();
    allows();
    assert_eq!(
        read(&writer, b"SELECT count(*) FROM t2").unwrap(),
        alloc::vec![alloc::vec![Value::Int(0)]]
    );
    // An `UPDATE` is asked for once per column written, and a column it
    // ignores keeps the value it had.
    rule(Action::Update, Answer::Deny);
    assert_eq!(
        writer
            .run(b"UPDATE t1 SET b=22, c=33")
            .unwrap_err()
            .message(),
        "not authorized"
    );
    rule_of(Action::Update, Answer::Ignore, b'b');
    writer.run(b"UPDATE t1 SET b=22, c=33").unwrap();
    allows();
    assert_eq!(
        read(&writer, b"SELECT b, c FROM t1").unwrap(),
        alloc::vec![alloc::vec![Value::Int(2), Value::Int(33)]]
    );
    // The `WHERE` of a statement that writes is read as well.
    rule_of(Action::Read, Answer::Deny, b'b');
    assert_eq!(
        writer
            .run(b"DELETE FROM t1 WHERE b=2")
            .unwrap_err()
            .message(),
        "access to t1.b is prohibited"
    );
    // A bare `rowid` of the `WHERE` is read under the name of the column
    // the key is another name for.
    rule_of(Action::Read, Answer::Deny, b'a');
    assert_eq!(
        writer
            .run(b"UPDATE t1 SET c=1 WHERE rowid=1")
            .unwrap_err()
            .message(),
        "access to t1.a is prohibited"
    );
    allows();
    writer.run(b"DELETE FROM t1").unwrap();
}

/// `PRAGMA`, `BEGIN`, `SAVEPOINT`, `ANALYZE` and `REINDEX` are each
/// asked for, and each is left undone where the function ignores it.
fn what_the_statements_beside_the_language_are_asked_for() {
    let mut writer = told();
    rule(Action::Pragma, Answer::Deny);
    assert_eq!(
        writer.run(b"PRAGMA user_version=9").unwrap_err().message(),
        "not authorized"
    );
    rule(Action::Pragma, Answer::Ignore);
    assert!(writer.run(b"PRAGMA user_version=9").unwrap().is_empty());
    allows();
    assert_eq!(
        writer.run(b"PRAGMA user_version").unwrap(),
        alloc::vec![alloc::vec![Value::Int(0)]]
    );
    // `BEGIN`, `COMMIT` and `ROLLBACK` are one action under three
    // words.
    rule(Action::Transaction, Answer::Deny);
    assert_eq!(
        writer.run(b"BEGIN").unwrap_err().message(),
        "not authorized"
    );
    rule(Action::Transaction, Answer::Ignore);
    assert!(writer.run(b"BEGIN").unwrap().is_empty());
    allows();
    writer.run(b"BEGIN").unwrap();
    rule(Action::Transaction, Answer::Ignore);
    writer.run(b"COMMIT").unwrap();
    writer.run(b"ROLLBACK").unwrap();
    allows();
    writer.run(b"COMMIT").unwrap();
    // A savepoint carries its name as the fourth argument.
    rule(Action::Savepoint, Answer::Deny);
    assert_eq!(
        writer.run(b"SAVEPOINT one").unwrap_err().message(),
        "not authorized"
    );
    rule(Action::Savepoint, Answer::Ignore);
    assert!(writer.run(b"SAVEPOINT one").unwrap().is_empty());
    allows();
    writer.run(b"SAVEPOINT one").unwrap();
    rule(Action::Savepoint, Answer::Ignore);
    writer.run(b"RELEASE one").unwrap();
    writer.run(b"ROLLBACK TO one").unwrap();
    allows();
    writer.run(b"RELEASE one").unwrap();
    // `ANALYZE` and `REINDEX` name the table and the index.
    writer.run(b"CREATE INDEX i1 ON t1(b)").unwrap();
    rule(Action::Analyze, Answer::Deny);
    assert_eq!(
        writer.run(b"ANALYZE").unwrap_err().message(),
        "not authorized"
    );
    rule(Action::Analyze, Answer::Ignore);
    writer.run(b"ANALYZE t1").unwrap();
    rule(Action::Reindex, Answer::Deny);
    assert_eq!(
        writer.run(b"REINDEX").unwrap_err().message(),
        "not authorized"
    );
    rule(Action::Reindex, Answer::Ignore);
    writer.run(b"REINDEX i1").unwrap();
    allows();
    assert_eq!(
        read(
            &writer,
            b"SELECT count(*) FROM sqlite_master WHERE name='sqlite_stat1'"
        )
        .unwrap(),
        alloc::vec![alloc::vec![Value::Int(0)]]
    );
}

/// A term of a `WITH` that reads its own name is asked for before a row
/// of it is read.
fn what_a_recursive_term_is_asked_for() {
    let writer = told();
    let sql = b"WITH k(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM k WHERE n<3) \
                SELECT count(*) FROM k";
    rule(Action::Recursive, Answer::Deny);
    assert_eq!(read(&writer, sql).unwrap_err(), "not authorized");
    // A term that reads no name of its own is asked for nothing.
    rule(Action::Recursive, Answer::Deny);
    assert_eq!(
        read(&writer, b"WITH k AS (SELECT 1) SELECT * FROM k").unwrap(),
        alloc::vec![alloc::vec![Value::Int(1)]]
    );
    rule(Action::Recursive, Answer::Ignore);
    assert_eq!(
        read(&writer, sql).unwrap(),
        alloc::vec![alloc::vec![Value::Int(3)]]
    );
}

/// `VACUUM` is asked for nothing, which is the one statement
/// `sqlite3RunVacuum` asks the function nothing about.
fn what_a_vacuum_is_asked_for() {
    let mut writer = told();
    rule(Action::Select, Answer::Deny);
    writer.run(b"VACUUM").unwrap();
    rule(Action::Delete, Answer::Deny);
    writer.run(b"VACUUM").unwrap();
    allows();
    assert_eq!(
        read(&writer, b"SELECT count(*) FROM t1").unwrap(),
        alloc::vec![alloc::vec![Value::Int(1)]]
    );
}

/// `ATTACH` and `DETACH` are asked for under the text the statement
/// wrote, which `sqlite3Attach` of `research/sqlite/src/attach.c:393`
/// hands the function, and nothing where the statement wrote an
/// expression of its own.
fn what_an_attach_is_asked_for() {
    let mut writer = told();
    writer.opens(|_| None);
    rule(Action::Attach, Answer::Deny);
    assert_eq!(
        writer
            .run(b"ATTACH ':memory:' AS aux")
            .expect_err("a refusal")
            .message(),
        "not authorized"
    );
    // A statement that writes no string is asked for under the empty
    // name, which the rule reads as any name.
    assert_eq!(
        writer
            .run(b"ATTACH ':'||'memory:' AS aux")
            .expect_err("a refusal")
            .message(),
        "not authorized"
    );
    allows();
    writer.run(b"ATTACH ':memory:' AS aux").unwrap();
    rule(Action::Detach, Answer::Deny);
    assert_eq!(
        writer.run(b"DETACH aux").expect_err("a refusal").message(),
        "not authorized"
    );
    allows();
    writer.run(b"DETACH aux").unwrap();
    assert!(writer.attached_names().is_empty());
}

/// A read the function denies of a column in a database other than the one
/// the statement writes carries the schema in the message, which
/// `sqlite3AuthReadCol` writes in front of the table.
fn what_a_denied_read_of_another_database_answers() {
    let mut writer = told();
    writer.opens(|_| None);
    writer.run(b"ATTACH ':memory:' AS aux").unwrap();
    writer.run(b"CREATE TABLE aux.u(b)").unwrap();
    rule_of(Action::Read, Answer::Deny, b'b');
    assert_eq!(
        writer
            .run(b"INSERT INTO t1(a) SELECT b FROM aux.u")
            .expect_err("a refusal")
            .message(),
        "access to aux.u.b is prohibited"
    );
    allows();
}

/// The authorizer of a connection, asked for one statement at a time.
///
/// The function the connection is told carries nothing and reads the
/// statics of this module, so the scenarios run one after another rather
/// than as tests of their own.
#[test]
fn what_the_authorizer_of_a_connection_is_asked() {
    what_a_connection_told_no_function_is_asked();
    what_a_denied_select_answers();
    what_a_read_the_function_refuses_answers();
    what_name_a_rowid_is_asked_under();
    what_a_table_no_column_of_is_read_is_asked_for();
    what_clauses_of_a_statement_are_read();
    what_a_denied_function_refuses();
    what_a_create_is_asked_for();
    what_a_drop_is_asked_for();
    what_an_alter_is_asked_for();
    what_a_statement_that_writes_is_asked_for();
    what_the_statements_beside_the_language_are_asked_for();
    what_a_name_no_table_answers_is_asked_for();
    what_a_recursive_term_is_asked_for();
    what_a_vacuum_is_asked_for();
    what_an_attach_is_asked_for();
    what_a_denied_read_of_another_database_answers();
}

/// One call of [`recording`]: the action and the four arguments after it.
struct Recorded {
    /// What the statement was about to do.
    action: Action,
    /// The third argument.
    first: Vec<u8>,
    /// The fourth argument.
    second: Vec<u8>,
    /// The fifth argument.
    schema: Vec<u8>,
    /// The sixth argument.
    inner: Vec<u8>,
}

/// What every call of [`recording`] was asked.
static RECORDED: std::sync::Mutex<Vec<Recorded>> = std::sync::Mutex::new(Vec::new());

/// The one test at a time that reads [`RECORDED`], which the tests share
/// with the function that writes it.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The lock one test holds while it reads [`RECORDED`].
fn alone() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A function that allows every action and writes down what it was asked.
fn recording(asked: &Asked<'_>) -> Answer {
    if let Ok(mut held) = RECORDED.lock() {
        held.push(Recorded {
            action: asked.action,
            first: asked.first.to_vec(),
            second: asked.second.to_vec(),
            schema: asked.schema.to_vec(),
            inner: asked.inner.to_vec(),
        });
    }
    Answer::Ok
}

/// What [`RECORDED`] holds for one action, as `name/schema` per call.
fn recorded(action: Action) -> alloc::string::String {
    let held = RECORDED.lock().expect("the recorded calls");
    let mut out = alloc::string::String::new();
    for one in held.iter() {
        if one.action != action {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&alloc::string::String::from_utf8_lossy(&one.first));
        out.push('/');
        out.push_str(&alloc::string::String::from_utf8_lossy(&one.schema));
    }
    out
}

/// Every call of [`recording`], as the word of the action and its four
/// arguments with a slash between them.
fn asked_all() -> alloc::string::String {
    let held = RECORDED.lock().expect("the recorded calls");
    let mut out = alloc::string::String::new();
    for one in held.iter() {
        if !out.is_empty() {
            out.push(' ');
        }
        for text in [
            one.action.word(),
            &one.first,
            &one.second,
            &one.schema,
            &one.inner,
        ] {
            out.push_str(&alloc::string::String::from_utf8_lossy(text));
            out.push('/');
        }
    }
    out
}

/// `REINDEX` asks about every index it writes again under the name of
/// that index and the schema it stands in, with the index made last
/// first, and `REINDEX <collation>` writes again every index one place of
/// whose entries is held in that collation.
#[test]
fn which_indexes_a_reindex_asks_about() {
    let _alone = alone();
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.opens(|_| Some(Vec::new()));
    writer.asks(recording);
    for sql in [
        b"CREATE TABLE t3(a PRIMARY KEY, b, c)".as_slice(),
        b"CREATE INDEX t3_idx1 ON t3(c COLLATE BINARY)",
        b"CREATE INDEX t3_idx2 ON t3(b COLLATE NOCASE)",
    ] {
        writer.run(sql).unwrap();
    }
    let clear = || RECORDED.lock().expect("the recorded calls").clear();
    clear();
    writer.run(b"REINDEX t3_idx1").unwrap();
    assert_eq!(recorded(Action::Reindex), "t3_idx1/main");
    // The key of the row ends every entry of an index over a table that
    // keeps a rowid, and it is held under `BINARY`, so every index of
    // such a table is written again.
    clear();
    writer.run(b"REINDEX BINARY").unwrap();
    assert_eq!(
        recorded(Action::Reindex),
        "t3_idx2/main t3_idx1/main sqlite_autoindex_t3_1/main"
    );
    clear();
    writer.run(b"REINDEX NOCASE").unwrap();
    assert_eq!(recorded(Action::Reindex), "t3_idx2/main");
    // A table names its indexes with the one made last first.
    clear();
    writer.run(b"REINDEX t3").unwrap();
    assert_eq!(
        recorded(Action::Reindex),
        "t3_idx2/main t3_idx1/main sqlite_autoindex_t3_1/main"
    );
    // An entry of an index over a table that keeps its rows in the key's
    // own tree ends with the columns of that key, under their own
    // collations, so `REINDEX NOCASE` writes such an index again.
    for sql in [
        b"CREATE TABLE w(a TEXT COLLATE NOCASE PRIMARY KEY, b) WITHOUT ROWID".as_slice(),
        b"CREATE INDEX wb ON w(b COLLATE BINARY)",
    ] {
        writer.run(sql).unwrap();
    }
    // The `PRIMARY KEY` of such a table is that tree and no index of its
    // own, so this engine names it in no call where the C library names
    // `sqlite_autoindex_w_1`.
    clear();
    writer.run(b"REINDEX NOCASE").unwrap();
    assert_eq!(recorded(Action::Reindex), "t3_idx2/main wb/main");
    // An index of the temp schema stands under that name.
    writer.run(b"CREATE TEMP TABLE t4(a, b)").unwrap();
    writer.run(b"CREATE INDEX temp.t4_idx ON t4(b)").unwrap();
    clear();
    writer.run(b"REINDEX temp.t4_idx").unwrap();
    assert_eq!(recorded(Action::Reindex), "t4_idx/temp");
    clear();
}

/// The statements of a trigger's body are asked for under the name of
/// that trigger, and an `UPDATE` asks about the value a column is written
/// with before it asks about the column.
#[test]
fn which_actions_the_body_of_a_trigger_is_asked_for() {
    let _alone = alone();
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.asks(recording);
    for sql in [
        b"CREATE TABLE t2(a,b,c)".as_slice(),
        b"CREATE TABLE tx(a1,a2,b1,b2,c1,c2)",
        b"INSERT INTO t2 VALUES(1,2,3)",
        b"CREATE TRIGGER r1 AFTER UPDATE ON t2 BEGIN \
          INSERT INTO tx VALUES(OLD.a,NEW.a,OLD.b,NEW.b,OLD.c,NEW.c); END",
    ] {
        writer.run(sql).unwrap();
    }
    let clear = || RECORDED.lock().expect("the recorded calls").clear();
    clear();
    writer.run(b"UPDATE t2 SET a=a+1").unwrap();
    assert_eq!(
        asked_all(),
        "SQLITE_READ/t2/a/main// SQLITE_UPDATE/t2/a/main// \
         SQLITE_INSERT/tx//main/r1/ SQLITE_READ/t2/a/main/r1/ SQLITE_READ/t2/a/main/r1/ \
         SQLITE_READ/t2/b/main/r1/ SQLITE_READ/t2/b/main/r1/ \
         SQLITE_READ/t2/c/main/r1/ SQLITE_READ/t2/c/main/r1/"
    );
    // The triggers of the table are read with the one made last first,
    // whatever time each runs at.
    for sql in [
        b"CREATE TRIGGER r0 BEFORE UPDATE ON t2 BEGIN SELECT OLD.b; END".as_slice(),
        b"CREATE TRIGGER r2 AFTER UPDATE ON t2 BEGIN SELECT NEW.c; END",
    ] {
        writer.run(sql).unwrap();
    }
    clear();
    writer.run(b"UPDATE t2 SET b=1 WHERE c=3").unwrap();
    assert_eq!(
        asked_all(),
        "SQLITE_UPDATE/t2/b/main// SQLITE_READ/t2/c/main// \
         SQLITE_SELECT////r2/ SQLITE_READ/t2/c/main/r2/ \
         SQLITE_SELECT////r0/ SQLITE_READ/t2/b/main/r0/ \
         SQLITE_INSERT/tx//main/r1/ SQLITE_READ/t2/a/main/r1/ SQLITE_READ/t2/a/main/r1/ \
         SQLITE_READ/t2/b/main/r1/ SQLITE_READ/t2/b/main/r1/ \
         SQLITE_READ/t2/c/main/r1/ SQLITE_READ/t2/c/main/r1/"
    );
    // A trigger whose body writes the table it is on is passed over where
    // it is already being read, so its body is read once.
    for sql in [
        b"DROP TRIGGER r0".as_slice(),
        b"DROP TRIGGER r1",
        b"DROP TRIGGER r2",
        b"CREATE TRIGGER s1 AFTER UPDATE ON t2 BEGIN UPDATE t2 SET c=1; END",
    ] {
        writer.run(sql).unwrap();
    }
    clear();
    writer.run(b"UPDATE t2 SET b=2").unwrap();
    assert_eq!(
        asked_all(),
        "SQLITE_UPDATE/t2/b/main// SQLITE_UPDATE/t2/c/main/s1/"
    );
    // A body that writes and one that takes rows out are read the same
    // way.
    for sql in [
        b"DROP TRIGGER s1".as_slice(),
        b"CREATE TRIGGER d1 AFTER DELETE ON t2 BEGIN DELETE FROM tx WHERE a1=OLD.a; END",
        b"CREATE TRIGGER u1 AFTER DELETE ON t2 BEGIN UPDATE t2 SET b=OLD.b; END",
    ] {
        writer.run(sql).unwrap();
    }
    clear();
    writer.run(b"DELETE FROM t2").unwrap();
    assert_eq!(
        asked_all(),
        "SQLITE_DELETE/t2//main// SQLITE_READ/t2/b/main/u1/ SQLITE_UPDATE/t2/b/main/u1/ SQLITE_DELETE/tx//main/d1/ SQLITE_READ/tx/a1/main/d1/ SQLITE_READ/t2/a/main/d1/"
    );
    clear();
}
