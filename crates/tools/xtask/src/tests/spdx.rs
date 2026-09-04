// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::spdx`.

use crate::spdx::{comment_prefix, header_problem};
use std::path::Path;

const GOOD: &str = "// SPDX-License-Identifier: AGPL-3.0-only\n// Copyright (C) 2026 Manuel Baesler and contributors\n\nfn main() {}\n";

#[test]
fn correct_header_passes() {
    assert_eq!(header_problem(GOOD, "//"), None);
    assert_eq!(header_problem(&GOOD.replace("//", "#"), "#"), None);
}

#[test]
fn missing_moved_and_wrong_headers_are_reported() {
    assert!(
        header_problem("fn main() {}\n", "//")
            .unwrap()
            .contains("line 1")
    );
    assert!(
        header_problem("", "//")
            .unwrap()
            .contains("missing header line 1")
    );
    let second_line = format!("\n{GOOD}");
    assert!(
        header_problem(&second_line, "//")
            .unwrap()
            .contains("line 1")
    );
    let wrong_license = GOOD.replace("AGPL-3.0-only", "MIT");
    assert!(
        header_problem(&wrong_license, "//")
            .unwrap()
            .contains("expected")
    );
    let only_first = "// SPDX-License-Identifier: AGPL-3.0-only\n";
    assert!(
        header_problem(only_first, "//")
            .unwrap()
            .contains("missing header line 2")
    );
}

#[test]
fn prefixes_follow_the_policy_and_exempt_files_are_skipped() {
    assert_eq!(comment_prefix(Path::new("a/b.rs")), Some("//"));
    assert_eq!(comment_prefix(Path::new("Cargo.toml")), Some("#"));
    assert_eq!(comment_prefix(Path::new("ci.yml")), Some("#"));
    assert_eq!(comment_prefix(Path::new("Cargo.lock")), None);
    assert_eq!(comment_prefix(Path::new("README.md")), None);
    assert_eq!(comment_prefix(Path::new("LICENSE")), None);
}
