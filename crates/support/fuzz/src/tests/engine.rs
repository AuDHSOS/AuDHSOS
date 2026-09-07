// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::engine`.
//!
//! A fuzz target proper is built with the instrumentation, which a test
//! binary is not. What stands in for it here is a range of counters this
//! module registers itself and a body that writes into it: the engine sees
//! coverage that answers to its input, which is all it asks of a target.
//!
//! They hold the lock of `super::GLOBALS`, because the counter registry
//! and the trace are one set for the process, and Miri skips them for the
//! same reason it skips the tests of `crate::counters`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::engine::{name_of, outran, run, tick};

use super::{GLOBALS, region_at, register_counters, scratch_path};

/// How many counters the body below writes into.
const COUNTERS: usize = 32;

/// A directory of its own for one test, removed when the test ends.
struct Scratch {
    /// Where it is.
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let path = scratch_path(name);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.path)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The command line of `words`.
fn line(words: &[&str]) -> Vec<OsString> {
    words.iter().map(OsString::from).collect()
}

/// A body that reaches one counter per distinct leading byte, so that the
/// engine sees an input that is worth keeping when it finds a new one, and
/// panics on the one input that is meant to be a find.
fn body(address: usize) -> impl FnMut(&[u8]) {
    move |input: &[u8]| {
        // SAFETY: the caller holds `GLOBALS`, and the engine is not inside
        // `crate::counters` while the body runs.
        let region = unsafe { region_at(address, COUNTERS) };
        let first = input.first().copied().unwrap_or(0);
        let slot = usize::from(first) % COUNTERS;
        region[slot] = region[slot].saturating_add(1);
        assert!(input != b"BOOM", "the input the test is looking for");
    }
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_run_of_a_fixed_number_of_inputs_grows_a_corpus_and_writes_it_out() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let scratch = Scratch::new("fuzz");
    scratch.write("seed", b"\x01seed");
    let mut target = body(address);
    let code = run(
        line(&[
            &scratch.path.to_string_lossy(),
            "-runs=20000",
            "-seed=7",
            "-max_len=32",
            "-print_final_stats=1",
            &format!("-artifact_prefix={}/", scratch.path.display()),
        ]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    assert!(
        scratch.files().len() > 1,
        "the run kept nothing it found: {:?}",
        scratch.files()
    );
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_run_stops_and_writes_the_input_that_made_the_target_panic() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let scratch = Scratch::new("crash");
    scratch.write("boom", b"BOOM");
    let mut target = body(address);
    let code = run(
        line(&[
            &scratch.path.to_string_lossy(),
            "-runs=1",
            &format!("-artifact_prefix={}/", scratch.path.display()),
        ]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
    assert!(
        scratch
            .files()
            .iter()
            .any(|name| name.starts_with("crash-")),
        "no artifact was written: {:?}",
        scratch.files()
    );
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_merge_keeps_the_files_that_add_coverage_and_leaves_out_the_rest() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let destination = Scratch::new("merge-dst");
    let source = Scratch::new("merge-src");
    destination.write("a", b"\x01one");
    source.write("same", b"\x01two");
    source.write("other", b"\x02three");
    let mut target = body(address);
    let code = run(
        line(&[
            "-merge=1",
            &destination.path.to_string_lossy(),
            &source.path.to_string_lossy(),
        ]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    assert_eq!(
        destination.files().len(),
        2,
        "the merge kept the wrong files: {:?}",
        destination.files()
    );
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_run_that_only_reads_reports_what_the_corpus_reaches() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let scratch = Scratch::new("once");
    scratch.write("a", b"\x01a");
    scratch.write("b", b"\x02b");
    let mut target = body(address);
    let code = run(
        line(&[
            "-runs_once=1",
            &scratch.path.to_string_lossy(),
            &format!("-artifact_prefix={}/", scratch.path.display()),
        ]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    assert_eq!(scratch.files().len(), 2, "a reading run wrote something");
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_shrink_makes_a_crashing_input_smaller_and_writes_it_out() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let scratch = Scratch::new("minimize");
    let file = scratch.write("big", b"\x07padding-padding-padding");
    let mut target = move |input: &[u8]| {
        // SAFETY: as in `body`.
        let region = unsafe { region_at(address, COUNTERS) };
        region[0] = region[0].saturating_add(1);
        assert!(
            input.first() != Some(&7),
            "every input that starts with seven"
        );
    };
    let code = run(
        line(&[
            "-minimize_crash=1",
            &file.to_string_lossy(),
            "-runs=4000",
            "-seed=3",
            &format!("-artifact_prefix={}/", scratch.path.display()),
        ]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    let minimized: Vec<String> = scratch
        .files()
        .into_iter()
        .filter(|name| name.starts_with("minimized-from-"))
        .collect();
    assert_eq!(minimized.len(), 1, "{:?}", scratch.files());
    let written = std::fs::read(scratch.path.join(minimized.first().unwrap())).unwrap();
    assert!(
        written.len() < 23,
        "nothing was shrunk: {} bytes",
        written.len()
    );
    assert_eq!(written.first(), Some(&7));
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_shrink_of_an_input_that_does_not_crash_says_so() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let scratch = Scratch::new("minimize-quiet");
    let file = scratch.write("fine", b"nothing wrong here");
    let mut target = body(address);
    let code = run(
        line(&["-minimize_crash=1", &file.to_string_lossy()]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_dictionary_is_read_and_a_missing_one_is_reported_rather_than_fatal() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let scratch = Scratch::new("dict");
    let words = scratch.write("words.txt", b"kw1=\"BEGIN\"\nkw2=\"END\"\n");
    let mut target = body(address);
    let code = run(
        line(&[
            "-runs=200",
            &format!("-dict={}", words.display()),
            &format!("-artifact_prefix={}/", scratch.path.display()),
        ]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    let mut again = body(address);
    let code = run(line(&["-runs=10", "-dict=/nowhere/at/all.txt"]), &mut again);
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    drop(guard);
}

#[test]
fn a_command_line_that_will_not_parse_stops_the_run() {
    let mut target = |_: &[u8]| {};
    let code = run(line(&["-nonsense=1"]), &mut target);
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
}

#[test]
fn one_input_has_one_name() {
    assert_eq!(name_of(b"abc"), name_of(b"abc"));
    assert_ne!(name_of(b"abc"), name_of(b"abd"));
    assert_ne!(name_of(b""), name_of(b"\0"));
    assert_eq!(name_of(b"abc").len(), 32);
    assert!(name_of(b"abc").chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_corpus_file_that_cannot_be_read_is_skipped() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let mut target = body(address);
    let missing = Path::new("/nowhere/at/all");
    let code = run(line(&[&missing.to_string_lossy(), "-runs=10"]), &mut target);
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_run_stops_when_its_time_is_up_and_says_where_it_got_to() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let scratch = Scratch::new("time");
    let mut target = body(address);
    let code = run(
        line(&[
            &scratch.path.to_string_lossy(),
            "-max_total_time=1",
            "-seed=11",
            &format!("-artifact_prefix={}/", scratch.path.display()),
        ]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_run_long_enough_to_report_without_a_find_reports_anyway() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let scratch = Scratch::new("pulse");
    let mut target = body(address);
    // The artifact prefix is set even though the run is not expected to
    // find anything: a run that does find something writes it where it was
    // told, and a test that told it nothing would write into whatever
    // directory the test binary was started in.
    let code = run(
        line(&[
            "-runs=1100000",
            "-seed=13",
            "-max_len=8",
            &format!("-artifact_prefix={}/", scratch.path.display()),
        ]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_place_that_will_not_take_a_file_is_reported_and_does_not_stop_the_run() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let scratch = Scratch::new("unwritable");
    let file = scratch.write("not-a-directory", b"\x01x");
    let mut target = body(address);
    // The corpus path names a file, so nothing a run finds can be written
    // beside it, and the artifact prefix names a directory that is not
    // there, so nothing can be written there either.
    let code = run(
        line(&[
            &file.to_string_lossy(),
            "-runs=3000",
            "-seed=17",
            "-artifact_prefix=/nowhere/at/all/",
        ]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_merge_whose_destination_already_panics_stops_before_it_reads_a_source() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let destination = Scratch::new("merge-bad-dst");
    let source = Scratch::new("merge-bad-src");
    destination.write("boom", b"BOOM");
    source.write("fine", b"\x03three");
    let mut target = body(address);
    let code = run(
        line(&[
            "-merge=1",
            &destination.path.to_string_lossy(),
            &source.path.to_string_lossy(),
            &format!("-artifact_prefix={}/", destination.path.display()),
        ]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_merge_stops_at_a_source_file_that_makes_the_target_panic() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let destination = Scratch::new("merge-boom-dst");
    let source = Scratch::new("merge-boom-src");
    destination.write("a", b"\x01one");
    source.write("boom", b"BOOM");
    let mut target = body(address);
    let code = run(
        line(&[
            "-merge=1",
            &destination.path.to_string_lossy(),
            &source.path.to_string_lossy(),
            &format!("-artifact_prefix={}/", destination.path.display()),
        ]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_shrink_of_a_file_that_is_not_there_says_so() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let mut target = body(address);
    let code = run(
        line(&["-minimize_crash=1", "/nowhere/at/all/file"]),
        &mut target,
    );
    let _ = &address;
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_run_that_keeps_the_value_profile_still_finds_what_it_reaches() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let address = register_counters(COUNTERS);
    let scratch = Scratch::new("value-profile");
    scratch.write("seed", b"\x05seed");
    let mut target = move |input: &[u8]| {
        // SAFETY: as in `body`.
        let region = unsafe { region_at(address, COUNTERS) };
        let slot = usize::from(input.first().copied().unwrap_or(0)) % COUNTERS;
        region[slot] = region[slot].saturating_add(1);
        let mut sum = 0u32;
        for byte in input {
            if *byte == 0x42 {
                sum = sum.wrapping_add(1);
            }
        }
        let _ = sum;
    };
    let code = run(
        line(&[
            &scratch.path.to_string_lossy(),
            "-runs=20000",
            "-seed=19",
            "-use_value_profile=1",
            &format!("-artifact_prefix={}/", scratch.path.display()),
        ]),
        &mut target,
    );
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    drop(guard);
}

#[test]
fn outran_before_a_run_begins_is_false() {
    assert!(!outran(0, 100, 1_000));
}

#[test]
fn outran_without_a_limit_is_false() {
    assert!(!outran(10, 0, 1_000));
}

#[test]
fn outran_inside_the_limit_is_false() {
    assert!(!outran(10, 100, 110));
}

#[test]
fn outran_past_the_limit_is_true() {
    assert!(outran(10, 100, 111));
}

#[test]
fn a_watchdog_tick_inside_the_limit_lets_the_process_run_on() {
    tick(10, 100, 110);
}
