// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::options`.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::options::{
    DEFAULT_LEN_CONTROL, DEFAULT_MAX_LEN, DEFAULT_TIMEOUT, Mode, OptionError, parse,
    parse_dictionary,
};

/// The command line of `words`.
fn line(words: &[&str]) -> Vec<OsString> {
    words.iter().map(OsString::from).collect()
}

#[test]
fn an_empty_command_line_is_a_fuzzing_run_with_the_defaults() {
    let options = parse(line(&[])).unwrap();
    assert_eq!(options.mode, Mode::Fuzz);
    assert!(options.paths.is_empty());
    assert_eq!(options.max_total_time, None);
    assert_eq!(options.runs, None);
    assert_eq!(options.max_len, DEFAULT_MAX_LEN);
    assert_eq!(options.len_control, DEFAULT_LEN_CONTROL);
    assert_eq!(options.timeout, DEFAULT_TIMEOUT);
    assert_eq!(options.seed, None);
    assert!(!options.value_profile);
    assert!(options.reduce_inputs);
    assert!(!options.print_final_stats);
}

#[test]
fn every_flag_the_engine_has_is_read() {
    let options = parse(line(&[
        "corpus",
        "-max_total_time=30",
        "-runs=1000",
        "-max_len=64",
        "-len_control=7",
        "-seed=99",
        "-timeout=5",
        "-artifact_prefix=/tmp/",
        "-dict=words.txt",
        "-use_value_profile=1",
        "-reduce_inputs=0",
        "-print_final_stats=1",
    ]))
    .unwrap();
    assert_eq!(options.paths, vec![PathBuf::from("corpus")]);
    assert_eq!(options.max_total_time, Some(30));
    assert_eq!(options.runs, Some(1000));
    assert_eq!(options.max_len, 64);
    assert_eq!(options.len_control, 7);
    assert_eq!(options.seed, Some(99));
    assert_eq!(options.timeout, 5);
    assert_eq!(options.artifact_prefix, PathBuf::from("/tmp/"));
    assert_eq!(options.dictionary, Some(PathBuf::from("words.txt")));
    assert!(options.value_profile);
    assert!(!options.reduce_inputs);
    assert!(options.print_final_stats);
}

#[test]
fn a_zero_time_or_seed_means_no_limit_and_no_seed() {
    let options = parse(line(&["-max_total_time=0", "-seed=0", "-runs=-1"])).unwrap();
    assert_eq!(options.max_total_time, None);
    assert_eq!(options.seed, None);
    assert_eq!(options.runs, None);
}

#[test]
fn the_modes_are_told_apart_and_asked_for_what_they_need() {
    assert_eq!(
        parse(line(&["-runs_once=1", "x"])).unwrap().mode,
        Mode::RunOnce
    );
    assert_eq!(
        parse(line(&["-merge=1", "a", "b"])).unwrap().mode,
        Mode::Merge
    );
    assert_eq!(
        parse(line(&["-minimize_crash=1", "f"])).unwrap().mode,
        Mode::MinimizeCrash
    );
    assert_eq!(parse(line(&["-merge=0", "a"])).unwrap().mode, Mode::Fuzz);
    assert_eq!(
        parse(line(&["-merge=1", "a"])).unwrap_err(),
        OptionError::MissingPath("-merge=1".to_owned())
    );
    assert_eq!(
        parse(line(&["-minimize_crash=1"])).unwrap_err(),
        OptionError::MissingPath("-minimize_crash=1".to_owned())
    );
}

#[test]
fn a_flag_that_is_wrong_is_named_rather_than_ignored() {
    assert_eq!(
        parse(line(&["-nonsense=1"])).unwrap_err(),
        OptionError::Unknown("nonsense".to_owned())
    );
    assert_eq!(
        parse(line(&["-jobs=4"])).unwrap_err(),
        OptionError::Unsupported("jobs".to_owned())
    );
    assert_eq!(
        parse(line(&["-runs=many"])).unwrap_err(),
        OptionError::NotANumber("runs".to_owned())
    );
}

#[test]
fn every_refusal_says_what_is_wrong() {
    let messages = [
        OptionError::Unknown("x".to_owned()).to_string(),
        OptionError::Unsupported("fork".to_owned()).to_string(),
        OptionError::NotANumber("runs".to_owned()).to_string(),
        OptionError::MissingPath("-merge=1".to_owned()).to_string(),
    ];
    for message in messages {
        assert!(!message.is_empty());
    }
}

#[test]
fn a_dictionary_is_read_the_way_libfuzzer_writes_one() {
    let text =
        "# a comment\n\nname=\"AB\"\n\"\\x30\\x31\"\n\"a\\\\b\\\"c\"\n\"\\n\\r\\t\\q\"\nbroken\n";
    let words = parse_dictionary(text);
    assert_eq!(
        words,
        vec![
            b"AB".to_vec(),
            b"01".to_vec(),
            b"a\\b\"c".to_vec(),
            b"\n\r\tq".to_vec(),
        ]
    );
}

#[test]
fn a_dictionary_line_that_ends_in_the_middle_of_an_escape_is_left_out() {
    assert_eq!(parse_dictionary("\"ab\\\"\n"), Vec::<Vec<u8>>::new());
    assert_eq!(parse_dictionary("\"\\x2\"\n"), Vec::<Vec<u8>>::new());
    assert_eq!(parse_dictionary("\"\\xzz\"\n"), Vec::<Vec<u8>>::new());
}
