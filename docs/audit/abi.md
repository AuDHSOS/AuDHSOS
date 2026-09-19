# audhsos-abi audit findings

Repository: AuDHSOS/AuDHSOS. Audit of audhsos-abi at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #81
Title: audhsos-abi: the rights table of document 02 omits INFO on Process
Labels: bug, part::abi
Body:
`docs/02-architecture.md:71` lists the rights of `Process` as `MANAGE`, `MAP`, `INSTALL`. `crates/abi/src/object.rs:67` gives `Process` the mask `[MANAGE, MAP, INSTALL, INFO]`. `crates/kernel/syscall/src/dispatch.rs:199-202` demands `INFO` on the `Process` handle for `process_watch` and `process_unwatch`. `crates/kernel/syscall/src/calls/process.rs:203-209` installs the creator's handle with `INFO` set.

A reader of the table derives a `Process` handle with `handle_duplicate(handle, MANAGE | MAP | INSTALL)`, believing it to be the full set, and the receiver's `process_watch` on that handle fails with `AccessDenied`. The table also lists no `watch` operation in the operations column of the same row.

Fix: add `INFO` to the rights cell and `watch`, `unwatch` to the operations cell of the `Process` row in `docs/02-architecture.md:71`; removing `INFO` from `object.rs:67` instead would break `process_watch`.

---

## F02 — issue #83
Title: audhsos-abi: the doc of Ecam::is_empty claims the all-zero window is empty
Labels: bug, part::abi
Body:
`crates/abi/src/ecam.rs:45-46` says `is_empty` is `true` for "the four zero words of a machine without an `MCFG` table". `crates/abi/src/ecam.rs:49` computes `self.last_bus < self.first_bus`, which is `false` for `first_bus == 0` and `last_bus == 0`. `crates/kernel/syscall/src/calls/device.rs:517-528` writes no ECAM words on such a machine, so userland reads four zeros.

`Ecam { base: 0, segment: 0, first_bus: 0, last_bus: 0 }.is_empty()` returns `false`, `buses()` returns `1` (`crates/abi/src/ecam.rs:33-37`), and `len()` returns one mebibyte (`crates/abi/src/ecam.rs:41-43`). `crates/user/sys-x86_64/src/gate.rs:1089-1091` works around this with an extra `ecam.base != 0` test, which is the check the doc says `is_empty` makes.

Fix: make `is_empty` return `self.base == 0 || self.last_bus < self.first_bus` and drop the `base != 0` test in `gate.rs:1091`; rewriting the doc comment to match the code instead leaves every future caller to repeat the gate's workaround.

---

## F03 — issue #84
Title: audhsos-abi: the doc of the INFO right names memory objects only
Labels: bug, part::abi
Body:
`crates/abi/src/rights.rs:44` documents `INFO` as "query the physical range of a memory object". `crates/kernel/syscall/src/dispatch.rs:199-202` demands `INFO` for `memory_info`, `memory_references`, `process_watch`, and `process_unwatch`. `crates/abi/src/object.rs:67` grants `INFO` on `Process`.

The generated rustdoc of `Rights::INFO` tells a reader that the right is meaningless on a `Process` handle, and a parent that strips it from the handle it installs for a child stops that child's `process_watch`.

Fix: change the string at `rights.rs:44` to "query the physical range of a memory object, or watch the end of a process".

---

## F04 — issue #86
Title: audhsos-abi: document 02 claims the loader marks the kernel ranges with their own region kinds
Labels: bug, part::abi
Body:
`docs/02-architecture.md:540-543` says the loader converts the memory map into the boot information structure and that "page tables, boot stack, kernel, and boot image are marked with their own kinds". `crates/abi/src/boot_info.rs:68-79` defines five kinds: `Usable`, `Reserved`, `AcpiReclaimable`, `AcpiNvs`, `MmioReserved`. The four ranges are header fields at `crates/abi/src/boot_info.rs:171-186`, and `crates/uefi/src/memory_map.rs:147-151` reports the frames the loader allocated as `Usable`. The kinds `Kernel`, `BootImage`, `PageTables`, `BootStack` exist only in the kernel's `MemoryRegionKind`, which `crates/kernel/hal-x86_64/src/bootinfo.rs:76-98` derives from the header fields after the parse. `docs/03-target-platform.md:239-240` states the five kinds and that loader data is `Usable`.

A reader of step 6 expects a `BootRegionKind::Kernel` code in the region array and a parser that rejects a structure without one; `BootInfoView::check_fixed_ranges` (`crates/abi/src/boot_info.rs:672-698`) instead requires each range to lie inside a region of any kind.

Fix: rewrite `docs/02-architecture.md:543` to say the loader reports the four ranges in the header fields of the structure and the HAL adapter gives them their own kinds.

---

## F05 — issue #88
Title: audhsos-abi: BYTES_PER_BUS and the Ecam type are defined three times
Labels: enhancement, part::abi
Body:
`crates/abi/src/ecam.rs:13`, `crates/kernel/acpi/src/mcfg.rs:41`, and `crates/pci/src/address.rs:36` each define `BYTES_PER_BUS = 1 << 20`. `crates/kernel/acpi/src/mcfg.rs:49-81` defines a second `Ecam` struct with the same four fields and the same `buses`, `len`, and `is_empty` bodies as `crates/abi/src/ecam.rs:18-50`. `crates/kernel/hal-x86_64/src/entry.rs:65-70` copies one into the other field by field. The kernel computes the device memory region of the window with the `mcfg.rs` copy (`crates/kernel/acpi/src/mcfg.rs:73`), and the root task and `app_lspci` compute bus offsets with the `pci` copy (`crates/user/programs/src/bin/server_init.rs:48`, `crates/user/programs/src/bin/app_lspci.rs:47`). The `abi` copy is used by `Ecam::len` only.

A change to one of the three copies changes the size of the window on one side and not the other, and no test compares them. `crates/kernel/acpi` depends on `audhsos-abi` (`docs/05-code-organization.md:179`), so the kernel copy has no layering reason to exist.

Fix: keep `BYTES_PER_BUS` and `Ecam` in `crates/abi/src/ecam.rs`, have `kernel-acpi` return `audhsos_abi::Ecam` with `base` as `u64` and `pci` import the constant; keeping three copies with a cross-crate test instead adds a test for a value that one definition removes.

---

## F06 — issue #89
Title: audhsos-abi: the fault label doc says the six kinds occupy the first six labels
Labels: enhancement, part::abi
Body:
`crates/abi/src/ipc_buffer.rs:89-92` says the six fault kinds "occupy the first six labels of the reserved range and 250 are left". `crates/abi/src/ipc_buffer.rs:97-104` computes the label as `FAULT_LABEL_BASE + code`, and the codes at `crates/abi/src/thread.rs:154-159` run from `1` to `6`. The fault labels are `FAULT_LABEL_BASE + 1` through `FAULT_LABEL_BASE + 6`; `FAULT_LABEL_BASE` itself names no fault (`fault_kind_of(KERNEL_LABEL_BASE)` returns `None` at `crates/abi/src/ipc_buffer.rs:109-123`).

A kernel message of a later phase given the label `KERNEL_LABEL_BASE + 6` collides with `AlignmentCheck`, and one given `KERNEL_LABEL_BASE` does not, which is the opposite of what the doc says.

Fix: change the sentence to "the six kinds occupy labels 1 to 6 of the reserved range; label 0 and labels 7 to 255 are left".

---

## F07 — issue #91
Title: audhsos-abi: the buffer-size assert in layout.rs counts 104 header bytes where the layout has 184
Labels: enhancement, part::abi
Body:
`crates/abi/src/layout.rs:187` asserts `MAX_MESSAGE_WORDS * 8 + MAX_MESSAGE_HANDLES * 8 + 3 * 8 + 10 * 8 <= 4096`, which puts the payload words at byte 104 plus the handle area. `crates/abi/src/ipc_buffer.rs:72` puts the payload words at byte `184`, and `crates/abi/src/ipc_buffer.rs:77-78` asserts the exact end `WORDS + MAX_MESSAGE_WORDS * WORD == 4024`.

`MAX_MESSAGE_WORDS = 495` passes the assert at `layout.rs:187` (`3960 + 32 + 104 = 4096`) while the payload ends at byte `4144`; only the assert at `ipc_buffer.rs:77` rejects it. The `layout.rs` assert is a check that can never be the one that fires.

Fix: delete the assert at `crates/abi/src/layout.rs:187`, since `ipc_buffer.rs:77-78` holds the exact bound; correcting the constants to `16 * 8` instead keeps a second copy of the layout arithmetic.

---

## F08 — issue #93
Title: audhsos-abi: Writer::append relies on an even MAX_MESSAGE_WORDS to write nothing on Full
Labels: enhancement, part::abi
Body:
`crates/abi/src/startup.rs:461` and `crates/abi/src/startup.rs:480` promise that nothing is written when the message area is full. `crates/abi/src/startup.rs:502-507` writes the role word at index `2n` and only then tries the payload word at `2n + 1`. Both writes stay inside the area or both fail because `MAX_MESSAGE_WORDS` at `crates/abi/src/layout.rs:127` is `480`, an even number; no assert states that.

With an odd `MAX_MESSAGE_WORDS`, the pair at `n = (MAX_MESSAGE_WORDS - 1) / 2` writes its role word, fails on its payload word, returns `Full`, and leaves a role word without a payload word in the area; `Writer::finish` (`crates/abi/src/startup.rs:519-524`) then writes a count that excludes it, so the stray word is unread but the doc's promise is broken.

Fix: check `payload_index < MAX_MESSAGE_WORDS` before the first `set_word` in `append`; a `const _: () = assert!(MAX_MESSAGE_WORDS.is_multiple_of(2))` instead pins the constant rather than the function.

---

## F09 — issue #94
Title: audhsos-abi: document 02 names the word bound MAX_WORDS
Labels: enhancement, part::abi
Body:
`docs/02-architecture.md:384` bounds the word count with `0..=MAX_WORDS`. The constant is `MAX_MESSAGE_WORDS` at `crates/abi/src/layout.rs:127`, and no item named `MAX_WORDS` exists in the crate.

A reader searching the crate for `MAX_WORDS` finds nothing.

Fix: replace `MAX_WORDS` with `MAX_MESSAGE_WORDS` at `docs/02-architecture.md:384`.

---

## F10 — issue #96
Title: audhsos-abi: fault_kind_of carries a bound that cannot fail
Labels: enhancement, part::abi
Body:
`crates/abi/src/ipc_buffer.rs:113-115` returns `None` when `label - FAULT_LABEL_BASE` exceeds `0xFFFF_FFFF`. `FAULT_LABEL_BASE` is `0xFFFF_FFFF_FFFF_FF00` (`crates/abi/src/ipc_buffer.rs:87` and `crates/abi/src/ipc_buffer.rs:93`), so the difference is at most `0xFF` for every `u64` label.

The branch is dead code, and the `#[expect(clippy::cast_possible_truncation)]` at `crates/abi/src/ipc_buffer.rs:116-120` cites it as the bound that keeps the cast inside `u32`.

Fix: replace the check and the cast with `u8::try_from(code)` and `FaultKind::from_code(u32::from(code))`, which makes the bound `255` explicit and the truncation lint unnecessary.
