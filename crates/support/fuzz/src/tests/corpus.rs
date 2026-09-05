// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::corpus`, covering the regression replay item of the
//! catalog 6.6.53. They touch the file system, which Miri does not have,
//! so Miri skips them; what Miri covers of this crate is the entry glue,
//! which is where the `unsafe` is.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::corpus::{Outcome, files_under, replay_args, replay_paths};

/// A directory of its own for one test, removed when the test ends.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("audhsos-fuzz-support-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the scratch directory is creatable");
        Scratch { path }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let file = self.path.join(name);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).expect("the parent is creatable");
        }
        std::fs::write(&file, bytes).expect("the file is writable");
        file
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
#[cfg_attr(miri, ignore = "Miri runs without a file system")]
fn every_file_of_a_directory_is_replayed_once_in_path_order() {
    let scratch = Scratch::new("directory");
    scratch.write("b", b"second");
    scratch.write("a", b"first");
    scratch.write("nested/c", b"third");
    let mut seen: Vec<Vec<u8>> = Vec::new();
    let outcome = replay_paths(std::slice::from_ref(&scratch.path), |bytes| {
        seen.push(bytes.to_vec());
    })
    .expect("the directory is readable");
    assert_eq!(
        outcome,
        Outcome {
            files: 3,
            bytes: 6 + 5 + 5
        }
    );
    assert_eq!(
        seen,
        vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()]
    );
}

#[test]
#[cfg_attr(miri, ignore = "Miri runs without a file system")]
fn a_path_that_names_a_file_replays_that_file() {
    let scratch = Scratch::new("single");
    let file = scratch.write("input", b"abc");
    let mut seen = Vec::new();
    let outcome = replay_paths(std::slice::from_ref(&file), |bytes| {
        seen.push(bytes.to_vec());
    })
    .expect("the file is readable");
    assert_eq!(outcome, Outcome { files: 1, bytes: 3 });
    assert_eq!(seen, vec![b"abc".to_vec()]);
}

#[test]
#[cfg_attr(miri, ignore = "Miri runs without a file system")]
fn an_empty_corpus_file_reaches_the_body() {
    let scratch = Scratch::new("empty-file");
    scratch.write("empty", b"");
    let mut seen = Vec::new();
    let outcome = replay_paths(std::slice::from_ref(&scratch.path), |bytes| {
        seen.push(bytes.len());
    })
    .expect("the directory is readable");
    assert_eq!(outcome, Outcome { files: 1, bytes: 0 });
    assert_eq!(seen, vec![0]);
}

#[test]
#[cfg_attr(miri, ignore = "Miri runs without a file system")]
fn no_path_replays_nothing() {
    let mut calls = 0u32;
    let outcome = replay_paths(&[], |_| {
        calls += 1;
    })
    .expect("nothing to read");
    assert_eq!(outcome, Outcome::default());
    assert_eq!(calls, 0);
}

#[test]
#[cfg_attr(miri, ignore = "Miri runs without a file system")]
fn an_empty_corpus_directory_replays_nothing() {
    let scratch = Scratch::new("empty-directory");
    let mut calls = 0u32;
    let outcome = replay_paths(std::slice::from_ref(&scratch.path), |_| {
        calls += 1;
    })
    .expect("the directory is readable");
    assert_eq!(outcome, Outcome::default());
    assert_eq!(calls, 0);
}

#[test]
#[cfg_attr(miri, ignore = "Miri runs without a file system")]
fn a_path_that_is_not_there_is_an_error_that_names_it() {
    let missing = std::env::temp_dir().join("audhsos-fuzz-support-missing-corpus");
    let _ = std::fs::remove_dir_all(&missing);
    assert_eq!(files_under(&missing), vec![missing.clone()]);
    let mut calls = 0u32;
    let error = replay_paths(std::slice::from_ref(&missing), |_| {
        calls += 1;
    })
    .expect_err("the path is not there");
    assert_eq!(error.path, missing);
    let text = format!("{error}");
    assert!(
        text.contains("audhsos-fuzz-support-missing-corpus"),
        "{text}"
    );
    assert!(!format!("{error:?}").is_empty());
    assert_eq!(calls, 0);
}

#[test]
#[cfg_attr(miri, ignore = "Miri runs without a file system")]
fn the_error_carries_the_reason_the_file_system_gave() {
    use std::error::Error;
    let missing = std::env::temp_dir().join("audhsos-fuzz-support-missing-source");
    let _ = std::fs::remove_dir_all(&missing);
    let error =
        replay_paths(std::slice::from_ref(&missing), |_| {}).expect_err("the path is not there");
    assert!(error.source().is_some());
}

#[test]
#[cfg_attr(miri, ignore = "Miri runs without a file system")]
fn the_command_line_replay_reports_what_it_did() {
    let scratch = Scratch::new("arguments");
    scratch.write("one", b"ab");
    scratch.write("two", b"cde");
    let mut seen = 0usize;
    let code = replay_args([OsString::from(scratch.path.as_os_str())], |bytes| {
        seen = seen.saturating_add(bytes.len());
    });
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    assert_eq!(seen, 5);
}

#[test]
#[cfg_attr(miri, ignore = "Miri runs without a file system")]
fn the_command_line_replay_fails_on_a_path_that_is_not_there() {
    let missing = std::env::temp_dir().join("audhsos-fuzz-support-missing-argument");
    let _ = std::fs::remove_dir_all(&missing);
    let code = replay_args([OsString::from(missing.as_os_str())], |_| {});
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
}

#[test]
#[cfg_attr(miri, ignore = "Miri runs without a file system")]
fn no_argument_replays_nothing() {
    let mut calls = 0u32;
    let code = replay_args([], |_| {
        calls += 1;
    });
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    assert_eq!(calls, 0);
}
