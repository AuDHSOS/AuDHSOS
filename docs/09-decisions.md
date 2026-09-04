# 9. Decision Register

Every binding decision as a statement. A decision is changed by a new entry
that supersedes the old one; entries are never edited after acceptance.

Status values: `decided` (stated by the project owner), `proposed`
(awaiting confirmation), `superseded by D-nn`.

| ID | Decision | Status | Date |
|----|----------|--------|------|
| D-01 | AuDHSOS is a microkernel. The kernel provides address spaces, threads with scheduling, IPC, capabilities, and interrupt forwarding, and nothing else. | decided | 2026-09-04 |
| D-02 | Everything else runs in userland: drivers including the console driver, program loading, naming, memory policy, fault handling, file systems, networking. | decided | 2026-09-04 |
| D-03 | The system is written in Rust. No assembly files, no `global_asm!`. Inline assembly only in adapter crates, inventoried and budgeted. Memory management contains no assembly. | decided | 2026-09-04 |
| D-04 | No external code. `Cargo.lock` lists only workspace members. No crates.io, git, or vendored third-party code in any dependency section, including build and dev dependencies. No external Cargo subcommands. The only external components are the Rust toolchain and QEMU with its bundled UEFI firmware. | decided | 2026-09-04 |
| D-05 | `unsafe` is confined to the allowlisted adapter crates `kernel-hal-x86_64`, `boot-uefi-x86_64`, `audhsos-sync`, `user-sys-x86_64`, and `fuzz-support`. Every other crate has `#![forbid(unsafe_code)]`. Every `unsafe` block has a `SAFETY:` comment and counts against a budget checked in CI. | decided | 2026-09-04 |
| D-06 | Every algorithm is written against a trait and tested on the host with doubles. Adapters implement the traits for the loader and the kernel. One implementation per algorithm. | decided | 2026-09-04 |
| D-07 | First target: QEMU `q35` on `x86_64`, booted through UEFI. Legacy BIOS boot is not supported. Second target: `aarch64` on QEMU `virt` under HVF. Third target: `riscv64` on QEMU `virt`. | decided | 2026-09-04 |
| D-08 | The loader is the project's own UEFI application `boot-uefi-x86_64`. UEFI structure layouts live in the logic crate `audhsos-uefi`. The loader reads `AUDHSOS/KERNEL.ELF` and `AUDHSOS/BOOT.IMG` from the boot volume, builds page tables with the kernel's mapper, and enters the kernel with the boot information structure. | decided | 2026-09-04 |
| D-09 | The disk image is written by the xtask: GPT with a protective MBR and one EFI system partition, FAT32 only, 8.3 names, no long file name entries. | decided | 2026-09-04 |
| D-10 | The toolchain is one pinned nightly. Unstable features: `abi_x86_interrupt`, `custom_test_frameworks`, `-Zsanitizer=fuzzer` on the host. Adding a feature requires a register entry. | decided | 2026-09-04 |
| D-11 | The kernel has no heap and does not link `alloc`. Kernel objects live in typed pools sized at boot from the kernel reserve, addressed by generation-checked ids. Every allocation is fallible. Each process has a kernel-object quota. | decided | 2026-09-04 |
| D-12 | The kernel allocates user memory never after boot. All free memory becomes memory objects of the root task; memory objects are split, never merged, by the kernel. The memory server overwrites every memory object with zeros before handing it out and again immediately when it is returned. | decided | 2026-09-04 |
| D-13 | Capabilities: per-process handle tables with generation-checked handles, rights that only decrease, badges on endpoint capabilities, no recursive revocation in the first release. A handle is 64 bits: 32-bit table index, 32-bit generation. | decided | 2026-09-04 |
| D-14 | IPC: synchronous rendezvous on endpoints (`call`, `send`, `recv`, `reply`, `reply_recv`, `try_recv`) plus notifications with 64 signal bits. Messages carry up to 480 words and 4 handles through IPC buffers. Bulk data uses shared memory objects. | decided | 2026-09-04 |
| D-15 | System calls enter through interrupt vector `0x80` with the `x86-interrupt` ABI. Arguments and results live in the calling thread's IPC buffer. No user pointers in the interface. The system call table is one declarative macro in `audhsos-abi`. The `syscall` instruction path is decided in Phase 8 by measurement. | decided | 2026-09-04 |
| D-16 | Faults in user mode become IPC calls to the process's fault handler endpoint. | decided | 2026-09-04 |
| D-17 | The kernel is non-preemptible and runs on one CPU in the first release. System calls are bounded and return `Partial` for range operations. The SMP path is per-CPU run queues and a big kernel lock first. | decided | 2026-09-04 |
| D-18 | Scheduling: 32 fixed priorities, FIFO run queue per priority, round-robin time slices in timer ticks, idle thread on the boot stack. | decided | 2026-09-04 |
| D-19 | I/O port access from userland is a system call on an `IoPortRange` handle. Userland drivers contain no inline assembly. | decided | 2026-09-04 |
| D-20 | Allocators compute offsets, not pointers. The frame allocator and the userland heap allocator are project code in safe Rust; the only `unsafe` is the `GlobalAlloc` adapter in `user-sys-x86_64`. | decided | 2026-09-04 |
| D-21 | Global state uses `audhsos-sync::Global<T>` with a runtime borrow flag. No spinlock crate. | decided | 2026-09-04 |
| D-22 | The ELF parser `audhsos-elf` and the UART register logic `driver-uart16550` are single logic crates shared by loader, kernel, and userland. | decided | 2026-09-04 |
| D-23 | Testing: logic on the host with the project's property-test engine and model-test runner; adapters in QEMU with the custom test framework, the serial protocol, and `isa-debug-exit`; parsers fuzzed with the toolchain's sanitizer runtime; coverage with the toolchain's `llvm-profdata` and `llvm-cov`. The edge-case catalog is the definition of done. | decided | 2026-09-04 |
| D-24 | Policies (layering, unsafe budgets, SPDX header, fuzz targets) are Rust tables in `xtask/src/policy.rs`. No configuration file parsers. | decided | 2026-09-04 |
| D-25 | License `AGPL-3.0-only` without exceptions, SPDX header in every file, copyright line `Copyright (C) 2026 Manuel Baesler and contributors`. | decided | 2026-09-04 |
| D-26 | Everything is written in English: code, comments, documentation, commits, tests. | decided | 2026-09-04 |
| D-27 | The boot image is a fixed header, the root task as a flat binary at `ROOT_TASK_BASE`, and a ustar tar archive. The kernel validates the header and never reads the archive. | decided | 2026-09-04 |
| D-28 | The boot information structure is `#[repr(C)]` in `audhsos-abi`, versioned, with a bounded region array. | decided | 2026-09-04 |
