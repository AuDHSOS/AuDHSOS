// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::process`.

use crate::process::Cmd;
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
    assert!(
        Cmd::new("sh")
            .args(["-c", "echo err 1>&2"])
            .capture_stderr()
            .unwrap()
            .contains("err")
    );
    assert!(
        Cmd::new("sh")
            .args(["-c", "exit 3"])
            .capture_stderr()
            .is_err()
    );
}

#[test]
fn toolchain_binary_lives_next_to_cargo() {
    let cmd = Cmd::toolchain_binary("rustc");
    assert!(cmd.display().ends_with("rustc"));
    assert!(Cmd::cargo().display().ends_with("cargo"));
    assert!(Cmd::cargo_plain().display().ends_with("cargo"));
}
