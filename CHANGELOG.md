# Changelog

All notable changes to this project are documented in this file. The format
follows Keep a Changelog; the project follows Semantic Versioning.

## [Unreleased]

### Added

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

### Changed

- `UnixTime` moves out of `audhsos-der` into the new `audhsos-time`
  crate, and the trust-anchor PEM decoding into `audhsos-encoding`
  (D-46, D-47); document 11 is amended accordingly.
- The crate catalog, the repository layout, the layering rules, and the
  duplication table of document 5 list the crates of document 12; the
  shared-crate list of layering rule 4 gains the three foundations.
- The roadmap and the document index state the status of track C as
  implemented through step T2, which the crates already were.
