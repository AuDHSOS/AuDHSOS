// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::engine`. They drive `sh` and `cat` rather than a
//! database, because what is tested here is the process and its streams.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::engine::{Engine, Sqlite3};

/// A shell that runs `script` and reads its standard input.
fn shell(script: &str, timeout: Duration) -> Sqlite3 {
    Sqlite3 {
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_owned(), script.to_owned()],
        timeout,
    }
}

#[test]
fn the_shell_is_given_an_in_memory_database_and_no_startup_file() {
    let engine = Sqlite3::new(Path::new("/usr/bin/sqlite3"), Duration::from_secs(1));
    assert_eq!(engine.args, ["-batch", "-init", "/dev/null", ":memory:"]);
    assert_eq!(engine.program, PathBuf::from("/usr/bin/sqlite3"));
}

#[test]
fn what_the_script_writes_comes_back_as_the_output() {
    let mut engine = Sqlite3 {
        program: PathBuf::from("/bin/cat"),
        args: Vec::new(),
        timeout: Duration::from_secs(10),
    };
    let run = engine.run("SELECT 1;\n").unwrap();
    assert_eq!(run.stdout, "SELECT 1;\n");
    assert!(run.stderr.is_empty());
    assert!(!run.timed_out);
}

#[test]
fn what_the_engine_complains_about_comes_back_separately() {
    let mut engine = shell(
        "cat >/dev/null; echo boom >&2; exit 1",
        Duration::from_secs(10),
    );
    let run = engine.run("SELECT 1;\n").unwrap();
    assert_eq!(run.stderr.trim(), "boom");
    assert!(run.stdout.is_empty());
    assert!(!run.timed_out);
}

#[test]
fn a_case_that_does_not_finish_is_killed_and_waits_for_nothing_else() {
    let mut engine = shell("sleep 30 | cat", Duration::from_millis(50));
    let started = std::time::Instant::now();
    let run = engine.run("").unwrap();
    assert!(run.timed_out);
    assert!(run.stdout.is_empty());
    // The shell's own child outlives the kill and holds the pipe; a run
    // that waited for it would wait out the whole sleep.
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn an_engine_that_cannot_be_started_is_an_error() {
    let mut engine = Sqlite3::new(
        Path::new("/definitely/missing/sqlite3"),
        Duration::from_secs(1),
    );
    assert!(engine.run("SELECT 1;").is_err());
}

#[test]
fn a_script_larger_than_a_pipe_buffer_is_not_a_deadlock() {
    let mut engine = Sqlite3 {
        program: PathBuf::from("/bin/cat"),
        args: Vec::new(),
        timeout: Duration::from_secs(30),
    };
    let script = "SELECT 1;\n".repeat(200_000);
    let run = engine.run(&script).unwrap();
    assert_eq!(run.stdout.len(), script.len());
    assert!(!run.timed_out);
}
