# 8. Roadmap

The roadmap orders the work so that every phase ends with a system that
builds, passes its tests, and is documented. Sizes are relative (S, M, L,
XL) and describe effort, not calendar time.

## 8.1 Phase overview

| Phase | Name | Size | Ends with |
|-------|------|------|-----------|
| 0 | Project foundation | M | `cargo xtask check` passes on a configured workspace with the property-test engine and `kernel-types` |
| 1 | Memory management logic | L | memory map, frame allocator, page tables, mapper, address spaces, pools: complete and host-tested |
| 2 | Loader, boot, and test harness | XL | own UEFI loader boots test kernels in QEMU, which report over serial and exit with a status |
| 3 | Kernel memory bring-up | M | kernel reserve, pools, physical window, kernel stacks, address-space activation in QEMU |
| 4 | Interrupts and timer | M | APIC-driven timer ticks and line masking |
| 5 | Objects, threads, user mode, system calls | XL | user-mode threads issue system calls through the IPC buffer |
| 6 | IPC and interrupt forwarding | L | endpoints, notifications, fault handlers, device capabilities |
| 7 | Userland foundation | XL | root task, name server, console driver, memory server, hello application |
| 8 | Consolidation and release 0.1.0 | M | budget review, measurements, documentation refresh, tag |

Every phase has the same definition of done: all catalog items for the
components in the phase have tests, `cargo xtask check` is green, the design
documents reflect the code, the changelog is updated.

## 8.2 Phase 0: Project foundation

Deliverables:

- `LICENSE` with the AGPL-3.0 text verbatim; SPDX headers everywhere;
  `README.md`, `CONTRIBUTING.md`, `CHANGELOG.md`.
- `Cargo.toml` workspace with shared metadata and the lint set; the first
  crates: `audhsos-abi` (errors, rights, address constants), `kernel-types`
  (complete), `kernel-hal-api` (trait skeletons and doubles),
  `audhsos-sync`, `test-support` (property-test engine, model-test runner,
  first builders), `xtask`.
- `rust-toolchain.toml`, `.cargo/config.toml`, `rustfmt.toml`.
- xtask subcommands: `lint`, `check-layering`, `check-deps`,
  `unsafe-budget`, `test --host`, `coverage`, `doc`, `check`; the policy
  tables; the toolchain verification at start.
- CI workflow with the host-only jobs.

Tests: catalog 6.6.1, 6.6.6 (rights items), 6.6.18, 6.6.19, and the xtask
items of 6.6.20 that exist at this point.

Acceptance: `cargo xtask check` passes locally and in CI; coverage of
`kernel-types` and `test-support` meets the thresholds; the first commit is
on `main`.

## 8.3 Phase 1: Memory management logic

Deliverables: memory map normalization, reserve selection, bitmap frame
allocator with contiguous allocation, `PageTableEntry`, `Mapper` over
`FrameAccess`, `TlbControl`, and `FrameSource`, address spaces with region
bookkeeping and quotas, generic `Pool<T>` with generation-checked ids and
reference counts, boot image header and boot information validation in
`audhsos-abi`.

Tests: catalog 6.6.2, 6.6.3, 6.6.4 including the model-based test, 6.6.5,
6.6.6 (pool items), 6.6.10.

Acceptance: every listed item has a test; coverage thresholds met; no QEMU
involved.

## 8.4 Phase 2: Loader, boot, and test harness

Deliverables: `audhsos-elf`; `audhsos-uefi`; `boot-uefi-x86_64` with file
loading, kernel placement, page-table construction through the mapper,
boot information, the naked entry function, diagnostics, and failure exit;
the disk image writer (GPT, FAT32, files) in the xtask; `kernel-hal-x86_64`
with the privileged instruction wrappers, GDT, TSS with double-fault stack,
IDT, exception handlers, `driver-uart16550` over direct port I/O, test
exit, boot information validation; `kernel-test-harness`;
`kernel-core::boot` generic over `Platform`, `Traps`, `DebugConsole`,
`TestExit`; xtask `image`, `run`, `qemu-runner`, `test --qemu`; CI QEMU
job.

Tests: catalog 6.6.13 (ELF items), 6.6.14, 6.6.15, 6.6.16 (descriptor
items), 6.6.17, 6.6.21 items boot, debug UART, exceptions (including the
`should_panic` double-fault kernel), and the loader failure images.

Acceptance: at least eight test kernels and three loader test images pass;
every `unsafe` block carries a `SAFETY:` comment; the policy table lists
the real counts.

## 8.5 Phase 3: Kernel memory bring-up

Deliverables: `PhysicalWindow` adapter, kernel reserve and pool
initialization from the boot information, adoption of the loader's page
tables, removal of the identity mapping, kernel stack pool with guard
pages, `TlbControl` and `activate` adapters.

Tests: catalog 6.6.21 memory items.

## 8.6 Phase 4: Interrupts and timer

Deliverables: safe MADT parser; local APIC and I/O APIC register blocks and
adapters; legacy PIC masking; PIT-based timer calibration;
`InterruptController` and `Timer` traits with doubles; tick handling in
`kernel-core`.

Tests: catalog 6.6.11 with the fuzz target, 6.6.16 (APIC items), 6.6.21
interrupt items.

## 8.7 Phase 5: Objects, threads, user mode, system calls

Deliverables: handle tables, quotas, object types `Process`, `Thread`,
`MemoryObject`; scheduler; the context-switch naked function; user-mode
entry with synthesized frames; IPC buffer; system call vector `0x80`; the
`syscalls!` table in `audhsos-abi`; dispatcher and validation; system calls
for processes, threads, memory, and handles; `debug_log`; faults put
threads into `Faulted`. User-mode test programs are `user-sys-x86_64`
binaries embedded in test kernels as flat binaries.

Tests: catalog 6.6.6 (handle items), 6.6.7, 6.6.9, 6.6.21 thread, isolation,
and system call items.

## 8.8 Phase 6: IPC and interrupt forwarding

Deliverables: `Endpoint`, `Reply`, `Notification`, badges, handle transfer,
fault handler endpoints and fault messages, `Interrupt`, `IoPortRange`,
`SystemControl`, `Device` memory objects, `system_info`.

Tests: catalog 6.6.8, 6.6.21 IPC items.

## 8.9 Phase 7: Userland foundation

Deliverables: `user-sys-x86_64`, `user-rt` with the safe allocator,
`user-proto`, `user-loader`, `server-init`, `server-name`,
`server-console` over `driver-uart16550`, `server-memory`, `app-hello`;
boot image with tar archive; release build with `debug-uart` off.

Tests: catalog 6.6.12, 6.6.13 (tar items) with fuzz targets, 6.6.22, 6.6.23.

Acceptance: `cargo xtask run --release` prints the greeting through the
userland console driver; `cargo xtask test --e2e` passes.

## 8.10 Phase 8: Consolidation

Measure the system call round trip and the IPC round trip; decide the
`syscall` instruction path from the numbers. Review the `unsafe` budget.
Refresh every design document against the code. Write the 0.1.0 changelog
entry and tag.

## 8.11 Later work, not scheduled

virtio-blk driver and a file system server; virtio-net and a network
stack; the `aarch64` port under HVF without a loader; SMP with per-CPU run
queues; hardware port permission bitmaps; kernel-object memory donation;
an interface definition language for protocols; recursive capability
revocation; a tickless timer; long file names in the disk image writer.

## 8.12 Risks

| Risk | Effect | Mitigation |
|------|--------|------------|
| A pinned nightly breaks a feature | build failures on update | update in an isolated commit; keep the previous pin until green; features limited to three |
| The loader's firmware interaction differs between QEMU's firmware builds | boot failures in CI but not locally, or the reverse | the loader uses six boot services and four protocols only; the firmware version is recorded in the test log |
| The disk image writer and the property-test engine are tooling written before the kernel | Phase 0 and 2 grow | both are bounded by the catalog; no features beyond what the pipeline needs |
| `x86_64` details cost more than planned (APIC calibration, descriptor tables) | Phase 2 and 4 grow | scope is fixed to QEMU's default machine |
| Test time under TCG on Apple Silicon | slow feedback | many test cases per kernel, few kernels; host tests carry the bulk |
| `unsafe` creeps into logic crates | goal G4 fails | `forbid` at crate level plus the layering check; budget in CI |
| Pool sizing at boot is wrong for real workloads | spurious `PoolExhausted` | sizes are overridable in the boot image header; `system_info` exposes usage |
| Scope creep toward drivers and file systems before the userland foundation exists | first release slips | the roadmap order is binding; later work is listed, not scheduled |

## 8.13 Open decisions

These need confirmation before Phase 0 starts.

| # | Decision needed | Default if not answered |
|---|-----------------|-------------------------|
| 1 | Project name and crate prefix | `AuDHSOS`, `audhsos` |
| 2 | Build entry point on the development machine: `~/.cargo/bin/cargo xtask ...` instead of the shell alias | the xtask refuses to run under the aliased Cargo |
