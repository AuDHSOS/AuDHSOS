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
    /// file as `<me> sqlite-suite --one <path>`. This one runs the path
    /// as a shell script, so each test writes what its files say.
    fn me(&self) -> PathBuf {
        let path = self.0.join("me");
        std::fs::write(&path, "#!/bin/sh\nexec sh \"$3\"\n").unwrap();
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
