// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::spdx`.

use crate::policy::{PORTED_HEADER, SPDX_HEADER};
use crate::spdx::{comment_prefix, expected_header, header_problem};
use std::path::Path;

const GOOD: &str = "// SPDX-License-Identifier: AGPL-3.0-only\n// Copyright (C) 2026 Manuel Baesler and contributors\n\nfn main() {}\n";

#[test]
fn correct_header_passes() {
    assert_eq!(header_problem(GOOD, "//", &SPDX_HEADER), None);
    assert_eq!(
        header_problem(&GOOD.replace("//", "#"), "#", &SPDX_HEADER),
        None
    );
}

#[test]
fn missing_moved_and_wrong_headers_are_reported() {
    assert!(
        header_problem("fn main() {}\n", "//", &SPDX_HEADER)
            .unwrap()
            .contains("line 1")
    );
    assert!(
        header_problem("", "//", &SPDX_HEADER)
            .unwrap()
            .contains("missing header line 1")
    );
    let second_line = format!("\n{GOOD}");
    assert!(
        header_problem(&second_line, "//", &SPDX_HEADER)
            .unwrap()
            .contains("line 1")
    );
    let wrong_license = GOOD.replace("AGPL-3.0-only", "MIT");
    assert!(
        header_problem(&wrong_license, "//", &SPDX_HEADER)
            .unwrap()
            .contains("expected")
    );
    let only_first = "// SPDX-License-Identifier: AGPL-3.0-only\n";
    assert!(
        header_problem(only_first, "//", &SPDX_HEADER)
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

#[test]
fn a_ported_file_is_held_to_the_header_that_names_both_licences() {
    let ported = Path::new("crates/support/fuzz/src/mutate.rs");
    assert_eq!(expected_header(ported), &PORTED_HEADER);
    assert_eq!(
        expected_header(Path::new("crates/support/fuzz/src/rng.rs")),
        &SPDX_HEADER
    );
    let mut header = String::new();
    for line in PORTED_HEADER {
        header.push_str("// ");
        header.push_str(line);
        header.push('\n');
    }
    assert_eq!(header_problem(&header, "//", &PORTED_HEADER), None);
    assert!(header_problem(&header, "//", &SPDX_HEADER).is_some());
}
