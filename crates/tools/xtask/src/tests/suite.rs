// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The reader of SQLite's own test files.

use crate::suite::{Step, braced, cases, elements, statements};

/// The name, the statements and the answer of one step, or `None`
/// where the step is not a case.
fn read(step: &Step) -> Option<(&str, &str, &[String])> {
    match step {
        Step::Case { name, sql, want } => Some((name, sql, want)),
        _ => None,
    }
}

#[test]
fn a_case_is_read_from_its_name_its_statements_and_its_answer() {
    let text = "do_execsql_test one-1.2 {\n  SELECT 1;\n} {1}\n";
    let steps = cases(text);
    assert_eq!(steps.len(), 1);
    let (name, sql, want) = read(&steps[0]).unwrap();
    assert_eq!(name, "one-1.2");
    assert_eq!(sql.trim(), "SELECT 1;");
    assert_eq!(want, ["1"]);
}

#[test]
fn the_older_way_of_writing_a_case_is_read_as_one() {
    let text = "do_test two-1 {\n  execsql {\n    SELECT 2;\n  }\n} {2}\n";
    let steps = cases(text);
    assert_eq!(steps.len(), 1);
    let (name, sql, want) = read(&steps[0]).unwrap();
    assert_eq!(name, "two-1");
    assert_eq!(sql.trim(), "SELECT 2;");
    assert_eq!(want, ["2"]);
    // A body that runs more than the statements is not a case. One
    // that reads alone leaves the database as the cases after it read
    // it; one that may have written stops the file.
    for (text, writes) in [
        ("do_test a {\n  execsql {SELECT 1}\n  set x 1\n} {1}", false),
        ("do_test a {\n  catchsql {SELECT 1}\n} {1}", false),
        (
            "do_test a {\n  execsql {INSERT INTO t VALUES(1)}\n  set x 1\n} {}",
            true,
        ),
        ("do_test a {\n  set x [db eval {SELECT 1}]\n} {1}", false),
        ("do_test a {\n  set x [sqlite3 db2 test.db]\n} {1}", true),
        ("do_test a {\n  set x [forcedelete test.db]\n} {1}", true),
    ] {
        let steps = cases(text);
        assert_eq!(steps.len(), 1, "{text}");
        assert!(
            matches!(steps[0], Step::Opaque(held) if held == writes),
            "{text}"
        );
    }
}

#[test]
fn statements_outside_a_case_are_the_file_setting_itself_up() {
    let text = "execsql {\n  CREATE TABLE t(a);\n}\ndo_execsql_test one {SELECT 1} {1}\n";
    let steps = cases(text);
    assert_eq!(steps.len(), 2);
    assert!(matches!(&steps[0], Step::Setup(sql) if sql.trim() == "CREATE TABLE t(a);"));
    assert_eq!(read(&steps[1]).unwrap().0, "one");
    // The `execsql` inside a case is the case, not setup.
    assert_eq!(cases("do_test a {execsql {SELECT 1}} {1}").len(), 1);
    // A case that expects a refusal is read past whole, and stops the
    // file, because this harness does not run it.
    let steps = cases("do_catchsql_test a {SELECT 1} {1 {oops}}\nexecsql {SELECT 2}");
    assert_eq!(steps.len(), 2);
    assert!(matches!(steps[0], Step::Opaque(false)));
    assert!(matches!(&steps[1], Step::Setup(sql) if sql.trim() == "SELECT 2"));
}

#[test]
fn a_case_whose_text_is_left_to_the_interpreter_is_passed_over() {
    // A substitution says what runs only once the interpreter has run,
    // so neither the statements nor the answer are known here, and the
    // file stops where one stands.
    for text in [
        "do_execsql_test a {SELECT $x} {1}",
        "do_execsql_test a {SELECT 1} {[expr 1]}",
    ] {
        let steps = cases(text);
        assert_eq!(steps.len(), 1, "{text}");
        assert!(matches!(steps[0], Step::Opaque(_)), "{text}");
    }
    // A case with no answer after it expects no row, and one whose
    // answer is a single word needs no braces around it.
    let steps = cases("do_execsql_test a {SELECT 1}\n");
    assert_eq!(read(&steps[0]).unwrap().2, Vec::<String>::new());
    assert_eq!(
        read(&cases("do_execsql_test a {SELECT 1} 1\n")[0])
            .unwrap()
            .2,
        ["1"]
    );
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
