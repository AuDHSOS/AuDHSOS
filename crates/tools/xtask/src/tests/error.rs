// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::error`.

use crate::error::Error;

#[test]
fn messages_mention_the_relevant_detail() {
    let io = Error::io("reading x", std::io::Error::other("boom"));
    assert!(io.to_string().contains("reading x") && io.to_string().contains("boom"));
    let failed = Error::CommandFailed {
        command: "cargo build".to_owned(),
        code: Some(101),
    };
    assert!(failed.to_string().contains("101"));
    let signal = Error::CommandFailed {
        command: "qemu".to_owned(),
        code: None,
    };
    assert!(signal.to_string().contains("signal"));
    let violations = Error::Violations(vec!["a".to_owned(), "b".to_owned()]);
    assert!(violations.to_string().starts_with("2 violation(s)"));
    assert_eq!(Error::Usage("u".to_owned()).to_string(), "u");
    assert_eq!(Error::Toolchain("t".to_owned()).to_string(), "t");
    assert_eq!(Error::Parse("p".to_owned()).to_string(), "p");
}

#[test]
fn empty_violation_list_is_ok() {
    assert!(Error::from_violations(Vec::new()).is_ok());
    assert!(Error::from_violations(vec!["x".to_owned()]).is_err());
}
