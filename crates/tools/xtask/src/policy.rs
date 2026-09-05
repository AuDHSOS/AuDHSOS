// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The policy tables: which crates exist, what they may depend on, where
//! `unsafe` is allowed and how much, which files need the SPDX header, the
//! coverage thresholds, and the fuzz targets.

/// What kind of crate this is under the safety policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Logic crate: `#![forbid(unsafe_code)]`, no assembly.
    Logic,
    /// Adapter crate: `unsafe` and inline assembly within a budget.
    Adapter {
        /// Maximum number of `unsafe` sites (blocks, functions, impls, traits).
        unsafe_budget: u32,
        /// Maximum number of `asm!` and `naked_asm!` sites.
        asm_budget: u32,
    },
    /// Host tool or test support: `#![forbid(unsafe_code)]`, no assembly.
    Host,
}

/// One workspace crate.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Crate {
    /// Package name.
    pub(crate) name: &'static str,
    /// Path relative to the workspace root.
    pub(crate) path: &'static str,
    /// Safety kind.
    pub(crate) kind: Kind,
    /// Workspace crates it may depend on (normal and build dependencies).
    pub(crate) deps: &'static [&'static str],
    /// Whether the coverage thresholds apply.
    pub(crate) coverage_gate: bool,
}

/// Every workspace crate, in layer order.
pub(crate) const CRATES: &[Crate] = &[
    Crate {
        name: "audhsos-abi",
        path: "crates/abi",
        kind: Kind::Logic,
        deps: &["test-support"],
        coverage_gate: true,
    },
    Crate {
        name: "audhsos-sync",
        path: "crates/sync",
        kind: Kind::Adapter {
            unsafe_budget: 2,
            asm_budget: 0,
        },
        deps: &[],
        coverage_gate: true,
    },
    Crate {
        name: "audhsos-elf",
        path: "crates/elf",
        kind: Kind::Logic,
        deps: &["test-support"],
        coverage_gate: true,
    },
    Crate {
        name: "audhsos-uefi",
        path: "crates/uefi",
        kind: Kind::Logic,
        deps: &["audhsos-abi"],
        coverage_gate: true,
    },
    Crate {
        name: "driver-uart16550",
        path: "crates/drivers/uart16550",
        kind: Kind::Logic,
        deps: &[],
        coverage_gate: true,
    },
    Crate {
        name: "kernel-x86-tables",
        path: "crates/kernel/x86-tables",
        kind: Kind::Logic,
        deps: &[],
        coverage_gate: true,
    },
    Crate {
        name: "kernel-types",
        path: "crates/kernel/types",
        kind: Kind::Logic,
        deps: &["audhsos-abi", "test-support"],
        coverage_gate: true,
    },
    Crate {
        name: "kernel-hal-api",
        path: "crates/kernel/hal-api",
        kind: Kind::Logic,
        deps: &["kernel-types"],
        coverage_gate: true,
    },
    Crate {
        name: "kernel-objects",
        path: "crates/kernel/objects",
        kind: Kind::Logic,
        deps: &["kernel-types", "audhsos-abi", "test-support"],
        coverage_gate: true,
    },
    Crate {
        name: "kernel-mm",
        path: "crates/kernel/mm",
        kind: Kind::Logic,
        deps: &[
            "kernel-types",
            "kernel-hal-api",
            "audhsos-abi",
            "test-support",
        ],
        coverage_gate: true,
    },
    Crate {
        name: "test-support",
        path: "crates/support/testing",
        kind: Kind::Host,
        deps: &[],
        coverage_gate: true,
    },
    Crate {
        name: "xtask",
        path: "crates/tools/xtask",
        kind: Kind::Host,
        deps: &[],
        coverage_gate: false,
    },
];

/// Crates every workspace crate may use as a dev-dependency.
pub(crate) const DEV_DEPENDENCIES: &[&str] = &["test-support"];

/// Crates whose tests run under Miri.
pub(crate) const MIRI_CRATES: &[&str] = &["audhsos-sync"];

/// The two header lines every source file starts with (comment syntax
/// added per file type).
pub(crate) const SPDX_HEADER: [&str; 2] = [
    "SPDX-License-Identifier: AGPL-3.0-only",
    "Copyright (C) 2026 Manuel Baesler and contributors",
];

/// File extensions that carry the header, with their comment prefix.
pub(crate) const HEADER_FILE_TYPES: &[(&str, &str)] = &[
    ("rs", "//"),
    ("toml", "#"),
    ("yml", "#"),
    ("yaml", "#"),
    ("sh", "#"),
];

/// Files that are generated and carry no header.
pub(crate) const HEADER_EXEMPT_FILES: &[&str] = &["Cargo.lock"];

/// Coverage thresholds in percent.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Thresholds {
    /// Minimum line coverage.
    pub(crate) lines: f64,
    /// Minimum branch coverage.
    pub(crate) branches: f64,
}

/// The thresholds for every gated crate.
pub(crate) const COVERAGE: Thresholds = Thresholds {
    lines: 90.0,
    branches: 85.0,
};

/// A fuzz target.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FuzzTarget {
    /// Name of the target crate under `fuzz/`.
    pub(crate) name: &'static str,
}

/// Every fuzz target.
pub(crate) const FUZZ_TARGETS: &[FuzzTarget] = &[];

/// Extensions of assembly files, which must not exist.
pub(crate) const ASSEMBLY_EXTENSIONS: &[&str] = &["S", "s", "asm"];

/// The crate with the given name.
pub(crate) fn find(name: &str) -> Option<&'static Crate> {
    CRATES.iter().find(|c| c.name == name)
}
