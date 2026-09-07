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
        name: "audhsos-deflate",
        path: "crates/deflate",
        kind: Kind::Logic,
        deps: &[],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "audhsos-collections",
        path: "crates/collections",
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
        name: "crypto-bignum",
        path: "crates/crypto/bignum",
        kind: Kind::Logic,
        deps: &["test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "crypto-ec",
        path: "crates/crypto/ec",
        kind: Kind::Logic,
        deps: &["crypto-bignum", "crypto-ct", "crypto-hash", "test-support"],
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
        name: "crypto-rsa",
        path: "crates/crypto/rsa",
        kind: Kind::Logic,
        deps: &["crypto-bignum", "crypto-ct", "crypto-hash", "test-support"],
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
            "crypto-rsa",
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
        name: "net-wire",
        path: "crates/net/wire",
        kind: Kind::Logic,
        deps: &["test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "net-eth",
        path: "crates/net/eth",
        kind: Kind::Logic,
        deps: &[
            "net-wire",
            "audhsos-time",
            "audhsos-collections",
            "test-support",
        ],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "net-ip",
        path: "crates/net/ip",
        kind: Kind::Logic,
        deps: &[
            "net-eth",
            "net-wire",
            "audhsos-time",
            "audhsos-collections",
            "test-support",
        ],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "net-ipv6",
        path: "crates/net/ipv6",
        kind: Kind::Logic,
        deps: &[
            "net-ip",
            "net-eth",
            "net-wire",
            "audhsos-time",
            "audhsos-collections",
            "test-support",
        ],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "net-udp",
        path: "crates/net/udp",
        kind: Kind::Logic,
        deps: &["net-wire", "crypto-rng", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "net-tcp",
        path: "crates/net/tcp",
        kind: Kind::Logic,
        deps: &[
            "net-wire",
            "audhsos-time",
            "audhsos-collections",
            "crypto-rng",
            "test-support",
        ],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "net-dns",
        path: "crates/net/dns",
        kind: Kind::Logic,
        deps: &[
            "net-udp",
            "net-wire",
            "audhsos-time",
            "audhsos-collections",
            "crypto-rng",
            "test-support",
        ],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "net-dhcp",
        path: "crates/net/dhcp",
        kind: Kind::Logic,
        deps: &[
            "net-udp",
            "net-wire",
            "audhsos-time",
            "audhsos-collections",
            "crypto-rng",
            "test-support",
        ],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "net-http",
        path: "crates/net/http",
        kind: Kind::Logic,
        deps: &["net-wire", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "net-stack",
        path: "crates/net/stack",
        kind: Kind::Logic,
        deps: &[
            "net-dhcp",
            "net-dns",
            "net-tcp",
            "net-udp",
            "net-ipv6",
            "net-ip",
            "net-eth",
            "net-wire",
            "audhsos-time",
            "audhsos-collections",
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
            unsafe_budget: 4,
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
        name: "gfx",
        path: "crates/gfx",
        kind: Kind::Logic,
        deps: &["audhsos-abi", "test-support"],
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
        name: "virtio-queue",
        path: "crates/virtio/queue",
        kind: Kind::Logic,
        deps: &["audhsos-collections", "test-support"],
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
        deps: &["audhsos-abi", "kernel-types"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "kernel-objects",
        path: "crates/kernel/objects",
        kind: Kind::Logic,
        deps: &["kernel-types", "kernel-mm", "audhsos-abi", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "kernel-sched",
        path: "crates/kernel/sched",
        kind: Kind::Logic,
        deps: &["kernel-objects", "kernel-types", "audhsos-abi"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "kernel-ipc",
        path: "crates/kernel/ipc",
        kind: Kind::Logic,
        deps: &[
            "kernel-objects",
            "kernel-sched",
            "kernel-types",
            "audhsos-abi",
            "test-support",
        ],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "kernel-syscall",
        path: "crates/kernel/syscall",
        kind: Kind::Logic,
        deps: &[
            "kernel-ipc",
            "kernel-objects",
            "kernel-sched",
            "kernel-mm",
            "kernel-types",
            "audhsos-abi",
        ],
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
            "audhsos-elf",
            "kernel-types",
            "kernel-hal-api",
            "kernel-mm",
            "kernel-objects",
            "kernel-sched",
            "kernel-syscall",
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
            unsafe_budget: 145,
            asm_budget: 28,
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
            unsafe_budget: 31,
            asm_budget: 0,
        },
        deps: &[
            "kernel-core",
            "kernel-hal-api",
            "kernel-hal-x86_64",
            "kernel-ipc",
            "kernel-mm",
            "kernel-objects",
            "kernel-sched",
            "kernel-syscall",
            "kernel-types",
            "audhsos-abi",
            "audhsos-sync",
        ],
        coverage_gate: false,
        target: Target::X86_64None,
    },
    Crate {
        name: "user-rt",
        path: "crates/user/rt",
        kind: Kind::Logic,
        deps: &["audhsos-abi", "audhsos-collections", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "user-proto",
        path: "crates/user/proto",
        kind: Kind::Logic,
        deps: &["audhsos-abi", "user-rt"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "user-loader",
        path: "crates/user/loader",
        kind: Kind::Logic,
        deps: &["audhsos-abi", "audhsos-elf", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "server-console",
        path: "crates/user/servers/console",
        kind: Kind::Logic,
        deps: &["audhsos-collections", "driver-uart16550"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "server-memory",
        path: "crates/user/servers/memory",
        kind: Kind::Logic,
        deps: &["audhsos-abi", "audhsos-collections", "test-support"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "server-name",
        path: "crates/user/servers/name",
        kind: Kind::Logic,
        deps: &["audhsos-abi", "audhsos-collections", "user-proto"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "user-sys-x86_64",
        path: "crates/user/sys-x86_64",
        kind: Kind::Adapter {
            unsafe_budget: 21,
            asm_budget: 1,
        },
        deps: &["audhsos-abi", "user-rt"],
        coverage_gate: false,
        target: Target::X86_64None,
    },
    Crate {
        name: "user-test-programs",
        path: "crates/user/test-programs",
        kind: Kind::Adapter {
            unsafe_budget: 78,
            asm_budget: 1,
        },
        deps: &["audhsos-abi", "user-rt", "user-sys-x86_64"],
        coverage_gate: false,
        target: Target::X86_64None,
    },
    Crate {
        name: "user-programs",
        path: "crates/user/programs",
        kind: Kind::Adapter {
            unsafe_budget: 18,
            asm_budget: 0,
        },
        deps: &[
            "audhsos-abi",
            "driver-uart16550",
            "server-console",
            "server-memory",
            "server-name",
            "user-loader",
            "user-proto",
            "user-rt",
            "user-sys-x86_64",
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
    // The fuzzing engine. Its `unsafe` is the boundary to the coverage
    // instrumentation and nothing else: the twelve callbacks the compiler
    // emits calls to, each of which is an unsafe attribute and some of
    // which are unsafe functions, and the handful of blocks that turn the
    // ranges the linker placed into slices. Everything above that, the
    // mutator, the corpus, and the loop, is safe code, and so is every
    // fuzz target. The budget counts the tests as well, which is most of
    // it: they call the callbacks the way a compiled target would.
    Crate {
        name: "fuzz-support",
        path: "crates/support/fuzz",
        kind: Kind::Adapter {
            unsafe_budget: 41,
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
        name: "doc-markdown",
        path: "crates/tools/markdown",
        kind: Kind::Host,
        deps: &[],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "doc-html",
        path: "crates/tools/html",
        kind: Kind::Host,
        deps: &["doc-markdown"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "doc-pdf",
        path: "crates/tools/pdf",
        kind: Kind::Host,
        deps: &["audhsos-deflate"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "doc-svg",
        path: "crates/tools/svg",
        kind: Kind::Host,
        deps: &["doc-html", "doc-pdf"],
        coverage_gate: true,
        target: Target::Host,
    },
    Crate {
        name: "docpdf",
        path: "crates/tools/docpdf",
        kind: Kind::Host,
        deps: &["doc-html", "doc-markdown", "doc-pdf", "doc-svg"],
        coverage_gate: false,
        target: Target::Host,
    },
    Crate {
        name: "xtask",
        path: "crates/tools/xtask",
        kind: Kind::Host,
        deps: &[
            "audhsos-abi",
            "audhsos-symbols",
            "kernel-test-harness",
            "user-loader",
        ],
        coverage_gate: false,
        target: Target::Host,
    },
];

/// Crates every workspace crate may use as a dev-dependency.
pub(crate) const DEV_DEPENDENCIES: &[&str] = &["test-support"];

/// A crate whose host-executable `unsafe` runs under Miri, and the tests
/// that reach that `unsafe`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MiriTarget {
    /// The crate.
    pub(crate) name: &'static str,
    /// What libtest selects by: a test whose name begins with one of these
    /// runs. An empty list runs every test of the crate.
    pub(crate) filters: &'static [&'static str],
}

/// What Miri runs. Miri interprets MIR instead of executing machine code,
/// so it costs one to two orders of magnitude more than the same test on
/// the host, and it is worth that only where it checks something no other
/// step can: the aliasing and the provenance of the `unsafe` an adapter
/// crate can execute on the host. Logic that happens to live in the same
/// crate is covered by `test --host` and by `coverage`, and running it
/// under Miri buys nothing.
///
/// A crate with no filters runs whole, which is right where every test
/// reaches the `unsafe`: `audhsos-sync` is two `unsafe` sites and the cell
/// around them. `fuzz-support` is the other case — its `unsafe` is the
/// counter registry of `counters.rs` and the sanitizer callbacks of
/// `sancov.rs`, and the mutators, the corpus, the pool, the dictionary,
/// the options, and the generator around them are safe Rust. The filters
/// name the module that tests each of those two files, and nothing else.
///
/// The line is where the `unsafe` is written, not where it is reached.
/// `tests::mutate` reaches the same global through `sancov::with_trace`,
/// which `tests::sancov` tests directly, and it does so over forty
/// thousand mutation rounds: interpreting those checks one dereference no
/// better than the one round that `tests::sancov` already interprets, and
/// it cost more than the whole rest of the step. The tests of
/// `tests::engine` that touch the registry are `#[cfg_attr(miri, ignore)]`
/// already, because Miri runs without a file system.
///
/// [`crate::unsafe_budget::miri_gaps`] holds the list to that promise: a
/// product file with `unsafe` whose module no filter names is a violation
/// of the `miri` step, so `unsafe` cannot appear in a new module and
/// quietly fall out of Miri's reach.
pub(crate) const MIRI_TARGETS: &[MiriTarget] = &[
    MiriTarget {
        name: "audhsos-sync",
        filters: &[],
    },
    MiriTarget {
        name: "fuzz-support",
        filters: &["tests::counters::", "tests::sancov::"],
    },
];

/// The two header lines every source file starts with (comment syntax
/// added per file type).
pub(crate) const SPDX_HEADER: [&str; 2] = [
    "SPDX-License-Identifier: AGPL-3.0-only",
    "Copyright (C) 2026 Manuel Baesler and contributors",
];

/// The header of a file that is in part a port of foreign source.
///
/// The project is `AGPL-3.0-only`, and one-way compatible with the licence
/// of what is ported here: LLVM's libFuzzer, which is Apache-2.0 with the
/// LLVM exception. A port is a derived work, so the files that carry one
/// name both licences and both sets of authors, and `NOTICE` at the root
/// carries the full text of the notice they refer to.
pub(crate) const PORTED_HEADER: [&str; 4] = [
    "SPDX-License-Identifier: AGPL-3.0-only AND Apache-2.0 WITH LLVM-exception",
    "Copyright (C) 2026 Manuel Baesler and contributors",
    "Copyright (C) the LLVM Project contributors, under Apache-2.0 WITH LLVM-exception",
    "Ported from LLVM's libFuzzer; see NOTICE at the root of this repository.",
];

/// The files that carry [`PORTED_HEADER`] instead of [`SPDX_HEADER`],
/// relative to the root and with `/` as the separator. Every one of them
/// is part of the fuzzing engine, which follows libFuzzer closely enough
/// that calling it anything but a port would be wrong.
pub(crate) const PORTED_FILES: &[&str] = &[
    "crates/support/fuzz/src/counters.rs",
    "crates/support/fuzz/src/dictionary.rs",
    "crates/support/fuzz/src/engine.rs",
    "crates/support/fuzz/src/feature.rs",
    "crates/support/fuzz/src/mutate.rs",
    "crates/support/fuzz/src/options.rs",
    "crates/support/fuzz/src/pool.rs",
    "crates/support/fuzz/src/sancov.rs",
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
    lines: 91.0,
    branches: 86.0,
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
    FuzzTarget {
        name: "http_response",
    },
    FuzzTarget { name: "ipv4" },
    FuzzTarget { name: "ipv6" },
    FuzzTarget { name: "madt" },
    FuzzTarget { name: "pem" },
    FuzzTarget { name: "rsa" },
    FuzzTarget { name: "tar" },
    FuzzTarget {
        name: "dns_message",
    },
    FuzzTarget {
        name: "tcp_segment",
    },
    FuzzTarget {
        name: "tls_handshake",
    },
    FuzzTarget { name: "tls_record" },
    FuzzTarget { name: "x509" },
];

/// Extensions of assembly files, which must not exist.
pub(crate) const ASSEMBLY_EXTENSIONS: &[&str] = &["S", "s", "asm"];

/// Directories the checks never descend into: what a build wrote, what
/// version control keeps, what is only kept to be read, and what an agent
/// writes. `research` holds source of other projects, which carries the
/// license headers of those projects and not this one; a check of this
/// project has no business in it. `.claude` holds the configuration of
/// the coding agent and, under `.claude/worktrees/`, whole further
/// checkouts of this repository: a file there is walked a second time and
/// judged by the path it has in the worktree, which is not the path the
/// tables of this policy name.
pub(crate) const EXCLUDED_DIRECTORIES: &[&str] = &["target", ".git", "research", ".claude"];

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
