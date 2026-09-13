// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The reader of SQLite's own test files.

use crate::suite::{braced, cases, elements, statements};

#[test]
fn a_case_is_read_from_its_name_its_statements_and_its_answer() {
    let text = "do_execsql_test one-1.2 {\n  SELECT 1;\n} {1}\n";
    let read = cases(text);
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].name, "one-1.2");
    assert_eq!(read[0].sql.trim(), "SELECT 1;");
    assert_eq!(read[0].want, ["1"]);
}

#[test]
fn a_case_whose_text_is_left_to_the_interpreter_is_passed_over() {
    // A substitution says what runs only once the interpreter has run,
    // so neither the statements nor the answer are known here.
    assert!(cases("do_execsql_test a {SELECT $x} {1}").is_empty());
    assert!(cases("do_execsql_test a {SELECT 1} {[expr 1]}").is_empty());
    // A case with no answer written after it is not a case.
    assert!(cases("do_execsql_test a {SELECT 1}\n").is_empty());
    assert!(cases("do_execsql_test\n").is_empty());
}

#[test]
fn braces_hold_the_braces_inside_them() {
    assert_eq!(braced("{a{b}c} rest"), Some(("a{b}c", " rest")));
    assert_eq!(braced("  \n {x} y"), Some(("x", " y")));
    assert_eq!(braced("{never closed"), None);
    assert_eq!(braced("x {y}"), None);
    // A brace written with a backslash before it is not a brace.
    assert_eq!(braced("{a\\}b} rest"), Some(("a\\}b", " rest")));
}

#[test]
fn a_list_is_read_as_the_words_it_holds() {
    assert_eq!(elements("1 a"), ["1", "a"]);
    assert_eq!(elements("  1   a  "), ["1", "a"]);
    assert_eq!(elements("1 {} a"), ["1", "", "a"]);
    assert_eq!(elements("{a b} c"), ["a b", "c"]);
    assert_eq!(elements("{a {b} c}"), ["a {b} c"]);
    assert!(elements("").is_empty());
}

#[test]
fn statements_are_parted_by_the_semicolons_outside_a_string() {
    assert_eq!(statements("SELECT 1; SELECT 2"), ["SELECT 1", " SELECT 2"]);
    assert_eq!(statements("SELECT ';'"), ["SELECT ';'"]);
    assert_eq!(
        statements("SELECT \";\" ; SELECT 2"),
        ["SELECT \";\" ", " SELECT 2"]
    );
}
