// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The three hooks a connection is told: what a commit asks, what a
//! transaction that goes back tells, and what a row a statement wrote
//! tells.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use crate::change::{Did, Peeked, Writer, Wrote};
use crate::header::Encoding;

/// How often the commit hook was asked since [`clear`].
static COMMITS: AtomicU32 = AtomicU32::new(0);

/// What the commit hook answers.
static REFUSES: AtomicBool = AtomicBool::new(false);

/// How often the rollback hook was told since [`clear`].
static ROLLBACKS: AtomicU32 = AtomicU32::new(0);

/// The rows the update hook was told of, in order.
static WROTE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The rows the preupdate hook was told of, in order.
static PEEKED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The function the tests tell the connection as its commit hook.
fn committing() -> bool {
    COMMITS.fetch_add(1, Ordering::Relaxed);
    REFUSES.load(Ordering::Relaxed)
}

/// The function the tests tell the connection as its rollback hook.
fn rolling() {
    ROLLBACKS.fetch_add(1, Ordering::Relaxed);
}

/// The function the tests tell the connection as its update hook.
fn writing(wrote: &Wrote<'_>) {
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    WROTE.lock().unwrap().push(format!(
        "{} {} {} {}",
        text(wrote.did.word()),
        text(wrote.schema),
        text(wrote.table),
        wrote.rowid
    ));
}

/// The function the tests tell the connection as its preupdate hook,
/// written as the action, the database, the table, the two keys, the
/// depth, then the old row and the new row.
fn peeking(peeked: &Peeked<'_>) {
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let shown = |row: Option<&[crate::value::Value]>| match row {
        None => String::from("-"),
        Some(row) => row
            .iter()
            .map(|value| {
                String::from_utf8_lossy(&value.text().unwrap_or_else(|| b"NULL".to_vec()))
                    .into_owned()
            })
            .collect::<Vec<String>>()
            .join(" "),
    };
    PEEKED.lock().unwrap().push(format!(
        "{} {} {} {} {} {} [{}] [{}]",
        text(peeked.did.word()),
        text(peeked.schema),
        text(peeked.table),
        peeked.was,
        peeked.key,
        peeked.depth,
        shown(peeked.old),
        shown(peeked.new)
    ));
}

/// The three counters and the rows back at nought.
fn clear() {
    COMMITS.store(0, Ordering::Relaxed);
    ROLLBACKS.store(0, Ordering::Relaxed);
    REFUSES.store(false, Ordering::Relaxed);
    WROTE.lock().unwrap().clear();
    PEEKED.lock().unwrap().clear();
}

/// The rows the update hook was told of, as one line.
fn rows() -> String {
    WROTE.lock().unwrap().join(", ")
}

/// The rows the preupdate hook was told of, as one line.
fn peeks() -> String {
    PEEKED.lock().unwrap().join(", ")
}

/// A connection over one table of rows, told all three hooks.
fn hooked() -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER PRIMARY KEY, b)")
        .unwrap();
    writer.commits(committing);
    writer.rolls_back(rolling);
    writer.writes_rows(writing);
    writer.peeks(peeking);
    clear();
    writer
}

/// Every action carries the word `tclsqlite.c` writes it as.
fn what_word_every_action_is_written_as() {
    assert_eq!(Did::Insert.word(), b"INSERT");
    assert_eq!(Did::Update.word(), b"UPDATE");
    assert_eq!(Did::Delete.word(), b"DELETE");
}

/// A connection told no function runs the same statements and is asked
/// nothing.
fn what_a_connection_told_no_function_is_asked() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.commits_nothing();
    writer.rolls_back_nothing();
    writer.writes_nothing();
    writer.peeks_nothing();
    clear();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"ROLLBACK").unwrap();
    assert_eq!(COMMITS.load(Ordering::Relaxed), 0);
    assert_eq!(ROLLBACKS.load(Ordering::Relaxed), 0);
    assert_eq!(rows(), "");
    assert_eq!(peeks(), "");
}

/// A statement of its own that writes a page asks the commit hook once,
/// and one that reads asks nothing.
fn what_a_statement_of_its_own_asks() {
    let mut writer = hooked();
    writer.run(b"INSERT INTO t VALUES(1, 'one')").unwrap();
    assert_eq!(COMMITS.load(Ordering::Relaxed), 1);
    writer.run(b"INSERT INTO t VALUES(2, 'two')").unwrap();
    writer.run(b"UPDATE t SET b='x' WHERE a=1").unwrap();
    writer.run(b"DELETE FROM t WHERE a=2").unwrap();
    assert_eq!(COMMITS.load(Ordering::Relaxed), 4);
    assert_eq!(ROLLBACKS.load(Ordering::Relaxed), 0);
    assert_eq!(
        rows(),
        "INSERT main t 1, INSERT main t 2, UPDATE main t 1, DELETE main t 2"
    );
}

/// A commit hook that answers true refuses the statement, sends the
/// transaction back and tells the rollback hook.
fn what_a_commit_the_function_refuses_answers() {
    let mut writer = hooked();
    writer.run(b"INSERT INTO t VALUES(1, 'one')").unwrap();
    REFUSES.store(true, Ordering::Relaxed);
    let refused = writer.run(b"INSERT INTO t VALUES(2, 'two')").unwrap_err();
    assert_eq!(refused.message(), "constraint failed");
    assert_eq!(refused.code().number, 19);
    assert_eq!(refused.code().extended, 531);
    assert_eq!(
        refused.code().extended_name,
        b"SQLITE_CONSTRAINT_COMMITHOOK"
    );
    assert_eq!(ROLLBACKS.load(Ordering::Relaxed), 1);
    REFUSES.store(false, Ordering::Relaxed);
    // The row the refused statement wrote is gone.
    let image = writer.written();
    let database = crate::db::Database::open(&image).unwrap();
    let answered = database.query(b"SELECT count(*) FROM t").unwrap();
    assert_eq!(answered.rows[0][0].to_integer(), 1);
}

/// A transaction asks the commit hook at its `COMMIT` and not at each
/// statement, and a `ROLLBACK` tells the rollback hook whether or not
/// the transaction wrote.
fn what_a_transaction_asks() {
    let mut writer = hooked();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"INSERT INTO t VALUES(1, 'one')").unwrap();
    writer.run(b"INSERT INTO t VALUES(2, 'two')").unwrap();
    assert_eq!(COMMITS.load(Ordering::Relaxed), 0);
    writer.run(b"COMMIT").unwrap();
    assert_eq!(COMMITS.load(Ordering::Relaxed), 1);
    assert_eq!(ROLLBACKS.load(Ordering::Relaxed), 0);
    clear();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"ROLLBACK").unwrap();
    assert_eq!(COMMITS.load(Ordering::Relaxed), 0);
    assert_eq!(ROLLBACKS.load(Ordering::Relaxed), 1);
}

/// A `COMMIT` the function refuses sends the transaction back and tells
/// the rollback hook.
fn what_a_transaction_the_function_refuses_answers() {
    let mut writer = hooked();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"INSERT INTO t VALUES(1, 'one')").unwrap();
    REFUSES.store(true, Ordering::Relaxed);
    let refused = writer.run(b"COMMIT").unwrap_err();
    assert_eq!(refused.message(), "constraint failed");
    assert_eq!(ROLLBACKS.load(Ordering::Relaxed), 1);
    REFUSES.store(false, Ordering::Relaxed);
    let image = writer.written();
    let database = crate::db::Database::open(&image).unwrap();
    let answered = database.query(b"SELECT count(*) FROM t").unwrap();
    assert_eq!(answered.rows[0][0].to_integer(), 0);
}

/// A statement of its own whose row breaks a constraint sends its own
/// transaction back and tells the rollback hook, and one the connection
/// could not read tells nothing.
fn what_a_statement_that_breaks_a_constraint_tells() {
    let mut writer = hooked();
    writer.run(b"INSERT INTO t VALUES(1, 'one')").unwrap();
    clear();
    let refused = writer.run(b"INSERT INTO t VALUES(1, 'again')").unwrap_err();
    assert_eq!(refused.message(), "UNIQUE constraint failed: t.a");
    assert_eq!(ROLLBACKS.load(Ordering::Relaxed), 1);
    clear();
    assert!(writer.run(b"INSERT INTO u VALUES(1)").is_err());
    assert_eq!(ROLLBACKS.load(Ordering::Relaxed), 0);
    // A statement inside a transaction leaves the transaction open, so
    // the hook is told nothing.
    writer.run(b"BEGIN").unwrap();
    clear();
    assert!(writer.run(b"INSERT INTO t VALUES(1, 'again')").is_err());
    assert_eq!(ROLLBACKS.load(Ordering::Relaxed), 0);
    writer.run(b"ROLLBACK").unwrap();
}

/// A row of a `WITHOUT ROWID` table and a row of a table the library
/// keeps for itself tell the update hook nothing.
fn what_rows_the_update_hook_is_told_of() {
    let mut writer = hooked();
    writer
        .run(b"CREATE TABLE w(a INT PRIMARY KEY, b) WITHOUT ROWID")
        .unwrap();
    writer
        .run(b"CREATE TABLE s(a INTEGER PRIMARY KEY AUTOINCREMENT, b)")
        .unwrap();
    clear();
    writer.run(b"INSERT INTO w VALUES(1, 'one')").unwrap();
    writer.run(b"UPDATE w SET b='x'").unwrap();
    writer.run(b"DELETE FROM w").unwrap();
    assert_eq!(rows(), "");
    // `sqlite_sequence` carries the key the table counted up, and no
    // row of it reaches the function.
    writer.run(b"INSERT INTO s VALUES(NULL, 'one')").unwrap();
    assert_eq!(rows(), "INSERT main s 1");
}

/// The rows a trigger's body writes carry the table the body names, and
/// a row of an attached database carries the name the `ATTACH` gave it.
fn what_the_rows_of_a_trigger_and_of_another_database_tell() {
    let mut writer = hooked();
    writer
        .run(b"CREATE TABLE u(c INTEGER PRIMARY KEY, d)")
        .unwrap();
    writer
        .run(b"CREATE TRIGGER tr AFTER INSERT ON t BEGIN INSERT INTO u VALUES(new.a, new.b); END")
        .unwrap();
    clear();
    writer.run(b"INSERT INTO t VALUES(1, 'one')").unwrap();
    assert_eq!(rows(), "INSERT main t 1, INSERT main u 1");
    writer.opens(|_| Some(Vec::new()));
    writer.run(b"ATTACH ':memory:' AS aux").unwrap();
    writer
        .run(b"CREATE TABLE aux.v(e INTEGER PRIMARY KEY, f)")
        .unwrap();
    clear();
    writer.run(b"INSERT INTO aux.v VALUES(3, 'three')").unwrap();
    assert_eq!(rows(), "INSERT aux v 3");
}

/// The preupdate hook is told the two keys, the depth, the old row and
/// the new row of each row a statement is about to write, and the column
/// the key is another name for carries the key.
fn what_rows_the_preupdate_hook_is_told_of() {
    let mut writer = hooked();
    writer.run(b"INSERT INTO t VALUES(1, 'one')").unwrap();
    assert_eq!(peeks(), "INSERT main t 1 1 0 [-] [1 one]");
    clear();
    writer.run(b"UPDATE t SET a=5, b='five'").unwrap();
    assert_eq!(peeks(), "UPDATE main t 1 5 0 [1 one] [5 five]");
    clear();
    writer.run(b"DELETE FROM t").unwrap();
    assert_eq!(peeks(), "DELETE main t 5 5 0 [5 five] [-]");
    // The row a `REPLACE` writes over is told as a row taken away, which
    // the update hook is not told of.
    writer.run(b"CREATE TABLE u(a UNIQUE, b)").unwrap();
    writer.run(b"INSERT INTO u VALUES(1, 'one')").unwrap();
    clear();
    writer.run(b"REPLACE INTO u VALUES(1, 'two')").unwrap();
    assert_eq!(
        peeks(),
        "DELETE main u 1 1 0 [1 one] [-], INSERT main u 2 2 0 [-] [1 two]"
    );
    assert_eq!(rows(), "INSERT main u 2");
    // A row of a table that keeps its rows in the key's own tree carries
    // no key of its own, so both keys are nought.
    writer
        .run(b"CREATE TABLE w(a INT PRIMARY KEY, b) WITHOUT ROWID")
        .unwrap();
    clear();
    writer.run(b"INSERT INTO w VALUES(1, 'one')").unwrap();
    writer.run(b"UPDATE w SET b='two'").unwrap();
    writer.run(b"DELETE FROM w").unwrap();
    assert_eq!(
        peeks(),
        "INSERT main w 0 0 0 [-] [1 one], UPDATE main w 0 0 0 [1 one] [1 two], DELETE main w 0 0 0 [1 two] [-]"
    );
    assert_eq!(rows(), "");
}

/// A row the statement writes inside a trigger's body carries how many
/// triggers deep the statement stands, and a row a trigger before the row
/// took away is told once.
fn what_depth_a_row_of_a_trigger_carries() {
    let mut writer = hooked();
    writer
        .run(b"CREATE TABLE u(c INTEGER PRIMARY KEY, d)")
        .unwrap();
    writer
        .run(b"CREATE TRIGGER tr AFTER INSERT ON t BEGIN INSERT INTO u VALUES(new.a, new.b); END")
        .unwrap();
    clear();
    writer.run(b"INSERT INTO t VALUES(1, 'one')").unwrap();
    assert_eq!(
        peeks(),
        "INSERT main t 1 1 0 [-] [1 one], INSERT main u 1 1 1 [-] [1 one]"
    );
    // A trigger before the row whose body takes the row away leaves the
    // statement nothing to take, so the hook is told once.
    writer
        .run(b"CREATE TABLE v(a INTEGER PRIMARY KEY, b)")
        .unwrap();
    writer
        .run(b"INSERT INTO v VALUES(1, 'one'),(2, 'two')")
        .unwrap();
    writer
        .run(b"CREATE TRIGGER vd BEFORE DELETE ON v BEGIN DELETE FROM v WHERE a=1; END")
        .unwrap();
    clear();
    writer.run(b"DELETE FROM v WHERE a=1").unwrap();
    assert_eq!(peeks(), "DELETE main v 1 1 1 [1 one] [-]");
    // The same for a trigger before an `UPDATE`.
    writer
        .run(b"CREATE TRIGGER vu BEFORE UPDATE ON v BEGIN DELETE FROM v WHERE a=2; END")
        .unwrap();
    clear();
    writer.run(b"UPDATE v SET b='x' WHERE a=2").unwrap();
    assert_eq!(peeks(), "DELETE main v 2 2 1 [2 two] [-]");
}

/// The scenarios run one after another, because the functions the
/// connection is told read the statics of this module.
#[test]
fn what_the_hooks_of_a_connection_are_told() {
    what_word_every_action_is_written_as();
    what_a_connection_told_no_function_is_asked();
    what_a_statement_of_its_own_asks();
    what_a_commit_the_function_refuses_answers();
    what_a_transaction_asks();
    what_a_transaction_the_function_refuses_answers();
    what_a_statement_that_breaks_a_constraint_tells();
    what_rows_the_update_hook_is_told_of();
    what_the_rows_of_a_trigger_and_of_another_database_tell();
    what_rows_the_preupdate_hook_is_told_of();
    what_depth_a_row_of_a_trigger_carries();
}
