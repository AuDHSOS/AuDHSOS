// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The reader of the statements of a case of SQLite's own test files,
//! the reader of what one run wrote, and the pool that runs the files
//! beside each other.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::suite::{Score, beside, read_score, statements};

#[test]
fn statements_are_parted_by_the_semicolons_outside_a_string() {
    assert_eq!(statements("SELECT 1; SELECT 2"), ["SELECT 1", " SELECT 2"]);
    assert_eq!(statements("SELECT ';'"), ["SELECT ';'"]);
    assert_eq!(
        statements("SELECT \";\" ; SELECT 2"),
        ["SELECT \";\" ", " SELECT 2"]
    );
}

#[test]
fn a_score_counts_the_lines_one_run_wrote_and_passes_over_the_rest() {
    let score = read_score(
        "x.test",
        "C passed\nC passed\nC refused\nW no ATTACH\nC failed\nF x-1.0\n  mine {}\n  want {1}\n\
         S SELECT 1 no such column\nnoise\nC\n",
    );
    assert_eq!(
        score,
        Score {
            passed: 2,
            failed: 1,
            refused: 1
        }
    );
    assert_eq!(score.ran(), 4);
    // The three lines of a case that failed are one record, so the two
    // under `F` are never read as cases of their own.
    assert_eq!(
        read_score("x.test", "F x-1.0\nC passed\nC passed\n").ran(),
        0
    );
}

/// A directory of its own per test, also across concurrent xtask runs.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("audhsos-suite-test-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// A stand-in for the xtask binary, which `beside` starts once per
    /// file as `<me> sqlite-suite …options… --one <path>`. This one
    /// runs the last argument as a shell script, so each test writes
    /// what its files say whatever options stand before the path.
    fn me(&self) -> PathBuf {
        let path = self.0.join("me");
        std::fs::write(&path, "#!/bin/sh\nshift $(($# - 1))\nexec sh \"$1\"\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// One stand-in for a file of the suite.
    fn file(&self, name: &str, script: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, script).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_files_run_beside_each_other_and_are_answered_in_the_order_given() {
    let scratch = Scratch::new();
    let release = scratch.0.join("release").display().to_string();
    // The first file ends only after the second has run, so a runner
    // that starts the second where the first ended never finishes it.
    let first = scratch.file(
        "first",
        &format!(
            "i=0; while [ ! -f {release} ]; do i=$((i+1)); [ \"$i\" -lt 1000 ] || exit 9; \
             sleep 0.01; done; echo 'C passed'"
        ),
    );
    let second = scratch.file("second", &format!("echo 'C failed'; : > {release}"));
    let asides = beside(&scratch.me(), &[first, second], 2).unwrap();
    let text: Vec<&str> = asides.iter().map(|aside| aside.text.as_str()).collect();
    assert_eq!(text, ["C passed\n", "C failed\n"]);
    assert!(asides.iter().all(|aside| aside.ended));
}

#[test]
fn one_worker_runs_every_file_and_no_file_answers_nothing() {
    let scratch = Scratch::new();
    let none: Vec<PathBuf> = Vec::new();
    assert!(beside(&scratch.me(), &none, 4).unwrap().is_empty());
    let files: Vec<PathBuf> = (0..5)
        .map(|at| {
            scratch.file(
                &format!("f{at}"),
                &format!("echo 'C passed'; echo 'W {at}'"),
            )
        })
        .collect();
    let asides = beside(&scratch.me(), &files, 1).unwrap();
    let text: Vec<&str> = asides.iter().map(|aside| aside.text.as_str()).collect();
    assert_eq!(
        text,
        [
            "C passed\nW 0\n",
            "C passed\nW 1\n",
            "C passed\nW 2\n",
            "C passed\nW 3\n",
            "C passed\nW 4\n",
        ]
    );
}

#[test]
fn more_workers_than_files_answer_every_file_once() {
    let scratch = Scratch::new();
    let files: Vec<PathBuf> = (0..3)
        .map(|at| {
            scratch.file(
                &format!("f{at}"),
                &format!("echo 'C passed'; echo 'W {at}'"),
            )
        })
        .collect();
    let asides = beside(&scratch.me(), &files, 16).unwrap();
    let text: Vec<&str> = asides.iter().map(|aside| aside.text.as_str()).collect();
    assert_eq!(
        text,
        ["C passed\nW 0\n", "C passed\nW 1\n", "C passed\nW 2\n"]
    );
}

#[test]
fn a_program_that_does_not_start_is_an_error() {
    let scratch = Scratch::new();
    let files = [scratch.file("f", "echo 'C passed'")];
    assert!(beside(Path::new("/definitely/missing/binary"), &files, 2).is_err());
}

/// The parameters one statement carries, in the order they are written,
/// with the ones inside a string, a comment or brackets left as text.
#[test]
fn the_parameters_of_a_statement_are_the_ones_outside_a_string() {
    use crate::suite::parameters;
    assert_eq!(
        parameters("INSERT INTO t VALUES(:1,?,:abc)"),
        [
            (b':', "1".to_owned()),
            (b'?', String::new()),
            (b':', "abc".to_owned())
        ]
    );
    // A `?N` names its place, and a `$name(...)` carries the brackets.
    assert_eq!(parameters("SELECT ?99"), [(b'?', "99".to_owned())]);
    assert_eq!(
        parameters("SELECT $x(-z-), @a, $::two"),
        [
            (b'$', "x(-z-)".to_owned()),
            (b'@', "a".to_owned()),
            (b'$', "::two".to_owned())
        ]
    );
    // A parameter inside a string, a comment or brackets is text.
    assert!(parameters("SELECT ':a', \"@b\", [$c] -- :d\n/* ?1 */").is_empty());
    // A colon that opens no name is no parameter at all.
    assert!(parameters("SELECT a:").is_empty());
}

/// The place of each parameter, and the name it carries there.
#[test]
fn the_place_of_a_parameter_is_the_one_it_was_first_written_at() {
    use crate::suite::{count_binds, named_parameters};
    assert_eq!(
        named_parameters("INSERT INTO t VALUES(:1,?,:abc)"),
        [":1", "", ":abc"]
    );
    assert_eq!(count_binds("INSERT INTO t VALUES(:1,?,:abc)"), 3);
    // A name written twice stands at one place.
    assert_eq!(named_parameters("SELECT :a, :b, :a"), [":a", ":b"]);
    assert_eq!(count_binds("SELECT :a, :b, :a"), 2);
    // A `?N` names the place it carries, and the `?` after it takes the
    // next one.
    assert_eq!(count_binds("SELECT ?4, ?"), 5);
    assert_eq!(named_parameters("SELECT ?2, ?"), ["", "", ""]);
    assert_eq!(count_binds("SELECT 1"), 0);
}

/// What the tester bound, written into the statement as literals.
#[test]
fn what_is_bound_is_written_into_the_statement_as_a_literal() {
    use crate::suite::bound_into;
    use std::collections::BTreeMap;
    let mut bound = BTreeMap::new();
    bound.insert(1, "'one'".to_owned());
    bound.insert(3, "3".to_owned());
    assert_eq!(
        bound_into("INSERT INTO t VALUES(:1,?,:abc)", &bound),
        "INSERT INTO t VALUES('one',NULL,3)"
    );
    // A statement of no parameter is the statement itself.
    assert_eq!(bound_into("SELECT 1", &bound), "SELECT 1");
    // A name written twice takes the one value bound at its place.
    let mut held = BTreeMap::new();
    held.insert(1, "7".to_owned());
    assert_eq!(bound_into("SELECT :a + :a", &held), "SELECT 7 + 7");
}

/// The name `sqlite3_next_stmt` answers for each name a connection
/// holds, which is a walk over them from the first to the last.
#[test]
fn the_statement_after_one_is_the_next_name_the_connection_holds() {
    use crate::suite::after_name;
    let held: Vec<String> = ["A", "B", "C"]
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    assert_eq!(after_name(&held, "0"), "A");
    assert_eq!(after_name(&held, ""), "A");
    assert_eq!(after_name(&held, "A"), "B");
    assert_eq!(after_name(&held, "C"), "");
    // A name the connection does not hold has nothing after it, and a
    // connection that holds none answers nothing for the first.
    assert_eq!(after_name(&held, "Z"), "");
    assert_eq!(after_name(&[], "0"), "");
}

/// The value a function of the tester answered, read off the kind of
/// value it named and the text beside it.
#[test]
fn what_a_function_answered_is_the_kind_it_named() {
    use crate::suite::valued;
    use db_sqlite::value::Value;
    let named = |kind: &str, text: &str| valued(&[kind.to_owned(), text.to_owned()]);
    assert_eq!(named("int", "12"), Value::Int(12));
    assert_eq!(named("real", "2.5"), Value::Real(2.5));
    assert_eq!(named("text", "abc"), Value::Text(b"abc".to_vec()));
    assert_eq!(named("blob", "abc"), Value::Blob(b"abc".to_vec()));
    assert_eq!(named("null", ""), Value::Null);
    // A number the kind does not hold and a count of values that is not
    // two are NULL.
    assert_eq!(named("int", "abc"), Value::Null);
    assert_eq!(valued(&["int".to_owned()]), Value::Null);
}

/// What `sqlite3_stmt_readonly` and `sqlite3_stmt_isexplain` answer for
/// one statement, which is read off its text.
#[test]
fn what_a_statement_writes_and_whether_it_explains() {
    use crate::suite::{explaining, readonly};
    // A statement that reads writes nothing, and one that changes rows
    // or the schema writes.
    assert!(readonly("SELECT * FROM t1"));
    assert!(readonly("  Explain Query Plan SELECT * FROM t1"));
    assert!(!readonly("CREATE TABLE t1(x)"));
    assert!(!readonly("EXPLAIN INSERT INTO t1 VALUES(5)"));
    assert!(!readonly("UPDATE t1 SET x=x+1 WHERE x<0"));
    assert!(!readonly("VACUUM"));
    // A transaction statement writes nothing itself, and the two that
    // take the file do.
    assert!(readonly("BEGIN"));
    assert!(readonly("COMMIT"));
    assert!(readonly("SAVEPOINT one"));
    assert!(readonly("RELEASE one"));
    assert!(!readonly("BEGIN IMMEDIATE"));
    assert!(!readonly("BEGIN EXCLUSIVE"));
    // An `ATTACH` and a `DETACH` change what the connection holds.
    assert!(readonly("ATTACH ':memory:' AS mem1"));
    assert!(readonly("DETACH mem1"));
    // A pragma that asks writes nothing, and one that sets does.
    assert!(readonly("PRAGMA integrity_check"));
    assert!(!readonly("PRAGMA application_id=1234"));
    assert!(!readonly("PRAGMA journal_mode=WAL"));
    assert!(!readonly("PRAGMA wal_checkpoint"));
    assert!(!readonly("PRAGMA main.wal_checkpoint"));
    // `sqlite3_stmt_isexplain` counts the two forms apart.
    assert_eq!(explaining("SELECT * FROM sqlite_master"), 0);
    assert_eq!(explaining(" explain SELECT * FROM sqlite_master"), 1);
    assert_eq!(explaining("  Explain Query Plan select * FROM t1"), 2);
}
