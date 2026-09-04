# 5. Code Organization

## 5.1 Repository layout

```
AuDHSOS/
├── Cargo.toml                 workspace: members, shared metadata, lints, profiles
├── Cargo.lock                 workspace members only
├── rust-toolchain.toml        pinned nightly, components, targets
├── .cargo/config.toml         `cargo xtask` alias; QEMU runner for the kernel target
├── LICENSE                    AGPL-3.0 text, verbatim from gnu.org
├── README.md
├── CONTRIBUTING.md
├── CHANGELOG.md
├── rustfmt.toml
├── docs/                      this document set and the decision register
├── crates/
│   ├── abi/                   audhsos-abi: syscall table, errors, rights, message layout, boot image header, boot information, address constants
│   ├── elf/                   audhsos-elf: ELF64 parser producing validated load segments
│   ├── uefi/                  audhsos-uefi: UEFI structure layouts, GUIDs, constants (no calls)
│   ├── sync/                  audhsos-sync: Global<T> cell (unsafe allowed)
│   ├── drivers/
│   │   └── uart16550/         driver-uart16550: register logic over a port access trait
│   ├── support/
│   │   ├── testing/           test-support: property-test engine, builders, strategies, model-test runner
│   │   └── fuzz/              fuzz-support: fuzzer entry glue (unsafe allowed, host only)
│   ├── boot/
│   │   └── uefi-x86_64/       boot-uefi-x86_64: the loader (unsafe allowed)
│   ├── kernel/
│   │   ├── types/             kernel-types: PhysAddr, VirtAddr, PhysFrame, Page, ranges, alignment
│   │   ├── hal-api/           kernel-hal-api: HAL traits and their test doubles
│   │   ├── mm/                kernel-mm: memory map, frame allocator, page tables, mapper, address spaces
│   │   ├── objects/           kernel-objects: pools, ids, handles, rights, object types, quotas
│   │   ├── sched/             kernel-sched: thread states, run queues, time slices
│   │   ├── ipc/               kernel-ipc: endpoints, notifications, rendezvous, message transfer
│   │   ├── syscall/           kernel-syscall: argument decoding, validation, dispatch
│   │   ├── core/              kernel-core: KernelState, boot sequence, reactions to traps and ticks
│   │   ├── hal-x86_64/        kernel-hal-x86_64: the adapter (unsafe allowed)
│   │   ├── test-harness/      kernel-test-harness: in-QEMU test runner, serial protocol
│   │   └── bin/               audhsos-kernel: the binary; tests/*.rs are QEMU test kernels
│   ├── user/
│   │   ├── sys-x86_64/        user-sys-x86_64: _start, trap instruction, GlobalAlloc adapter (unsafe allowed)
│   │   ├── rt/                user-rt: typed handles, syscall wrappers, allocator logic, panic handler, logging
│   │   ├── proto/             user-proto: protocol encodings
│   │   ├── loader/            user-loader: tar reader, process creation from ELF
│   │   ├── servers/
│   │   │   ├── init/          server-init: the root task
│   │   │   ├── name/          server-name
│   │   │   ├── console/       server-console
│   │   │   └── memory/        server-memory
│   │   └── apps/
│   │       └── hello/         app-hello: end-to-end client
│   └── tools/
│       └── xtask/             build, image (GPT + FAT32 writer, CRC32), run, test, lint, check-layering, check-deps, unsafe-budget, fuzz, coverage; policy tables
├── fuzz/                      fuzz target crates and corpora
└── .github/workflows/         CI definitions
```

## 5.2 Crate catalog

| Crate | Layer | Target | `unsafe` | Host tests | May depend on |
|-------|-------|--------|----------|------------|---------------|
| `audhsos-abi` | 0 | all | no | yes | - |
| `audhsos-elf` | 0 | all | no | yes, fuzz | - |
| `audhsos-uefi` | 0 | all | no | yes (layouts) | - |
| `audhsos-sync` | 0 | all | allowlisted | Miri | - |
| `kernel-types` | 1 | all | no | yes | `audhsos-abi`; `test-support` behind the feature `test-strategies` |
| `kernel-hal-api` | 1 | all | no | doubles are tested | `kernel-types`; features `test-doubles`, `port-io` |
| `driver-uart16550` | 1 | all | no | yes | - |
| `kernel-mm` | 2 | all | no | yes | `kernel-types`, `kernel-hal-api` |
| `kernel-objects` | 2 | all | no | yes | `kernel-types`, `audhsos-abi` |
| `kernel-sched` | 2 | all | no | yes | `kernel-objects` |
| `kernel-ipc` | 3 | all | no | yes | `kernel-objects`, `kernel-sched`, `audhsos-abi` |
| `kernel-syscall` | 3 | all | no | yes | layers 0-2, `kernel-ipc` |
| `kernel-core` | 4 | all | no | yes, with doubles | layers 0-3, `audhsos-sync` |
| `kernel-hal-x86_64` | 5 | `x86_64-unknown-none` | allowlisted | pure sub-modules | `kernel-hal-api`, `kernel-types`, `audhsos-abi`, `driver-uart16550`, `audhsos-sync` |
| `kernel-test-harness` | 5 | `x86_64-unknown-none` | no | - | `kernel-hal-api` |
| `audhsos-kernel` | 6 | `x86_64-unknown-none` | no | QEMU | `kernel-core`, `kernel-hal-x86_64`, `kernel-test-harness` |
| `boot-uefi-x86_64` | b | `x86_64-unknown-uefi` | allowlisted | pure sub-modules | `audhsos-abi`, `audhsos-elf`, `audhsos-uefi`, `kernel-types`, `kernel-mm`, `kernel-hal-api` |
| `user-sys-x86_64` | u0 | `x86_64-unknown-none` | allowlisted | Miri | `audhsos-abi`, `audhsos-sync` |
| `user-rt` | u1 | `x86_64-unknown-none` | no | yes | `audhsos-abi`, `user-sys-x86_64` |
| `user-proto` | u1 | `x86_64-unknown-none` | no | yes | `audhsos-abi` |
| `user-loader` | u2 | `x86_64-unknown-none` | no | yes, fuzz | `user-rt`, `user-proto`, `audhsos-elf` |
| servers and apps | u3 | `x86_64-unknown-none` | no | logic on host, e2e in QEMU | `user-rt`, `user-proto`, `user-loader`, `driver-uart16550` |
| `test-support` | dev | host | no | yes | - (depends on no workspace crate, so that every crate can use it as a dev-dependency without a cycle) |
| `fuzz-support` | dev | host | allowlisted | Miri | - |
| `xtask` | host | host | no | yes | - |

## 5.3 Layering rules

1. Dependencies point downward only. A crate may depend on crates of lower
   layers as listed in the catalog, never sideways or upward.
2. Logic crates never depend on adapter crates, with one exception:
   `kernel-core`, `kernel-hal-x86_64`, and `user-sys-x86_64` depend on
   `audhsos-sync`.
3. `kernel-hal-api` depends on `kernel-types` and nothing else. Its test
   doubles live behind the feature `test-doubles`. Generators for property
   tests live in the crate that owns the types, behind the feature
   `test-strategies`; `test-support` itself depends on no workspace crate.
4. The crates shared between loader, kernel, and userland are exactly
   `audhsos-abi`, `audhsos-elf`, `audhsos-uefi`, `audhsos-sync`,
   `kernel-types`, `kernel-hal-api`, `kernel-mm`, and `driver-uart16550`.
5. Userland crates never depend on kernel crates other than those in rule 4.
6. `cfg(target_arch = ...)` and `cfg(target_os = "uefi")` appear only in
   adapter crates and in the binaries' `Cargo.toml` target tables.
7. The allowed edges are a table in `xtask/src/policy.rs`. `cargo xtask
   check-layering` reads `cargo tree --edges normal,build,dev --prefix
   depth` and fails on any edge not in the table.
8. No dependency section of any manifest references a crate outside the
   workspace. `cargo xtask check-deps` verifies `Cargo.lock` and every
   manifest.

## 5.4 Workspace configuration

- `[workspace.package]` holds version, edition (`2024`), license
  (`AGPL-3.0-only`), repository, and `rust-version`. Every crate inherits
  them with `.workspace = true`.
- `[workspace.dependencies]` lists only workspace members by path.
- `[workspace.lints]` holds the complete lint configuration. Every crate
  declares `[lints] workspace = true`. Crate roots contain only the
  attributes that differ per crate: `#![no_std]`, `#![forbid(unsafe_code)]`
  or the adapter header, and the crate documentation.
- Bare-metal targets abort on panic by definition; host test crates keep
  unwinding for `should_panic` tests.
- Cargo features are limited to five: `debug-uart` and `test-exit` on the
  kernel binary and adapter, `test-doubles` and `port-io` on
  `kernel-hal-api`, `test-strategies` on crates that own types used in
  property tests. Host tests use `#![cfg_attr(not(test), no_std)]` and need
  no feature. No feature changes behavior in release builds.

## 5.5 Conventions

### 5.5.1 File header

Every Rust, TOML, shell, and YAML file starts with:

```
// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
```

(with the comment syntax of the file type). `cargo xtask lint` checks the
header.

### 5.5.2 Naming

- Crates: kebab-case with the prefixes shown in the catalog.
- Modules: snake_case, one concept per module. A module that needs a
  paragraph to describe its purpose is two modules.
- Types name the invariant they carry: `PhysFrame` rather than `u64`,
  `Rights` rather than `u32`. Constructors validate and return `Result`.
- Functions are verbs. Predicates start with `is_`, `has_`, or `can_`.
- Abbreviations are limited to `abi`, `hal`, `ipc`, `mm`, `tcb`, `tlb`,
  `apic`, `irq`, `elf`, `id`, `uefi`.
- Tests: `<subject>_<condition>_<expected>`, for example
  `frame_allocator_exhausted_returns_out_of_frames`.

### 5.5.3 Test files

Unit tests live in `src/tests/<module>.rs`, declared by `#[cfg(test)]
mod tests;` in the crate root and by `src/tests/mod.rs`. Product source
files contain no test code, so that coverage measures product code only.
Tests reach private items through `pub(crate)` visibility where needed.

### 5.5.4 Errors

- Each logic crate defines one exhaustive `enum Error` with `Display`. No
  string errors, no boxed errors, no error codes as integers outside `abi`.
- `kernel-syscall` maps every crate error to `abi::Error` through one `From`
  implementation per crate. The mapping table is exhaustively tested.
- Functions that can fail return `Result`. A function that cannot fail does
  not return `Result`.

### 5.5.5 Lint set

Configured once in the workspace. Level `deny` unless stated.

- Rust: `unsafe_code` (deny; adapter crates allow it locally),
  `unsafe_op_in_unsafe_fn`, `missing_docs`, `unreachable_pub`,
  `unused_crate_dependencies`, `rust_2024_compatibility`, `warnings`.
- Clippy: `all`, `pedantic`, `nursery` (warn, individual lints raised to
  deny as they prove useful), `cargo`, plus the safety set from rule R7:
  `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`,
  `unreachable`, `indexing_slicing`, `arithmetic_side_effects`,
  `as_conversions`, `undocumented_unsafe_blocks`,
  `multiple_unsafe_ops_per_block`.
- Exceptions use `#[expect(lint, reason = "...")]`.

### 5.5.6 Documentation

- Every public item has a doc comment. Module docs start with the
  invariants the module maintains.
- `cargo doc --workspace --document-private-items` runs with
  `RUSTDOCFLAGS="-D warnings"` in CI.
- Doc examples on host-testable crates run as doctests.
- Each crate has a `README.md` that `lib.rs` includes as crate
  documentation.

### 5.5.7 Logging

Project-defined logging macros: `klog!` in `kernel-core` writes through the
`DebugConsole` trait when the feature is on and compiles to nothing
otherwise; `log!` in `user-rt` sends to the log endpoint from the startup
message.

## 5.6 Avoiding duplication

| Situation | Approach |
|-----------|----------|
| The same algorithm runs in the loader, the kernel, and under test | one generic implementation over HAL traits; adapters and doubles differ, the algorithm does not (mapper, memory map normalization) |
| ELF parsing in the loader and in userland | `audhsos-elf`, one parser |
| UART register handling in the kernel debug console and in the userland console driver | `driver-uart16550` over a port access trait; two adapters (direct port I/O, `IoPortRange` system calls) |
| System call numbers, names, argument counts, kernel dispatch, userland wrappers | one declarative table in `audhsos-abi` (a `syscalls!` macro) consumed by the kernel dispatcher and by `user-rt` |
| Object types, their rights masks, and `TryFrom<u32>` conversions | one declarative table in `audhsos-abi` |
| Error mapping | one `From` implementation per crate pair, tested by a table |
| Test doubles | one implementation in `kernel-hal-api` behind `test-doubles` |
| Test fixtures, generators, and the model-test runner | `test-support` crate |
| Lint, metadata, dependency lists | workspace inheritance |
| Protocol encodings | `user-proto` defines each message once as a type with `encode`/`decode` |
| Repeated `match` on object type | `KernelObject::as_<type>()` accessors generated from the object table |
| Policy tables (layering, unsafe budgets, SPDX header, fuzz targets) | `xtask/src/policy.rs`, type-checked constants |

## 5.7 Build automation

All developer and CI commands go through `cargo xtask`. The xtask uses only
the standard library and the toolchain binaries (`cargo`, `rustc`,
`rustfmt`, `cargo-clippy`, `cargo-miri`, `llvm-profdata`, `llvm-cov`,
`llvm-objcopy`) plus QEMU.

| Subcommand | Purpose |
|------------|---------|
| `build [--release]` | build the loader, the kernel, the userland binaries, and the boot image |
| `image` | assemble the boot image (root task flat binary plus tar archive) and the disk image (GPT, FAT32 file system, loader, kernel, boot image) |
| `run` | boot the system in QEMU with the serial console on the terminal |
| `qemu-runner <elf>` | the Cargo runner for the kernel target: wraps a test kernel into a disk image, runs QEMU with a timeout, parses the serial protocol, maps the exit status |
| `test [--host] [--qemu] [--e2e]` | run the selected test levels; default runs all |
| `lint` | `rustfmt --check`, `clippy` with the workspace lint set, SPDX header check |
| `check-layering` | verify the layering table against `cargo tree`, verify `forbid(unsafe_code)` in every logic crate, reject assembly files, verify the adapter-function-to-QEMU-test tables |
| `check-deps` | verify that `Cargo.lock` and all manifests reference workspace members only |
| `unsafe-budget` | count `unsafe` blocks and `asm!` sites per adapter crate against the policy table |
| `fuzz [--target <name>] [--time <s>]` | build fuzz targets with `-Zsanitizer=fuzzer` and run them |
| `coverage` | build host tests with `-C instrument-coverage`, merge profiles with `llvm-profdata`, export LCOV with `llvm-cov`, enforce thresholds |
| `miri` | run the tests of the host-executable adapter crates under Miri |
| `doc` | build documentation with warnings as errors |
| `check` | everything CI runs, in CI order |

The xtask verifies at start that `RUSTUP_TOOLCHAIN`, which rustup's proxies
set for child processes, names the pinned channel, and stops with
instructions otherwise. Every Cargo it starts receives `RUSTC` and
`RUSTDOC` pointing into the same toolchain, so a foreign `rustc` earlier
on the `PATH` is never used.

## 5.8 Version control

- Conventional Commits: `feat(mm): ...`, `fix(ipc): ...`, `test(sched):
  ...`, `docs: ...`, `chore: ...`, `refactor(objects): ...`. The scope is the
  crate's short name. The body states what changes. A footer
  `Decision: D-07` links a decision register entry when one applies.
- Trunk-based development on `main` with short-lived branches. Every merge
  passes `cargo xtask check`.
- `CHANGELOG.md` follows Keep a Changelog and is updated in the same commit
  as the change.

## 5.9 Definition of done for a change

1. Tests exist for the new behavior and for every edge case listed in the
   catalog for the affected component.
2. Documentation of every touched public item is current.
3. `cargo xtask check` passes locally.
4. No `unsafe` budget increase without a decision register entry.
5. The change contains no duplicated logic that the review could point at.
6. The changelog entry exists.
