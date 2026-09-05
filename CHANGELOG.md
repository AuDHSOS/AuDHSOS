# Changelog

All notable changes to this project are documented in this file. The format
follows Keep a Changelog; the project follows Semantic Versioning.

## [Unreleased]

### Added

- `audhsos-abi`: `KERNEL_STACKS_BASE`, `KERNEL_STACK_PAGES`,
  `KERNEL_STACK_SLOT_PAGES`, `KERNEL_STACK_SLOTS`, and
  `MAX_PHYS_WINDOW_BYTES`.
- `kernel-mm`: the kernel stack pool. A slot is one unmapped guard page
  followed by four mapped pages; an allocation that runs out of frames
  leaves no slot taken and no page mapped. `NoFrames`, a frame source for
  a walk that creates no table, and `Mapper::frames_mut`.
- `kernel-core`: `config`, the number of each kind of kernel object and
  the size of the fixed tables of the kernel address space; `memory`, the
  bring-up. It takes the reserve out of the normalized map in the size the
  boot image header asks for, adopts the loader's page tables by walking
  the kernel image, the physical window, the boot stack, and the boot
  information page into a kernel region table, drops the loader's identity
  mapping without giving a frame back, and stores the result in the
  `MEMORY` cell.
- `kernel-hal-x86_64`: `PhysicalWindow` with `frame_bytes_mut`, which
  replaces `WindowAccess`; `active_root` and `activate` beside `LocalTlb`;
  and `memory`, the walk of the active tables through the window that the
  bring-up and the boot information page need.
- `audhsos-kernel`: the kernel takes its memory over after the boot report
  and reports the reserve, the regions of its address space, and the
  identity mapping it dropped. Three test kernels, `memory`,
  `memory_fault`, and `kernel_stack`, cover the memory and kernel stack
  items of catalog 6.6.21.
- `kernel-core`: `KernelMemory::allocate_stack` and `release_stack`, which
  build the mapper out of the root frame and the reserve and drive the
  stack pool.
- `kernel-hal-x86_64`: `testing::write_byte`, the write a test image needs
  to show that a page it mapped carries what it wrote and that a guard
  page faults.
- Catalog 6.6.21 gains the kernel stack items and the boot information
  address item.
- Plan 10.5.0: what the kernel reserve carries and what the kernel image
  carries, with the arithmetic behind D-57.
- Planning documents and the decision register under `docs/`.
- `docs/11-cryptography-and-tls.md`: the design and implementation plan
  for the TLS 1.3 client track (constant-time primitives, hashes and
  HKDF, AEADs, elliptic curves, random generator, DER, X.509, the sans-I/O
  protocol crate), its test catalog entries 6.6.30 to 6.6.38, its roadmap
  track 8.17, and decisions D-36 to D-44.
- `docs/12-parallel-work.md`: the design and implementation plan for the
  work that runs beside the kernel phases — the admission test for
  parallel work, the sans-I/O network stack (`net-wire`, `net-eth`,
  `net-ip`, `net-udp`, `net-tcp`, `net-dns`, `net-dhcp`, `net-http`,
  `net-stack`), the shared foundations (`audhsos-time`,
  `audhsos-encoding`, `audhsos-collections`), the device logic without
  devices (`virtio-queue`, `fs-fat`), the tooling (`fuzz-support`,
  `audhsos-symbols`), the phase work that may be pulled forward, and the
  capacity rule; its test catalog entries 6.6.39 to 6.6.53, its roadmap
  tracks 8.18 to 8.22, and decisions D-45 to D-54.
- `crypto-ct`: `Choice`, constant-time comparison, selection, exchange and
  copy, `Secret<N>` with a best-effort erase on drop.
- `crypto-hash`: SHA-256, SHA-384, SHA-512, HMAC, and HKDF, against the
  vectors of FIPS 180-4, RFC 4231, and RFC 5869.
- `crypto-aead`: `ChaCha20`, `Poly1305`, and the `ChaCha20-Poly1305`
  authenticated cipher of RFC 8439, sealing and opening in place, with
  verification before decryption.
- `crypto-aead`: AES-128 and AES-256 bitsliced over four blocks without a
  lookup table, table-free GHASH, and AES-128-GCM and AES-256-GCM, against
  the published test cases of the mode.
- `crypto-ec`: the field of `2^255 - 19` and X25519 with a constant-time
  Montgomery ladder, against the vectors of RFC 7748.
- `crypto-ec`: Ed25519 verification and deterministic signing behind
  `test-signing`, with strict canonicality and small-order checks, against
  the vectors of RFC 8032.
- `crypto-ec`: P-256 with Montgomery arithmetic for both moduli, Jacobian
  point arithmetic, ECDSA verification, and deterministic signing per
  RFC 6979 behind `test-signing`.
- `crypto-rng`: the `Entropy` and `Rng` traits, a `ChaCha20` generator that
  rekeys after every request and mixes fresh material into its key when it
  reseeds, and the doubles the protocol tests will need.
- `audhsos-der`: a strict, zero-copy reader for the distinguished encoding
  rules, with bounded nesting and one encoding per value. Its time
  conversion waits on `audhsos-time` (D-46); section 11.14 of document 11
  lists that seam and the others.
- `audhsos-x509`: certificate parsing and signature verification, chain
  validation against caller-supplied trust anchors, and RFC 6125 name
  matching, with a builder behind `test-certificates` that writes and signs
  the certificates the tests use.
- Decision D-56: the key exchange of the TLS client is `x25519` alone.
- Workspace foundation: pinned toolchain, workspace lint set, SPDX headers,
  license, CI workflow.
- `audhsos-abi`: error codes, rights, object types, handles, layout
  constants.
- `kernel-types`: physical and virtual addresses, frames, pages, ranges,
  alignment.
- `kernel-hal-api`: hardware abstraction traits with test doubles.
- `audhsos-sync`: the `Global<T>` cell for global state.
- `test-support`: property-test engine with integrated shrinking and
  model-test runner.
- Unit tests live in `src/tests/` so that coverage measures product code
  only.
- `xtask`: `lint`, `check-layering`, `check-deps`, `unsafe-budget`,
  `test`, `coverage`, `miri`, `doc`, `check`.
- `audhsos-abi`: boot image header and boot information structure with
  validating parsers and writers, and generators behind the feature
  `test-strategies`.
- `kernel-objects`: fixed-capacity object pool with generation-checked ids,
  reference counts, and first-in-first-out slot reuse; quotas.
- `kernel-mm`: memory map normalization, kernel reserve selection, bitmap
  frame allocator, `x86_64` page-table entries behind an
  architecture-neutral trait, the mapper over the HAL traits with bounded
  range operations, and the region table of an address space.
- `kernel-types`: `PhysFrame::ZERO` and `PhysFrameRange::EMPTY`.
- `kernel-hal-api`: `MemoryFrameAccess::with_lazy_tables`, which
  materializes a page table on the first modifying access to a frame of a
  declared memory range.
- `audhsos-elf`: validating ELF64 parser with an image builder behind the
  feature `test-strategies`.
- `driver-uart16550`: register logic of the 16550 serial controller with a
  recording register double behind the feature `test-doubles`.
- `kernel-x86-tables`: encoding and decoding of the global descriptor
  table, the interrupt descriptor table, and the task state segment.
- `audhsos-uefi`: layouts, status codes, and identifiers of the UEFI
  interfaces the loader uses, the memory map reader that honors the
  firmware's stride, the conversion into boot regions, and UTF-16
  encoding; the Graphics Output Protocol structures, `LocateProtocol`, and
  the conversion of a graphics mode into a framebuffer description.
- `audhsos-abi`: the boot information carries the framebuffer the firmware
  set up. The fixed part grows to 136 bytes; the version stays 1.
- `xtask`: the disk image writer with its own CRC-32, GUID partition
  table, and FAT32 file system, the boot image writer, and the `image`
  subcommand; the policy table names the target every crate is built for,
  and the host commands skip the crates that are not built for the host.
- `kernel-core`: the boot report, the trap report, and the cell holding the
  global kernel state.
- `kernel-test-harness`: the test runner of a kernel image and the serial
  line protocol it writes.
- `kernel-hal-api`: a mutable reference to a debug console or an exit
  device is one, so that the kernel can hand one out without giving it
  away.
- `kernel-hal-x86_64`: the first adapter crate. Privileged instruction
  wrappers, the descriptor tables, the trap handlers, the boot information
  as a `Platform`, the serial debug console, the exit device, and page
  table memory through the physical window.
- `audhsos-kernel`: the kernel image with its linker script, the entry the
  loader jumps to, and the panic handler.
- `audhsos-abi`: `BOOT_STACK_TOP`, `BOOT_STACK_PAGES`, and
  `BOOT_INFO_VADDR`.
- `xtask`: the `build` subcommand, and a check that the constants the
  linker scripts repeat agree with the ABI.
- `boot-uefi-x86_64`: the loader. It reads the kernel and the boot image
  from the boot volume, places the kernel image in one physical range,
  builds the physical memory window, an identity mapping, the kernel
  segments, the boot stack, and the boot information page with the
  kernel's own mapper, writes the boot information from the memory map it
  reads last, and enters the kernel.
- `xtask`: `qemu-runner`, `run`, and `test --qemu`. The runner wraps a
  test kernel into a disk image, runs the reference machine with a time
  limit, reads the serial protocol, and maps the exit status; `test
  --qemu` runs every test kernel and the three images the loader has to
  reject; `check` runs it last.
- `kernel-hal-x86_64`: `testing`, what a kernel test image needs: the
  harness over the debug console and the exit device, the `test_kernel!`
  macro that writes the entry point and the panic handler once, the trap
  hook a test image registers, and the instructions that raise the
  exceptions the trap tests expect.
- `audhsos-kernel`: ten test kernels under `tests/`: boot, console,
  descriptors, breakpoint, divide error, invalid opcode, general
  protection, page fault, the double fault of a kernel stack overflow, and
  a panic in an image that expects one.
- `audhsos-abi`: the boot stack is 64 pages, not 16. An unoptimized test
  image needs more than 64 KiB before it reaches the harness.
- CI installs QEMU and the UEFI firmware and names the firmware the
  distribution installed, so that `check` can run `test --qemu`.
- Planning for graphics output and input devices: roadmap Phases 9 to 11,
  decisions D-29 to D-33, catalog sections 6.6.24 to 6.6.29, and the
  framebuffer fields of the boot information structure in the documents.

### Fixed

- `kernel-hal-x86_64`: the boot information page was reported as a memory
  region with its virtual address, which `PhysAddr::new` rejects, so the
  region was silently dropped and no boot report ever showed a
  `boot-info` line. The entry point now walks the loader's tables for
  `BOOT_INFO_VADDR` and reports the frame that walk names.

### Changed

- The object pools live in the `.bss` of the kernel image, not in the
  kernel reserve (D-57): a `Pool<T, N>` is a typed array, and putting one
  into raw frames would need `unsafe` in a logic crate. Document 2.4.1 said
  the reserve holds them and is corrected. The reserve holds page tables,
  kernel stacks, and the IPC buffers of threads.
- The handle slots of the machine are one shared arena of `HANDLE_ENTRIES`
  (16384) and `HANDLES_PER_PROCESS` (4096) is the ceiling the quota
  enforces against it, not memory set aside per process (D-58). This
  supersedes the handle part of D-57: 1024 handles were too few for
  `server-memory`, which holds one handle per memory object it hands out
  and can therefore reach `MEMORY_OBJECTS`, and 4096 inside every `Process`
  would have been 8 MiB of `.bss` for the one process that needs them.
  Plan 10.5.2 describes the arena, and 2.3.2 is worded for it.
- `THREADS` and `KERNEL_STACKS` are 256 and `KERNEL_STACK_SLOTS` follows
  them, because every thread costs five frames of the reserve and 1024 of
  them would need 5120 against the 4048 the reference machine has.
  `PROCESSES` is 64 and `HANDLES_PER_PROCESS` is 1024, because the handle
  table lives in the `Process` object and 256 tables of `1 << 16` entries
  would be 512 MiB of `.bss`. The reference machine keeps its 256 MiB.
- `kernel-hal-x86_64::WindowAccess` becomes `PhysicalWindow` in a module
  of its own and gains the byte access the IPC buffers and the boot image
  header need; `paging` keeps the lookaside buffer and the page-table
  root.
- The kernel binary carries `kernel-hal-api`, `kernel-mm`, and
  `kernel-types` as dev-dependencies for its test kernels; the kernel
  image itself keeps its three dependencies.
- `UnixTime` moves out of `audhsos-der` into the new `audhsos-time`
  crate, and the trust-anchor PEM decoding into `audhsos-encoding`
  (D-46, D-47); document 11 is amended accordingly.
- The crate catalog, the repository layout, the layering rules, and the
  duplication table of document 5 list the crates of document 12; the
  shared-crate list of layering rule 4 gains the three foundations.
- The roadmap and the document index state the status of track C as
  implemented through step T2, which the crates already were.
