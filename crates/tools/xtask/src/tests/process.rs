// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::process`.

use crate::process::{Cmd, run_parallel, run_parallel_report};
use std::path::Path;

#[test]
fn display_joins_program_and_arguments() {
    let cmd = Cmd::new("echo")
        .arg("a")
        .args(["b", "c"])
        .env("K", "V")
        .cwd(Path::new("."));
    assert_eq!(cmd.display(), "echo a b c");
}

#[test]
fn capture_returns_stdout_and_failures_are_errors() {
    assert_eq!(Cmd::new("echo").arg("hi").capture().unwrap().trim(), "hi");
    assert!(Cmd::new("false").run().is_err());
    assert!(Cmd::new("/definitely/missing/binary").run().is_err());
    assert!(Cmd::new("sh").args(["-c", "exit 3"]).capture().is_err());
}

#[test]
fn toolchain_binary_lives_next_to_cargo() {
    let cmd = Cmd::toolchain_binary("rustc");
    assert!(cmd.display().ends_with("rustc"));
    assert!(Cmd::cargo().display().ends_with("cargo"));
    assert!(Cmd::cargo_plain().display().ends_with("cargo"));
}

/// Each test owns its scratch directory, also across concurrent xtask runs.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("audhsos-process-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn shell(&self, script: &str) -> Cmd {
        Cmd::new("sh").args(["-c", script]).cwd(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn parallel_jobs_report_complete_streams_in_completion_order() {
    let scratch = Scratch::new();
    // The first process can finish only after the reporter receives the
    // second. A sequential runner or input-order reporter fails this test.
    let slow = scratch.shell(
        r#"i=0; while [ ! -f release ]; do i=$((i+1)); [ "$i" -lt 1000 ] || exit 9; sleep 0.01; done; printf slow; printf slow-err >&2"#,
    );
    let fast = scratch
        .shell("printf '%s' \"$PAYLOAD\"; printf fast-err >&2")
        .env("PAYLOAD", "fast");
    let mut completed = Vec::new();
    run_parallel_report(&[slow, fast], 2, |_, result| {
        let output = result.unwrap();
        assert!(output.status.success());
        if output.stdout == b"fast" {
            assert_eq!(output.stderr, b"fast-err");
            std::fs::write(scratch.0.join("release"), "").unwrap();
        } else {
            assert_eq!(output.stderr, b"slow-err");
        }
        completed.push(output.stdout);
        Ok(())
    })
    .unwrap();
    assert_eq!(completed, [b"fast".to_vec(), b"slow".to_vec()]);
}

#[test]
fn one_worker_never_overlaps_children() {
    let scratch = Scratch::new();
    let command = scratch.shell("mkdir occupied || exit 9; sleep 0.01; rmdir occupied");
    run_parallel(&vec![command; 6], 1).unwrap();
    run_parallel(&[], 2).unwrap();
}

#[test]
fn failures_propagate_after_the_remaining_jobs_finish() {
    let scratch = Scratch::new();
    let result = run_parallel(
        &[
            scratch.shell("printf failure >&2; exit 7"),
            scratch.shell("printf done > finished"),
        ],
        1,
    );
    assert!(matches!(
        result,
        Err(crate::error::Error::CommandFailed { code: Some(7), .. })
    ));
    assert_eq!(std::fs::read(scratch.0.join("finished")).unwrap(), b"done");
    assert!(run_parallel(&[Cmd::new("/definitely/missing/binary")], 2).is_err());
}
