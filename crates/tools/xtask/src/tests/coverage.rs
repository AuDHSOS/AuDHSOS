// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::coverage`.

use crate::coverage::{Instrumentation, Totals, crate_of, evaluate, totals_by_crate};
use std::collections::BTreeMap;
use std::path::Path;

#[test]
fn lcov_records_are_summed_per_crate() {
    let root = Path::new("/w");
    let lcov = "SF:/w/crates/abi/src/error.rs\nDA:1,1\nLF:100\nLH:90\nBRF:20\nBRH:16\nend_of_record\n\
SF:crates/abi/src/rights.rs\nLF:50\nLH:50\nBRF:10\nBRH:10\nend_of_record\n\
SF:/w/crates/kernel/types/src/phys.rs\nLF:30\nLH:27\nend_of_record\n\
SF:/w/unknown/src/x.rs\nLF:30\nLH:3\nBRF:2\nBRH:1\nend_of_record\n\
SF:/w/crates/sync/src/lib.rs\nLF:not-a-number\nend_of_record\n";
    let totals = totals_by_crate(lcov, root);
    let abi = totals["audhsos-abi"];
    assert_eq!(
        abi,
        Totals {
            lines: 150,
            missed_lines: 10,
            branches: 30,
            missed_branches: 4
        }
    );
    let types = totals["kernel-types"];
    assert_eq!(
        types,
        Totals {
            lines: 30,
            missed_lines: 3,
            branches: 0,
            missed_branches: 0
        }
    );
    assert_eq!(types.branch_percent(), 100.0);
    assert!(!totals.contains_key("unknown"));
    assert_eq!(totals["audhsos-sync"], Totals::default());
    assert!((abi.line_percent() - 93.333).abs() < 0.01);
}

#[test]
fn crate_lookup_prefers_the_longest_path() {
    let root = Path::new("/w");
    assert_eq!(
        crate_of(Path::new("/w/crates/kernel/types/src/lib.rs"), root),
        Some("kernel-types")
    );
    assert_eq!(
        crate_of(Path::new("/w/crates/abi/src/lib.rs"), root),
        Some("audhsos-abi")
    );
    assert_eq!(crate_of(Path::new("/w/other/src/lib.rs"), root), None);
}

#[test]
fn evaluation_flags_gated_crates_below_the_thresholds_and_missing_data() {
    let mut totals = BTreeMap::new();
    totals.insert(
        "audhsos-abi".to_owned(),
        Totals {
            lines: 100,
            missed_lines: 20,
            branches: 10,
            missed_branches: 0,
        },
    );
    totals.insert(
        "xtask".to_owned(),
        Totals {
            lines: 100,
            missed_lines: 90,
            branches: 0,
            missed_branches: 0,
        },
    );
    let (table, violations) = evaluate(&totals, Instrumentation::Branch);
    assert!(table.contains("audhsos-abi"));
    assert!(table.contains("reported only"));
    assert!(
        violations
            .iter()
            .any(|v| v.contains("audhsos-abi") && v.contains("line coverage"))
    );
    assert!(
        violations
            .iter()
            .any(|v| v.contains("kernel-types") && v.contains("no coverage data"))
    );
    assert!(!violations.iter().any(|v| v.contains("xtask")));
}

#[test]
fn empty_totals_are_full_coverage() {
    assert_eq!(Totals::default().line_percent(), 100.0);
    assert_eq!(Totals::default().branch_percent(), 100.0);
}
