# kernel-mm and kernel-hal-api audit findings

Repository: AuDHSOS/AuDHSOS. Audit of kernel-mm, kernel-hal-api at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #59
Title: kernel-mm: unmap flushes the page before it clears the entries that reference the freed tables
Labels: bug, part::kernel
Body:
`Mapper::unmap` clears the leaf entry, flushes the page, and only then calls `collect_empty_tables` (`crates/kernel/mm/src/mapper.rs:229-238`). `collect_empty_tables` clears the parent entry of every empty table and releases the table frame to the frame source with no flush after the write (`crates/kernel/mm/src/mapper.rs:242-260`). The kernel adapter implements `flush_page` as `invlpg` (`docs/03-target-platform.md:337`). Intel SDM Vol. 3A, 5.10.4.2 (the section `docs/16-more-than-one-processor.md:489` names) requires the invalidation after the modification of an entry that references another paging structure, because the processor may cache such an entry at any time while it is present.

Sequence on the machine: a process unmaps the last mapped page of a 2 MiB region; line 234 clears the PTE, line 235 executes `invlpg`, the processor caches the still-present PDE during a speculative walk of a neighboring address, line 255 clears the PDE, line 256 releases the page-table frame, `BitmapFrameAllocator::allocate` hands the same frame out for the next table or kernel stack page (`crates/kernel/mm/src/frame_allocator.rs:181-186`). The cached PDE still names that frame, and a later access in the 2 MiB region walks the new content as a page table. `destroy_address_space` in `kernel-core` flushes everything after `free_user_half`; the per-page `unmap` path has no such flush (`crates/kernel/core/src/syscall.rs:209-211`).

Fix: move `self.tlb.flush_page(page)` in `unmap` to after `collect_empty_tables`, which keeps one flush per call and satisfies the order of 5.10.4.2; a `flush_all` after each released table was not taken because it costs the whole TLB for a common operation.

---

## F02 — issue #60
Title: kernel-mm: map takes the user bit from the caller while it derives the global bit from the page
Labels: enhancement, part::kernel
Body:
`Mapper::map` computes `global` from `page.is_user()` but writes `perms.user` into the leaf unchecked (`crates/kernel/mm/src/mapper.rs:204-221`); `protect` writes `perms` unchecked as well (`crates/kernel/mm/src/mapper.rs:291-301`). `X86Entry::table` sets `X86_USER` on every intermediate entry (`crates/kernel/mm/src/page_table.rs:609-611`), so the leaf alone decides whether user mode reaches a page. The system call layer builds `user: true` for every user mapping (`crates/kernel/syscall/src/calls/memory.rs:29-32`) and nothing checks that a kernel-half page keeps `user: false`.

A caller that passes `Permissions::for_user()` for a page in the kernel half maps kernel memory user-accessible; `translate` reports the mapping as valid (`crates/kernel/mm/src/mapper.rs:305-315`). The mapper is the one place every mapping passes through, and it already reads the half of the page.

Fix: in `map` and `protect`, replace `perms.user` with `page.is_user()` the way `global` is derived; returning an error on a mismatch was not taken because no caller has a use for a user page that user mode cannot reach.

---

## F03 — issue #61
Title: kernel-mm: share copies the whole top-level table through a 4 KiB stack array
Labels: enhancement, part::kernel
Body:
`share` builds `[F::EMPTY; ENTRIES]` with `ENTRIES = 512` (`crates/kernel/mm/src/kernel_half.rs:24`, `crates/kernel/mm/src/kernel_half.rs:69-77`), which is 4096 bytes for `X86Entry` (`crates/kernel/mm/src/page_table.rs:539`). `shares_kernel_half` builds the same array (`crates/kernel/mm/src/kernel_half.rs:127-130`). Only the entries from `FIRST_KERNEL_ENTRY` on are read back (`crates/kernel/mm/src/kernel_half.rs:82-87`).

`kernel-core` calls `share` at process creation on the kernel stack of the calling thread (`crates/kernel/core/src/syscall.rs:174`); a kernel stack is eight pages (`crates/abi/src/layout.rs:81`). D-70 records a double fault from a 20 KiB frame in `Mapper::walk` and per-level 4 KiB frames in `free_subtree` (`docs/09-decisions.md:80`); a build without optimization materializes the array more than once.

Fix: copy one entry per iteration through sequential borrows (`let entry = access.table(kernel)?.entry(index); access.table_mut(target)?.set_entry(index, entry);`), which needs no array; copying only the upper 256 entries was not taken because it still keeps 2 KiB on the stack.

---

## F04 — issue #62
Title: kernel-mm: allocate scans the bitmap one bit at a time
Labels: enhancement, part::kernel
Body:
`first_free` tests every bit from `from` upward with `is_set`, which divides and takes the remainder per call (`crates/kernel/mm/src/frame_allocator.rs:142-148`, `crates/kernel/mm/src/frame_allocator.rs:165-174`). `allocate` starts at bit zero every time (`crates/kernel/mm/src/frame_allocator.rs:181-186`). The cost is O(n) per allocation with n up to `MAX_MANAGED_FRAMES = 16384` (`crates/kernel/mm/src/frame_allocator.rs:26`).

Every page table the mapper creates (`crates/kernel/mm/src/mapper.rs:164`) and every kernel stack page (`crates/kernel/mm/src/stack.rs:522-525`) pays one such scan; with the low frames in use a thread creation with eight stack pages costs eight scans of up to 16384 bits each. A word scan that skips words equal to `u64::MAX` and takes `trailing_ones` of the first other word is O(n/64).

Fix: scan `bits` word by word, skip full words, and locate the bit with `trailing_ones`; a cached next-free hint was not taken because `free` would have to lower it and the word scan already removes the 64-fold cost.

---

## F05 — issue #63
Title: kernel-hal-api: add_bytes says the bytes of a lazy frame appear on the first access
Labels: bug, part::kernel
Body:
The doc comment of `MemoryFrameAccess::add_bytes` states that a frame of the lazy range needs no call because "its bytes appear on the first access" (`crates/kernel/hal-api/src/doubles.rs:96-99`). `frame_bytes` reads the map only (`crates/kernel/hal-api/src/doubles.rs:108-110`); only `frame_bytes_mut` materializes a frame of the lazy range (`crates/kernel/hal-api/src/doubles.rs:112-117`).

`MemoryFrameAccess::with_lazy_tables(ram)` followed by `frame_bytes(frame)` for a frame of `ram` returns `None`; the crate's own test asserts exactly that (`crates/kernel/hal-api/src/tests/doubles.rs:304-306`). The struct doc uses the correct wording, "first modifying access" (`crates/kernel/hal-api/src/doubles.rs:51-52`).

Fix: change the comment at lines 96-99 to "first modifying access"; materializing on read was not taken because it changes the assertion at line 304 and the kernel's reserve frames are written before they are read.

---

## F06 — issue #64
Title: kernel-hal-api: document 3 credits FrameBytes with a range-of-frames slice the trait has no method for
Labels: bug, part::kernel
Body:
`docs/03-target-platform.md:336` gives `FrameBytes` the responsibility "a physical frame as bytes, and a range of frames as a slice". The trait has two methods, `frame_bytes` and `frame_bytes_mut`, each over one frame (`crates/kernel/hal-api/src/paging.rs:38-48`).

The range-as-slice operation is the inherent `unsafe fn PhysicalWindow::bytes` of the adapter (`crates/kernel/hal-x86_64/src/window.rs:92`), which a caller with only a `dyn FrameBytes` cannot reach. A reader of document 3 who writes logic against the trait looks for a method that is not there.

Fix: remove the clause "and a range of frames as a slice" from the row at line 336; adding a range method to the trait was not taken because no logic crate needs one.

---

## F07 — issue #65
Title: kernel-mm: document 2 says the mapper reports which pages need flushing
Labels: bug, part::kernel
Body:
`docs/02-architecture.md:247` lists among the mapper's duties "reports which pages need flushing". `Mapper::map`, `unmap`, and `protect` call `TlbControl::flush_page` themselves (`crates/kernel/mm/src/mapper.rs:219`, `crates/kernel/mm/src/mapper.rs:235`, `crates/kernel/mm/src/mapper.rs:299`) and return `Result<(), MapError>` or `Result<PhysFrame, MapError>` with no page list (`crates/kernel/mm/src/mapper.rs:204-210`, `crates/kernel/mm/src/mapper.rs:229`).

A reader of the row expects a return value to act on and finds none; the module invariant states the actual contract, one flush per changed translation (`crates/kernel/mm/src/mapper.rs:6-8`).

Fix: replace "reports which pages need flushing" with "flushes each changed page through `TlbControl`".

---

## F08 — issue #66
Title: kernel-mm: map_range reports a frame-number overflow as UnreachableFrame
Labels: enhancement, part::kernel
Body:
`map_range` computes each frame with `first_frame.checked_add(done)` and maps `None` to `MapError::UnreachableFrame` (`crates/kernel/mm/src/mapper.rs:337-339`). `PhysFrame::checked_add` returns `None` when the number passes `MAX_FRAME_NUMBER` (`crates/kernel/types/src/phys.rs:183-190`). `UnreachableFrame` is documented as "a table frame is not reachable through the frame access" and prints that text (`crates/kernel/mm/src/mapper.rs:26-27`, `crates/kernel/mm/src/mapper.rs:38`).

The loader is the caller (`crates/boot/uefi-x86_64/src/loader.rs:319`, `crates/boot/uefi-x86_64/src/loader.rs:365`, `crates/boot/uefi-x86_64/src/loader.rs:381`); a range whose frames leave physical memory is reported as an unreachable table frame, which points a reader at the identity mapping instead of the range.

Fix: compute the last frame before the loop with `first_frame.checked_add(pages.count().saturating_sub(1))` and return a new `MapError::FrameOverflow` when it fails; reusing `Entry` was not taken because no entry was read.

---

## F09 — issue #67
Title: kernel-hal-api: CountingFrameSource accepts a release of any frame
Labels: enhancement, part::kernel
Body:
`release_frame` pushes the frame onto `released` without checking that it was handed out or that it is not already released (`crates/kernel/hal-api/src/doubles.rs:284-286`). `outstanding` is `allocated.len() - released.len()`, saturating (`crates/kernel/hal-api/src/doubles.rs:262-266`).

Two `allocate_frame` calls followed by two `release_frame` calls with the first frame leave `outstanding() == 0`: the double release of one frame hides the leak of the other. The mapper tests assert on `outstanding` (`crates/kernel/mm/src/tests/mapper.rs:238-247`), so a rollback that releases one frame twice and forgets another passes.

Fix: keep a set of live frames; `release_frame` removes from it and records a frame that is not live in a `stray` list that a new accessor exposes, so a test asserts both zero; panicking in the double was not taken because the workspace denies `panic` and the recorded list keeps the assertion in the test.

---

## F10 — issue #68
Title: kernel-hal-api: MemoryFrameAccess lets one frame hold a table and bytes at once
Labels: enhancement, part::kernel
Body:
`MemoryFrameAccess` keeps `tables` and `bytes` in two maps keyed by frame (`crates/kernel/hal-api/src/doubles.rs:28-32`). `frame_bytes_mut` and `table_mut` each consult only their own map (`crates/kernel/hal-api/src/doubles.rs:112-117`, `crates/kernel/hal-api/src/doubles.rs:125-130`), so `table_mut(f)` and `frame_bytes_mut(f)` both succeed for one `f` in the lazy range and hand out two unrelated contents.

On the machine both reach the same 4096 bytes through the window (`docs/03-target-platform.md:335-336`). A kernel path that writes an IPC buffer into a frame that is in use as a page table passes every host test and corrupts translations on the machine.

Fix: `frame_bytes_mut` returns `None` for a frame present in `tables` and `table_mut` returns `None` for one present in `bytes`, so such a path fails on the host with `UnreachableFrame`.

---

## F11 — issue #69
Title: kernel-mm: walk records at most three created tables while the format declares its level count
Labels: enhancement, part::kernel
Body:
`walk` keeps the created tables in `[Option<Created>; 3]` (`crates/kernel/mm/src/mapper.rs:144`) and drops a fourth silently, because `created.get_mut(created_len)` yields `None` (`crates/kernel/mm/src/mapper.rs:180-187`). The number of tables a walk creates is `F::LEVELS - 1` (`crates/kernel/mm/src/page_table.rs:407-408`, `crates/kernel/mm/src/mapper.rs:147-148`).

For a format with five levels, a walk that creates four tables and then fails leaves the fourth installed and its frame unreleased by `rollback` (`crates/kernel/mm/src/mapper.rs:134-139`), which breaks the invariant at `crates/kernel/mm/src/mapper.rs:7-8`. `X86Entry` has four levels, so no current format reaches it.

Fix: add an associated constant to the `Mapper` impl, `const LEVELS_FIT: () = assert!(F::LEVELS <= 4);`, and read it in `new`, so a format with more levels fails the build.

---

## F12 — issue #70
Title: kernel-mm: validate reports HugePage for bit 7 of a level-zero entry, where the bit is PAT
Labels: enhancement, part::kernel
Body:
`validate` rejects every present entry with `X86_HUGE` set (`crates/kernel/mm/src/page_table.rs:637-648`), and `X86_HUGE` is bit 7 (`crates/kernel/mm/src/page_table.rs:524-525`). Intel SDM Vol. 3A, 5.5.1, Table 5-20 defines bit 7 of a page-table entry that maps a 4 KiB page as PAT; the bit means a large page only in a page-directory or page-directory-pointer entry. `Mapper::read` validates entries of every level with the same function (`crates/kernel/mm/src/mapper.rs:117-122`).

A leaf entry with PAT set makes `unmap`, `protect`, and `translate` of that page fail with `Entry(HugePage)` (`crates/kernel/mm/src/mapper.rs:232`, `crates/kernel/mm/src/mapper.rs:294`, `crates/kernel/mm/src/mapper.rs:313`). No code of this repository sets PAT (`crates/kernel/mm/src/page_table.rs:613-621`), so the path has no trigger today.

Fix: give `validate` the level and check `X86_HUGE` only for `level > 0`; leaving it as is was not taken because the decoder is the one place a stale table is judged and its verdict is wrong for a valid entry.

---

## F13 — issue #71
Title: kernel-mm: EntryFormat::INDEX_BITS is read by no code
Labels: enhancement, part::kernel
Body:
The trait declares `INDEX_BITS` (`crates/kernel/mm/src/page_table.rs:409-410`) and `X86Entry` implements it (`crates/kernel/mm/src/page_table.rs:586`). `X86Entry::index` uses the module constant `X86_INDEX_BITS` instead (`crates/kernel/mm/src/page_table.rs:590-594`). The only read is a test asserting the value 9 (`crates/kernel/mm/src/tests/page_table.rs:223`).

An implementer of a new format sets a constant that changes nothing; the test asserts a literal against itself.

Fix: remove `INDEX_BITS` from the trait, the impl, and the test.

---

## F14 — issue #72
Title: kernel-mm: the entry count of a table is spelled in four places
Labels: enhancement, part::kernel
Body:
`EntryFormat::ENTRIES` (`crates/kernel/mm/src/page_table.rs:411-412`), `X86_ENTRIES` (`crates/kernel/mm/src/page_table.rs:579`), the literal 512 in `PageTable::entries` (`crates/kernel/mm/src/page_table.rs:452`), and `kernel_half::ENTRIES` (`crates/kernel/mm/src/kernel_half.rs:24`) all carry the same number. `free_subtree` iterates `F::ENTRIES` (`crates/kernel/mm/src/kernel_half.rs:207`) while `share` sizes its array with `kernel_half::ENTRIES` (`crates/kernel/mm/src/kernel_half.rs:69`) and `PageTable` fixes 512 for every format.

A format whose `ENTRIES` differs from 512 compiles and gets a 512-entry `PageTable`, so `free_subtree` and `PageTable::entry` disagree on the range.

Fix: add `pub const ENTRIES: usize = 512;` to `page_table`, use it for the array and in `kernel_half`, and drop `EntryFormat::ENTRIES` and `X86_ENTRIES`.
