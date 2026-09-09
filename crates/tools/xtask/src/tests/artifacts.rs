// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Cargo artifact discovery, including fresh builds and unusual paths.

use std::path::Path;

use crate::artifacts::{Executable, build_tests, executables_of};
use crate::process::Cmd;

#[test]
fn artifacts_preserve_paths_package_directories_and_test_profiles() {
    let output = r#"compiler progress
{"reason":"compiler-artifact","executable":null}
{"reason":"build-finished","success":true}
{"reason":"compiler-artifact","manifest_path":"/w/Crate ü/Cargo.toml","target":{"name":"unit"},"profile":{"test":true},"executable":"/custom build (ü)/unit-123","fresh":true}
{"reason":"compiler-artifact","manifest_path":"/w/fuzz/elf/Cargo.toml","target":{"name":"elf"},"profile":{"test":false},"executable":"/w/fuzz/target/debug/elf","fresh":false}
"#;
    let executables = executables_of(output).unwrap();
    assert_eq!(executables.len(), 2);
    assert_eq!(executables[0].path, Path::new("/custom build (ü)/unit-123"));
    assert_eq!(executables[0].directory, Path::new("/w/Crate ü"));
    assert!(executables[0].test);
    assert_eq!(executables[1].name, "elf");
    assert!(!executables[1].test);
    assert!(executables_of("no artifacts").unwrap().is_empty());
}

#[test]
fn malformed_artifacts_fail_instead_of_silently_skipping_tests() {
    assert!(executables_of("{broken").is_err());
    for fields in [
        r#""target":{"name":"x"},"profile":{"test":true}"#,
        r#""manifest_path":"/w/Cargo.toml","profile":{"test":true}"#,
        r#""manifest_path":"/w/Cargo.toml","target":{"name":"x"}"#,
    ] {
        assert!(
            executables_of(&format!(
                r#"{{"reason":"compiler-artifact","executable":"/w/test",{fields}}}"#
            ))
            .is_err()
        );
    }
}

#[test]
fn test_builds_exclude_ordinary_binaries_and_require_a_test_executable() {
    let command = Cmd::new("sh").args(["-c", r#"printf '%s\n' '{"reason":"compiler-artifact","manifest_path":"/w/Cargo.toml","target":{"name":"helper"},"profile":{"test":false},"executable":"/w/helper"}' '{"reason":"compiler-artifact","manifest_path":"/w/Cargo.toml","target":{"name":"unit"},"profile":{"test":true},"executable":"/w/unit"}'"#]);
    let executables = build_tests(command).unwrap();
    assert_eq!(executables.len(), 1);
    assert_eq!(executables[0].name, "unit");
    assert!(build_tests(Cmd::new("true")).is_err());
    assert!(build_tests(Cmd::new("false")).is_err());
}

#[test]
fn an_executable_runs_in_its_package_directory() {
    let directory = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let executable = Executable {
        name: "fixture".to_owned(),
        path: "sh".into(),
        directory: directory.clone(),
        test: true,
    };
    let output = executable
        .command()
        .args(["-c", "pwd; printf '%s' \"$CARGO_MANIFEST_DIR\""])
        .capture()
        .unwrap();
    assert_eq!(
        output,
        format!("{}\n{}", directory.display(), directory.display())
    );
}
