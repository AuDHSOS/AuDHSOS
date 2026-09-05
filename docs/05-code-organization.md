# 5. Code Organization

## 5.1 Repository layout

```
AuDHSOS/
├── Cargo.toml                 workspace: members, shared metadata, lints, profiles
├── Cargo.lock                 workspace members only
├── rust-toolchain.toml        pinned nightly, components, targets
├── .cargo/config.toml         `cargo xtask` alias; relocation model of the kernel target
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
│   ├── gfx/                   gfx: framebuffer logic, bitmap font, damage tracking (Phase 9)
│   ├── sync/                  audhsos-sync: Global<T> cell (unsafe allowed)
│   ├── time/                  audhsos-time: UnixTime, CivilTime, Instant, Duration (document 12)
│   ├── encoding/              audhsos-encoding: Base64, hex, PEM (document 12)
│   ├── collections/           audhsos-collections: fixed-capacity containers over indices (document 12)
│   ├── symbols/               audhsos-symbols: ELF symbol table and DWARF line lookup (document 12)
│   ├── drivers/
│   │   ├── uart16550/         driver-uart16550: register logic over a port access trait
│   │   └── i8042/             driver-i8042: PS/2 controller and decoder logic over a port access trait (Phase 10)
│   ├── support/
│   │   ├── testing/           test-support: property-test engine, builders, strategies, model-test runner
│   │   └── fuzz/              fuzz-support: fuzzer entry glue (unsafe allowed, host only)
│   ├── virtio/
│   │   └── queue/             virtio-queue: split virtqueue and initialization logic (document 12)
│   ├── fs/
│   │   └── fat/               fs-fat: FAT32 over a block device trait (document 12)
│   ├── boot/
│   │   └── uefi-x86_64/       boot-uefi-x86_64: the loader (unsafe allowed)
│   ├── kernel/
│   │   ├── types/             kernel-types: PhysAddr, VirtAddr, PhysFrame, Page, ranges, alignment
│   │   ├── hal-api/           kernel-hal-api: HAL traits and their test doubles
│   │   ├── mm/                kernel-mm: memory map, frame allocator, page tables, mapper, address spaces, kernel stacks
│   │   ├── objects/           kernel-objects: pools, ids, handles, rights, object types, quotas
│   │   ├── sched/             kernel-sched: thread states, run queues, time slices
│   │   ├── ipc/               kernel-ipc: endpoints, notifications, rendezvous, message transfer
│   │   ├── syscall/           kernel-syscall: argument decoding, validation, dispatch
│   │   ├── core/              kernel-core: KernelState, boot sequence, memory bring-up, reactions to traps and ticks
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
│   │   │   ├── memory/        server-memory
│   │   │   ├── display/       server-display: framebuffer owner, surfaces, cursor (Phase 9)
│   │   │   └── input/         server-input: i8042 driver process, event rings (Phase 10)
│   │   └── apps/
│   │       ├── hello/         app-hello: end-to-end client
│   │       └── canvas/        app-canvas: graphical demonstration and e2e client (Phase 11)
│   ├── crypto/                (document 11)
│   │   ├── ct/                crypto-ct: Choice, constant-time selection and comparison, Secret<N>
│   │   ├── hash/              crypto-hash: SHA-256, SHA-384/512, HMAC, HKDF
│   │   ├── aead/              crypto-aead: ChaCha20-Poly1305, bitsliced AES-GCM, GHASH
│   │   ├── ec/                crypto-ec: fe25519, X25519, Ed25519 verify, P-256 ECDSA verify
│   │   └── rng/               crypto-rng: Entropy and Rng traits, ChaCha20 generator
│   ├── net/                   (documents 11 and 12)
│   │   ├── der/               audhsos-der: strict zero-copy DER reader
│   │   ├── x509/              audhsos-x509: certificates, path validation, name matching
│   │   ├── tls/               audhsos-tls: TLS 1.3 client, sans-I/O
│   │   ├── wire/              net-wire: addresses, cursor, internet checksum
│   │   ├── eth/               net-eth: Ethernet II frames, ARP cache
│   │   ├── ip/                net-ip: IPv4, reassembly, ICMP, routes
│   │   ├── udp/               net-udp: sockets and datagrams
│   │   ├── tcp/               net-tcp: the RFC 9293 state machine, timers, congestion control
│   │   ├── dns/               net-dns: message format and resolver state machine
│   │   ├── dhcp/              net-dhcp: client state machine and lease timers
│   │   ├── http/              net-http: HTTP/1.1 client encoding and parsing
│   │   └── stack/             net-stack: interface, demultiplexing, poll
│   └── tools/
│       └── xtask/             build, image (GPT + FAT32 writer, CRC32), run, test, lint, check-layering, check-deps, unsafe-budget, fuzz, coverage; policy tables
├── fuzz/                      fuzz target crates and corpora
└── .github/workflows/         CI definitions
```

## 5.2 Crate catalog

| Crate | Layer | Target | `unsafe` | Host tests | May depend on |
|-------|-------|--------|----------|------------|---------------|
| `audhsos-abi` | 0 | all | no | yes | `test-support` behind the feature `test-strategies` |
| `audhsos-elf` | 0 | all | no | yes, fuzz | `test-support` behind the feature `test-strategies` |
| `audhsos-uefi` | 0 | all | no | yes (layouts) | `audhsos-abi` |
| `audhsos-sync` | 0 | all | allowlisted | Miri | - |
| `audhsos-time` | 0 | all | no | yes | `test-support` behind the feature `test-strategies` |
| `audhsos-encoding` | 0 | all | no | yes, fuzz | `test-support` behind the feature `test-strategies` |
| `audhsos-collections` | 0 | all | no | yes | `test-support` behind the feature `test-strategies` |
| `kernel-types` | 1 | all | no | yes | `audhsos-abi`; `test-support` behind the feature `test-strategies` |
| `kernel-x86-tables` | 1 | all | no | yes | - |
| `kernel-hal-api` | 1 | all | no | doubles are tested | `kernel-types`; features `test-doubles`, `port-io` |
| `driver-uart16550` | 1 | all | no | yes | - (feature `test-doubles`) |
| `driver-i8042` | 1 | all | no | yes, fuzz | - (feature `test-doubles`) |
| `gfx` | 1 | all | no | yes | `audhsos-abi`; `test-support` behind the feature `test-strategies` |
| `audhsos-symbols` | 1 | all | no | yes | `audhsos-elf` |
| `virtio-queue` | 1 | all | no | yes | `audhsos-collections`; feature `test-doubles` |
| `fs-fat` | 1 | all | no | yes | `audhsos-time`, `audhsos-collections`; feature `test-doubles` |
| `kernel-mm` | 2 | all | no | yes | `kernel-types`, `kernel-hal-api`, `audhsos-abi`; `test-support` behind the feature `test-strategies` |
| `kernel-objects` | 2 | all | no | yes | `kernel-types`, `audhsos-abi`; `test-support` behind the feature `test-strategies` |
| `kernel-sched` | 2 | all | no | yes | `kernel-objects` |
| `kernel-ipc` | 3 | all | no | yes | `kernel-objects`, `kernel-sched`, `audhsos-abi` |
| `kernel-syscall` | 3 | all | no | yes | layers 0-2, `kernel-ipc` |
| `kernel-core` | 4 | all | no | yes, with doubles | layers 0-3, `audhsos-sync` |
| `kernel-hal-x86_64` | 5 | `x86_64-unknown-none` | allowlisted | the pure parts live in `kernel-x86-tables` | `kernel-hal-api`, `kernel-types`, `audhsos-abi`, `driver-uart16550`, `audhsos-sync`, `kernel-x86-tables`, `kernel-mm`, `kernel-test-harness` |
| `kernel-test-harness` | 5 | all | no | yes | `kernel-hal-api` |
| `audhsos-kernel` | 6 | `x86_64-unknown-none` | allowlisted (the entry point, the memory bring-up, and the test images) | QEMU | `kernel-core`, `kernel-hal-x86_64`, `audhsos-abi`; `kernel-hal-api`, `kernel-mm`, `kernel-types` for the test images |
| `boot-uefi-x86_64` | b | `x86_64-unknown-uefi` | allowlisted | pure sub-modules | `audhsos-abi`, `audhsos-elf`, `audhsos-uefi`, `kernel-types`, `kernel-mm`, `kernel-hal-api` |
| `user-sys-x86_64` | u0 | `x86_64-unknown-none` | allowlisted | Miri | `audhsos-abi`, `audhsos-sync` |
| `user-rt` | u1 | `x86_64-unknown-none` | no | yes | `audhsos-abi`, `user-sys-x86_64` |
| `user-proto` | u1 | `x86_64-unknown-none` | no | yes | `audhsos-abi` |
| `user-loader` | u2 | `x86_64-unknown-none` | no | yes, fuzz | `user-rt`, `user-proto`, `audhsos-elf` |
| servers and apps | u3 | `x86_64-unknown-none` | no | logic on host, e2e in QEMU | `user-rt`, `user-proto`, `user-loader`, `driver-uart16550`, `driver-i8042`, `gfx` |
| `crypto-ct` | c0 | all | no | yes | - |
| `audhsos-der` | c0 | all | no | yes, fuzz | `audhsos-time` when it exists (11.14); `test-support` as a dev-dependency |
| `crypto-hash` | c1 | all | no | yes | `crypto-ct` |
| `crypto-aead` | c1 | all | no | yes | `crypto-ct` |
| `crypto-ec` | c2 | all | no | yes | `crypto-ct`, `crypto-hash`; feature `test-signing` |
| `crypto-rng` | c2 | all | no | yes | `crypto-ct`, `crypto-aead`; feature `test-doubles` |
| `audhsos-x509` | c3 | all | no | yes, fuzz | `audhsos-der`, `crypto-hash`, `crypto-ec`; feature `test-certificates` |
| `audhsos-tls` | c4 | all | no | yes, fuzz | `crypto-ct`, `crypto-hash`, `crypto-aead`, `crypto-ec`, `crypto-rng`, `audhsos-der`, `audhsos-x509` |
| `net-wire` | n0 | all | no | yes | - |
| `net-eth` | n1 | all | no | yes | `net-wire`, `audhsos-time`, `audhsos-collections` |
| `net-ip` | n2 | all | no | yes, fuzz | `net-eth` and below |
| `net-udp` | n3 | all | no | yes | `net-ip` and below, `crypto-rng` |
| `net-tcp` | n3 | all | no | yes, fuzz | `net-ip` and below, `crypto-rng` |
| `net-dns` | n4 | all | no | yes, fuzz | `net-udp` and below, `crypto-rng` |
| `net-dhcp` | n4 | all | no | yes | `net-udp` and below, `crypto-rng` |
| `net-http` | n4 | all | no | yes, fuzz | `net-wire` |
| `net-stack` | n5 | all | no | yes | every `net-` crate |
| `test-support` | dev | host | no | yes | - (depends on no workspace crate, so that every crate can use it as a dev-dependency without a cycle) |
| `fuzz-support` | dev | host | allowlisted | Miri | - |
| `xtask` | host | host | no | yes | `audhsos-abi`, `kernel-test-harness` (the boot image header, the layout constants, and the serial protocol grammar exist once), `fs-fat`, `audhsos-encoding`, `audhsos-symbols` |

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
   `audhsos-time`, `audhsos-encoding`, `audhsos-collections`,
   `kernel-types`, `kernel-hal-api`, `kernel-mm`, and `driver-uart16550`.
5. Userland crates never depend on kernel crates other than those in rule 4.
6. `cfg(target_arch = ...)` and `cfg(target_os = "uefi")` appear only in
   adapter crates and in the binaries' `Cargo.toml` target tables.
7. The cryptography and TLS crates (layers c0 to c4, document 11) depend
   on each other only, never on kernel, loader, or userland crates.
   Userland crates depend on them, not the reverse.
8. The allowed edges are a table in `xtask/src/policy.rs`. `cargo xtask
   check-layering` reads `cargo tree --edges normal,build,dev --prefix
   depth` and fails on any edge not in the table.
9. No dependency section of any manifest references a crate outside the
   workspace. `cargo xtask check-deps` verifies `Cargo.lock` and every
   manifest.
10. The network crates (layers n0 to n5, [document 12](12-parallel-work.md))
    depend on each other, on the layer-0 foundations, and on `crypto-rng`
    for unpredictable numbers, never on kernel, loader, or userland
    crates. Userland depends on them, not the reverse. `audhsos-tls` and
    the network crates never reference each other; the transport that
    joins them lives in a userland process.
11. `audhsos-symbols`, `virtio-queue`, and `fs-fat` are logic crates at
    layer 1. They depend on layer-0 crates only and are used by the
    xtask and, when the phases reach them, by driver and server
    processes.

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
- Cargo features are limited to five in the kernel, the loader, and
  userland: `debug-uart` and `test-exit` on the kernel binary and adapter,
  `test-doubles` and `port-io` on `kernel-hal-api`, `test-strategies` on
  crates that own types used in property tests. The cryptography track
  adds `test-signing` on `crypto-ec` and `test-certificates` on
  `audhsos-x509`, and reuses `test-doubles` on `crypto-rng`. Both exist to generate test data, both are off in every
  product build, and `cargo xtask check-layering` fails if a crate other
  than a test target or the xtask enables them. Host tests use `#![cfg_attr(not(test), no_std)]` and need
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
  `apic`, `irq`, `elf`, `id`, `uefi`; in the cryptography track,
  `ct`, `aead`, `ec`, `rng`, `der`, `tls`, `hmac`, `hkdf`, `oid`, `spki`;
  and in the parallel tracks of document 12, `arp`, `dhcp`, `dns`, `fat`,
  `http`, `ip`, `mac`, `mss`, `mtu`, `rto`, `tcp`, `udp`.
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
| i8042 register handling and PS/2 decoding | `driver-i8042` over its own port access trait, following the UART pattern; one adapter over `IoPortRange` system calls |
| Pixel operations in the display server and in applications | `gfx`: one surface type, one font, one damage tracker; the display server and applications draw with the same code |
| System call numbers, names, argument counts, kernel dispatch, userland wrappers | one declarative table in `audhsos-abi` (a `syscalls!` macro) consumed by the kernel dispatcher and by `user-rt` |
| Object types, their rights masks, and `TryFrom<u32>` conversions | one declarative table in `audhsos-abi` |
| Error mapping | one `From` implementation per crate pair, tested by a table |
| Test doubles | one implementation in `kernel-hal-api` behind `test-doubles` |
| FAT32 structures in the image writer and in a later file system server | `fs-fat` over a block device trait; the xtask and the server use one implementation |
| Calendar arithmetic in certificate validity, file timestamps, and network timers | `audhsos-time`; every interface takes time as a parameter, no crate reads a clock |
| Fixed-capacity containers in kernel queues, the network stack, and userland | `audhsos-collections`; one model-tested implementation per container |
| Base64 and PEM in the trust-anchor tool and in generated test data | `audhsos-encoding` |
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
| `run [--display]` | boot the system in QEMU with the serial console on the terminal; `--display` opens QEMU's display window instead of `-display none` |
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
