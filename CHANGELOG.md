# Changelog

All notable changes to this project are documented in this file. The format
follows Keep a Changelog; the project follows Semantic Versioning.

## [Unreleased]

### Added

- Planning documents and the decision register under `docs/`.
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
- Planning for graphics output and input devices: roadmap Phases 9 to 11,
  decisions D-29 to D-33, catalog sections 6.6.24 to 6.6.29, and the
  framebuffer fields of the boot information structure in the documents.
