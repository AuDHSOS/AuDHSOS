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

/// What a crate is built and tested for. A `Host` crate runs its tests on
/// the host; being `no_std`, it still compiles for every target. The other
/// crates are built only for their target and tested in QEMU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// Built and tested on the host.
    Host,
    /// Built for `x86_64-unknown-none`.
    X86_64None,
    /// Built for `x86_64-unknown-uefi`.
    X86_64Uefi,
}

impl Target {
    /// Every target that is not the host, in a fixed order.
    pub(crate) const CROSS: [Target; 2] = [Target::X86_64None, Target::X86_64Uefi];

    /// The target triple, or `None` for the host.
    pub(crate) const fn triple(self) -> Option<&'static str> {
        match self {
            Target::Host => None,
            Target::X86_64None => Some("x86_64-unknown-none"),
            Target::X86_64Uefi => Some("x86_64-unknown-uefi"),
        }
    }
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
    /// What the crate is built and tested for.
    pub(crate) target: Target,
}

/// Every workspace crate, in layer order.
pub(crate) const CRATES: &[Crate] = &[
    Crate {
        name: "audhsos-abi",
        path: "crates/abi",
        kind: Kind::Logic,
        deps: &["test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "audhsos-time",
        path: "crates/time",
        kind: Kind::Logic,
        deps: &["test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "audhsos-encoding",
        path: "crates/encoding",
        kind: Kind::Logic,
        deps: &["test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "crypto-ct",
        path: "crates/crypto/ct",
        kind: Kind::Logic,
        deps: &["test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "crypto-hash",
        path: "crates/crypto/hash",
        kind: Kind::Logic,
        deps: &["crypto-ct", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "crypto-aead",
        path: "crates/crypto/aead",
        kind: Kind::Logic,
        deps: &["crypto-ct", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "crypto-ec",
        path: "crates/crypto/ec",
        kind: Kind::Logic,
        deps: &["crypto-ct", "crypto-hash", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "crypto-rng",
        path: "crates/crypto/rng",
        kind: Kind::Logic,
        deps: &["crypto-ct", "crypto-aead", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "audhsos-der",
        path: "crates/net/der",
        kind: Kind::Logic,
        deps: &["audhsos-time", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "audhsos-x509",
        path: "crates/net/x509",
        kind: Kind::Logic,
        deps: &[
            "audhsos-der",
            "audhsos-time",
            "crypto-ec",
            "crypto-hash",
            "test-support",
        ],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "audhsos-tls",
        path: "crates/net/tls",
        kind: Kind::Logic,
        deps: &[
            "audhsos-der",
            "audhsos-time",
            "audhsos-x509",
            "crypto-aead",
            "crypto-ct",
            "crypto-ec",
            "crypto-hash",
            "crypto-rng",
            "test-support",
        ],
        coverage_gate: true,
        target: Target::Host,
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
        target: Target::Host,
    },
    Crate {
        name: "audhsos-elf",
        path: "crates/elf",
        kind: Kind::Logic,
        deps: &["test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "audhsos-symbols",
        path: "crates/symbols",
        kind: Kind::Logic,
        deps: &["audhsos-elf", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "audhsos-uefi",
        path: "crates/uefi",
        kind: Kind::Logic,
        deps: &["audhsos-abi"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "boot-uefi-x86_64",
        path: "crates/boot/uefi-x86_64",
        kind: Kind::Adapter {
            unsafe_budget: 39,
            asm_budget: 2,
        },
        deps: &[
            "audhsos-abi",
            "audhsos-elf",
            "audhsos-uefi",
            "kernel-hal-api",
            "kernel-mm",
            "kernel-types",
        ],
        coverage_gate: false,
        target: Target::X86_64Uefi,
    },
    Crate {
        name: "driver-uart16550",
        path: "crates/drivers/uart16550",
        kind: Kind::Logic,
        deps: &[],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "kernel-x86-tables",
        path: "crates/kernel/x86-tables",
        kind: Kind::Logic,
        deps: &[],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "kernel-types",
        path: "crates/kernel/types",
        kind: Kind::Logic,
        deps: &["audhsos-abi", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "kernel-acpi",
        path: "crates/kernel/acpi",
        kind: Kind::Logic,
        deps: &["kernel-types"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "kernel-hal-api",
        path: "crates/kernel/hal-api",
        kind: Kind::Logic,
        deps: &["kernel-types"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "kernel-objects",
        path: "crates/kernel/objects",
        kind: Kind::Logic,
        deps: &["kernel-types", "audhsos-abi", "test-support"],
        coverage_gate: true,
        target: Target::Host,
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
        target: Target::Host,
    },
    Crate {
        name: "kernel-core",
        path: "crates/kernel/core",
        kind: Kind::Logic,
        deps: &[
            "kernel-types",
            "kernel-hal-api",
            "kernel-mm",
            "kernel-objects",
            "audhsos-abi",
            "audhsos-sync",
        ],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "kernel-hal-x86_64",
        path: "crates/kernel/hal-x86_64",
        kind: Kind::Adapter {
            unsafe_budget: 122,
            asm_budget: 22,
        },
        deps: &[
            "kernel-acpi",
            "kernel-hal-api",
            "kernel-types",
            "audhsos-abi",
            "audhsos-sync",
            "driver-uart16550",
            "kernel-x86-tables",
            "kernel-mm",
            "kernel-test-harness",
        ],
        coverage_gate: false,
        target: Target::X86_64None,
    },
    Crate {
        name: "audhsos-kernel",
        path: "crates/kernel/bin",
        kind: Kind::Adapter {
            unsafe_budget: 19,
            asm_budget: 0,
        },
        deps: &[
            "kernel-core",
            "kernel-hal-api",
            "kernel-hal-x86_64",
            "kernel-mm",
            "kernel-types",
            "audhsos-abi",
        ],
        coverage_gate: false,
        target: Target::X86_64None,
    },
    Crate {
        name: "kernel-test-harness",
        path: "crates/kernel/test-harness",
        kind: Kind::Logic,
        deps: &["kernel-hal-api"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "fuzz-support",
        path: "crates/support/fuzz",
        kind: Kind::Adapter {
            unsafe_budget: 9,
            asm_budget: 0,
        },
        deps: &[],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "test-support",
        path: "crates/support/testing",
        kind: Kind::Host,
        deps: &[],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "xtask",
        path: "crates/tools/xtask",
        kind: Kind::Host,
        deps: &["audhsos-abi", "audhsos-symbols", "kernel-test-harness"],
        coverage_gate: false,
        target: Target::Host,
    },
];

/// Crates every workspace crate may use as a dev-dependency.
pub(crate) const DEV_DEPENDENCIES: &[&str] = &["test-support"];

/// Crates whose tests run under Miri.
pub(crate) const MIRI_CRATES: &[&str] = &["audhsos-sync", "fuzz-support"];

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

/// Every fuzz target. The name is the binary under `fuzz/` and the
/// directory of its corpus under `fuzz/corpus/`.
pub(crate) const FUZZ_TARGETS: &[FuzzTarget] = &[
    FuzzTarget {
        name: "boot_image_header",
    },
    FuzzTarget { name: "boot_info" },
    FuzzTarget { name: "der" },
    FuzzTarget { name: "elf" },
    FuzzTarget { name: "madt" },
    FuzzTarget { name: "pem" },
    FuzzTarget {
        name: "tls_handshake",
    },
    FuzzTarget { name: "tls_record" },
    FuzzTarget { name: "x509" },
];

/// Extensions of assembly files, which must not exist.
pub(crate) const ASSEMBLY_EXTENSIONS: &[&str] = &["S", "s", "asm"];

/// Directories the checks never descend into: what a build wrote, what
/// version control keeps, and what is only kept to be read. `research`
/// holds source of other projects, which carries the license headers of
/// those projects and not this one; a check of this project has no
/// business in it.
pub(crate) const EXCLUDED_DIRECTORIES: &[&str] = &["target", ".git", "research"];

/// Every crate that is built for `target`.
pub(crate) fn crates_for(target: Target) -> Vec<&'static str> {
    CRATES
        .iter()
        .filter(|krate| krate.target == target)
        .map(|krate| krate.name)
        .collect()
}

/// The crate with the given name.
pub(crate) fn find(name: &str) -> Option<&'static Crate> {
    CRATES.iter().find(|c| c.name == name)
}
