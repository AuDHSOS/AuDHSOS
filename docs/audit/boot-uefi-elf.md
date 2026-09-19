# boot-uefi-x86_64, audhsos-uefi, audhsos-elf audit findings

Repository: AuDHSOS/AuDHSOS. Audit of boot-uefi-x86_64, audhsos-uefi, audhsos-elf at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #138
Title: audhsos-elf: an empty segment between two segments hides their overlap
Labels: bug, part::userland
Body:
`check_overlaps` compares only adjacent pairs of the list sorted by virtual address (`crates/elf/src/image.rs:413-430`), and `Segment::overlaps` reports `false` whenever either segment has `mem_size == 0` (`crates/elf/src/image.rs:90-100`). A `PT_LOAD` entry with `p_memsz == 0` sorted between two segments that share memory breaks the chain: neither pair containing the empty segment overlaps, and the two non-empty segments are never compared. The module invariant "no two segments share memory" (`crates/elf/src/image.rs:7-10`) and the README claim "do not overlap" (`crates/elf/README.md:5-6`) do not hold.

Trigger: three `PT_LOAD` entries, in file order, `A` at `vaddr 0x1000, memsz 0x2000, filesz 0x2000`, `B` at `vaddr 0x2000, memsz 0, filesz 0`, `C` at `vaddr 0x2000, memsz 0x1000, filesz 0x1000`, with offsets that satisfy the alignment rule. `parse` returns `Ok` with three segments instead of `Err(ElfError::SegmentsOverlap)`. In the loader, `place` copies `C` over the last page of `A` in the image buffer (`crates/boot/uefi-x86_64/src/placement.rs:372-385`) and the mapper then refuses the second mapping of page `0x2000` with `MapError::AlreadyMapped` (`crates/boot/uefi-x86_64/src/loader.rs:318-325`), so the loader reports "the page tables: the page is already mapped" for a defective image. The user program loader and the root task loader carry their own monotone page check (`crates/user/loader/src/program.rs:186-196`, `crates/kernel/core/src/root.rs:196-208`) and refuse the image with their own error. The test `segments_that_share_memory_are_rejected` covers only an empty segment next to one non-empty segment (`crates/elf/src/tests/image.rs:313-322`).

Fix: in `check_overlaps`, keep the last non-empty segment as `left` and compare every following non-empty segment against it, so that an empty segment does not advance the comparison; the alternative of rejecting `p_memsz == 0` entries outright would refuse images a linker can legitimately emit.

---

## F02 — issue #142
Title: boot-uefi-x86_64: the final memory map is read into a buffer sized without slack
Labels: bug, part::loader
Body:
`map_buffer` keeps the first buffer into which `GetMemoryMap` succeeds (`crates/boot/uefi-x86_64/src/loader.rs:219-236`). Between that read and the final read in `leave_boot_services` (`crates/boot/uefi-x86_64/src/loader.rs:393-410`), the loader allocates the kernel image, the boot stack, the boot information page, and the page table pool (`crates/boot/uefi-x86_64/src/loader.rs:141-144`) and calls `LocateProtocol` and `GetTime` (`crates/boot/uefi-x86_64/src/loader.rs:155-158`), each of which can add descriptors to the map. `leave_boot_services` reads into the same buffer with the same length and returns `Failure::Firmware("the memory map", BUFFER_TOO_SMALL)` on the first failure. `Firmware::memory_map` discards the size the firmware writes back on `EFI_BUFFER_TOO_SMALL` (`crates/boot/uefi-x86_64/src/firmware.rs:277-299`), so no caller can size a retry from it. UEFI 2.11 section 7.2.3 (`docs/uefi/UEFI_Spec_Final_2.11.pdf`, page 160) says the buffer for the consequent call should be bigger than the returned `MemoryMapSize` because allocating the buffer itself grows the map.

Trigger: a firmware with `DescriptorSize` 48 whose map holds 1360 descriptors at the first read (65280 bytes, inside the 16-page buffer of 65536 bytes); the four allocations and the firmware's own pool allocations add six descriptors (288 bytes) before the final read, which needs 65568 bytes and fails. The loader dies with a firmware error on a machine that is fine.

Fix: in `map_buffer`, after the first successful read, keep the buffer only if `size` plus one page fits, otherwise free it and allocate `size` rounded up plus one page before reading again; alternatively make `leave_boot_services` grow the buffer on `BUFFER_TOO_SMALL`, which costs an extra allocation cycle right before `ExitBootServices`.

---

## F03 — issue #147
Title: boot-uefi-x86_64: console output after a failed ExitBootServices
Labels: bug, part::loader
Body:
`leave_boot_services` returns `Failure::Firmware("leaving the boot services", status)` after `EXIT_ATTEMPTS` failed calls of `ExitBootServices` (`crates/boot/uefi-x86_64/src/loader.rs:397-409`). `efi_main` passes the failure to `exit::fail_with` (`crates/boot/uefi-x86_64/src/main.rs:46-47`), which calls `Firmware::output_string` three times (`crates/boot/uefi-x86_64/src/exit.rs:284-289`, `crates/boot/uefi-x86_64/src/exit.rs:301-305`), which calls `OutputString` of the console protocol (`crates/boot/uefi-x86_64/src/firmware.rs:356-369`). UEFI 2.11 section 7.4.6 (`docs/uefi/UEFI_Spec_Final_2.11.pdf`, page 204) allows the firmware a partial shutdown of the boot services during the first `ExitBootServices` call and says a loader should call nothing but the memory allocation services after that call.

Trigger: a firmware whose map key changes on every `GetMemoryMap` call, so that all three `ExitBootServices` calls return `EFI_INVALID_PARAMETER`; the console protocol of a firmware that shut its drivers down on the first call hangs or faults in `OutputString`, and the machine never reaches the exit device write in `die`, so the test runner sees no exit status.

Fix: make `leave_boot_services` call `exit::die()` directly when the attempts are used up, which skips the console; the alternative of keeping the message costs the guarantee that every loader failure reaches the exit device.

---

## F04 — issue #149
Title: boot-uefi-x86_64: the crate catalog claims host tests the crate does not have
Labels: bug, part::loader
Body:
The crate catalog lists `boot-uefi-x86_64` with host tests for "pure sub-modules" (`docs/05-code-organization.md:182`). The crate's manifest sets `test = false` for its only target (`crates/boot/uefi-x86_64/Cargo.toml:16`), and no source file of the crate contains a `#[cfg(test)]` module or a `#[test]` function. The pure work the loader once did is in `audhsos-uefi` (`crates/uefi/README.md:6-11`), which the catalog lists separately.

Trigger: `cargo test -p boot-uefi-x86_64` builds no test target and runs zero tests, which contradicts the catalog row.

Fix: change the row's host tests column to "no", and name `audhsos-uefi` as the crate where the memory map conversion, the framebuffer description, and the UTF-16 encoding are tested.

---

## F05 — issue #152
Title: audhsos-elf: segment_bytes documents an emptiness rule the code does not apply
Labels: bug, part::userland
Body:
The doc comment on `Image::segment_bytes` says "A segment that does not belong to this image yields an empty slice" (`crates/elf/src/image.rs:153-154`). The body slices `self.bytes` by `file_offset` and `file_size` alone (`crates/elf/src/image.rs:156-163`) and has no notion of membership. The test `segment_bytes_of_a_foreign_segment_are_empty` uses only foreign segments whose file range leaves the file (`crates/elf/src/tests/image.rs:435-453`).

Trigger: a `Segment { file_offset: 0, file_size: 4, .. }` that was never returned by `segments()` yields the four magic bytes of the file, not an empty slice.

Fix: state the actual rule in the comment: a file range inside the file yields those bytes, a range that leaves the file yields an empty slice; the alternative of checking membership against the segment table would add a linear scan per call for a guarantee no caller needs.

---

## F06 — issue #156
Title: boot-uefi-x86_64: the page table pool reserve for the kernel image is a constant
Labels: enhancement, part::loader
Body:
`pool_frames` adds `RESERVE_FRAMES = 24` for the kernel image, the boot stack, and the boot information page (`crates/boot/uefi-x86_64/src/paging.rs:560-571`). Of those, five are fixed: the PDPT of PML4 slot 511, the page directory of the kernel image, the page directory that holds both the boot stack and the boot information page (`crates/abi/src/layout.rs:58-66`), and one page table each for the stack and the page. The remaining 19 page tables cover at most 19 two-MiB windows of the kernel image. The pool is allocated after `placement::place` has computed the image span (`crates/boot/uefi-x86_64/src/loader.rs:141-144`), so the count of page tables the image needs is known at that point and is not used.

Trigger: a kernel image whose page-aligned span crosses 20 or more 2 MiB boundaries; `Mapper::map_range` returns `MapError::OutOfKernelMemory` and the loader reports "the page tables: no frame is left for a page table". The current debug kernel spans about 2 MiB, so the bound is safe today.

Fix: pass `placement.frames.count()` into `pool_frames` and add `tables_for(placement.frames.bytes())` plus the five fixed frames instead of the constant.

---

## F07 — issue #161
Title: boot-uefi-x86_64: an empty segment widens the placed image span
Labels: enhancement, part::loader
Body:
`span` folds every segment's `vaddr` into `lowest` and every segment's end into `highest` without regard to `mem_size` (`crates/boot/uefi-x86_64/src/placement.rs:325-329`). The parser admits a `PT_LOAD` entry with `p_memsz == 0` anywhere inside the constraints, since `Constraints::allows` accepts an empty range that starts inside the bounds (`crates/elf/src/image.rs:113-127`), and the loader's constraints span the whole kernel half (`crates/boot/uefi-x86_64/src/loader.rs:54-57`).

Trigger: a kernel image with an empty `PT_LOAD` entry at `vaddr 0xFFFF_8000_0000_0000` beside the real segments at `0xFFFF_FFFF_8000_0000`; `span_len` becomes about 2 TiB, `allocate_pages` fails, and the loader reports the firmware's `EFI_OUT_OF_RESOURCES` instead of naming the segment.

Fix: skip segments with `mem_size == 0` in `span` and in the placement loop, so that the span covers only memory the image occupies.

---

## F08 — issue #164
Title: boot-uefi-x86_64: the allowlist row names less unsafe content than the crate holds
Labels: bug, part::loader
Body:
The allowlist row for the loader names five kinds of `unsafe` content: firmware calls through function pointers, the memory map buffer from a raw pointer, page-table memory through the identity mapping, the `CR3` write, and the kernel entry (`docs/04-safety-policy.md:36`). The crate also dereferences the system table, the boot services table, the runtime services table, and every protocol structure it opens (`crates/boot/uefi-x86_64/src/firmware.rs:162-187`, `crates/boot/uefi-x86_64/src/firmware.rs:342`, `crates/boot/uefi-x86_64/src/graphics.rs:216-223`, `crates/boot/uefi-x86_64/src/files.rs:105`, `crates/boot/uefi-x86_64/src/files.rs:150`), reads the configuration table as a slice (`crates/boot/uefi-x86_64/src/firmware.rs:379-381`), and builds byte slices over the frames of the two files, the placed kernel image, and the boot information page (`crates/boot/uefi-x86_64/src/memory.rs:235`, used at `crates/boot/uefi-x86_64/src/files.rs:164-165`, `crates/boot/uefi-x86_64/src/placement.rs:367-368`, `crates/boot/uefi-x86_64/src/loader.rs:134`, `crates/boot/uefi-x86_64/src/loader.rs:424`). The row for `kernel-hal-x86_64` in the same table enumerates its content at that level of detail (`docs/04-safety-policy.md:34`).

Trigger: a reviewer applying checklist item 1 of section 4.9 against the row cannot match the `SAFETY` comments of `firmware.rs:160-189`, `firmware.rs:379-381`, or `memory.rs:232-235` to a listed kind of content.

Fix: extend the row to "firmware calls through function pointers, the firmware tables and protocol structures behind the pointers the firmware hands out, the configuration table as a slice, file buffers, the kernel image, the boot information page, the memory map buffer, and page-table memory through the identity mapping, `CR3` write, kernel entry".
