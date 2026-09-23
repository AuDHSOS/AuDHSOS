# 6. Testing Strategy

The requirement is complete testing with complete edge cases. This document
defines the test levels, the harnesses, the edge-case catalog that every
component must cover before it is done, and the CI gates. All test tooling
is project code on top of the Rust toolchain.

## 6.1 Test levels

| Level | What is tested | Runs on | Tooling | When |
|-------|----------------|---------|---------|------|
| L1 Unit | functions and types of logic crates | host | `cargo test` | every push |
| L2 Property | invariants under generated inputs | host | `test-support` property engine | every push |
| L3 Model-based | stateful components against a simple reference model | host | `test-support` model-test runner | every push |
| L4 Fuzz | parsers and decoders | host | `-Zsanitizer=fuzzer` from the toolchain with `fuzz-support` | nightly schedule; regressions on every push |
| L5 Loader and kernel integration | loader, HAL adapter, and kernel core in QEMU | QEMU | custom test framework, serial protocol, exit device | every push |
| L6 End-to-end | the full system with userland test programs | QEMU | same runner | every push |
| L7 Miri | host-executable `unsafe` in adapter crates: the tests of the modules that hold it | host | `cargo xtask miri` | every push |
| L8 Static | lints, layering, external code, unsafe budget, documentation | host | `cargo xtask check` | every push |

Locally every one of these levels is started through the wrapper scripts of
[07 section 7.5](07-toolchain-and-environment.md#75-findings-about-the-development-machine),
`sh tools/xtask.sh <subcommand>` and `sh tools/xtask-check.sh`.

`test --host`, `coverage`, and `fuzz --regression` build their executables
first, then run them in a bounded worker pool. `AUDHSOS_TEST_JOBS` sets the
maximum number of concurrent processes; it defaults to the available CPU
count and must be a positive integer. For example:

```sh
AUDHSOS_TEST_JOBS=4 sh tools/xtask.sh test --host
AUDHSOS_TEST_JOBS=4 sh tools/xtask.sh coverage
AUDHSOS_TEST_JOBS=4 sh tools/xtask.sh fuzz --regression
```

The host harness thread count divides the available CPUs among the worker
slots, with at least one thread per process. An explicit `RUST_TEST_THREADS`
overrides that allocation. Each completed process sends its captured stdout
and stderr to one reporter, which prints a complete block in completion
order. The two streams retain their own order; their original interleaving
is not preserved. Quiet checks show output only for failed processes.
All queued processes finish even if one fails, and any failure fails the
step. The full check still runs its steps sequentially.

Host executables run from their package directories. Host doc tests run
through Cargo after the executable tests pass. Coverage merges profiles
only after every instrumented test executable passes. Fuzzing regressions
run selected targets concurrently, preserving sequential corpus replay
within each target and skipping targets whose corpus directory is absent.

## 6.2 Host testing of `no_std` crates

- Every logic crate uses `#![cfg_attr(not(test), no_std)]`. Tests use `std`
  freely.
- Test doubles live in `kernel-hal-api` behind the `test-doubles` feature:
  `MemoryFrameAccess` (a `HashMap<PhysFrame, Box<PageTable>>`, optionally
  with a frame range that materializes a default table on the first
  modifying access),
  `RecordingTlb`, `CountingFrameSource`, `FakeTimer`,
  `FakeInterruptController` (records mask, unmask, and end-of-interrupt
  calls), `ScriptedPlatform` (memory maps from builders), `RecordingPorts`
  (port I/O double for the UART logic).
- Generators for the types of a crate live in that crate behind the
  feature `test-strategies` (for example `kernel_types::strategies`), so
  that the tested crate and the generator see the same types.
- The `test-support` crate holds:
  - the property-test engine: a deterministic pseudo-random generator seeded
    per test, generator combinators (`integers in range`, `one of`,
    `vectors of`, `mapped`, `filtered`), shrinking toward minimal failing
    inputs, a fixed number of cases per property (default 512), and replay
    of a failing seed through `AUDHSOS_PROPTEST_SEED`;
  - builders (`MemoryMapBuilder`, `AddressSpaceFixture`,
    `HandleTableFixture`, `MessageBuilder`, `ElfBuilder`, `TarBuilder`,
    `UefiMemoryMapBuilder`);
  - strategies (`any_phys_addr`, `any_page_aligned_range`, `any_rights`,
    `any_handle`, `any_message`, `any_elf_header`);
  - the model-test runner: applies a generated operation sequence to the
    component under test and to a reference model and compares observable
    results after every step, shrinking the sequence on failure. A test
    may name the states its sequences have to arrive at (`required`) and
    have the model record what it arrived at (`reached`); a run that
    misses one fails although nothing disagreed. That is the one way a
    model test rots — the generator drifts, or the component grows a
    state the operations no longer reach — and reaching nothing looks
    exactly like reaching everything and finding no fault.

## 6.3 In-QEMU testing

- Kernel tests use the `custom_test_frameworks` feature. Functions marked
  `#[test_case]` are collected per test kernel. The harness runs them in
  order, prints the serial protocol from [3.1.7](03-target-platform.md#317-test-exit-protocol),
  and exits through the exit device.
- Every file under `crates/kernel/bin/tests/` is one test kernel. `cargo
  test` for the kernel target invokes `cargo xtask qemu-runner`, which
  builds a disk image around the test kernel, runs QEMU with a wall-clock
  timeout of 60 seconds, parses the serial output, and maps the exit status.
- The loader is tested by every test kernel boot and by dedicated loader
  test images: a kernel with a segment layout at the boundaries of the
  catalog, a corrupt kernel file (loader failure exit code expected), a
  missing boot image (loader failure exit code expected).
- Tests that must end in a kernel panic are one kernel each. The harness
  marks such a kernel `should_panic`; the panic handler then reports
  success.
- End-to-end test kernels carry a boot image with userland test programs.
  The programs report through the console driver in the same protocol.
- A measuring image is a test kernel like every other, with `[bench]` lines
  beside its test lines: `bench` reports what a system call round trip and
  an IPC round trip cost in ticks of the time-stamp counter (08 8.10). Its
  test cases assert that the measurement happened and say nothing about the
  figures, because a threshold on a number an emulator produces would fail
  for the load of the host and not for a change of this system.
- A hang is a failure. There are no retries; a test that fails once without
  a code change is a bug to fix.

## 6.4 Coverage

- Host coverage: `cargo xtask coverage` builds host tests with
  `-C instrument-coverage -Z coverage-options=branch`, merges the profiles
  with `llvm-profdata`, and exports LCOV with `llvm-cov`, both from the
  `llvm-tools-preview` component. Files under `src/tests/` are excluded, so
  the thresholds apply to product code only. Thresholds: 91 percent of
  lines and 86 percent of branches per gated crate. A crate that is built
  for a target rather than for the host is not gated, and neither are
  `xtask` and `docpdf`, which are reported only. CI fails below the
  thresholds. Uncovered lines must be justified in review. A crate named
  in `COMPLETE` is held to all of it instead — 100 percent of lines and of
  branches — which is what document 16, decision D4, asks of the SQLite
  port and what `db-sqlite` meets.
- Condition coverage: `cargo xtask check` runs the coverage step twice,
  once with `-Z coverage-options=branch` and once with
  `branch,condition`, which counts every operand of a compound decision
  and not only the decision, so a decision whose second operand no test
  settles fails the check. The same thresholds apply to both columns.
  Each mode builds into its own target directory, so running both
  rebuilds the workspace once each rather than twice each.
- MC/DC: `cargo xtask mcdc` reads the typed tree of every crate of
  `COMPLETE` through `-Zunpretty=thir-tree` and reports two things: a
  bitwise `&`, `|` or `^` over booleans, and a decision that names one
  condition twice. Over decisions that hold neither, condition coverage
  at 100 percent is unique-cause MC/DC, which document 16, decision D4,
  derives. A span the compiler wrote from a macro is passed over,
  because a derived `PartialEq` compares each field under one span. The
  pinned toolchain refuses `-Z coverage-options=mcdc`, which is why the
  objective is met by argument and check rather than by a report.
- QEMU coverage is not measured. Each adapter crate keeps a table that maps
  every public function to at least one QEMU test. `cargo xtask
  check-layering` verifies that every function and every test named in the
  table exists.

## 6.5 Test structure

- Name: `<subject>_<condition>_<expected>`.
- One behavior per test. Enumerations (state transition tables, error
  mappings, rights checks) are table-driven so that adding a variant fails
  to compile until the table is extended.
- Arrange, act, assert. Inputs are built with builders; tests contain no
  logic beyond that.
- Every bug fix adds a regression test whose name references the issue.

## 6.6 Edge-case catalog

The catalog is the checklist for the definition of done. A component is not
done until every applicable item has a test. Items are added, never removed.

### 6.6.1 Address types (`kernel-types`)

- Zero address; maximum address; address one below and one above every
  alignment boundary (4 KiB, 2 MiB, 1 GiB).
- Non-canonical virtual addresses (bit 47 not sign-extended) are rejected.
- The lowest and highest canonical user address; the lowest kernel address.
- `checked_add` at the top of the address space returns `None`.
- Page rounding up of the last page in the address space does not overflow.
- Ranges: empty range, single page, range whose end would overflow, range
  crossing the canonical hole, unaligned start or end.
- Frame and page conversions round-trip; frame number times page size
  round-trips for the highest representable frame.

### 6.6.2 Memory map normalization (`kernel-mm`)

- Empty map; a single region; a single region of one frame; a region of
  zero length.
- Unsorted input; duplicate regions; regions that touch; regions that overlap
  partially; a region entirely inside another.
- Usable region overlapping a reserved region at its start, at its end, in
  the middle (splits into two), and covering it entirely.
- Unaligned region start (rounds up) and unaligned region end (rounds down);
  a region that becomes empty after alignment.
- Regions above 4 GiB; a region whose end would overflow `u64`.
- Kernel image, boot image, page tables, and boot stack overlapping usable
  regions at every position.
- Region count at the capacity limit and one above (error, not truncation).
- Total usable memory smaller than the kernel reserve: an error.

### 6.6.3 Frame allocator (`kernel-mm`)

- Allocate until exhaustion returns `OutOfFrames` and never panics.
- Free then allocate returns a frame (reuse).
- Double free is detected and reported; freeing a frame that was never
  allocated is detected.
- Contiguous allocation of `n` frames: `n = 0` is an error; `n = 1` equals
  single allocation; `n` larger than any region fails; `n` spanning a region
  boundary fails even if the total is sufficient; alignment larger than the
  region fails.
- Bitmap word boundaries: regions of 63, 64, 65, 127, 128, 129 frames;
  allocation straddling a word boundary.
- Property: the set of allocated frames is always disjoint and inside the
  managed regions; freed count plus allocated count equals capacity.

### 6.6.4 Page-table entries and walker (`kernel-mm`)

- Flag round-trips for every flag and every combination of present, writable,
  user, no-execute, and the cache bits.
- Physical address masking: bits 12 to 51 only; an unaligned frame address
  is rejected; the highest frame address round-trips.
- Reserved bits set in an entry read from memory are reported, not ignored.
- Mapping the same page twice returns `AlreadyMapped` and changes nothing.
- Unmapping an unmapped page returns `NotMapped`.
- Mapping the first page requires creating tables on every level; the frame
  source is called exactly three times; mapping a neighbor calls it zero
  times.
- Unmapping the last page of a table frees the table; unmapping a page whose
  table still has other entries does not.
- Mapping at the highest user page and at the lowest kernel page; mapping
  across the canonical hole is rejected.
- Permission combinations: read-only, read/write, execute, no-execute, user
  and supervisor; the walker never produces writable-and-executable unless
  explicitly requested for the root task.
- `protect` on a mapped page changes only the flags and emits exactly one
  flush; `protect` on an unmapped page is an error.
- Every operation that changes an entry emits exactly one flush for that
  page; operations that fail emit none.
- Bounded operations: a request over more pages than the per-call limit
  returns `Partial` with the exact count processed and can be resumed to
  completion.
- Frame source exhaustion in the middle of creating tables leaves no
  half-created table behind and reports `OutOfKernelMemory`.
- The loader's use: identity mapping of a range that overlaps the physical
  window mapping produces two entries for the same frame; the kernel
  segment mapping with mixed permissions per segment.
- Model-based: random map, unmap, and protect sequences against a
  `HashMap<Page, (PhysFrame, Flags)>`; after every step, `translate` agrees
  with the model for every page ever touched.

### 6.6.5 Address space regions (`kernel-mm`)

- Map with a fixed address that overlaps an existing region at the start, the
  end, inside, or covering it: `AddressInUse`.
- Map two regions that touch: allowed, remain separate.
- Zero-length region, unaligned start, unaligned length, `start + len`
  overflow, region in the forbidden low 64 KiB, region in the kernel half:
  each is its own error.
- Unmap exactly one region; unmap the middle of a region (split); unmap
  spanning two regions; unmap a range with no region.
- `protect` a sub-range: splits into up to three regions.
- Region quota: exactly at capacity succeeds; one more fails; a split that
  would exceed the quota fails before any change.
- Exhausting the address space with maximal regions.
- Property: regions are sorted, disjoint, aligned, and inside the user half
  after every operation.

- A mapping that continues one already in the table grows it instead of
  adding a region (D-104): same object, same permissions, the offset
  running on, and beginning where the other ends. A range that differs in
  any of the four is a region of its own, a gap between them included.
- A removal says what became of the region it took from: one that only
  shortens a region reports that the region stayed, one that takes the
  middle out reports that the region was divided, and one that takes the
  whole region reports that it is gone. A protection says how many regions
  the table gained by the split it made — none, one, or two. The system
  call layer holds one reference to the backing object per region: it gives
  one back for a region that is gone and takes one for every region a split
  added, so an object is held by every region that names it and by no more.
- `memory_unmap` unmaps the pages of the pieces the region table gave up
  and no others: a range that reaches over a gap between two mappings
  unmaps what is mapped and leaves the gap alone, and a range over more
  mappings than one call takes comes back as `Partial` at the mapping it
  did not reach.

### 6.6.6 Object pools, ids, handles, and rights (`kernel-objects`, `audhsos-abi`)

- Allocating from a full pool returns `PoolExhausted`.
- An id with a stale generation is rejected after the slot was freed and
  reused.
- Generation wrap-around: after the maximum number of reuses the generation
  wraps and the slot still works.
- Reference count: last reference destroys; destroying twice is impossible
  by construction (tested through the API).
- Handle `0` is invalid. Handles with an out-of-range index, with a stale
  generation, and with a generation of zero are invalid.
- Handle encoding round-trips at the boundaries: index 0 with generation 1,
  the highest index, the highest generation; index and generation never
  overlap in the 64-bit value.
- Duplicate with a superset of rights fails; with an equal set succeeds;
  without `DUPLICATE` fails; the original is unchanged in every case.
- Close twice: the second close fails with `InvalidHandle`.
- Handle table full: the operation fails and no object reference leaks.
- FIFO slot reuse: after closing slots `a` then `b`, allocation returns `a`
  first.
- Wrong object type for an operation: `WrongObjectType`, checked before
  rights.
- Kernel-object quota: creating the object that exceeds the quota fails;
  quota accounting returns to the previous value after destruction.
- `Rights`: empty set, full set, subset and superset checks for every pair
  of single rights, union and intersection, unknown bits rejected on decode,
  every named right has a unique bit (table-driven).
- A pool and the handle arena built by the `const` constructor are all
  zeros, byte for byte, and behave like the ones built at run time: the
  first `allocate` hands out generation 1, slots come from the high-water
  mark in index order while none has been released, and released slots
  keep the FIFO order (D-66).

### 6.6.7 Scheduler and thread states (`kernel-sched`)

- Empty run queues pick the idle thread.
- A single ready thread is picked repeatedly.
- Equal priorities alternate round-robin in FIFO order.
- A higher-priority thread becoming ready preempts the running thread
  immediately; a lower-priority one does not.
- A blocked thread is never picked.
- Priority change of a ready thread moves it between queues; of the running
  thread triggers a reschedule when a higher thread exists.
- Time slice expiry exactly at the boundary tick; a thread that blocks before
  expiry keeps no credit.
- Tick counter wrap-around does not disturb time-slice accounting.
- A thread that exits while ready, while running, and while blocked is
  removed from every queue.
- Waking an already-ready thread is idempotent.
- Priority above the thread's maximum is rejected.
- Every illegal state transition in the transition table returns an error
  (table-driven test over all state pairs).
- The page-table root is loaded on a switch between threads of two
  processes and not on a switch between threads of one, observed in
  `kernel-core` through the recording `AddressSpaceControl` double (D-65).

### 6.6.8 IPC (`kernel-ipc`)

- `call` with no receiver blocks the caller; a later `recv` completes the
  rendezvous; the receiver sees the badge and gets a reply object.
- `recv` with no sender blocks; a later `call` completes.
- `reply` on a reply object that was answered fails: the answer consumed
  the object and the handle that named it, so the second attempt reaches
  nothing (D-93). A run of a thousand calls leaves a server holding no more
  handles than it started with.
- `reply` on a reply object whose caller was killed fails without touching
  memory; dropping a reply object wakes the caller with `ReplyDropped`.
- Messages with zero words, the maximum number of words, and one more than
  the maximum (rejected before any copy).
- Zero handles, four handles, five handles (rejected).
- Transferring a handle without `TRANSFER` fails and no other handle of the
  same message is installed.
- Receiver's handle table has room for fewer handles than sent: the message
  is delivered, the handle count is truncated, and the receiver's result
  carries the error flag.
- Badge propagation through duplicate: a badged capability keeps its badge; a
  re-badge attempt fails.
- Sender killed while blocked: removed from the queue; a later `recv` does
  not see it.
- Receiver killed while a sender is blocked: the sender stays blocked until
  another receiver arrives or the endpoint is destroyed.
- Endpoint destroyed with waiters on both queues: everyone wakes with
  `ObjectDestroyed`.
- Multiple senders: served by priority, then FIFO.
- `try_recv` with nobody waiting returns `WouldBlock` and leaves queues
  unchanged.
- `reply_recv` delivers the reply before blocking in `recv`.
- Notifications: signal without a waiter accumulates; signal zero is a
  success no-op; wait consumes and clears everything present; poll on zero
  returns zero without blocking; two signals before a wait are merged; a
  second concurrent waiter gets `Busy`; an interrupt bound to it sets
  exactly its bit, and two interrupts bound to one notification each set the
  bit they were bound on (D-108).
- Fault message: has the reserved label range, carries fault kind, address,
  instruction pointer, and error code; the reply resumes the thread; the
  handler killing the process ends the wait cleanly.
- A message whose label lies in the range the kernel reserves for its own
  messages is refused before anything is copied.
- A thread suspended while it waits leaves the queue it waited in, finds
  `Cancelled` in its status word, and is in no queue when it resumes.
- The last handle to an endpoint closes while threads wait on both queues:
  the endpoint is destroyed and everyone wakes with `ObjectDestroyed`
  (D-75).

### 6.6.9 System call decoding and dispatch (`kernel-syscall`)

- Unknown system call number; number above the table; every known number is
  in the table (table-driven).
- Argument count mismatch is rejected before any handle lookup.
- Validation order is observable: invalid handle beats wrong type beats
  missing right beats invalid argument beats quota (tested pairwise).
- IPC buffer with the result area at the last bytes of the page: no
  out-of-page access.
- Every error variant of every system call in its documentation table has a
  test that produces it, for the calls the phase implements; a call it does
  not implement yet is refused before it does anything, with `Unsupported`
  or, where its first argument names an object type of a later phase, with
  `WrongObjectType`.
- Round-trip encode/decode of every request and result layout, including
  maximum values of every field.
- A result that does not fit into two return words is written as message
  words of the caller's buffer with label zero and handle count zero, and
  the first return word says how many: `thread_info` reports the kind,
  address, instruction pointer, and error code of a thread that faulted and
  a word count of zero for one that did not, and `system_info` reports the
  capacity and the live count of every pool, the tick rate, the root system
  description pointer, and the six words of the framebuffer, which are zero
  throughout on a machine that has none.
- `memory_create_device` refuses a range that meets memory the machine
  reported as usable, one that lies in no aperture it reported as device
  memory — past one, beginning before one, or running past the end of one —
  and every range at all on a machine that reported no aperture.

- Watching the end of a process: the end signals the bit it was watched
  on; the last thread of a process taking the process with it signals, and
  a thread that leaves a process with threads left does not; a process that
  has already ended signals at once; an end is told once, however often the
  kernel walks past it afterwards; every watcher of a process hears of it
  and a fifth is refused; the same watch twice is refused; a watch needs
  `INFO` on the process and `BIND` on the notification; a bit above
  sixty-three and a handle that names nothing are refused; a notification
  destroyed before the end signals nothing and is no error; a thread
  waiting on the notification wakes with the bit.

### 6.6.10 Boot image header and boot information (`audhsos-abi`, `kernel-core`)

- Boot image: wrong magic; wrong version; header length shorter than the
  minimum, longer than the image; root task offset unaligned, before the
  header end, beyond the image; length zero, `offset + length` overflow,
  `offset + length` beyond the image; archive overlapping the root task at
  either end; archive of length zero (allowed); non-zero flags; reserve size
  unaligned, larger than RAM; image exactly the header size.
- Boot information: wrong magic; wrong version; `size` smaller than the
  fixed part, not matching `region_count`, larger than one page;
  `region_count` zero, at `MAX_BOOT_REGIONS`, one above; unknown region
  kind; region with zero length; kernel, boot image, page table, and boot
  stack ranges overlapping each other or lying outside every region; ACPI
  pointer zero; ACPI pointer above RAM.

### 6.6.11 ACPI parsers (`kernel-acpi`)

- Missing RSDP; RSDP with a bad checksum; revision 0 versus 2 (RSDT versus
  XSDT); an announced length that does not reach the extended checksum or
  leaves the structure; table length shorter than the header; table length
  beyond the bytes; a table of the wrong signature; entry length zero (must
  not loop forever); entry length beyond the table; an entry of a known type
  with the wrong length; unknown entry types skipped; local APIC address
  override; zero I/O APICs; more I/O APICs than the fixed capacity (error,
  not truncation); more overrides than the fixed capacity; interrupt source
  overrides for IRQ 0 and IRQ 4; flags that say `conforms` and flags the
  specification reserves; a line without an override; an address that does
  not fit the physical address width. `Missing RSDP` is tested where the
  absence is decided, which is the boot information parser of 6.6.10
  (`ACPI pointer zero`): this crate is handed bytes and never sees the
  absence of a pointer.
- Fuzz target over the raw bytes.

### 6.6.12 Userland allocator (`user-rt`)

- Size zero (returns a valid unique offset or an error, decided in the
  design and tested); size larger than the arena; alignment 1, 8, 4096;
  non-power-of-two alignment rejected; alignment larger than the arena.
- Allocate `a`, `b`, `c`; free `b`; free `a`; the two merge; a following
  allocation of `a + b` fits.
- Exhaustion returns an error, never panics; after freeing everything the
  arena is one free block again.
- `size + align - 1` overflow in `u64` is rejected.
- Property: live allocations never overlap, every returned offset is aligned
  and inside the arena, freeing in any order restores full capacity.

### 6.6.13 tar reader and ELF parser (`user-loader`, `audhsos-elf`)

- tar: empty archive; one file; truncated header; bad checksum; non-ustar
  magic; long names using the prefix field; zero-size file; size not a
  multiple of 512; two trailing zero blocks; trailing garbage after the end
  marker; directory entries; a name containing `..` or an absolute path
  (rejected); a name with embedded NUL.
- ELF: bad magic; 32-bit class; big-endian; wrong machine; wrong type (not
  executable); program header table beyond the file; program header count of
  zero; `memsz < filesz`; segment file range beyond the file; segments that
  overlap in memory; unaligned segment with `p_align` not a power of two;
  segment overlapping the forbidden low range (userland) or outside the
  kernel half (loader); entry point outside every executable segment;
  writable-and-executable segment (rejected); sizes and offsets that overflow
  when added; program header entry size smaller than the structure; more
  load segments than the fixed capacity.
- Fuzz targets for both parsers.

### 6.6.14 UEFI structures and loader (`audhsos-uefi`, `boot-uefi-x86_64`)

- Structure layouts: size and offset of every field of every defined
  structure match the UEFI specification values (table-driven).
- Memory map conversion: descriptor size larger than the structure (stride
  honored); zero descriptors; descriptors out of order; descriptors that
  overlap; every UEFI memory type maps to exactly one region kind
  (table-driven); loader code and data become usable; conventional memory
  below 1 MiB kept; a descriptor with zero pages; a descriptor whose end
  overflows; more descriptors than `MAX_BOOT_REGIONS` after merging (error,
  loader failure exit).
- File loading: file larger than the buffer, file of zero length, file
  missing, read returning fewer bytes than requested.
- Kernel placement: segment at the lowest and at the highest kernel-half
  address; segments with different permissions; `.bss` larger than
  `filesz` zero-filled; entry point in a non-executable segment rejected.
- Page-table construction: physical window covering the highest usable
  frame; identity mapping of the loader's own range; boot stack guard page
  unmapped; boot information page mapped read-only.
- Boot information: `region_count` and `size` consistent; regions sorted;
  every fixed field inside a region.
- In QEMU: a corrupt kernel file and a missing boot image both produce the
  loader failure exit status and a diagnostic line.

### 6.6.15 Disk image writer (`xtask`)

- GPT: primary header at sector 1 and backup header at the last sector
  reference each other; header CRC32 and partition array CRC32 verify after
  writing; partition array has 128 entries of 128 bytes with exactly one in
  use; the entry carries the EFI system partition type GUID and the fixed
  unique GUID; first and last usable sector enclose the partition; a disk
  too small for the GPT structures is rejected. The structures themselves
  are `fs-gpt`'s and 6.6.72 tests them; what is tested here is the image
  the xtask makes of them.
- FAT32: cluster count below 65525 is rejected; boot sector fields (bytes
  per sector, sectors per cluster, reserved sectors, number of FATs, FAT
  size, root cluster, FSInfo sector, backup boot sector); FSInfo free count
  and next-free hint; the backup boot sector equals the boot sector; two
  FAT copies identical; media byte and end-of-chain marker in the first
  two FAT entries; end-of-chain marker `0x0FFFFFFF` at every chain end.
- Files: empty file; file of exactly one cluster; file spanning several
  clusters; file that fills the last cluster exactly; a directory whose
  entries span more than one cluster; 8.3 name conversion (lowercase,
  padding, invalid characters rejected, name longer than 8 or extension
  longer than 3 rejected); nested directory `EFI/BOOT` with `.` and `..`
  entries.
- Image size: default 64 MiB; files that need more grow the image in 1 MiB
  steps; files that exceed the partition are rejected.
- The written image is read back by a project-defined GPT and FAT32 reader
  in the xtask tests and every file compares equal.

### 6.6.16 Descriptor tables and register blocks (`kernel-hal-x86_64`, pure modules)

- GDT: null descriptor; kernel code and data; user code and data with
  privilege level 3; TSS descriptor as a 16-byte system descriptor with
  the base split across the fields; selector values with the requested
  privilege level bits.
- IDT: entry with the handler address split into three fields; interrupt
  stack table index 0 and 1; descriptor privilege level 0 and 3 (vector
  `0x80`); present bit; type field for interrupt gates.
- TSS: `RSP0` offset; IST offsets; I/O map base equals the structure size.
- Local APIC and I/O APIC register offsets (table-driven against the
  specification values); I/O APIC redirection entry encoding for masked,
  unmasked, level, edge, vector, destination.
- The frame `prepare_user` writes into a kernel stack: the six callee-saved
  words, the trampoline address, and `rip`, `cs`, `rflags`, `rsp`, `ss` at
  the offsets `switch` and `iretq` read them from; the returned stack
  pointer names the first of those words; a stack too small for the frame
  is refused (D-67).

### 6.6.17 UART register logic (`driver-uart16550`)

- Initialization sequence writes the divisor, line control, FIFO control,
  and modem control registers in order (recorded by the port double).
- Write waits for the transmitter-empty bit; a port double that never sets
  the bit makes the write return `Timeout` after the bounded number of
  polls.
- Read with no data returns `WouldBlock`; read with data returns the byte
  and clears nothing else.
- Interrupt enable and identification register handling for receive and
  transmit interrupts.
- The same tests run against the kernel adapter double and the userland
  system call double.

### 6.6.18 Global cell (`audhsos-sync`)

- First borrow succeeds; a second borrow while the first is alive returns
  `AlreadyBorrowed`; after release a new borrow succeeds.
- Initialization exactly once; a second initialization returns
  `AlreadyInitialized`; borrow before initialization returns
  `Uninitialized`.
- The `const`-initialized cell: the value is reachable without an
  initialization step, borrowing follows the same rules as `Global`, and a
  cell holding a type whose `const` value is all zeros produces a `static`
  in `.bss` (D-66).
- Miri: the tests above run under Miri with the cell holding a type with a
  destructor, and with the `const`-initialized cell.

### 6.6.19 Property-test engine and model-test runner (`test-support`)

- Seeded generation is deterministic: the same seed yields the same
  sequence.
- Integer generators respect inclusive bounds, including single-value
  ranges and the full `u64` range.
- Vector generators respect the length bounds; length zero.
- Shrinking terminates; the shrunk input still fails the property; the
  shrunk integer is the smallest failing value for a monotone property.
- `AUDHSOS_PROPTEST_SEED` replays a failure and reports the same input.
- The model-test runner reports the shortest failing operation sequence for
  a component with an injected bug.

### 6.6.20 xtask tools (`xtask`)

- Serial protocol parser: well-formed lines; a `FAILED` line with an empty
  message; a summary whose counts disagree with the parsed lines (reported
  as a runner error); output without a summary (crash); interleaved
  non-protocol output.
- Unsafe counter: `unsafe` inside comments and strings is not counted;
  `unsafe fn`, `unsafe impl`, `unsafe {` and `asm!` are counted; a budget
  exactly met passes, one above fails.
- Miri coverage: a module holding `unsafe` that a filter of `MIRI_TARGETS`
  names is no gap; one that no filter names is reported, and the report
  names both the file and the filter that would close it; a crate that runs
  whole has no gaps (D-76).
- `cargo tree` parser: nested depth prefixes; a crate appearing twice;
  workspace members versus the toolchain's own crates.
- `check-deps`: a lock file with a non-workspace package fails; a manifest
  with a `git`, `version`, or registry dependency fails; a path dependency
  outside the workspace fails.
- SPDX check: missing header; header on the second line; wrong license
  identifier.
- Runner: QEMU exit status mapping for 33, 35, 37, 0, 1, and a killed
  process; timeout produces a crash report with the captured output.
- Command line: a subcommand that takes no option refuses one and names
  it; `check` refuses an unknown option before it runs a step; `run`
  refuses one before it builds anything.
- The second disk: a run that asks for it carries the two
  `virtio-blk-pci` lines at the end of the machine line and the boot
  volume unchanged beside them, a run that does not carries neither; the
  disk is created blank once at the size the format wants, kept as it
  stands on every run after that, and its path is the run's name under
  `target/qemu/` (D-136).
- Quiet mode: a command that succeeds under `--quiet` prints nothing and
  is still an `Ok`, a command that fails is still an error.

### 6.6.21 Kernel integration tests in QEMU

- Boot: reaches the harness, prints the protocol, exits with success.
- Debug UART: writes a known string that the runner finds.
- Exceptions: breakpoint returns to the next instruction; page fault at a
  known address reports that address; divide error; invalid opcode; double
  fault on kernel stack overflow lands on the IST stack and reports vector
  eight; general protection on a selector beyond the descriptor table, and
  from Phase 5 on a privileged instruction in user mode.
- Panic: a panic in a test image reaches the panic handler, which names the
  running test (`should_panic` kernel).
- Interrupts: timer ticks increase a counter; the local APIC end-of-interrupt
  path lets a second tick arrive; masking a line stops its delivery; spurious
  vector is handled.
- Memory: allocate every reserve frame and free them; map a frame at a user
  address, write through the physical window, read through the mapping, unmap,
  and verify a page fault; the loader's identity mapping is gone after boot;
  the boot information page is reported with the physical address a walk of
  the loader's tables gives.
- Address spaces: a root created after boot carries the kernel half, so the
  window, the boot information page, and the kernel image are reachable
  through it; a kernel stack allocated after that root exists is reachable
  through it as well; the kernel occupies exactly the page-map level four
  entries 256 and 511 and gains no further one after boot (D-65).
- Kernel stacks: every page of an allocated stack carries what the kernel
  writes into it; a released stack is unmapped and its frames are back in
  the reserve; the slot is handed out again with the same pages; a write to
  the guard page below a stack raises a page fault at the guard address.
- Threads: create a user thread that executes `thread_exit`; two threads of
  equal priority alternate (observed through a shared counter); a
  higher-priority thread preempts.
- Isolation: a user thread that reads a kernel address faults, stops in
  `Faulted`, and leaves the rest of the system running; a user thread that
  executes `hlt` faults the same way.
- Isolation, from Phase 6: the fault handler endpoint of the process
  receives the message for both faults, and a reply resumes the thread.
- System calls: every system call the phase implements has at least one
  success and one failure test issued from user mode; every call it does
  not implement yet is refused, with `Unsupported` where the call is
  reachable and `WrongObjectType` where its first argument names an object
  type of a later phase, and that is tested for each of them.
- IPC: call and reply between two user threads; handle transfer; notification
  from a timer-bound interrupt to a user thread.

### 6.6.22 End-to-end tests in QEMU

- The root task starts, parses the archive, and starts the name server, the
  console driver, and the memory server.
- `app-hello` looks up the console by name, writes a line, and the line
  appears on the serial port through the userland driver, which owns COM1
  from the moment it created the `IoPortRange` over it.
- Name lookup of a missing name returns `NotFound`.
- Two clients write interleaved lines; no line is torn: every line the
  second client writes carries its own number and has to stand whole and
  exactly once in the output, so a line that lost bytes to the other writer
  is a violation and not a line the run happened not to look at.
- A client that faults is reported by the root task and the system keeps
  running.
- Console input: the runner sends bytes over the serial port and a test
  program echoes them.
- The root task ends the machine when every child that reports has reported,
  and the run insists on that rather than killing it (D-94).
- Memory server: allocate, release, allocate again returns zeroed memory.
- Out of memory: exhausting the memory server produces an error in the
  client, not a system failure.

### 6.6.23 Memory server logic (`server-memory`, host-tested with a recording double for map, zero, and unmap)

- `allocate` issues exactly one zeroing pass over the whole object before
  the handle is handed out; the recorded zero range equals the object
  range.
- `release` issues exactly one zeroing pass immediately, before the object
  is marked free.
- Memory received from the root task at start is zeroed before the first
  `allocate`.
- Double `release` of the same object is rejected; `release` of an unknown
  object is rejected; the second call issues no zeroing.
- Memory of a client reported dead by the root task is treated as returned:
  zeroed once and marked free.
- Alignment: requests of 4 KiB, 2 MiB, and an alignment larger than any free
  object; length zero rejected; length not page-aligned rounded up.
- Exhaustion returns `OutOfMemory`; after releases the same request
  succeeds again.
- Adjacency bookkeeping: two released neighbors are handed out as one
  object for a request of their combined size. This is what `memory_merge`
  was added for (D-90); against a kernel that only splits it cannot be
  satisfied at all. A join the kernel refuses leaves the two objects where
  they are, and no memory is lost by it (D-95).
- An object that comes back under another handle than the one it went out
  under is recognized by the memory it covers, and the second name is given
  up; one returned at a length the store never handed out is refused
  (D-95).
- Property: every handed-out range is disjoint from every other live range
  and from the free set; every range handed out was zeroed after its last
  release.

### 6.6.24 Graphics Output Protocol and framebuffer boot information (`audhsos-uefi`, `audhsos-abi`, `boot-uefi-x86_64`)

- Layout tests for the protocol, mode, and mode information structures.
- Pixel format conversion: `RedGreenBlueReserved8BitPerColor` becomes
  `Rgbx8888`, `BlueGreenRedReserved8BitPerColor` becomes `Bgrx8888`,
  `BitMask` and `BltOnly` report an absent framebuffer; a zero base or a
  zero resolution reports an absent framebuffer.
- Boot information: an absent framebuffer has every framebuffer field
  zero; a non-zero framebuffer field with format `0` is rejected; a base
  that is not frame-aligned is rejected; a length shorter than
  `height * stride * 4` is rejected; a length that is not a multiple of
  the frame size is rejected; `stride < width` is rejected; zero width or
  height with a present framebuffer is rejected; a framebuffer overlapping
  a `Usable` region is rejected; a framebuffer without an enclosing
  `MmioReserved` region is rejected; an unknown format code is rejected;
  the writer produces what the parser accepts, with and without a
  framebuffer (property).
- Startup message: a role carries either a handle or a value and says
  which; a value role is read as a number and not as a handle, so a word
  that is no handle is no error under one; a handle written under a value
  role and a value written under a handle role are refused and write
  nothing; the width, height, stride, and format of a mode survive the two
  words that carry them, the widest one included; a format code that names
  no format is no mode; half a description is no mode; a value role that
  appears twice is refused like a handle role that does.
- Loader in QEMU: booting with `-vga none` reports an absent framebuffer
  and the kernel reaches the harness.

### 6.6.25 i8042 controller and PS/2 decoding (`driver-i8042`)

- Regression: corrupt every position of the Pause tail; reject bogus Pause
  events and reconsider a mismatching byte as the start of the next key.

- Controller: a failed self-test returns an error; an output buffer that
  never fills or an input buffer that never empties hits the poll limit
  and returns an error instead of spinning; a missing keyboard or a
  missing mouse is reported and the other device still works; translation
  is off in the configuration byte; both interrupts are enabled only after
  both devices are initialized; the output buffer is flushed before the
  self-test.
- Demultiplexing: status bit 5 routes a byte to the mouse decoder,
  otherwise to the keyboard decoder; a read with the output buffer empty
  returns nothing and feeds no decoder.
- Keyboard: make and break codes of set 2; the `E0` prefix; the `F0`
  release marker; the `E1` pause sequence; an unknown code is dropped
  without losing the decoder state; a prefix at the end of the stream
  waits for the next byte; the ACK and resend bytes of commands are
  consumed by the command state, not by the decoder.
- Mouse: the sync bit (bit 3 of the first byte) is checked; an
  out-of-sync byte is dropped until a valid first byte arrives; 3-byte and
  4-byte packets; sign extension of the deltas; the overflow bits clamp
  the deltas; the wheel byte of the 4-byte packet; the IntelliMouse
  detection sequence is issued and the reported id selects the packet
  length.
- Property: any byte stream produces events without panicking and with
  bounded decoder state. Fuzz targets `scancode` and `mouse_packet`.

### 6.6.26 Framebuffer logic (`gfx`)

- Rectangle fill and blit: fully inside, partially outside, fully outside,
  zero width or height; a `stride` larger than `width` leaves the padding
  untouched; the last row and the last column are written.
- Pixel formats: `Rgbx8888` and `Bgrx8888` byte order; a color decodes
  back to the same value; a surface whose byte slice is too short for
  `height * stride * 4` is rejected.
- Damage rectangles: union of two rectangles, merge of overlapping ones,
  empty rectangles are dropped, the set never exceeds its capacity and
  collapses to one bounding rectangle when full.
- Present: exactly the damaged pixels are copied, checked with a
  recording target.
- Font: exactly 95 glyphs for the printable ASCII range; every glyph
  matches its checksum; a character outside the range renders the
  replacement glyph; a string longer than the row is clipped at the
  surface edge.
- Property: for any rectangle no byte outside the surface is written.

### 6.6.27 Display and input servers (`server-display`, `server-input`, host-tested logic with doubles)

- Regression: concurrent ring reader/writer preserve whole records and FIFO
  order; overflow exchange accounts for concurrent increments. Invalid shared
  capacity is rejected. Left/right modifiers remain independent, caps-lock
  repeat does not toggle, and the German AltGr level produces its characters.
- Regression: received handle snapshots survive nested IPC; invalid labels,
  counts, extra handles and failed subscriptions close all unadopted handles.
- Lifecycle: retained notifications still signal after a client exits; its
  process watch reports that exit independently. Unwatch frees watcher capacity
  and delayed bits cannot remove a new live subscriber in the reused slot.
- Memory: returned pages with foreign handles or mappings are retired, not
  zeroed or reallocated; only exclusive ownership permits reclamation.

- Event ring: a full ring drops the newest event and sets the overflow
  flag; the reader clears the flag; sequence numbers are contiguous
  otherwise; a subscriber whose notification cannot be signalled is
  removed.
- Input: the modifier state follows press and release; a release without
  a press is delivered as a release; pointer button state is tracked
  across packets; a wheel delta is delivered as its own event; both
  interrupts are acknowledged after the output buffer is drained, whichever
  of the two woke the thread, and a drain that finds nothing acknowledges
  them all the same; the `AUX` bit routes a byte to the mouse decoder and
  every other byte to the keyboard decoder; a request without a badge gets
  no ring; one badge holds one subscription and a slot that was let go of
  is given out again; a subscriber that cannot be woken is dropped and its
  slot freed, while a ring that is full or missing costs the event only.
- Display: a client that carries no badge — which is what a capability
  found under a name looks like — gets no surface, because a server that
  keeps one per client cannot tell two of nobody apart; `present` with
  damage rectangles copies exactly those pixels
  from the surface to the framebuffer (recording double); the cursor
  sprite saves and restores the background; the cursor is clamped to the
  screen; a surface larger than the screen is rejected; a client
  presenting a surface it does not own is rejected by badge; a client
  that goes away releases its surface; the sprite is the shape the client
  asked for, the resize shape is the same under a turn of half a circle,
  and each shape has a white body inside a black edge.

### 6.6.28 QMP client and screendump reader (`xtask`)

- The greeting is parsed and `qmp_capabilities` is negotiated before the
  first command; an error response becomes an error value, not a panic;
  asynchronous event lines interleaved with responses are skipped.
- JSON subset: objects, arrays, strings with escapes, integers, booleans,
  `null`; nesting depth is bounded; malformed input is an error; the
  writer output parses back to the same value (property).
- `input-send-event` for a key press and release by `qcode`, for relative
  pointer motion, and for a button press and release (Phase 10).
- `screendump`: the PPM file is parsed (`P6`, comments, `maxval` 255); a
  truncated file is an error and not a black pixel; a pixel is read at
  given coordinates and a color is counted over a rectangle; coordinates
  outside the image, and a rectangle that reaches past it, are an error.
- A socket that never answers hits the timeout.

### 6.6.29 Graphical end-to-end tests in QEMU

- Input regression: more malformed/duplicate requests than the input server's
  handle capacity do not exhaust it; a ring retained across unsubscribe is
  not reused, including when only its mapping remains; repeated subscriptions
  do not exhaust watch slots. A client exits subscribed and its ring is
  released before the runner injects any input. Run with and without VGA.

- Output: every pixel of a filled rectangle carries the color it was
  filled with and the pixels around it are untouched; a rendered string
  matches the glyph table pixel for pixel; the resolution used by the test
  is what the display server reported out of the boot information, never
  one assumed by the runner.
- Input: a key sequence injected through QMP is echoed as `[input]` lines
  through the console driver; a pointer path produces motion events whose
  sum equals the injected path; a button press and release arrive in
  order.
- The end of a client: the program that draws exits without giving its
  surface up, and the display server takes it back — the kernel signals the
  end on the notification, the watching thread of the server turns it into
  a message, and the surface and its memory go back.
- Combined: the cursor pixels move with the pointer, and every pixel of
  the sprite is what the display server's own shape says it is, while the
  place it left carries the background again; a stroke drawn while the
  button is held carries the pen at every step along its longer axis;
  typed text stands at the text cursor pixel for pixel against the glyph
  table. Each of the three is checked against a position the canvas said
  on the console, never one the runner worked out for itself, and the
  three run after the program that listens has ended, so what is injected
  for them is no part of what that one was checked against.
- Absent hardware: with `-vga none` the kernel reports
  `[info] framebuffer=absent`, the display server reports that there is no
  screen, the program that draws says it drew nothing, the canvas says it
  has no screen and ends without waiting for input, the run still ends by
  itself, and the input tests still pass (Phase 10).

### 6.6.30 Constant-time helpers (`crypto-ct`)

- `ct_eq` returns 1 exactly for equal slices and 0 otherwise; slices of
  differing length are unequal; the empty slice equals the empty slice.
- `ct_select_*` returns the first argument for `Choice(1)` and the second
  for `Choice(0)`, for both extreme and random values (property).
- `ct_swap` exchanges the buffers for `Choice(1)` and leaves them
  untouched for `Choice(0)`; `ct_copy` writes for `Choice(1)` and not for
  `Choice(0)`. Both take arrays of one length, so a mismatch is a compile
  error and there is no runtime case to test.
- `Secret<N>`: `Debug` prints the length and no byte of the content;
  equality goes through `ct_eq`; `clear` leaves zeros. `Drop` delegates to
  `clear` in one line, which safe Rust cannot observe from outside, so the
  erase is checked on `clear`.

### 6.6.31 Hashes, HMAC, and HKDF (`crypto-hash`)

- SHA-256, SHA-384, and SHA-512 against the FIPS 180-4 vectors, including
  the empty message, exactly one block, one block minus one byte, one
  block plus one byte, and the million-character message, which runs with
  the rest because it costs a second.
- Incremental hashing: any splitting of a message into chunks produces the
  digest of the one-shot call (property).
- Padding boundaries: a message whose length leaves 55, 56, or 57 bytes in
  the final block is padded into one or two blocks correctly.
- HMAC against the RFC 4231 vectors, including keys shorter than, equal
  to, and longer than the block length, and the empty key.
- HKDF against the RFC 5869 vectors; an empty salt behaves as a zero salt;
  an output longer than `255 * OUTPUT_LEN` is `OutputTooLong` and leaves
  the buffer untouched; a zero-length output is accepted; the expansion is
  the documented chain of codes.
- The provided `digest` of the trait agrees with the inherent one, and a
  cloned state continues the message it was cloned from.

### 6.6.32 Authenticated encryption (`crypto-aead`)

- ChaCha20 block function and keystream against RFC 8439 §2.3.2 and
  §2.4.2; a counter that would wrap is an error.
- Poly1305 against RFC 8439 §2.5.2, a message of exactly one block, a
  message one byte past a block, and the empty message. The edge cases of
  the reduction are derived rather than transcribed: with a multiplier of
  one and an addend of zero the tag is the accumulator itself, so an
  all-ones message and the four sums that land at the modulus and just
  above it have expected values that follow from the definition. That is
  the case a final reduction which subtracts once too often or too seldom
  gets wrong.
- ChaCha20-Poly1305 against RFC 8439 §2.8.2, with associated data, without
  it, and with neither message nor data.
- AES-128 and AES-256 block encryption against FIPS 197 appendices B and
  C; the substitution box against the published table for all 256 inputs;
  the bitsliced batch of four blocks equals four single-block encryptions.
- GHASH on its own against the value the second published case of the mode
  implies, and AES-GCM against cases one to four and thirteen to sixteen
  of the test set that SP 800-38D adopted, covering empty plaintext, empty
  associated data, both empty, and lengths that are not a multiple of the
  block size; thirteen further lengths around the four-block group that
  the lanes are encrypted in.
- Seal then open returns the plaintext for arbitrary inputs (property);
  flipping a bit in any byte of the ciphertext or of the tag, and any
  change to the nonce, the associated data, or the key, makes `open` fail;
  a failed `open` leaves no plaintext in the buffer. A key or a nonce of
  the wrong length is refused before anything is written.

### 6.6.33 Elliptic curves (`crypto-ec`)

- `fe25519`: addition, multiplication, and squaring agree with a
  reference implementation over `u128` limbs on random inputs (property);
  inversion of a non-zero element yields the identity when multiplied
  back; the encoding reduces values at or above the prime instead of
  refusing them, which is what RFC 7748 asks of X25519, and a test pins
  the three cases that says: the prime itself, one above it, and all ones.
- X25519 against RFC 7748 §5.2 and §6.1, including the iterated test at
  one thousand rounds and, behind a slow test, at one million; a peer
  value that produces an all-zero shared secret is rejected; non-canonical
  peer encodings are handled as the RFC prescribes.
- Ed25519 against RFC 8032 §7.1 in both directions: every vector verifies,
  and signing reproduces the public key and the signature the vector
  states, signing being deterministic. Rejection of `S >= L`, of
  non-canonical point encodings, of small-order public keys, and of a
  signature over a modified message.
- The masked multiplications of D-135 answer what the branching ones do:
  `Point::mul_secret` agrees with `Point::mul` on the base point and on
  another point, for zero, for a scalar of all ones, and for generated
  scalars; `Scalar::mul_secret` agrees with `Scalar::mul` over a small
  square of factors. The RFC 8032 signatures are the second half of that
  check, since signing takes the masked path and its vectors are pinned.
- P-256: the generator and its first multiples against the published
  points; the group law, including that the multiple by the order is the
  neutral element and the multiple by one less is the negation of the
  generator; both moduli round-tripping through their encodings and
  refusing a value at or above them. ECDSA against the two P-256 vectors
  of RFC 6979, appendix A.2.5, which pin the signature as well as the
  verifier because the nonce is derived rather than chosen. Rejection of
  `r` or `s` equal to zero or at or above the order, of a key that is not
  an uncompressed point, of a coordinate at the field prime, of a pair
  that is not on the curve, and of a signature over a different digest; a
  digest longer than the order is truncated to its leftmost bytes, so a
  change beyond them does not change the outcome and a change within them
  does.
- Signing then verifying round-trips for generated Ed25519 keys, and a
  modified message does not verify. With `test-signing`: the same round
  trip for ECDSA, whose deterministic nonce matches the RFC 6979 example.

### 6.6.34 Random generator (`crypto-rng`)

- `ChaChaRng` produces the documented stream for a fixed seed, for
  requests of one byte, of a block, and of a length that crosses a block;
  a request of nothing produces nothing and leaves the state alone. The
  seed decides the stream and the entropy source does not, whatever the
  pattern of request lengths (property).
- The generator rekeys after each request: one request of sixty-four bytes
  and two of thirty-two agree on their first half and differ on their
  second.
- The reseed budget triggers exactly one `Entropy` call at the boundary and
  none before it; a failing source surfaces as an error and leaves the
  caller's buffer untouched; a reseed on demand changes the stream; a
  seeded generator differs from one that took the same bytes as a seed,
  because seeding mixes rather than replaces.
- `ScriptedRng` returns the scripted bytes, refuses a request longer than
  what is left without consuming any of it, and reports exhaustion instead
  of repeating.

### 6.6.35 DER reader (`audhsos-der`)

- Header parsing: single-byte and multi-byte lengths; a non-minimal
  length, an indefinite length, a length beyond the input, and a length
  whose encoding is longer than needed are all rejected.
- Integers: a leading zero that is not required, a negative value where
  unsigned is expected, and the empty integer are rejected.
- Bit strings with a non-zero unused-bit count where zero is required;
  octet strings of length zero; object identifiers compared as bytes.
- Times: `UTCTime` and `GeneralizedTime` in the forms RFC 5280 allows, and
  rejection of the forms it forbids — no seconds, no zone, a lower-case
  zone, an offset, fractional seconds, a letter among the digits; the
  two-digit year window at both sides of its boundary; every field out of
  range. A day beyond the true length of its month is still accepted, and
  a test says so: that check needs a calendar, which decision D-46 puts in
  `audhsos-time`, and section 11.14 of document 11 carries the seam.
- Nesting deeper than `MAX_DEPTH` is rejected, and nesting exactly to it is
  read; trailing bytes after the outermost value are rejected; a tag that
  was not expected leaves the reader where it was.
- Property: no input makes the reader panic. A value is a slice of the
  input, so it re-encodes to what it came from by construction rather than
  by a test. The fuzz target `der` walks the structure of arbitrary bytes,
  descending into every constructed value to `MAX_DEPTH`, and asserts that
  a value read is shorter than what it was read from and that the reader
  moved past it. It hands the whole input to the two time forms as well,
  because a walk reaches them only for an input that carries their tag,
  and checks the fields of a time it accepts against their ranges.

### 6.6.36 Certificates and path validation (`audhsos-x509`)

- Parsing: a minimal valid certificate, and an authority with its
  constraints; a version other than three and a public key that does not
  match its algorithm are rejected; an unknown critical extension is
  rejected and an unknown extension that is not critical is passed over;
  a repeated extension is rejected; a key algorithm outside the supported
  set is rejected; an extension marked critical that carries the default
  value is rejected.
- Signatures: each supported algorithm verifies a correct signature and
  rejects one over a modified `tbs`; an algorithm mismatch between the
  outer and inner fields is rejected.
- Validity: `not_before` in the future, `not_after` in the past, and the
  exact boundary instants.
- Chain: a two-link and a three-link chain verify; a broken issuer name
  link, a broken signature link, a missing intermediate, a chain longer
  than eight, a self-signed leaf without an anchor, and an anchor that
  signs nothing in the chain are all rejected.
- Constraints: an intermediate without `cA`, an intermediate without
  `keyCertSign`, a path length exceeded, and a leaf without `serverAuth`
  are rejected.
- Names: an exact `dNSName` match; a wildcard in the leftmost label
  standing for exactly one label; a wildcard elsewhere, a partial-label
  wildcard, a wildcard matching nothing, and one matching two labels are
  rejected; a common name that would match is ignored when no alternative
  name is present at all; case is compared case-insensitively for ASCII; a
  trailing dot on either side is the same name; an IP address matches only
  an `iPAddress` entry, and a `dNSName` that spells an address does not.
- RSA: the three `sha*WithRSAEncryption` identifiers are read with NULL
  parameters and with the field absent, and both are right (RFC 4055,
  section 5) where absence is the only right answer for ECDSA and
  Ed25519, which still refuse a NULL; a parameter that is neither is
  rejected. `id-RSASSA-PSS` is read for the three parameter sets that
  pair a hash with MGF1 over that same hash and a salt as long as its
  output, and rejected for a salt of another length, a mask over another
  hash, a hash this crate does not know, a mask function that is not
  MGF1, and no parameters at all. An `rsaEncryption` key is the two
  integers of RFC 3279, section 2.3.1 and its parameters must be NULL.
  The size bound of D-79 is applied at each edge — two thousand and
  forty-eight, three thousand and seventy-two, and four thousand and
  ninety-six bits accepted; a thousand and twenty-four, two thousand and
  forty, and four thousand one hundred and four refused — and it is
  applied here rather than in `crypto-rsa`. A chain verifies under each of
  the six schemes and a modified body under none of them; the widest key
  is built and verified once, which is what took `MAX_CERTIFICATE` past a
  kibibyte.
- Property: mutating any byte of a valid certificate, in either of two
  ways, makes parsing or verification fail; mutating any byte of an
  intermediate makes the chain fail. The fuzz target `x509` parses
  arbitrary bytes and, for a certificate that parses, reads everything a
  caller reads off one — the names, the matching, the signature — and
  asserts two things: every field is a view of the input, and nothing
  verifies against an empty trust store, neither on its own nor as its own
  issuer. The corpus holds the certificate of RFC 8448 and three the
  builder writes.

### 6.6.37 TLS record layer and key schedule (`audhsos-tls`)

- Records: the maximum plaintext and ciphertext lengths are accepted, one
  byte more is `RecordOverflow`; a header with an unexpected content type
  before the handshake is rejected; a `change_cipher_spec` record is
  dropped inside the compatibility window and rejected after the server's
  `Finished`, and one that carries anything but the single byte one is
  rejected wherever it arrives; a record spanning two `read_tls` calls is
  reassembled; a zero-length inner plaintext without a content type byte
  is rejected.
- Padding: trailing zeros are stripped, an all-zero inner plaintext is
  rejected, padding of the maximum length is accepted.
- Sequence numbers: the nonce is the IV xor the sequence number; exhaustion
  of the sequence space is an error rather than a wrap.
- Key schedule: every secret of RFC 8446 §7.1 from the early stage to the
  application secrets, the traffic key and nonce base, the finished key,
  and the secret after a key update, for both hash lengths. RFC 8446
  publishes no vectors of its own, so the expected values were computed
  from section 7.1 by an implementation outside this repository over
  inputs the test file fixes. The trace of RFC 8448 pins a whole
  handshake's schedule on top of that, in the replay of 6.6.38.
- The nonce is the base with the sequence number exclusive-ored into its
  tail, at zero, at one, and at a number that touches every byte.
- Transcript: the hash equals the digest of the messages concatenated, for
  both lengths and for any split of a message; the `message_hash`
  substitution reproduces what the standard describes, and applies to each
  hash with its own length so that both stay usable.

### 6.6.38 TLS handshake and connection (`audhsos-tls`)

- The RFC 8448 §3 trace, replayed through the pieces this crate is built
  of: the transcript, every secret of the schedule, both traffic keys, the
  finished key of each side, the verify data of both `Finished` messages,
  and the client's own record opened under the keys the document names.
  Each message of the trace is read by this crate's reader and says what
  the document says it says. The replay does not run the state machine,
  and the test file names the reason: matching the document byte for byte
  would mean writing another implementation's `ClientHello`, which enters
  the transcript every later secret depends on. It does check the server's
  signature — the `CertificateVerify` of the trace is a real
  `rsa_pss_rsae_sha256` signature under the key section 2 of that document
  prints, and it verifies through `crypto-rsa` against the transcript this
  crate computes, and fails against a transcript one bit different. That
  key is a thousand and twenty-four bits, so no chain is walked with it
  (D-79); the signature inside the chain is checked, the chain around it
  is not.
- The state machine driven end to end against a server built in the test
  file, which uses the same pieces from the other side: handshake, data
  in both directions, a key update, and a close. Once with an Ed25519
  chain, once with P-256, once with P-384, and twice with RSA — at two
  thousand and forty-eight bits and at four thousand and ninety-six, the
  width `MAX_SPKI` is sized for — so that every signature path carries a
  whole handshake, with certificate verification on in each.
- The two lists of signature schemes are not the same list, and the RSA
  code points make the difference visible (D-82). The `ClientHello`
  offers all nine, the three `rsa_pkcs1_*` among them, which the written
  hello is checked against byte for byte; a `CertificateVerify` naming any
  of those three is refused as an illegal parameter, and the PSS code
  point over the same key is accepted. Both directions have a test, and
  they do not contradict each other: the offer is about the chain, the
  refusal is about the handshake signature.
- `HelloRetryRequest` is recognised, must name a group, and replaces the
  transcript with the hash of it — and is then refused, because D-56
  leaves one group and a retry can only ask for a group that was already
  offered.
- Rejections: a `ServerHello` negotiating anything other than 1.3; the
  TLS 1.2 downgrade sentinel in the server random; a cipher suite not
  offered; a key share group not offered; a missing `key_share`; a
  `Finished` with a wrong verify data; a `CertificateVerify` this client
  cannot check; a chain that does not reach an anchor; a record that does
  not open; an alert from the server; a message in the wrong place.
- After any fatal error the connection is poisoned: every further call
  returns the same error, while `write_tls` still hands over the alert
  that says why.
- Buffers: a buffer below the documented minimum is rejected at
  construction; a `ClientHello` or a handshake message that does not fit
  is `BufferTooSmall`; `write_tls` into a short output buffer makes
  progress across calls without losing bytes.
- Application data: `send` and `recv` round-trip against that server;
  `close` emits `close_notify`, and a peer `close_notify` surfaces as
  `PeerClosed`.
- A `NewSessionTicket` is recognised and skipped, both inside the flight
  and after the handshake.
- Property: arbitrary byte streams fed to `read_tls` never panic; the
  connection either makes no progress or ends, and once it has ended it
  gives the same answer to every later call. The fuzz target `tls_record`
  frames arbitrary bytes, asserts that a record's length is its header and
  its body, tries to open it under keys the input does not know, and seals
  the input and opens it again, which must return the bytes that went in.
  The fuzz target `tls_handshake` reads the input as the stream of
  messages a server sends and then hands the whole input to every reader,
  because a body that never frames would otherwise never be parsed; a
  message that frames must say how long it is, and nothing a reader hands
  back may be longer than the message it came from. Both corpora start
  from the trace of RFC 8448.

### 6.6.39 Time and calendar (`audhsos-time`)

- `civil_from_days` and `days_from_civil` round-trip for every day from
  1601-01-01 to 9999-12-31 (property) and agree with hand-computed values
  at the epoch, at 2000-02-29, at 1900-03-01, and at 2100-03-01.
- The same range walked day by day: every day is the successor of the one
  before it, which a month length that were wrong by a day would fail even
  though a round trip through the same wrong length would still close.
- Leap years: 1900 and 2100 are common, 2000 and 2400 are leap; February
  has 28 or 29 days accordingly; day 0 and day 32 of any month are
  rejected.
- Field ranges: month 0 and 13, hour 24, minute 60, second 60, and a
  negative year are rejected; second 59 and hour 23 are accepted.
  `validate` reports the first field that is wrong, in the order year,
  month, day, hour, minute, second.
- The ends of the calendar: 0000-01-01 and 9999-12-31T23:59:59Z convert in
  both directions; one day beyond either end is `Year`, and a day number so
  large that the shift onto the era overflows is `OutOfRange`.
- `UnixTime`: the epoch is zero; negative values represent times before
  1970 and convert back; `checked_add` and `checked_sub` at the extremes
  of `i64` return `None` rather than wrapping; the part of a `Duration`
  below one second does not move a point, because the type resolves
  seconds.
- `Instant` and `Duration`: addition saturates at the maximum;
  `saturating_duration_since` of an earlier instant is zero; ordering is
  total; `checked_add`, `checked_sub`, and `checked_mul` report the ends
  instead of saturating.
- The generators behind `test-strategies` produce only values the crate
  accepts: a day inside the calendar, a `CivilTime` that validates, a
  `UnixTime` that converts, and an `Instant` that leaves room for the
  longest generated `Duration`.

### 6.6.40 Encodings (`audhsos-encoding`)

- Base64 against the RFC 4648 §10 vectors for lengths zero to six;
  encoding into a buffer one byte too small is an error and writes
  nothing; round trip for arbitrary input (property).
- Base64 decoding rejects a missing pad, an excess pad, a pad in the
  middle, a character outside the alphabet, whitespace, and non-zero
  trailing bits in the final quantum. The two texts that differ from a
  canonical one only in those bits, `Zh==` for `Zg==` and `Zm9=` for
  `Zm8=`, are named as such.
- A length that is not a multiple of four is refused before any character
  is read, so whitespace inside a quantum is a length error and
  whitespace that keeps the length is a character error; both are tested.
- Hex: round trip for arbitrary input (property); an odd length, an
  upper-case and a lower-case digit pair, and a non-hex character are
  handled as specified; a character error names its offset.
- PEM against RFC 7468: a minimal certificate block; a label mismatch
  between the begin and end line, a missing end line, a line longer than
  64 characters other than the last, data after the end line, and an
  empty payload are rejected; a block preceded by explanatory text is
  accepted, as the RFC's lax parsing permits, and the text is not
  returned.
- PEM further: a text with no end line reports the missing line and not
  the length of a body line, because the rule for the last body line
  applies only to a block that has one; a pad before the last body line
  is refused; `CRLF` is accepted as a terminator; a label with a leading
  or trailing space, two spaces, a hyphen, or a character outside the
  printable range is refused; line terminators after the end line are not
  data.
- PEM at a width that is not RFC 7468: a block wrapped at seventy, the
  width `openssh-key-v1` is written at, round-trips and carries lines of
  exactly seventy characters but the last; the strict reader refuses it
  and names both the line and the width it expected. The wrapped reader
  takes a text wrapped more narrowly and refuses one wrapped wider, an
  empty body line, a pad before the last line, and a body whose characters
  do not fill a quantum. A character outside the alphabet is named by its
  offset in the body and not in the quantum it fell in, which is what a
  reader four characters at a time would otherwise report. A buffer one
  byte short of `encoded_len_wrapped` is an error and writes nothing.
- Property: no input causes a panic, and every accepted block re-encodes
  to a canonical form that decodes to the same bytes. The generator of
  near-valid blocks is itself checked to reach both an accepted and a
  refused text, so that the property does not silently test one rule.
  Fuzz target `pem`, which also asserts that a text the Base64 decoder
  accepts re-encodes to exactly itself.

### 6.6.41 Fixed-capacity collections (`audhsos-collections`)

- `ArrayVec`: push to capacity succeeds and one more is `Full`; pop from
  empty is `None`; `insert` and `remove` at the ends and in the middle
  keep the order; clearing leaves length zero.
- `RingBuffer`: write and read across the wrap boundary; a full buffer
  rejects the write rather than overwriting; the free space reported
  equals the number of writes that then succeed.
- `BitSet`: set, clear, and test at bit zero, at a word boundary, and at
  the last bit; `first_set` and `first_clear` on an empty, a full, and a
  mixed set; an index at or beyond the size is an error.
- `IndexList`: push front and back, unlink from the middle, from the
  head, and from the tail; a list of one; iteration order matches the
  insertion order; unlinking a node that is not in the list is rejected.
- `IndexMap`: insert, look up, and remove in order; a duplicate key
  replaces the value and does not grow the map; capacity exhaustion is
  `Full`; iteration is sorted by key.
- `IndexList` further: a node outside the caller's slice is an index
  error; a node already in a list is refused by that list and by every
  other, and unlinking it from another one is refused as well, which is
  the shape of one run queue per priority over one array of threads; a
  list identified by `NONE` is refused at construction; a walk over links
  a caller has corrupted into a cycle still ends, because it takes as many
  steps as the list says it is long.
- `IndexList::insert_after`: an insert after `None` is a `push_front`;
  after the tail is a `push_back`; in the middle links both neighbours; a
  node that is already linked, one outside the slice, and an `after` that
  belongs to another list or to none are each refused and change nothing.
  The item belongs to 6.6.59 as well, which is the phase that added it.
- `BitSet` further: a set of no words holds no bit and refuses every
  index; a first word that is full sends `first_clear` into the second;
  `count` and `is_empty` agree with the bits that are set.
- Every owning container holds a value that is neither `Copy` nor
  `Default` and moves it in and out, because the storage is what makes
  that possible and a test that only ever held a `u32` would not say so.
- Model tests: every container against `Vec`, `VecDeque`, and `BTreeMap`
  under generated operation sequences, and the bit set against a
  `BTreeSet` of the indices that are set; no operation panics for any
  sequence. The generators produce indices, node numbers, and keys that
  reach beyond the container as well as inside it, so the error paths are
  part of the sequence rather than a separate test.

### 6.6.42 Wire primitives (`net-wire`)

- Addresses: `MacAddr` and `Ipv4Addr` parse from and format to their
  canonical forms; the broadcast, unspecified, loopback, and multicast
  predicates; an `Ipv4Cidr` with prefix length 0, 32, and 33, the last
  rejected; `contains` at both ends of a range.
- `Ipv6Addr`: the canonical text of RFC 5952, section 4 in both
  directions, against the examples the recommendation itself gives; every
  rule of section 4 refused in turn — a leading zero, a `::` that could
  have been longer, a `::` over a single zero group, a run written out
  where a `::` belongs, the rightmost of two equal runs, two `::`, and the
  dotted form of an IPv4-mapped address; upper case read and never
  written; the unspecified, loopback, multicast, link-local, and
  unique-local predicates at both ends of each prefix; the solicited-node
  address against the example of RFC 4291, section 2.7.1, and two
  addresses that differ above the low 24 bits sharing one; an `Ipv6Cidr`
  with prefix length 0, 128, and 129, the last rejected.
- `IpAddr` and `IpCidr`: a text with a colon is read as the second family
  and every other as the first; the predicates answer through the enum for
  both; a route of one family never contains a destination of the other.
- No text either parser accepts has a second spelling: the uncompressed
  form of a generated address is accepted exactly when it is the canonical
  one (property).
- Cursor: reading a `u8`, `u16`, and `u32` in big-endian order; a read
  past the end returns an error and leaves the position unchanged; a
  write into a buffer one byte too small fails and writes nothing;
  skipping beyond the end is an error.
- Internet checksum against the RFC 1071 worked example, over an odd
  number of bytes, over an empty slice, and over data whose sum carries
  repeatedly; the checksum of a buffer that already contains its own
  checksum is zero (property).
- Pseudo-header checksums for UDP and TCP against hand-computed values,
  including a zero-length payload.
- The IPv6 pseudo-header of RFC 8200, section 8.1 against hand-computed
  values: a UDP datagram, and an `ICMPv6` echo request, whose checksum
  covers a pseudo-header where `ICMPv4`'s does not. The family-agnostic
  form dispatches on the addresses and answers `MixedFamilies` for a pair
  that is not one family, in both directions.
- Checksum further: the same eight bytes of the RFC 1071 example split
  into a group of three and a group of five, and then one byte at a time,
  give the sum the memo's last table prints, which is what holds the
  pending byte to its job; an empty call between two odd ones keeps that
  byte; a segment longer than a sixteen-bit length is refused rather than
  summed against a wrapped one; and no split of any generated block
  changes the sum (property).
- The tables: an `EtherType` and a `Protocol` this system reads report
  themselves registered and the others do not; a type field below 0x0600
  is a length and not a type; TCP, UDP, and `ICMPv6` carry a
  pseudo-header and `ICMPv4` does not; the four IPv6 extension header
  numbers report themselves as such and no upper-layer protocol does; and
  each writes its name or, for a value this crate does not name, its
  number.
- Cursor further: an address of either family reads and writes in the
  width of that family and one that does not fit writes nothing; a length
  that would overflow the position is out of bounds rather than a wrap;
  `patch_u16` reaches only what has been written; a reader driven through every width until it runs out never
  passes the end of its buffer and never moves on a failure (property).
- Every error variant renders a sentence of its own.

### 6.6.43 Ethernet and ARP (`net-eth`)

- Frames: a minimum-length frame, a maximum-length frame, one byte too
  short, one byte past the MTU, and an unregistered ether type; the
  destination filter accepts the interface address, the broadcast
  address, and any multicast group, and drops everything else; the three
  registered types pass it. A frame that does not fit the buffer writes
  nothing, and a payload past the MTU is refused before the buffer is
  measured. Round trip and totality as properties: what is written reads
  back, and no byte stream makes either entry point do anything but
  answer.
- ARP encoding against the RFC 826 field layout, as a transcribed
  28-byte vector read and written both ways; a request for the interface
  address produces exactly one reply; a request for another address
  produces none; a reply is never answered; each of the four fields that
  name the address spaces is wrong in turn and refused; an operation that
  is neither a request nor a reply is refused; every prefix shorter than
  the packet is refused; a packet that does not fit writes nothing.
- Cache: an entry moves `Incomplete` to `Reachable` on a reply and
  `Reachable` to `Stale` at the age boundary, one microsecond before
  which nothing happens; a packet to a `Stale` neighbor goes out and
  moves the entry to `Delay`, which becomes `Probe` at
  `DELAY_FIRST_PROBE_TIME` and `Reachable` again on an answer; an entry
  evicted at capacity is the least recently used; a second packet for a
  destination with a pending request replaces the first rather than
  queueing two; a packet longer than the entry holds is dropped and
  leaves nothing behind; one cache holds neighbors of both families and a
  solicitation names the address, from which the caller reads the family.
- Retransmission: solicitations are emitted at the scheduled instants and
  stop after the configured count, after which the entry and the packet
  behind it are gone and the caller sees an unreachable result. Both
  counts are exercised, the multicast one from `Incomplete` and the
  unicast one from `Probe`. A schedule of the caller's own is used as
  given, and one instant falling due for two neighbors yields two events.
- Gratuitous ARP refreshes a reachable entry with the same address and
  does not replace one with a different address; an observation fills an
  entry nobody has answered for, which releases the packet waiting behind
  it; an entry that is not reachable is taken over by whoever claims it
  last, which the test states so that the limit of the protection is on
  the record.

### 6.6.44 IPv4 and ICMP (`net-ip`)

- Header: a minimum header, a header with options that are skipped, a
  wrong version, a header length below five words, a total length beyond
  the frame, and a bad checksum are each handled as specified.
- Fragmentation: a datagram exactly at the MTU is not fragmented, one
  byte more produces two fragments whose reassembly equals the original;
  the don't-fragment bit turns an oversized datagram into an error and
  leaves one that fits alone; an empty payload still makes one datagram;
  an MTU with no room for a header and eight bytes behind it is refused;
  the pieces are counted before they are walked.
- Reassembly: fragments in order, in reverse order, in a third order, and
  with a duplicate that agrees; an overlapping fragment that disagrees
  discards the datagram; a missing fragment expires at the deadline and
  frees its buffer, and one microsecond earlier it does not; more
  concurrent datagrams than buffers evicts the oldest; a fragment beyond
  the buffer is refused before a byte is copied; a datagram that was
  never fragmented is not copied at all; a timeout of the caller's own is
  used as given.
- ICMP: an echo request produces a reply with the payload copied and
  nothing else produces one; a destination-unreachable message is
  delivered to the upper layer with the embedded header parsed, and the
  quote is shorter when the datagram was; the unreachable codes map both
  ways; a type this crate does not read is carried and refused on the way
  out; a message that is short or does not verify is refused. Generated
  errors stop at the token-bucket limit and resume as it refills, the
  bucket never fills past its capacity, and its remainder is kept so the
  rate does not drift. No error is generated in answer to an error, to a
  broadcast or multicast destination, to this interface's own broadcast
  address, to a link-layer broadcast, to a non-initial fragment, or to a
  source that names no single host — the five restrictions of RFC 1122,
  section 3.2.2, each on its own.
- Fuzz target `ipv4`: no byte stream makes the datagram parser, the
  quoted-header parser, the `ICMPv4` parser, or the reassembler panic; a
  datagram a parser accepts stays inside the bytes it came from and its
  header, payload, and total length agree; a fragment offset is a multiple
  of eight; an `ICMPv4` message that is written reads back as itself; and
  the reassembly buffers empty once time passes every deadline. The seed
  corpus carries a minimal datagram, one with an option, a first and a
  later fragment, one that may not be fragmented, an echo request, and a
  port-unreachable message.
- Routing: longest-prefix match with a host route, a subnet route, and
  the default route; a destination with no route is an error; an on-link
  destination resolves through ARP, an off-link one through the gateway.
  A route of one family never reaches a destination of the other, and a
  default route of each covers its own. Adding a prefix that is already
  there replaces it, in kind as well as in gateway, and a full table
  refuses rather than evicting.
- The quoted header: a quote is read where a whole datagram cannot be,
  because the total length it declares is longer than what arrived; the
  version, the header length, and the checksum are checked as they are in
  a datagram, each wrong in turn; a quote of the header alone carries no
  payload and is still read; a quote with options steps over them.
- The send path: a datagram to a known neighbor on this link goes out at
  once and the frame is addressed to that neighbor; one to a far host is
  addressed to the router while the datagram keeps the far address; one
  to an unknown neighbor waits and goes when the answer comes, once and
  not twice; one longer than the MTU leaves in several frames that
  rejoin to the original and each fit an Ethernet; a destination with no
  route, a pair of the second family, a buffer smaller than the MTU, and
  an emitter that refuses each come back as an error; a datagram the
  cache will not hold is dropped and said so. A frame this host writes is
  one `net-eth` accepts for the neighbor and drops for this host.

### 6.6.45 UDP (`net-udp`)

- Datagrams: an empty payload, a maximum payload, a length field shorter
  and longer than the data, a zero checksum accepted on receipt, and a
  bad non-zero checksum rejected; the checksum written on send verifies.
- Sockets: binding a port twice is rejected; an ephemeral port is inside
  the documented range and is not one already bound; a datagram for an
  unbound port produces an ICMP port-unreachable request to the layer
  below.
- Receive ring: a full ring drops the newest datagram and counts it; the
  count is observable; a datagram larger than the ring is rejected at
  entry.

### 6.6.46 TCP (`net-tcp`)

- Sequence arithmetic: the window comparisons at the wrap boundary, a
  window that spans the boundary, equality, and the empty window
  (table-driven over the RFC 9293 conditions).
- Connection setup: the three-way handshake for active and passive open;
  simultaneous open; a SYN with an unacceptable acknowledgment produces a
  reset; a retransmitted SYN in `SYN-RECEIVED` is absorbed.
- Teardown: active close through `FIN-WAIT-1`, `FIN-WAIT-2`,
  `TIME-WAIT`; passive close through `CLOSE-WAIT` and `LAST-ACK`;
  simultaneous close through `CLOSING`; `TIME-WAIT` ends at twice the
  maximum segment lifetime and not before.
- Data transfer: a segment at the window edge, one byte beyond it, a
  duplicate, an out-of-order segment that is queued and then completed,
  and a segment covering data already acknowledged.
- Windows: a zero window stops transmission and starts the persist
  timer; the probe is emitted at the scheduled instants; a reopened
  window resumes; the advertised window never shrinks.
- Retransmission: the RTO after the first sample, after a smoothed
  series, at the one-second floor, and at the sixty-second ceiling;
  Karn's rule ignores a retransmitted segment's sample; backoff doubles
  per attempt and the connection aborts after the configured count.
- Congestion control: slow start doubles per round trip; the transition
  to congestion avoidance at the threshold; three duplicate
  acknowledgments trigger fast retransmit and halve the window; a
  timeout resets to one segment.
- Delayed acknowledgments: an acknowledgment is sent at the second
  full-sized segment or at 500 milliseconds, whichever comes first; a
  segment that fills a previously zero window is acknowledged at once.
- Resets: a reset inside the window tears the connection down; one
  outside it is ignored and answered with a challenge acknowledgment; a
  reset is generated for a segment to a closed port.
- Options: the maximum segment size is honored and clamped to the
  interface MTU; an unknown option is skipped; a malformed option list
  is rejected.
- Model test: two instances over a network double that delays,
  duplicates, reorders, and drops; for every generated schedule both
  sides transfer their byte streams intact, in order, and reach
  `CLOSED`; `poll_at` never reports an instant at which the stack has no
  work. Fuzz target `tcp_segment`.

### 6.6.47 DNS (`net-dns`)

- Encoding: a question, an answer with an `A` record, one with an `AAAA`
  record, a `CNAME` chain of depth two, and a response with no answers.
  Every message and every record reads back as what was written.
- Names: a label of 63 characters, one of 64 rejected, a name of 255
  bytes, one of 256 rejected, an empty name, a name with a compression
  pointer to an earlier label, and one whose labels are assembled from
  pointers past the 255-byte limit. The preferred syntax is enforced: a
  leading or trailing hyphen, an underscore, a space, and a byte outside
  ASCII are each refused. Case is written as it came and ignored when two
  names are compared, and two names that compare equal hash alike.
- Compression: a pointer loop, a forward pointer, and a pointer chain
  longer than the jump limit are rejected without unbounded work. A
  length octet of a reserved kind is refused.
- Records: a body that is not the length its type has is rejected for `A`
  and for `AAAA`; a `CNAME` that does not end where its body does is
  rejected; a type this crate has no use for and a class other than `IN`
  are carried whole and stepped over; a `CNAME` may point into the
  message it stands in.
- Resolver: both questions go out at once, to different servers and with
  different transaction ids, and the answers of both come back together.
  A matching response is accepted; responses with a wrong transaction id,
  a wrong question section, a wrong source address, or a wrong source
  port are ignored, as are a message that is a query, bytes that are no
  message, and one whose answer section cannot be walked. The query is
  retried at the scheduled instants and rotates servers, the transaction
  id is kept across retries so that a late answer is still an answer, and
  exhaustion is an error; so is a deadline that passes with nothing
  settled. A response longer than the 512 bytes of RFC 1035, section 4.2.1
  is ignored, and a buffer with no room for a query costs no attempt. A
  name that does not exist is an answer and not a failure; a
  truncated answer is a failure, because there is no TCP here; a server
  that answers a failure code is asked again at once rather than after
  the retry. One address that came back twice is one address, and a
  record of the other family does not answer this question.
- `CNAME`: a chain inside one answer is followed; a chain that leaves the
  answer is asked on its own with a new transaction id; chains longer
  than eight and a chain that returns to a record it has used are
  rejected. Fuzz target `dns_message`.

### 6.6.48 DHCP (`net-dhcp`)

- The four-message exchange produces a bound lease with address, mask,
  router, and DNS servers taken from the options, and the discover and
  the request carry the parameter request list and the BROADCAST flag.
- Options: an unknown option is skipped; a truncated option is rejected;
  the end marker is required; padding is accepted; a missing message type
  is rejected, as is one whose body is not one byte; an option that
  stands twice is read as the last of them.
- Messages: the magic cookie is required; a hardware type or length that
  is not Ethernet's is refused; an op code that is neither direction is
  refused; a message shorter than the fixed part is no message; what this
  client writes is padded to 300 bytes and reads back as itself.
- An offer with a foreign transaction id is ignored, as are one for
  another hardware address, one from a wrong source port, one over IPv6,
  and one that is not a reply; a NAK returns the machine to the start;
  two offers select the first and ignore the second; an offer with no
  server identifier is no offer.
- Lease timers: renewal at T1 through unicast to the server that granted
  the lease, with neither the requested address nor the server identifier
  repeated; rebinding at T2 through broadcast; and expiry that clears the
  address and starts a new exchange. A renewal answered late keeps the
  lease. Backoff grows exponentially and stays inside the jitter bounds
  at both ends of the generator's range; a renewal waits half of what is
  left and never less than a minute, and never past the deadline. The T1
  and T2 the server sends are used where they make sense and fall back to
  the defaults where they do not.
- An acknowledgment with no mask, no lease time or no server identifier
  is ignored and the request stands; so is one whose mask is not a prefix,
  one whose mask is not four bytes, and one whose address is the
  unspecified one, a broadcast, a multicast group or a loopback address —
  an offer of such an address is no offer either. `Lease::from_reply` says
  which of them it was. A buffer with no room for a message costs neither
  an attempt nor a step of the backoff.

### 6.6.49 HTTP/1.1 client (`net-http`)

- Requests: the request line and headers for a minimal `GET`, with a host
  header always present; a body brings its own `Content-Length`; a header
  value with a control character is rejected at encoding time, as is a
  name that is not a token, a target that is not an origin-form path, an
  empty host, and any attempt to write `Host`, `Content-Length` or
  `Transfer-Encoding` from the caller's header list.
- Responses: a minimal response, a response with a body of declared
  length, a chunked body in one and in several reads, a chunk with an
  extension, the terminating zero chunk with and without trailers, and a
  body that ends when the connection does.
- Rejections: a status line that is too long, more headers than the
  limit, a header longer than the limit, a head longer than the buffer it
  was given, obsolete line folding, both `Content-Length` and
  `Transfer-Encoding` present, two `Content-Length` headers that
  disagree, a non-numeric length, a transfer encoding that is not chunked
  alone, a chunk size that is not hexadecimal or overflows, and a chunk
  that does not end where it said it would. Two `Content-Length` values
  that agree are the one value they agree on.
- Framing: a response split across every read boundary produces the same
  result as one read, for a chunked body and for one of declared length
  (property); a body larger than the caller's buffer is delivered in
  parts without loss; a message that said how long it was and was not is
  truncated, and the decoder says the same error to every further call.
- A response to `HEAD` and one of status 1xx, 204 or 304 carries no body
  whatever its fields say. A redirect status is reported with its
  location and is not followed; 300 and 304 are 3xx and are not among the
  five a client follows. Fuzz target `http_response`.

### 6.6.50 Interface and demultiplexing (`net-stack`)

- Demultiplexing: an ARP frame for this host's address and one for
  another, an IPv4 frame for a bound UDP socket, one for a TCP
  connection, one for an unbound port, one for a foreign address, one for
  another station, and a frame of a type this stack does not read, each
  reach the right layer or are dropped. The same over IPv6, with a
  neighbor solicitation, a neighbor advertisement, a duplicate address
  probe, and a router advertisement. Bytes that are not the thing they
  claim to be — a broken ARP packet, a broken header, a datagram whose
  checksum does not verify — are dropped without an answer.
- Sockets and connections: handles are generation-checked, a handle from
  a closed socket is rejected and so is a second close of it, the slot is
  used again under a new generation, a handle to no slot at all is
  refused, a slot that has been through every generation hands out no
  more, and the table reports exhaustion rather than reusing a live slot.
- `poll_at` returns the earliest deadline of every layer; after `poll` at
  that instant the deadline has advanced; an idle stack reports no
  deadline; a stack with a backlog reports the present instant.
- `poll` drains the outgoing work into a transmit buffer of one frame
  across several calls without losing or reordering a frame, and the
  queue itself keeps its order however it wraps (property).
- An interface without an address answers no ARP request, no neighbor
  solicitation, no echo request and no broadcast, and produces no IP
  traffic; the four-message exchange of DHCP makes all of it work, a
  refusal or an expiry takes the address and the routes away again, and a
  router advertisement does the same for IPv6 once duplicate address
  detection has finished — an address somebody else claims is not used.
- A datagram to an unresolved neighbor waits for the answer and goes when
  it arrives, over both families; a neighbor that never answers is given
  up on.
- Address selection: the policy table and the scopes of RFC 6724, a
  source of the other family is never one, the destination itself wins,
  the longest matching prefix decides between equals, a name of both
  families is tried in the document's order, and what nothing can reach
  comes last.
- Connecting: a list of addresses is tried in the order it is given and
  the next is taken when one is refused; every candidate refusing leaves
  the attempt failed and the buffers come back; a name is resolved,
  ordered and connected to; a second connection or a second resolution
  waits for the first; a resolution with no server to ask is refused; and
  a server of the other family is not one this resolution asks.

### 6.6.51 Virtqueue logic (`virtio-queue`)

- Descriptors: a single-descriptor request, a chain of three with both
  directions, a chain that takes every descriptor, a chain that exhausts
  the free list and gives back what it had taken, a chain of no buffers,
  a chain whose length exceeds the queue size, and a chain that puts a
  buffer the device writes before one it reads are handled as
  specified.
- Rings: the available index wraps at 2^16 while the ring slot wraps at
  the queue size, which is the case the two wraps have to agree in — a
  legal size is a power of two no larger than 32768 and therefore always
  divides 2^16, so what is checked is that the driver's index, the
  published index and the slot stay in step across the wrap, not that
  they can fall out of it. A used element that names a descriptor outside
  the table, and one that names a descriptor that is free, are rejected;
  a used index that moved backwards, and one that moved past the
  outstanding chains, are rejected. Chains come back in an order the
  driver did not add them in.
- The reported length: a device reporting exactly the writable bytes of
  the chain, and one reporting fewer, are believed; one reporting more is
  refused with both numbers, and the chain is freed and the element
  consumed either way. A chain the device only reads may report nothing
  and nothing else. Only the device-writable descriptors count towards
  the room.
- Chains that cannot be walked: one that loops back on itself and one
  that leaves the table are rejected, and what the walk reached before
  the bad step is back in the free set. A chain leaving the queue is
  rejected also when the free set is wider than the queue, so that the
  bound is the size and not the number of bits.
- Regions: a size that is not a power of two, and one wider than the free
  set, are refused; a table, an available ring, or a used ring too short
  for the field being accessed is refused with the byte it wanted, and a
  short descriptor table costs no descriptor. A region that shrinks after
  the queue was built is refused rather than indexed past.
- Notification suppression: with the no-notify flag set the driver emits
  no notification; clearing it resumes them. The driver's own
  no-interrupt flag reaches the available ring. Publishing and reaping
  each ask for a barrier.
- Initialization: the status sequence reset, acknowledge, driver,
  features-ok, driver-ok, checked as the exact sequence of writes; a step
  out of order changes nothing; a device that clears features-ok leaves
  the machine in failure; a device offering no `VERSION_1` is refused; a
  driver asking for one of the three unimplemented features is refused
  before anything is written, and a device offering one is driven without
  it; a queue is filled from features-ok on and read only from driver-ok
  on, which is where 3.1.1 puts the boundary — populating a virtqueue is
  step 7 and driver-ok is step 8, and what step 7 may not do is notify;
  nothing at all reaches a queue before features-ok; a device that sets
  `DEVICE_NEEDS_RESET` refuses every further operation, a live one is
  noticed by polling, and a reset is the way out.
- Model test: descriptors are allocated and freed against a reference
  free-list model over generated sequences of adding, completing out of
  order, and reaping; no sequence leaks a descriptor or hands out one
  twice, and a run that never reaches a full free set, a chain of three,
  or an out-of-order completion fails as vacuous.

### 6.6.52 FAT32 logic (`fs-fat`)

- Boot parameter block: a valid FAT32 block; a sector size other than
  512, a cluster size that is not a power of two, zero FATs, a FAT12 or
  FAT16 block, and a bad signature are rejected.
- Chains: a one-cluster file, a multi-cluster file, a chain with a loop,
  a chain that reaches a free cluster, and a chain that leaves the FAT
  are rejected or terminated as specified.
- Allocation: allocating in a full FAT is an error; a freed chain returns
  every cluster; the free count stays consistent across allocation and
  release (property).
- Directories: an 8.3 name with and without an extension, a name needing
  padding, a lower-case name rejected, a deleted entry skipped, the
  volume label skipped, an entry crossing a cluster boundary, and a full
  directory.
- Files: read at an offset, across a cluster boundary, and at the end;
  write that extends the file, that overwrites, and that fills the last
  cluster exactly; timestamps written through `audhsos-time` round-trip.
- The image the xtask writes is read back by the same crate and every
  file compares equal (this replaces the ad-hoc check of 6.6.15 without
  removing that item's other cases).

### 6.6.53 Fuzz support and symbolization (`fuzz-support`, `audhsos-symbols`)

- `fuzz-support`: the entry glue passes the input slice through
  unchanged, including the empty slice; the regression list replays every
  stored corpus file; Miri covers the counter registry and the sanitizer
  callbacks, which is where the crate's `unsafe` is (D-76).
- The watchdog's decision: a run that has not begun and a run without a
  limit outrun nothing; an input inside its limit and one past it are told
  apart; a tick inside the limit lets the process run on. The decision is a
  function of three numbers and not the body of the watchdog thread, so its
  coverage does not depend on whether that thread woke before the process
  ended.
- Symbol table: an address inside a function, at its first byte, at its
  last byte, and one past it; an address in no function; a symbol that is
  not a function; the narrowest of two functions that enclose each other;
  a function of no size; a file with no symbol table.
- Section header table: a section by name and by index; a section without
  content; a name in no section; a file with no section table; an entry
  shorter than the structure; a table that reaches beyond the file.
- Line program: a DWARF version 4 and a version 5 program; the standard
  opcodes, a special opcode sequence, and an end-of-sequence marker; an
  unknown standard opcode skipped by its declared length and an unknown
  extended one by its length; the sixty-four bit form of a unit; the forms
  a version 5 file table uses, an unknown one rejected, and more formats
  than the fixed capacity rejected; a directory or a file index no entry
  matches; an address before the first row and after the last; a version
  the crate does not read and a line range of zero are rejected; a
  truncated program is rejected without panic.
- Demangling: a path of a crate, a module, and a function; a method of an
  inherent implementation, of a primitive, and of a slice; every basic
  type letter; the shapes a type can have, a function signature, and a
  trait object; the shapes a constant can have; a trait definition and a
  disambiguated implementation; a backreference in a path and in a type; a
  closure inside a closure, where two identifiers of no length stand next
  to each other; a legacy name without its hash; a name the parser does
  not read is written unchanged; a backreference that points forward and
  nesting without end are refused.
- Property: no input file causes a panic and every lookup either yields a
  location inside the file's ranges or reports none; no name makes the
  demangler panic or loop.
- The xtask: the addresses of a trap report are found once each and in
  order, and a number too short to be one is left alone; a resolved
  address reads as one line; a report over a file that is not there says
  nothing and fails nothing.

### 6.6.54 IPv6 and Neighbor Discovery (`net-ipv6`)

The number follows the catalog rather than the layer, because items are
added and never renumbered; the crate belongs beside 6.6.44 (D-69).

- Header: a packet of the minimum length, one byte short, a payload
  length that disagrees with the buffer, and a hop limit of zero, each
  handled as specified. There is no header checksum to verify, which the
  tests state so that its absence is not read as an omission.
- The extension header chain: none, one of each kind, all four in the
  order RFC 8200 recommends, one that repeats, a chain longer than the
  bound, a header whose length field runs past the packet, and a chain
  that points at itself — the last three are drops and not loops.
- `ICMPv6`: an echo request answered, a packet-too-big delivered to the
  upper layer, and a checksum that covers the pseudo-header, which is
  where it differs from `ICMPv4`.
- Neighbor Discovery: a solicitation addressed to the solicited-node
  group of the target; an advertisement moving an entry to `Reachable`;
  the state machine through `Stale`, `Delay`, and `Probe` against a
  reference model with time as an argument; duplicate address detection
  refusing an address another node answers for.
  The reference model is the model-test runner's: one neighbor, the five
  states of RFC 4861, section 7.3.2 and the schedule of its section 10
  written beside the cache rather than out of it, driven by generated
  sequences of packets, advertisements, solicitations, waits, and polls.
  It names all eleven states and events it has to arrive at, so a
  generator that stopped reaching `Probe` or the giving-up fails instead
  of passing; and a second run against a model with one transition
  removed shows that a regression in that transition is found and shrunk.
  One neighbor and not several, because the cache holds its entries "in
  no particular order" and therefore does not say which of several due
  neighbors `poll` picks — a model that fixed that would be testing an
  accident. Capacity and eviction stay with `net-eth`.
- Router advertisements: a prefix and a router learned, an address formed
  by SLAAC, the lifetimes expiring, and the recursive DNS servers of
  RFC 8106 read out. An advertisement with a prefix length that is not 64
  forms no address.
- Path MTU discovery: a packet-too-big lowers the path MTU, the value is
  clamped below at the IPv6 minimum of 1280, and it is not raised by a
  message that claims more.
- Fragmentation: a datagram fragmented on send, reassembled by the
  machinery of `net-ip`, with overlapping fragments discarding the whole
  datagram.
- Fuzz target `ipv6`, whose corpus is seeded with the chains above.

Seven items were added while writing them, each because the code had a
boundary the list above does not name.

- A message that did not arrive with a hop limit of 255 is not read as
  Neighbor Discovery, and neither is one whose code is not zero or whose
  checksum does not verify. RFC 4861, section 7.1 makes all three
  conditions of reading the message at all, and the first is what keeps
  the protocol link-local.
- An option of length zero, one that runs past the message, and a
  trailing byte that cannot begin one are each a drop of the whole
  message; an option this crate does not read, and one whose body is not
  the length its type requires, are stepped over. RFC 4861, section 4.6
  draws the line there and it is not obvious from either side.
- The multicast mapping of RFC 2464, section 7, in `net-eth` beside the
  frames: the all-nodes group, a solicited-node group, and two addresses
  that share one because they differ above the low twenty-four bits.
- The clamp of RFC 8201 is tested where it acts, which is on the arriving
  message: a report below 1280 is discarded, and a link narrower than
  1280 is answered as it stands so that the send path refuses it.
- The floor of RFC 4862, section 5.5.3 (e), as its three rules and as the
  attack it exists for: an advertisement carrying a valid lifetime of
  none, of a second, or of a minute leaves a held address alone; one
  above two hours or above what is left is taken as given; one below both
  brings the lifetime down to two hours and no further. A prefix this
  host formed no address under is withdrawn at once, and one whose
  preferred lifetime exceeds its valid lifetime is ignored (section
  5.5.3 (c)).
- A withdrawn prefix and a withdrawn DNS server leave through `poll`, so
  that the caller which installed a route for the prefix hears that it
  must go. A prefix or a server this host never held is not taken up by a
  withdrawal either.
- The two halves of RFC 4861, section 7.2.5 I, in `net-eth` beside the
  cache: an advertisement without the override bit whose address
  disagrees takes a `Reachable` entry to `Stale` without touching the
  address, and moves an entry in any other state not at all.
- The model-test runner itself, for the requirement it grew: a run that
  reaches what it requires passes, one that reaches nothing fails though
  nothing disagreed, a requirement that is reached is not reported beside
  one that is not, and a disagreement is reported before a missed
  requirement.

### 6.6.55 Wide arithmetic and RSA (`crypto-bignum`, `crypto-rsa`)

Built with steps R1 to R3 of document 11, section 11.15. The number
follows the catalog rather than the layer, as 6.6.54 records.

- The modulus: a value that is even, one that is zero, one wider than
  `MAX_LIMBS`, and one whose top limb is zero are each refused by
  `Modulus::new`; an accepted one has zero in every limb at or above its
  used count, which is the invariant the rest of the crate rests on.
- The derived constants: `n0inv` is the *negative* inverse the reduction
  step wants, so `n0inv` multiplied by the low limb of the modulus is
  minus one modulo `2^64`; `R2` agrees with `2^(128*used)` reduced by the
  reference, for a modulus of each of the four widths.
- Montgomery multiplication and squaring agree with a schoolbook
  reference in the test module on random inputs at 1024, 2048, 3072, and
  4096 bits (property), in the form 6.6.33 uses for `fe25519`. The
  reference is the one place where the arithmetic is written twice on
  purpose.
- Exponentiation: `pow` against the reference for small exponents, for
  65537, and for an exponent with its top and bottom bits set; a round
  trip that signs with a wide exponent and verifies with a small one over
  the key of RFC 8448, section 2.
- The secret exponentiation, which is the half of the crate written for
  a value that must not be observable: `pow_secret` against the same
  schoolbook reference on exponents the generator chooses, at each of the
  four widths (property), and against `pow` on the exponents that one
  takes. An exponent of zero gives one however many bytes it is written
  in, and leading zero bytes change the rounds and not the value — the
  two properties that say the ladder runs over the buffer rather than
  over the value. A base that is not below the modulus, one wider than
  the arithmetic, and an output buffer narrower than the modulus are each
  refused; the last is refused before the ladder starts, because
  refusing afterwards would be a decision about the value.
- The masked product: `montgomery_secret` against `montgomery` over every
  case of the final subtraction, the one it happens in and the one it
  does not, and separately against the definition, so that the two
  variants agreeing is not the only thing checked.
- The key: the bounds of D-79 at each edge — an exponent of one, of two,
  of four, and of three; a modulus above the upper bound, and one whose
  top bit is clear — refused where the rule says and accepted where it
  does not. The lower bound is not among them, and a test says so by
  name: the thousand-and-twenty-four-bit key of RFC 8448 is accepted
  here, because that bound belongs to `audhsos-x509` and is checked in
  6.6.36.
- PKCS #1 v1.5, the positive direction: a signature this crate made
  verifies, for SHA-256, SHA-384, and SHA-512, and the encoded message it
  builds matches the `DigestInfo` prefixes of RFC 8017, section 9.2
  note 1 byte for byte.
- PKCS #1 v1.5, the negative direction, which is what the construction of
  D-80 is for: padding shorter than eight bytes of `0xff`; a
  `DigestInfo` moved inside the block with the padding adjusted to fit; a
  digest followed by trailing bytes; a missing `0x00` separator; a first
  byte that is not `0x00`; a second that is not `0x01`; a `DigestInfo`
  whose `SEQUENCE` carries an indefinite length, which PKCS #1 v1.5
  allowed and this crate refuses; and a forgery constructed against an
  exponent of three.
- PSS: a signature this crate made verifies for each of the three hashes;
  and each rejection rule separately — a trailer that is not `0xBC`, a
  leftmost bit set where `emBits` says it must be zero, a separator that
  is not `0x01`, a salt of a length other than the hash output, and a
  recomputed `H'` that does not match.
- The one vector from outside: the `CertificateVerify` of the simple
  1-RTT handshake of RFC 8448 verifies as `rsa_pss_rsae_sha256` under the
  key that document's section 2 prints. It is 1024 bits and therefore
  never reaches a chain (D-79); it reaches the primitive. The test lives
  in the replay of 6.6.38, because that is where the transcript the
  signature was made over is computed.
- Fuzz target `rsa`: a key and a signature from the same input, parsed
  and verified, must not panic and must not loop. Everything it reaches is
  bounded before it runs — the modulus by `MAX_LIMBS`, the exponent by the
  sixty-four bits it is read into, the encodings by the width of the key —
  so a verification that took a long time would be a bound that is
  missing, and the engine would find it as a timeout. The one thing
  asserted of a key that parses is that it reads back as the key it was
  built from. The corpus holds a valid signature under each of the six
  schemes, the eight malformed encodings above, and two inputs that are
  not a key at all.

### 6.6.56 Protocol encodings (`user-proto`)

The catalog covered these messages only through the end-to-end items of
6.6.22, which is a test of the whole system and not of an encoding. This
item is what 12.9 asked for before the encodings could be written
(D-89).

- Labels: the version, the protocol, and the message number come back out
  of a label they were built into; every label of every protocol lies
  below the range the kernel keeps for its own messages; a bit set between
  the version and the protocol is refused; a version this release does not
  speak is refused, and it is refused *before* an unknown protocol, so
  that a client of a later release is told which of the two it is; a
  protocol code and a message number the release does not have are each
  refused.
- Round trip, per message of each of the six protocols: what was encoded
  decodes to what it was. The names and the chunks are tested at zero
  bytes and at the full width of their field.
- Replies: every reply carries a status word first, and the payload only
  behind a status that says the request succeeded — a failed lookup
  carries no endpoint, a failed allocation no memory object, a failed
  write no count. Each is checked on the encoded message and not only
  through the decoder.
- Every error of the interface travels: a status word built from each
  `Error` of the table reads back as that error, and a word that names no
  error is refused.
- Truncation: a message whose counts are cut so that a field is missing —
  the handle of a registration, the alignment of an allocation, the object
  of a release, the count of a write — is refused rather than read as a
  zero.
- A byte string whose length word is longer than the field that carries it
  is refused where it is read, which is the case an encoder of this crate
  cannot produce and a sender of another release could.
- A message of one protocol handed to the decoder of another is refused
  with both protocols named. A `SetCursor` whose shape word names no
  sprite is refused the same way, because a shape is a name and not a
  number the server clamps.
- The parent protocol has one message and no reply, because the child that
  sends it exits behind it (D-94): the status it carries comes back, a
  message number the protocol does not have is refused, and a report
  without its status word is refused rather than read as a zero.
- The input protocol carries two messages and its records live in shared
  memory rather than in a message. An event record is sixteen bytes, its
  key and pointer forms round-trip, the bytes the kind does not use are
  zero, and a record whose kind byte names neither, whose reserved byte is
  not zero, or whose key code names no key is refused. A ring over one page
  holds 254 records: what the writer pushes the reader pops in order, the
  sequence numbers stay contiguous across the wrap, a full ring drops the
  newest event and counts it, the reader clears that count when it reports
  it, and a record somebody wrote nonsense into is stepped over rather than
  read for ever. A subscription carries a notification and a process handle and its reply carries
  the memory object only behind a status that says it succeeded.
- The layouts of the client side: a word typed on `us` comes out as that
  word, the German layout swaps the two letters the United States layout
  calls `Y` and `Z` and carries the umlauts, shift picks the other
  character of a pair, the lock turns over on the press and changes the
  letters only, the modifier state follows press and release, a release
  without a press changes nothing, and a key that stands for no character
  answers nothing.

### 6.6.57 The wrappers of the gate against the table (`user-sys-x86_64`, QEMU)

The constant assertion beside the wrappers holds a list of values to the
system call table; it cannot see the methods themselves (D-92). This item
is the other half, and it needs a machine: the numbers a wrapper writes are
what the kernel dispatches on, so the check is what the kernel saw.

- `every_wrapper` calls every method of `Gate` in the order of the
  table, each with a handle that names nothing, so that every call is
  refused and none of them waits for a partner or ends the thread;
  `thread_exit` is last, because it does not come back.
- The kernel writes down the number each call arrived under and the image
  holds the sequence against `Syscall::ALL`, with `thread_exit` moved to
  the end: as many calls as the table has entries, each entry in its place,
  and no entry without one.
- Each of the three failures is checked against the code it names: two
  wrappers whose calls are swapped, a wrapper the program does not call,
  and a call of the table with no wrapper that reached the kernel.

### 6.6.58 The document toolchain (`audhsos-deflate`, `doc-markdown`, `doc-html`, `doc-svg`, `doc-pdf`, `docpdf`)

- Bits and codes (`audhsos-deflate`): a bit written is the bit read, and a
  Huffman code goes out with its first bit first; a writer with no room and
  a reader that runs out say so instead of wrapping; what is aligned is
  written whole; the fixed code is the one RFC 1951 states, and every
  length and every distance is written as a code that means it again; a
  tree of no symbols reads nothing and a tree of one symbol is still a
  code; more symbols than a tree holds are dropped and not written past; a
  code length the format does not have is ignored; `bound` counts the
  blocks a result can take.
- Streams (`audhsos-deflate`): nothing, one byte, a run of one byte, and a
  run that reaches back into itself all come out again; text that repeats
  gets smaller, and what will not compress is carried unchanged, in more
  than one stored block when it is long; a zlib stream carries its wrapper
  and its Adler checksum; a wrong checksum, a wrapper the crate does not
  know, a buffer with no room, and a stream of nonsense are each refused,
  the last of them without running away. Property: whatever goes in comes
  back out, whether it repeats or not. Vectors from outside the crate: a
  block with a table of its own, a block that carries its bytes, and a
  zlib stream made elsewhere all read back as the sentence they were made
  from.
- Blocks (`doc-markdown`): an empty document has no block; an ATX heading
  carries its level and a hash without a space is not one; a setext
  underline makes the line above a heading; a fence keeps its lines
  exactly, may contain the other fence character, and ends with the
  document when it never closes; four spaces and a tab indent alike; a
  quotation holds blocks of its own and ends where a block of its own
  begins; a list keeps the number it starts at, may hold a list or a code
  block in an item, becomes two lists when its kind changes, and ends
  after two blank lines; a table reads its alignments, pads a short row,
  and stays a paragraph without its delimiter row; an escaped pipe does not
  split a cell. Property: the parser never invents text.
- Inline runs (`doc-markdown`): strong and emphasis are told apart and
  nest; an underscore inside a word is a character, so `saturating_add`
  stays an identifier; a code span keeps what is inside it, may contain a
  backtick, may run over a line break, and drops one space at each end; a
  link keeps its text and its target and neither loses the brackets it
  balances; an autolink and a mail address are links and a generic in angle
  brackets is not; a backslash hides the mark behind it and stays itself
  before anything else.
- Syntax (`doc-html`): character references by name, by both bases, and in
  the legacy forms that need no semicolon, with an unknown one left as it
  was; attributes quoted, single-quoted, bare, and alone, with the name
  lower-cased and the value not; comments, doctypes, and instructions read
  and dropped; the content of a script or a style is text and not markup,
  and an unterminated comment, tag, or script ends the document; an element
  left open is closed at the end, an end tag that names nothing open is
  dropped, and the implied end tags of paragraphs, items, cells, rows, and
  terms fire where real documents rely on them.
- Blocks (`doc-html`): text outside any element is still a paragraph and
  the whitespace of the markup becomes one space; a heading counts the
  sections around it and never goes deeper than six; lists, tables,
  quotations, notes, terms, captions, and rules come out as the blocks the
  Markdown parser produces; preformatted text is kept line for line and a
  grammar gets a line per production; what carries no prose is dropped with
  everything in it, and an element the tables do not name is read for its
  children; a link out of the document keeps its address and one into it
  keeps only its text.
- Parts (`doc-svg`): numbers as thousandths, with an exponent, as a list
  however it is separated, and a length in a unit other than pixels
  refused; the transforms `translate`, `scale`, `matrix`, and `rotate`,
  applied left to right, with an unknown one changing nothing; colours by
  word, by digits, and by components; the stylesheet cascade, where the
  stronger selector stands last and a selector the crate does not read is
  dropped; every path command except the elliptical arc, which is the
  straight line to where it ends, with a smooth curve mirroring its
  control and a quadratic becoming the cubic it is; a path of nonsense
  stops where the nonsense starts; rectangles with and without round
  corners, circles, and polygons, each of no size yielding nothing.
- Drawings (`doc-svg`): a figure is as large as its view box says, is
  moved to the origin when the box does not start there, and is turned over
  so that y grows upwards; a group hands its transform and its colours
  down; a `use` draws what it points at where it stands and nothing when it
  points at nothing; a figure that points at itself does not run for ever;
  an anchor moves a line of text back by what it measures; a marker is
  drawn at the end of the stroke it belongs to and grows with the stroke
  when it is measured in stroke widths; a placed figure keeps its text and
  its curves at every point they bend through.
- Text and paths (`doc-pdf`): a page of prose is one text object; a run
  that begins where the last one ended says nothing about its place and one
  a hair away still does; a face is named again only when it changes and a
  colour is set once for as long as it stands; the delimiters of a string
  are escaped and a high byte is written as an octal escape; a rectangle
  with no area, a rule of no thickness, and a path that is neither filled
  nor stroked are not drawn. Fonts: every face has its own resource name;
  the fixed-pitch faces advance by the same amount everywhere; bold is
  never narrower than regular; a character the encoding has no place for is
  offered the symbol font first and is a question mark when neither font
  has it; width grows with the point size.
- Files (`doc-pdf`): a file starts with the header and ends with the
  marker; the page tree names every page, every stream declares the length
  it has, the cross-reference table points at every object, and no two
  objects carry the same number; the outline is a tree the catalogue points
  at, a heading that skips a level is lifted, and a document with no
  heading has none; both kinds of link reach the page and a page with no
  link carries no annotation array; a compressed document says so, reads
  back, is smaller, and changes nothing else, and a page with nothing on it
  is not compressed. Property: two runs over the same document write the
  same bytes.
- Filing and rendering (`docpdf`): a document is filed by what it is, a
  crate is named after its package and falls back to its directory without
  a manifest, what a build wrote is not a document, a document no rule
  claims is still converted, and none is converted twice; the title is the
  first heading, or the file name where there is none; an RFC is called by
  its number, laid out one page per sheet, keeps every column where it was,
  and gives up its sections as an outline while a table-of-contents line
  does not; a picture alone on its line is drawn and one inside a sentence,
  one whose file is missing, and one the crate cannot draw are said
  instead; a link between two documents becomes a link between two files
  and one to something that was not converted is left alone.
- Breaking and the pool (`docpdf`): a line that does not fit is broken at a
  space, a word wider than the measure is cut rather than left hanging, and
  a break the author asked for is kept; a code span is measured in the face
  it will be set in; a link carries its target down to the piece and strong
  inside a link is both. Property: breaking never loses a character and
  never overruns the measure. The worker pool does every job exactly once
  and returns the results in the order the jobs were given, and one worker
  gives the same answer as many.
- Against the repository itself, as the corpus that is at hand: every
  Markdown document of the repository parses into blocks without losing a
  fence or emptying a heading; the HTML standards come out as documents
  with nothing left as markup; every SVG figure comes out as marks that
  stand inside it and keeps its shape when it is placed.

### 6.6.59 The clock and the thread that waits until (`kernel-sched`, `kernel-syscall`, QEMU)

- `IndexList::insert_after` (`audhsos-collections`, and the item belongs to
  6.6.41 as well): an insert after `None` is a `push_front`; after the tail
  is a `push_back`; in the middle links both neighbours; a node that is
  already linked, one outside the slice, and an `after` that belongs to
  another list are each refused and change nothing; the length and both
  ends are what a model built from a vector says after the same sequence.
- Microseconds from ticks: zero ticks is zero; one tick is the frequency's
  reciprocal rounded as the conversion states; a tick count that would
  overflow the microsecond saturates rather than wrapping, so a machine
  left running does not travel backwards.
- The deadline list: an insert into an empty list, at the front, in the
  middle, and at the back; two threads with the same deadline both come
  out and in the order they went in; a thread signalled before its
  deadline leaves the list in the same call that changes its state and is
  not woken twice; a thread that exits while it waits leaves no entry
  behind. Against a model that keeps the same deadlines in a sorted
  vector, the same sequence of inserts, removals, and expiries yields the
  same threads in the same order.
- Expiry: a deadline exactly at `now` expires; one a microsecond later
  does not; a tick that expires nothing costs one comparison, which is
  the walk stopping at the front; every thread of a list whose deadlines
  have all passed comes out in one tick.
- `notification_wait_until`: bits already present are taken and the
  deadline is never consulted; a deadline in the past returns at once with
  no bits; a signal before the deadline returns the bits; a deadline that
  comes first returns zero; the wrong right on the handle is
  `AccessDenied`, and a handle of the wrong type is `WrongObjectType`, as
  `notification_wait` answers them.
- `clock_now`: takes no handle, takes no argument, and needs no right; two
  calls with a wait between them differ by that wait to within one tick,
  which is the QEMU assertion.

### 6.6.60 Randomness and message interrupts (`kernel-hal-x86_64`, `kernel-syscall`, QEMU)

- `RDSEED` through the scripted double: four words filled at the first
  attempt; a word that fails once and succeeds inside the retry bound;
  a word that fails through the whole bound is `Unavailable`, a code the
  phase adds, and no partial result reaches the caller; the retry counter is per word and
  not per call.
- `random_bytes`: four result words; two calls in one program differ,
  which is the QEMU assertion and is a smoke test and not a statement
  about the distribution.
- The vector allocator: a vector is handed out once; a vector freed with
  its interrupt object is handed out again; an exhausted space is
  `NoVector` and not a reused vector; the message address and data are
  the ones the fixed-delivery, edge-triggered encoding states for the
  vector.
- `interrupt_create_msi`: the right `MANAGE` on `SystemControl` is
  required; the object it makes has no line, so `interrupt_bind` binds it
  as it binds any other and `interrupt_ack` on it calls no controller and
  clears the outstanding flag; an `interrupt_ack` on an object that has no
  outstanding signal is not an error.
- In QEMU: an MSI vector created by the root task and raised by a test
  kernel arrives as the bit it was bound to.

### 6.6.61 The bus (`kernel-acpi`, `pci`, QEMU)

- The window the kernel keeps (`kernel-core`): `system_info` reports the
  base address, the segment group, and the first and last bus as its words
  twenty-six to twenty-nine, and four zeros on a machine whose firmware
  published none; the range is among the device apertures even when the
  memory map marked no region at all, a frame above it belongs to none, and
  `boot::run` writes `[info] ecam=<base> segment=<n> buses=<first>..=<last>`
  or `[info] ecam=absent`.
- `MCFG` (`kernel-acpi`): a table with one allocation and one with several;
  a wrong signature, a length below the header, a length beyond the
  buffer, and a bad checksum are each refused before any field is read; an
  allocation whose last bus is below its first, and one whose base is not
  page aligned, are refused; more allocations than the bounded array holds
  are refused rather than truncated silently.
- ECAM arithmetic (`pci`): bus, device and function at their lowest and
  highest values land at the offsets the mechanism states; an offset
  beyond the configuration space of a function is refused; the window
  length follows from the bus range and a function outside that range is
  never addressed.
- The header: an absent function reads `0xFFFF` as its vendor and is not
  an error; a header type this crate does not read is reported and
  skipped; bit 7 of the header type makes a device multi-function, and
  without it only function zero is read.
- Base address registers: a 32-bit memory register, a 64-bit one that
  consumes the next index, an I/O register, and a register that reads
  zero; size probing restores the command register it cleared, and
  restores it also when the probe found nothing; a 64-bit register in the
  last index is refused rather than read past; an index the previous
  register consumed is not read again.
- The capability list: no capabilities; one; several; a pointer that
  points at itself and a chain that returns to an earlier entry are each
  refused by the bound and never hang; a pointer below the header, and one
  beyond configuration space, are refused.
- MSI-X: the table size is the encoded field plus one; the enable and
  function-mask bits are read and written where the specification puts
  them; the table and the pending-bit array may name different base
  address registers; a table entry the crate writes carries the address,
  the data, and a clear mask bit at the offsets stated.
- The virtio capabilities: the four structures the driver needs are found
  by their type; the notify capability carries its multiplier and the
  other three do not; the three types the specification defines and this
  driver does not use are recognized and passed over, and a value the
  specification reserves is skipped and does not end the walk; a
  capability whose length is shorter than its type needs is refused, and
  so is one naming a BAR index outside `0` to `5`. Two structures of the
  same type both come back, in the order the capability list had them,
  which is the device's order of preference, and the crate chooses
  neither.
- Against the recorded configuration space of a `q35` machine: the
  virtio-net function is found at its address, its four structures and its
  MSI-X table are read back, every base address register decodes to the
  range the machine reports, and the device id is `0x1041`, which is the
  network device's `0x1040 + 1` of virtio section 4.1.2 and not the
  transitional `0x1000`.
- Fuzz target `pci_config`: arbitrary bytes as a configuration space;
  enumeration terminates with devices or with an error, reads nothing
  outside the buffer, and never loops.
- Fuzz target `mcfg`: arbitrary bytes as a table; the parse answers or
  refuses and reads nothing outside the buffer.
- The volatile accessor (`user-sys-x86_64`, QEMU): every read and write
  answers inside the region it was made with and `None` outside it, and a
  sub-window narrows a region and reaches no further. It is proved by the
  bus walk of `app-lspci`, which reaches the configuration space of a real
  machine through nothing else.
- End to end (QEMU): the run of the reference machine carries
  `[info] ecam=`, the window `app-lspci` was given, the virtio-net function
  with the device identifier `0x1041`, the four structures it published in
  whatever order it prefers, and a message table of four vectors; the run
  without the two network lines carries the host bridge, no virtio device,
  and ends by itself.

### 6.6.62 The network device (`driver-virtio-net`)

- Negotiation: a device offering exactly the two features the driver wants
  is accepted; one offering neither `VIRTIO_F_VERSION_1` is refused as a
  legacy device; each refused feature is refused by name, and the table of
  names covers every bit the driver reads; a device that does not clear
  `FEATURES_OK` after the driver set it fails initialization at that step
  and not later.
- Initialization: the sequence reaches `DRIVER_OK` over the state machine
  of `virtio-queue`; a failure at each step leaves the device in `FAILED`
  and reports which step; a queue whose size the device reports as zero is
  a device without that queue and is refused; the MSI-X vector is
  configured per queue and the device's rejection of a vector is read back
  and reported.
- Receive: every buffer is in the available ring after initialization; a
  used element yields the frame behind its twelve-byte header; the buffer
  is back in the available ring in the call that took it, so the device is
  never left with fewer buffers than the driver believes; a used element
  whose length is below the header, one whose length exceeds the buffer,
  and one naming a descriptor that is free are each refused and the
  element still consumed.
- Transmit: a frame goes out behind a zeroed header as one chain; the
  notify write lands at the queue's own offset with the multiplier
  applied; completions are drained before the next send; a send with no
  free buffer is refused and changes nothing, so the caller may retry;
  a frame longer than a buffer is refused before anything is written.
- The device configuration: the MAC address is the six bytes at the
  offset the specification states; a device that did not offer
  `VIRTIO_NET_F_MAC` yields none rather than six zeroes.
- Fuzz target `virtio_net_rx`: a used element and a buffer of arbitrary
  bytes; the driver yields a frame or refuses, and never reads outside the
  buffer.

### 6.6.63 The network server and the socket protocol (`server-net`, `user-proto`)

- Encodings: every message of the protocol encodes and decodes to itself;
  a truncated message, one with a length field that disagrees with its
  buffer, and one with a kind byte the protocol does not have are each
  refused; a socket handle of a generation that has passed is refused
  rather than answered for the socket that reused the slot.
- The ring: a write and a read of one record; a ring exactly full; a
  reader that stops and a writer that therefore stops; sequence numbers
  that wrap; a capacity that is not a power of two is refused at creation.
- The loop: a frame in produces the frames out that the stack produces; a
  poll that yields nothing does not wake the server again before the
  deadline `poll_at` gave; a client request and a device interrupt arrive
  under different badges and are told apart by them; a device that reports
  no link at startup makes the server report no interface and exit.
- The deadline word: a deadline written while the timer thread sleeps on a
  later one wakes it and is the one it then keeps; a deadline written
  while it sleeps on an earlier one does not move that wake earlier and
  the tick it sends is answered with a poll that finds nothing to do; a
  deadline that has already passed makes the tick immediate; `poll_at`
  answering `None` parks the timer thread with no deadline at all.
- Against the network double of `net-stack` and a scripted device: a
  lease is taken and renewed, a name is resolved, a connection is opened,
  carries bytes both ways and closes, each with the clock advanced by the
  test rather than by a machine.
- Back pressure: a client that never reads fills its ring, the window
  stops advancing, and nothing in the server grows; the same client
  reading again lets the connection continue.

### 6.6.64 The network end to end (QEMU)

- The driver reports the MAC address the command line gave the device.
- DHCP reaches a lease, and the address is the first one the built-in
  server hands out.
- ARP resolves the gateway before the first datagram leaves for it.
- A DNS query the forwarder answers comes back with an address.
- A TCP connection through the forwarded port carries a payload both ways
  and closes cleanly, with the close seen from both ends.
- An HTTP `GET` over that connection returns a response the client parses
  into a status line and a body.
- With the two network lines dropped, the server reports no interface and
  the run ends by itself, as the display server ends on a machine with no
  framebuffer.

### 6.6.65 TLS on the target (QEMU)

- An HTTPS `GET` against a server the test starts on the development
  machine, with a chain the test certificate builder wrote, returns a
  status line the client parses.
- A chain with an expired certificate, one whose name does not match, and
  one signed by an anchor the image does not carry are each refused, and
  each with the alert the standard names for it.
- A connection the peer closes without `close_notify` is reported as a
  truncation and not as a clean end.
- The handshake respects a deadline: a peer that stops answering ends the
  attempt at the deadline rather than blocking the server.


### 6.6.66 Finite-field Diffie-Hellman (`crypto-dh`)

Step S2 of 8.26, the half of it that is built (D-122, D-123). The
constant-time exponentiation this rests on is in `crypto-bignum` and is
covered by 6.6.55.

- The group of RFC 3526, section 3: the prime is the value the document
  prints, checked against a second transcription of the same rows so that
  the constant and the test do not share one slip; it is 2048 bits wide,
  its two ends are the ones the closed form of that section gives it, and
  the generator is the two the document states. The exponent length the
  crate recommends is 256 bits, which is above the 112 bits of security
  RFC 9142, table 4, gives the group.
- The generator raised to a small exponent is that power of two, for
  exponents whose result stays below the prime and is therefore
  expressible without a second exponentiation; raised to the width of the
  prime it is that power reduced exactly once, which the test computes by
  subtracting the prime rather than by exponentiating again.
- An exchange between two sides reaches one secret, and does so for
  exponents the generator chooses (property). The same exchange runs over
  the 1536-bit group of section 2 of the same document, so that nothing
  in the code is tied to one width.
- The range check of RFC 8268, section 4, which corrects RFC 4253,
  section 8: a value inside the open interval is accepted, and zero, one,
  `p-1`, `p`, `p+1`, and a value wider than the arithmetic are each
  refused. A value carrying leading zero bytes is accepted, because that
  is the shape an SSH `mpint` has once its length prefix is gone.
- An exponent of zero produces a public value that must not be sent and a
  shared secret that must not be used, and both are refused where they
  are produced rather than passed to a caller.
- Both operations refuse an output buffer narrower than the group, and a
  public value may be written into one wider than it.
- A group whose generator is below two, and one whose prime the
  arithmetic cannot hold, are refused by the constructor.
- Every refusal renders a sentence of its own.

What is not here, and belongs to step S8: a handshake against an
implementation this project did not write. No document publishes a
complete SSH key exchange with the values that made it, so the arithmetic
is checked against a reference inside the repository (6.6.55) and the
protocol above it is checked against a live OpenSSH.
### 6.6.67 JavaScript core (`jrs`, `jrs-cli`)

- Lexical boundaries: all implemented radix literals and separators; malformed
  digits, exponents and escapes; comments and ASI line terminators; legacy
  literals and unsupported syntax rejected before host effects occur.
- Primitive semantics: NaN, both signed zeroes and infinities; primitive
  conversions and strict/loose equality; UTF-16 surrogate preservation and
  code-unit ordering; short-circuiting and conditional evaluation order.
  Bitwise operations truncate modulo 2^32, shifts mask the count modulo 32,
  signed shifts propagate the sign and unsigned shifts fill with zeroes.
  Property tests compare loop sums and integer conversions with integer models
  and escaped strings with arbitrary UTF-16 code-unit sequences.
- Bindings and control: duplicate declarations, lexical shadowing, TDZ before
  initialization (including `typeof`), const assignment, re-entered scopes,
  nested loops, break and continue, and a fresh environment on every run.
- Budgets: exact source/stack/string/token/bytecode/depth boundaries and
  infinite loops stopped by fuel; a failed host or exhausted run does not
  poison the next execution. Fuzz target `jrs_source` compiles arbitrary
  UTF-8 and executes accepted programs under small explicit limits.
- CLI: expression, stdin and file inputs, usage errors, invalid UTF-8, bounded
  reads, output failures, exit failure paths, and compile-once timing options.
- Functions: declaration hoisting, recursion, named self-bindings, first-class
  native calls, arrow parameter uniqueness and inherited strict directives;
  `return` ASI and missing return values; shared captures, independent calls,
  multi-level captures, lexical versus var loop bindings, nested call errors,
  and per-run function identity. Heap collection preserves roots and frees
  unreachable cycles, rejects stale generations, and reports exhaustion;
  recursive calls stop at the frame or fuel limit without using Rust recursion.
- Objects: UTF-16 keys and lone surrogates, computed keys evaluated once,
  shorthand and duplicate data keys, duplicate literal prototype setters,
  inheritance versus own writes, non-writable/non-configurable data properties,
  integrity levels, `SameValue` for NaN and signed zero, method receivers,
  lexical arrow receivers, property/capture/prototype cycles and live GC roots;
  numeric key order before string creation order, deletion/reinsertion order,
  property quotas and bounded nesting of object literals.
- Exceptions: exact thrown identity, cross-frame throws, catch scope, strict
  throw line terminators, finally on normal/return/throw/break/continue,
  overriding completions and pending-value GC roots. Embedding resource
  and host failures cannot be swallowed by JavaScript catch clauses.
- Accessors/coercion: original receivers, descriptor field getter order,
  data/accessor conversion and non-configurable validation, frozen setters,
  valueOf/toString hint ordering, double array-length conversion, Math extrema
  coercing all arguments even after NaN, signed-zero selection, real host
  effects through getters, native exception boundaries and intermediate GC
  roots. Recursive native reentry returns a resource error before stack overflow.
- Regular expression engine: the dedicated `crates/regex/` contract requires
  Thompson NFA/DFA operation bounds and dedicated fuzzing; no backtracking
  implementation or fallback is permitted inside the automaton crate. Memory, states and compile expansion
  need independent quotas even with linear text-length matching.
- Arrays: holes versus undefined, uint32 length limits and property indices,
  length shrink rollback at non-configurable elements, frozen length, generic
  methods, callback validation/order/mutation/early exit, receiver forwarding,
  inherited elements, private callback helpers unaffected by script overrides,
  GC and shared quotas, and vector/sparse property models. Constructors cover
  prototype inheritance, primitive versus object returns, exceptions/finally,
  optional argument lists, property constructors, and non-constructible arrows
  and methods.
- Templates/control/parameters: nested template braces, comments and RegExp
  literals within substitutions, UTF-16 escapes, CRLF normalization, immediate
  string-hint conversion before later substitutions; default/rest TDZ, earlier
  parameter access, default closures isolated from body vars, omitted versus
  undefined versus null arguments, strict parameter restrictions and function
  length; switch selector short-circuiting/fallthrough/default placement,
  break/continue across switches and finally; enumerable-key order, shadowing,
  deletion and prototype traversal, for-of array mutation/string code points,
  nested array loop binding patterns, per-iteration cells and temporary GC roots.
- Promise/async: synchronous executors and delayed FIFO reactions, repeated
  resolve/reject calls, then getters versus queued then calls, self-resolution,
  adoption and chaining, finally pass-through and override, combinator order,
  empty inputs, await of values/pending promises/rejections, suspension inside
  loops/catch/finally, host effects, pending-frame GC roots, reclamation of
  unreachable pending cycles, and non-catchable job/frame/operand/fuel limits.
  Unhandled rejections must reach the host checkpoint policy.
- WPT/Test262 acceptance is separate from local regression coverage. A parse
  error in the original harness is a failure, never a passed or skipped test.
  No claim of browser compatibility follows from host-core execution.
- WPT shell integration: use `jrs --wpt ROOT FILE...` with the original checkout
  harness and scripts. Require exactly one successful completion, nonzero
  subtest results, matching result totals, and no failing assertions. Extraction
  and reporting fixtures test transport only, not conformance. Modules, URL
  variants and async/defer HTML scripts fail explicitly; no browser scheduler
  or DOM is claimed. The core README records pinned results and failures.
- Builtin progression: stable sort/hole preservation, comparator exceptions and
  GC during mutation; complete descriptor collection before definitions;
  mapped/unmapped arguments and freeze detachment; apply/bind forwarding;
  Error prototype identity/cause; UTF-16 split/replace/substitution and bounded
  class lookahead. Regex complexity tests still require one visit per state
  and input position, including failed assertions and greedy captures.
- Boxing: distinct wrapper/prototype identity and brands, sloppy versus strict
  this receivers, inherited primitive accessors, virtual immutable UTF-16 string
  properties and key ordering, generic array methods boxing once, GC roots and
  enumeration quotas. Regex split adds capture/empty-match/limit/lastIndex and
  flags-getter ordering cases; differential probes use isolated reference realms.
- Weak associations: identity-keyed WeakMap updates/deletions and constructor
  getter order; native/resolving-function keys; getOrInsertComputed mutation;
  dead maps, live keys, unrooted key/value cycles, long dependency chains and
  reused heap generations. Generated ephemeron graphs are compared with an
  independent least-fixed-point reachability model. Entry and collector-work
  quotas must fail explicitly; live associations survive callback-triggered GC.
- Symbols: identity versus description, registered/well-known identities,
  string/Symbol property separation, symbol descriptor and accessor ordering,
  freeze/seal/delete, hidden symbols in string-key enumeration, conversion hints,
  tag getters and thrown values, property-key GC roots and ephemeron collection.
  Registry/property/string/heap quotas must still terminate hostile inputs.
- Iterators: cached next methods and ordered done/value getters; live-array
  mutation, string code points, next reentry, iterator brands and completion;
  return on break/return/throw and binding patterns, elisions without value
  access, nested finally order, original-throw precedence and non-catchable host
  failure. Custom iterables feed Promise combinators and WeakMap; suspended
  frames keep iterator/next/source roots alive across GC. Infinite iterators
  are fuel-bounded and are not run unbounded in reference-engine comparisons.
- Classes: strict method grammar, class-name TDZ and immutable inner binding,
  computed/static methods and accessor descriptors, default constructor argument
  forwarding without iterator hooks, derived this TDZ/duplicate initialization,
  constructor return rules/new.target, lexical arrow this/super, dynamic home
  prototype lookup, native-subclass prototype visibility and explicit constructor
  frame limits. Unsupported fields/private/static blocks remain explicit errors.
- Promise capabilities/species: custom constructors receive an executor that is
  callable but not constructible; repeated calls and invalid resolver pairs are
  rejected. Test species getter order, identity preservation, generic resolve,
  reject and combinators, cached resolver functions, reentrant thenables, finally
  without public resolve lookup, callback GC, resolver exceptions and shared
  resource limits. Original WPT Promise-subclassing must run without rewriting.
- Persistent realms: independent script parsing and strict directives, global
  declaration conflicts before publication, var/property synchronization,
  lexical TDZ across failed scripts, closures resolving later declarations,
  prior-code GC roots, pending await resumed by a later script, per-script jobs,
  cumulative quotas and realm poisoning on embedding failure. Runner fixtures
  assert no cross-script hoisting and a checkpoint before the next script.
  The jrs_source fuzz target exercises isolated and multi-script realm execution.
- Embedding boundary: host function identity/attributes and non-constructibility,
  exact receivers, delayed JavaScript callbacks/checkpoints, descriptor accessors,
  fallible conversions, callback GC roots, release/stale handles and independent
  realm ownership. Wrong-kind/foreign host returns and thrown values are fatal;
  ordinary language errors remain catchable. Fuzzing installs echo/throw host
  functions and invokes returned callbacks/property access through the Rust API.
- Microtasks: opt-in installation, callback conversion, no arguments, strict/sloppy
  receivers, ignored return values, Promise/thenable FIFO interleaving, recursive
  enqueue, GC after host release, distinct exception versus rejection reporting,
  drain-after-reported-error and immediate fatal-host failure. Original
  queue-microtask.any.js runs unchanged. ErrorEvent/MutationObserver/cross-realm
  tests still fail and must not be counted as successful window/worker runs.
- Concat: holes and inherited elements, isConcatSpreadable getters before length,
  species construction before spreading, ordinary-object result descriptors,
  final length setter, mutations during getters, GC, u32 index and fuel quotas.
- Standalone events: listener type/callback/capture identity, per-phase snapshots,
  once removal before recursion, removal/re-addition, passive cancellation,
  propagation flags, composedPath cleanup, Event/CustomEvent initialization,
  callback receiver/handleEvent lookup, exception reporting and GC roots. The
  independent event-target crate's fuzz target compares registration/snapshot
  mutations to a sequential model. Adapter fuzzing runs through jrs_source.
  DOMException brands/legacy codes and invalid redispatch are tested. Missing
  tree/Window behavior is not inferred from standalone target passes.
- Abort: stable signal/DOMException identity, reason brands, synchronous trusted
  event dispatch, repeated aborts, onabort listener positioning and conversion,
  cancellation, exception reporting, removals before abort callbacks, duplicate
  and replacement listener identity. Any-composition validates all sequence values,
  flattens dependencies, deduplicates sources and sets all reasons before callbacks.
  Test reentrant aborts, pending-event GC roots, weak sources/dependents, observed
  dependent retention and generation reuse. Independent heap tests check collection
  of unobserved or aborted dependents and weak source backedges. Extend event-target
  fuzzing with ID removal; run adapter seeds through jrs_source. With the optional
  timer host, three original Abort timer files pass (19 subtests); iframe/realm
  behavior remains incomplete and is not inferred from those shell passes.
- Timers: independent `timer-queue` model/fuzzing checks stable equal-deadline
  ordering, cancellation, capacity and token exhaustion. VM tests use a controlled
  monotonic host clock and explicit one-task pumping, including interval self-cancel,
  microtask ordering/nesting clamps, global Script string handlers, policy rejection,
  reported exceptions, exact Web IDL conversions, huge deadlines and GC roots.
  Real WPT uses wall-clock waits, never simulated elapsed time or changed assertions.
  Promise/queueMicrotask callbacks do not inherit timer-task nesting. Original
  timer tests exposed an incorrect inherited clamp and premature single-file
  completion; local regressions now reject missing/late-failing done and exercise
  delayed test creation. Nine pinned JavaScript timer files pass; four HTML timer
  files remain blocked by missing Window/DOM/performance/cross-realm features.
  Missing completion remains failure, with a wall-time cap. Active-document and
  worker suspension, CSP/Trusted Types and cross-realm details remain separate gaps.
- Spread: call/construct/super/array evaluation order, actual iterator overrides,
  cached next/done/value processing, errors without spurious iterator closing,
  literal elisions versus iterated holes, data-property creation without inherited
  setters, nesting, UTF-16 code points and Symbol values. Expanded argument counts
  share operand quotas and survive async suspension; saved super constructors are
  rooted before argument-side prototype mutation. Compare terminating cases with
  a reference engine; infinite iterators remain local fuel-exhaustion tests.
- JSON: strict grammar (no JavaScript fallback), duplicate names and own
  `__proto__` data properties, number overflow/signed zero, all 65536 individual
  UTF-16 units and paired/lone surrogate quoting. Flat parse ranges drive
  reviver context.source; test postorder, snapshot invalidation by mutations,
  ignored failed deletion/redefinition and fresh contexts. Stringify tests
  getters/toJSON/replacer order, wrapper conversions, property-list deduplication,
  gap truncation, array length snapshots, inherited indices, omitted values,
  rawJSON branding/integrity, cycles, callback GC and shared nesting/fuel limits.
  `json_codec` fuzzes parsing/range validity/quoting without the VM; `jrs_source`
  exercises hooks and realm use. The original passive-listener WPT stays unchanged.

### 6.6.68 The wire types and the binary packet (`audhsos-ssh`)

Step S1 of 8.26 (D-123). Everything here is host-tested; the packet layer
is checked against itself until step S3 puts the cipher and its worked
example (D-134) over it.

- The vectors RFC 4251, section 5, prints: the five `mpint` encodings and
  the three name-lists, each read into the value the document names and
  written back into the bytes it prints; the `uint32` 699921578, and the
  string `"testing"`.
- The form of an `mpint` is the value or it is refused: zero is a string
  of no bytes and a single `00` is not zero; `00` before a byte whose top
  bit is clear and `ff` before one whose top bit is set are each an
  unnecessary leading byte; the necessary ones are kept. A negative value
  has no magnitude, which is what every `mpint` of the key exchange is
  read as.
- An unsigned value written as an `mpint` gets the encoding RFC 8731,
  section 3.1, requires — leading zeros off, one zero byte back on when
  the top bit is set — and reads back as the number that was written
  (property).
- A name-list holds no name of zero length, so a leading, a trailing, and
  a doubled comma are each refused; so are a byte that is not US-ASCII,
  a byte sequence that is not UTF-8, and a null. The writer refuses the
  same, and a name with a comma in it besides.
- A failed read leaves the cursor where it was and a write that does not
  fit writes nothing, so no field is half read or half written.
- A packet is a whole number of blocks with at least four bytes of
  padding and never fewer than sixteen bytes altogether, for every block
  size this crate takes and for payloads across the block; what was
  framed comes back out (property).
- A decoder judges the length from the four bytes that hold it and before
  it waits for the packet: below sixteen, not a whole number of blocks,
  and above the largest packet that can satisfy both bounds of section
  6.1 at once are each refused there. Padding outside its packet or under
  four bytes is refused, and so is a payload above 32768 in a packet
  whose length is otherwise one a packet can have.
- A block size that no packet can be padded to is refused, and the one in
  use stands; a block size that is taken does not reset the sequence
  number, which section 6.4 forbids for a re-exchange.
- The padding is one call on the generator (D-121), a generator with
  nothing left frames no packet, and a buffer short by any number of
  bytes frames none either.
- The sequence number of section 6.4 counts every packet, counts nothing
  for a packet that was not whole, runs independently in each direction,
  and wraps to zero after 2^32.
- A packet of the largest mandatory payload fits in a buffer of the
  mandatory size, and two packets in one buffer are read one after the
  other.
- Every refusal renders a sentence of its own.

### 6.6.69 The greeting and the negotiation (`audhsos-ssh`)

The front of step S2 of 8.26: what both key exchange methods start with.
No document publishes an identification exchange or a `SSH_MSG_KEXINIT`
with the lists that made it, so these are checked against the rules the
documents state and against this crate's own writer.

- The identification string this client sends is the form of RFC 4253,
  section 4.2: the prefix, a software version that is printable US-ASCII
  with no space and no minus, CR LF, and under 255 characters. A buffer
  too small for it writes nothing.
- The peer's is the line without its CR LF, which is what the exchange
  hash takes; the lines a server may send before it are skipped and
  counted, and the caller is told where the binary packet protocol
  starts. Nothing is said before a line has ended.
- Every line is held to 255 characters whether it has ended or not, so
  what is refused does not depend on how the bytes were split on the way
  here; one byte under the limit is a line either way.
- Refused: a version this client does not speak, which includes the
  `SSH-1.99-` of section 5.1; a software version of no length; a byte
  that is not printable US-ASCII; bytes that are not UTF-8.
- The message numbers are the ones RFC 4250, section 4.1.2, assigns,
  transcribed a second time so that the constant and the test do not
  share one slip, and each falls in the range section 4.1.1 gives it.
- `SSH_MSG_KEXINIT` holds the fields of RFC 4253, section 7.1, in that
  order, with the cookie one call on the generator (D-121), empty
  language lists, and no guess. What was written reads back as the lists
  that were offered. Refused: another message number, a message that ends
  early at any point, a name that may not be in a list, a buffer too
  small, and a generator with nothing left.
- The negotiation takes the first name on the client's list that the
  server also has, whatever the server prefers. The key exchange method
  and the host key algorithm are chosen together, so a method both sides
  have but no host key this client can check a signature with is not
  chosen, and says so.
- Nothing in common ends the connection and names the list it ended on.
  A cipher that carries its own integrity needs no MAC; any other cipher
  does, and a proposal that offers one is what says so both ways.
- An `ext-info-c` or `ext-info-s` that ends up chosen is a disconnect and
  not a method (RFC 8308, section 2.2).
- A guessed packet is one to ignore unless both of the peer's first names
  are the chosen ones — the right method under the wrong host key is
  still wrong — and a message that announces no guess is nothing to
  ignore whatever its first names are.

### 6.6.70 The key exchange and the cipher (`audhsos-ssh`)

The rest of step S2 of 8.26, and the whole of S3. The cipher has a
published vector and the key exchange has none, so the hash and the key
derivation are checked against the same computation written a second time
in the tests, and the two methods against each other.

- Both sides of `curve25519-sha256` reach one secret, and so do both
  sides of `diffie-hellman-group14-sha256`. A generator with nothing left
  makes no key pair.
- The first message carries the public value as its method encodes it: a
  string for the curve (RFC 5656, section 4), an `mpint` for the group
  (RFC 4253, section 8). The two message numbers are 30 and 31; RFC 5656,
  section 7.1, prints them for the curve method and no document this
  repository holds prints them for the other, so what says they are right
  there is the interop run of step S8.
- The reply is the host key, the public value and the signature; another
  message number, a message that ends early, a non-canonical `mpint` and
  a negative one are each refused.
- The aborts are refusals and not values: a public value that is not
  thirty-two bytes and a point of small order for the curve (RFC 8731,
  section 3), and for the group the open interval of RFC 8268, section 4,
  whose two ends the closed form of RFC 4253 would have admitted.
- The exchange hash is the concatenation of RFC 4253, section 8, in that
  order: every one of the eight fields changes it, and no two of them can
  be swapped without it changing.
- The shared secret enters the hash as an `mpint` and not as it stands.
  A value whose top bit is set hashes differently from the same bytes as
  a fixed-length string; one whose top bit is clear hashes the same,
  which is why the mistake succeeds on about half of all connections. A
  leading zero is not part of the value.
- The six keys of section 7.2 are the six letters; a key shorter than the
  hash is its first bytes, and one longer is extended by hashing the
  whole key so far, which is what the 64 bytes of the cipher need. The
  shared secret is hashed as an `mpint` here too.
- The cipher is appendix A of the draft D-134 keeps: the packet of that
  example seals to the bytes it prints, tag included, and those bytes
  open as the packet they were. The length field is read from its four
  bytes alone. A tag that does not check decrypts nothing, and the same
  packet under another sequence number neither seals the same nor opens.
- The packet layer under that cipher frames the example from its payload
  up, which is what says the padding aligns the region the length field
  is outside of. A sealed packet round-trips, one whose bytes were
  changed anywhere is refused and not counted, and a decoder waits for
  the tag as well as for the packet.
- Taking keys into use does not reset the sequence number, so the first
  packet under the new keys carries the number the last one under the old
  did not (RFC 4253, sections 7.3 and 6.4).
- A buffer of this side's own is not a key exchange that failed: a shared
  secret that does not fit is refused as a buffer and not as a disconnect
  the peer earned. A buffer with room for a sealed frame but not for its
  tag says how much it holds, not how much was written into it.

### 6.6.71 The wall clock (`audhsos-uefi`, `audhsos-abi`, `kernel-syscall`, QEMU)

D-137. The conversion and the refusals are host tests; what QEMU adds is
that the firmware of the reference machine answers at all and that the
date reaches ring three.

- `EFI_TIME` becomes a count of seconds: a reading in universal time is
  its own second, and a named offset is added to reach one, in the
  direction UEFI 2.11, section 8.3.1 gives — `Localtime = UTC -
  TimeZone`, so an offset of 480 moves 13:00 to 21:00 and not to 05:00.
  The bounds the section names, -1440 and 1440, are accepted and the
  values one past them are not.
- `EFI_UNSPECIFIED_TIMEZONE` is a usable reading and not a refusal: the
  value is read as universal time and the source says the zone was never
  named. Nothing else in the structure carries that, which is why the
  source travels with the seconds.
- The daylight bits are checked and change nothing. The firmware moves
  the offset with the time when daylight saving begins, so a correction
  applied here would be applied twice; a bit the section does not define
  is refused.
- A firmware without a clock answers an all-zero structure, whose month
  of zero is what refuses it. A field outside the calendar, a year the
  calendar will not take, and a moment that is not after the epoch are
  each refused with the field that was wrong, so that a clock which was
  never set cannot become a date.
- The boot information carries the pair or neither: a source this version
  does not know, a source with seconds that are not after the epoch, and
  seconds with no source are three separate refusals, and version 1 is
  refused as a version. The fixed part is 144 bytes, and the fuzz corpus
  of `boot_info` carries a version 2 structure.
- `clock_wall` is the boot moment plus what `clock_now` answers, in
  microseconds, with the source in the second word. A machine without a
  clock is `Unavailable`, and so is a boot moment the microsecond scale
  cannot hold — nothing is answered with a wrong date.
- On the reference machine the date lies between 2026 and 2100, moves
  forward by a wait of fifty milliseconds to within a tick, and leaves a
  boot moment in the same range when the monotonic count is taken off it.
  A clock that read nothing usable fails naming `GetTime`, because that
  is the one part of this no host test can reach.

### 6.6.72 Partition table structures (`fs-gpt`)

D-138. The structures are UEFI 2.11, sections 5.2.3 and 5.3, and every
offset is tested against that table.

- CRC-32: the standard check value for the string `123456789`; empty
  input; a single byte; the same bytes fed in one piece and in two, split
  at both ends and across a block; a single bit changes it.
- Protective record: the signature bytes, one record of type `0xEE`
  starting at block 1 and covering the device, the size capped at the
  largest a thirty-two-bit field carries, the rest of the block zero. It
  is read back as protective; a zeroed block, one without the signature,
  and one carrying a legacy partition type are not; the type is looked for
  in all four records.
- Header: written and parsed back whole; the checksum covers the header
  with its own field read as zero; a torn byte is refused; a signature or
  a revision the format does not have; a size below 92 and one above the
  block, and both bounds accepted; a header read from a block other than
  the one it names; an entry size that is not 128 times a power of two or
  that would straddle a block, and 128, 256 and 512 accepted with the
  count that keeps the array the same length; an empty usable range; an
  array that runs off the device, into the usable range, or is empty, and
  a length that overflows; the blocks an array takes when it does not fill
  its last one.
- Table: written onto a device and read back, with the header fields, the
  entry, and its unique identifier as they were written; the backup names
  the primary and lies in the last block; a torn primary header sends the
  read to the backup, a torn array does the same, and both torn report
  what the primary refused; a device partitioned the legacy way; a device
  below the blocks a table needs, and the smallest one that holds it; a
  walk that finds nothing; eight partitions walked in order; an entry
  outside the usable range; a header naming a block the device does not
  have; a device that refuses a block on read and one that refuses a
  write; more entries than the array holds; a partition below the first
  usable block and one above the last; two partitions that touch; an
  unused entry among the written ones; and that a write cut short at the
  primary header leaves a backup that reads.
- Entry: written and parsed back; a name of every code unit it holds and
  one more; a name outside the basic plane, which costs two units per code
  point; a partition that ends before it begins; an entry of no type; a
  slice too short for a field, which takes none of it; the blocks a
  partition covers.
- Properties: a partition written onto a device of any size between the
  smallest and eight thousand blocks reads back as the partition it was;
  and any single bit flipped anywhere in the first two thousand blocks of
  a table leaves a read that refuses, that recovers, or that answers the
  partition that was written — never a different one.

### 6.6.73 The block device (`driver-virtio-blk`)

D-139. The device is virtio 5.2 and the transport is virtio 4.1; every
offset and every bit is tested against the section it came from.

- Registers: each width moves the bytes it names and keeps only the bits
  its register holds; what is written to a structure is read back; the
  interrupt status clears as it is read.
- Common configuration: every field of the layout follows the one before
  it with no gap, and the last ends where the structure this driver reads
  does; the offered features come out of both windows and the accepted
  ones go into both; the status is one byte; nothing is read until it is
  asked for, so the features answer zero before they have been read.
- Features: every bit is the one the specification numbers; the driver
  asks for the version, the flush and the read-only flag and no more;
  every bit of the device's range has a name and no two share one; a bit
  of the transport's range has none; what the driver does not take is
  what `refused` reports.
- Configuration space: the capacity is read out of the device
  configuration through the generation dance of virtio 2.5.1, and a
  device whose generation changes under every read is refused after a
  bounded number of attempts rather than read in a loop.
- Bringing the device up: the status writes are the five of section
  3.1.1 in order; what the device offers and the driver wants is what is
  taken; the rings, the queue size and both vectors reach the registers
  and the queue is enabled last; a device offering a smaller queue
  settles the size; a device with no request queue, rings of a size that
  is not a power of two, a device that was not reset, one that will not
  take a vector, one that clears `FEATURES_OK`, and one that does not
  offer `VERSION_1` are each refused by name; a reset puts back
  everything the driver had learned; a device driven without message
  interrupts writes `NO_VECTOR` and is not refused for it.
- Requests: a read and a write are three buffers in the order of section
  5.2.6, the data device-writable for a read and device-readable for a
  write, the status device-writable in both; a flush is two; a request
  carrying the wrong parts, data that is not whole sectors, a request
  past the last sector including one whose sector would overflow the
  sum, a write or a flush to a read-only device, and a flush of a sector
  other than zero are each refused; a queue with fewer descriptors free
  than the chain needs refuses with the count.
- The header: the type, the reserved word and the sector, with nothing
  written past the sixteen bytes; a buffer too short is refused and left
  as it was; only `VIRTIO_BLK_S_OK` is success.
- Notification: the write goes to the offset the queue's own offset and
  the capability's multiplier make, and carries the queue index.
- Properties: a header written for any sector and any type carries that
  sector and that type and reaches no further than its length; and a
  request is taken exactly when every framing rule of 5.2.6.1 holds for
  it, which is checked against the rules said a second time rather than
  against the driver.

### 6.6.74 The NoREC fuzzer (`norec`)

D-140. The tool finds bugs in another program, so what its own tests hold
is that a case says what it means and that a finding is real: the
generator, the two queries, the reading of an answer, and the reduction.
The engine is a fake in every test but one group, because a test that
needs SQLite is a test that does not run where SQLite is not.

- Values and literals: a quote in text is doubled and nothing else is
  escaped; a double keeps a decimal point, so it reads back as a double;
  a double that is not finite is written as `NULL`, which the generator
  never produces.
- Expressions: every node is parenthesized when it is written, so the
  text means what the tree says; a column is qualified in a query and
  bare in an index term; a name the case does not have renders as `NULL`
  rather than as nothing; children are listed in the order they are
  written and replaced by position, and a position that is not there
  changes nothing.
- Candidates: every candidate of a tree is strictly smaller than the tree,
  and a tree of one node offers none. This is what makes reduction
  terminate.
- The database: a column without a declared type is created without one;
  a collation follows the type; an empty table has no `INSERT`; an index
  carries its terms unqualified and its filter, and a `UNIQUE` one says
  so.
- The queries: the `FROM` clause writes each join as the form it is and
  copies it into both queries; the optimized query counts by rows or by
  `COUNT(*)`; the unoptimized one sums `IS TRUE` over the same clause.
- The generator, over hundreds of seeds: the same seed generates the same
  case; every table has a column and rows as wide as it; the predicate
  names only tables the `FROM` clause has; every index term names a
  column, which is what SQLite requires of one; a `UNIQUE` index is built
  only over a single plain column without a collation; and no case calls a
  function of the clock or of a random source, or writes `DISTINCT` or a
  subquery, which section 3.4 of the paper excludes.
- Reading an answer: equal counts agree and different ones are a finding;
  a sum over no rows is `NULL` and counts as zero; a message on the error
  stream refuses the case; a case the engine did not finish is refused and
  not read; output without the markers, and a count that is not a number,
  are errors rather than verdicts.
- Reduction: a case that always disagrees is reduced to its smallest form;
  what the disagreement needs — a column of the predicate, one of two
  indexes — survives; a case that stops disagreeing is left as it was; and
  the budget stops the search.
- The run loop: a campaign over an engine that agrees finds nothing; what
  the engine refuses is counted by the message it refused with; a
  disagreement is written out under the seed it came from; `--stop` leaves
  the rest unrun; and `--help` and `--script` need no engine at all.
- The process: the tests of the engine drive `sh` and `cat` rather than a
  database, because what is tested there is the child and its three
  streams — what it printed, what it complained about, a script larger
  than a pipe buffer, a run that had to be killed, and a program that
  cannot be started.

What the tool found is not a test of this repository and is not run by
`check`: SQLite 3.28.0, built by `sh tools/sqlite.sh --version 3.28.0`,
answers three of twenty thousand cases two ways, each a `LEFT JOIN` whose
unmatched row the `WHERE` clause drops and the sum keeps. The same cases
agree on 3.53.4.

### 6.6.74a The fuzzing campaign over `db-sqlite`

D-185. `sh tools/xtask.sh fuzz --target <name> --time <seconds>` runs one
target for as long as it is given, which is what the regression replay of
`cargo xtask check` does not: the replay reads the inputs a test cites
and says nothing about inputs no one has written down.

| Target | Executions in five minutes | Coverage | Crashes |
|--------|---------------------------|----------|---------|
| `sqlite_tokens` | 122 735 253 | 164 | 0 |
| `sqlite_expr` | 56 881 762 | 905 | 1, then 0 |
| `sqlite_eval` | 41 615 287 | 1 440 | 0 |
| `sqlite_wal` | 18 309 135 | 161 | 0 |
| `sqlite_format` | 16 814 787 | 367 | 0 |
| `sqlite_journal` | 5 646 169 | 586 | 0 |
| `sqlite_image` | 2 712 610 | 857 | 0 |

The one crash was the target's own invariant: `sqlite_expr` held that a
column's place in a primary key is at most the count of columns, and
`CREATE TABLE t(a INT,b,PRIMARY KEY(a, a, b))` gives `b` the place 3 in
a table of two columns, which is what `PRAGMA table_info` answers for
that statement as well. The case is
`fuzz/corpus/sqlite_expr/a_key_naming_one_column_twice`.

The differential fuzzer of 6.6.74 is the other half: 3 000 generated
cases put to this engine and to the pinned shell agreed on 2 998, were
refused by the shell on 2, and found nothing.

### 6.6.75 The SQLite file format, read (`db-sqlite`)

D-141, document 16 step Q1. Five databases written by the `sqlite3` shell
are the fixtures: the format as it is, not as this crate reads it. Each is
named in the test that reads it, together with the statement that produced
it.

- Numbers: a reader answers nothing where the bytes end first, at any
  offset including one no page has; a varint reads back what section 1.6's
  own encoding wrote, over the whole range and at both ends of every
  length; nine bytes is the longest varint there is; one that does not end
  inside its bytes is refused; a rowid is the same bits read as a signed
  number.
- The header: the fields of a file the shell wrote are the fields the
  shell wrote, page size, encoding, versions and counters; a page size of
  512 and one of 65536, which the field writes as 1, are both read; a
  first sixteen bytes that are not the header string, a file shorter than
  the header, a page size that is not a power of two in range, reserved
  space that leaves a page too small, the three fixed fractions, and an
  encoding code the format does not have are each refused by name.
- Pages: every page type is the byte the format gives it and nothing else
  is; an interior page has the longer header and the right-most pointer; a
  leaf has neither; page 1 carries the database header before its own; a
  cell pointer array that does not fit the page, a pointer that points
  outside it, and a cell index the page does not have are each refused; a
  content area written as zero is the largest page there is.
- Records: every serial code is the type and the length the format gives
  it, including the two the format reserves; an integer is read at the
  width it was stored in and keeps its sign, at one, two, three, six and
  eight bytes; the two constants take no bytes; text and blob borrow the
  bytes they were stored in; a header that reaches past the payload and a
  value that runs past the body are refused, and the walk that found one
  ends rather than repeating it.
- Reading a file: a file is as many pages as it is long; page zero and a
  page past the end are refused; the schema names the tables; the rows of
  a table come back as the shell wrote them, with the storage class each
  value was stored in — a real with no fractional part is an integer on
  disk, which section 2.1 calls an internal optimization and which reading
  it back as a real needs the column's affinity for; four hundred rows over
  512-byte pages are walked in rowid order through the interior pages that
  hold them; text comes back in the encoding the header names, UTF-16
  included; an eighteen-thousand-byte value is read off the chain that
  follows its page, and a buffer shorter than the payload is refused
  rather than filled; a root that names an index tree is refused by a walk
  of rows, and so is a root the file does not have.

- The configuration matrix of document 16, section 16.11: the same three
  rows and the same index, written by the shell under eleven
  configurations — page sizes 512, 1024, 4096 and 65536, the three text
  encodings, 32 reserved bytes per page, a file that has been in
  write-ahead logging, and both vacuum settings — and read back through
  the same four assertions. Each fixture holds the header it was written
  under, names the same table and the same index, answers the same three
  rows with the same values, and keeps its index entries in index pages in
  the order the indexed column collates in. A configuration the shell
  cannot write is not in the table: `PRAGMA legacy_file_format` is a no-op
  in the version that writes these, so schema format 1 is reached by
  editing a header rather than by writing a file.
- `sqlite_image`, a fuzz target over the whole reader: arbitrary bytes to
  `Image::open`, then every page, every cell, every tree the schema names,
  and every record, with a bound on the rows read and on the payload
  assembled. What it holds is that a file that lies is refused rather than
  followed. It found a child pointer of zero — a page number no file has,
  which the cell reader handed back — and that input is in the regression
  corpus under the name it was fixed by.

What is not tested here is everything the crate does not do: writing, the
free list, pointer maps, the write-ahead log, and index walks. Document 15,
section 15.4, has the order they arrive in.

### 6.6.76 The SQL tokenizer (`db-sqlite`)

D-141, document 16 step Q4, first half. The tokenizer is checked against
the one it is a port of, and not against a reading of it.

- The recorded oracle: `fixtures/tokens.corpus` holds nine hundred and
  fifty-nine pieces of SQL, one per NUL-terminated string — every rule of
  `src/tokenize.c` by hand, every way of writing something illegal, raw
  bytes for the cases no text editor writes, and eight hundred statements
  taken out of SQLite's own test suite. `fixtures/tokens.golden` is what
  `sqlite3GetToken` answered for each, one line per case, written by a
  program that links the C library. The test compares the two, kind by
  kind and length by length, so a difference in any rule fails it. The two
  arrows are the one place the notation differs: the C answers one code for
  `->` and `->>` and lets the length say which, and the comparison folds
  the port's two kinds back into that one name.
- The partition: for every case of the corpus, the tokens begin where the
  one before them ended, none is empty, and together they are the whole
  input. A tokenizer that can answer a token of no bytes is a parser that
  can hang.
- The keyword table: every word of it is found whatever case it is written
  in, and a word that is not a keyword is an identifier — including one
  that is a keyword with a letter added, and one that is a keyword with a
  letter taken away.
- `sqlite_tokens`, a fuzz target: arbitrary bytes through the walk, with
  the partition held on every one of them, and a keyword token that does
  not look up as that keyword treated as a defect. Twenty-eight million
  executions found nothing.

The corpus is regenerated by hand and not by a build: what wrote it is a
program against the C library, which CI does not have, so what is checked
in is what it wrote. Document 15, section 15.3, rule 8, is the reasoning.

### 6.6.77 The expression parser (`db-sqlite`)

D-141, document 16 step Q4, second half of the first half. The tree is
built in an arena, so the tests hold two things: that the tree is the one
the grammar says, and that what SQLite refuses this parser refuses too.

- Precedence and associativity: the table `src/parse.y` declares, read
  back out of the tree. `1+2*3` is an addition over a multiplication and
  `1*2+3` is the other way round; `1-2-3` leans left; `1 AND 2 OR 3` and
  `1 OR 2 AND 3` group as the table says; `NOT` binds looser than a
  comparison and tighter than `AND`; `COLLATE` binds tighter than `||`;
  the two arrows lean left.
- Every shape of expression, written out as a bracketed form and compared
  against what it should be: constants of all five kinds, the four ways to
  quote a name, one to three parts of a column name, calls with `*` and
  `DISTINCT` and `ALL`, a cast to a type with a length and to no type at
  all, both shapes of `CASE`, rows, `BETWEEN`, `IN` with a list and with
  none, the four pattern operators with and without `NOT` and `ESCAPE`,
  `ISNULL`, `NOTNULL`, `NOT NULL`, and all four forms of `IS`.
- The recorded oracle: `fixtures/expr.corpus` holds five hundred and
  forty-one expressions, and `fixtures/expr.golden` records, for each,
  whether SQLite's own parser accepted it. Everything it refuses, this
  parser refuses; everything it accepts, this parser accepts, except six
  that wait for `SELECT` — a subquery, `EXISTS`, `IN (SELECT ...)`, `IN
  table`, and the two window clauses — which the test names and holds to
  being refused, so the list shrinks rather than rots.
- Refusals: every place the parser reads an expression inside something
  else, with something that is not one in it; a type that does not end; a
  word that cannot be a name where a type continues; and the two bounds.
- The two bounds, which are not the same bound. The parser's own recursion
  is what brackets drive, and five hundred of them are refused. The height
  of the tree is what a chain of left-associative operators drives, and
  `1+1+1+…` five hundred times long is refused as well, though the parser
  never recursed once while reading it. That case came out of the fuzzer:
  `sqlite_expr` walks the tree it gets back, and a tree taller than the
  parser nests is what made it overflow. SQLite bounds the same thing in
  `sqlite3ExprCheckHeight`, and the arena now carries the height of every
  node so that the bound is a check and not a hope.
- `sqlite_expr`, a fuzz target: arbitrary bytes to the parser; a refusal
  must point inside the input, and a tree must be one — every child a node
  the arena holds, and nothing taller than the bound. Thirty-three million
  executions found nothing after the height bound went in.

### 6.6.78 The statement parser (`db-sqlite`)

D-141, document 16 step Q4, finished. `SELECT`, `VALUES` and `WITH`, with
everything the grammar hangs off them, and the subqueries that let an
expression hold a statement.

- Every clause, written out as a bracketed form and compared: `DISTINCT`
  and `ALL`; a result column with `AS`, with a name and no `AS`, and as
  `*` or `t.*`; `FROM` with a comma, with each of the five joins, with
  `NATURAL`, with `ON` and with `USING`; a schema-qualified table, an
  alias, `INDEXED BY` and `NOT INDEXED`; a statement or a table-valued
  function as a table; `WHERE`, `GROUP BY`, `HAVING`; `ORDER BY` with
  `ASC`, `DESC`, `NULLS FIRST` and `NULLS LAST`; `LIMIT`, `LIMIT OFFSET`,
  and the older `LIMIT skip, count`, which counts the other way round; the
  four compound operators and the clauses that belong to the whole of a
  compound rather than to its last half; `WITH`, `WITH RECURSIVE`, column
  names, `MATERIALIZED` and `NOT MATERIALIZED`; and the four shapes of
  subquery — a value, `EXISTS`, `IN (SELECT ...)` and `IN table`.
- The recorded oracle: `fixtures/stmt.corpus` holds seven hundred and
  ninety-two statements, seven hundred of them taken out of SQLite's own
  test suite, and `fixtures/stmt.golden` records whether SQLite's parser
  accepted each. Everything it refuses, this parser refuses. What it
  accepts and this parser does not is counted rather than listed — thirty
  -six today, the window clauses and the statements that carry a clause of
  a later step — and the test fails if that number rises.
- Refusals: one case for every place a clause reads something, with
  something that is not it there. A `WITH` without `AS`, a `USING` whose
  names are not names, a join whose word is not a join, a `NULLS` that
  says neither `FIRST` nor `LAST`, a `LIMIT` whose second half is not an
  expression, and thirty more.
- The bounds hold over statements as well: three hundred nested
  subqueries are refused by the parser's own depth, and a `VALUES` row one
  taller than the greatest height there may be is refused where the row is
  built, which is the height bound reached from a different direction.
- `sqlite_expr` now walks statements too, so the fuzzer holds that a
  statement is a tree: every child a node the arena has, every subquery a
  statement the arena has, and nothing deeper than the parser walks.

### 6.6.79 Doubles as decimal text (`db-sqlite`)

D-142, document 16 step Q5, first half. A double has no decimal spelling
of its own, so two engines that pick differently disagree about what a
query answers. `sqlite3FpDecode` is ported rather than approximated: the
128-bit multiply against the table of powers of ten, the eighteen digits
it extracts, the rounding to seventeen, and the rule that tries a shorter
rendering where the shorter one reads back as the same double.

- The recorded oracle: `fixtures/fp.corpus` holds eight thousand four
  hundred doubles, one per line as the sixteen hex digits of its bits —
  the values a reader would pick by hand, every power of ten with two
  neighbours of each, four hundred quotients, the powers of two across the
  whole exponent range, and three thousand arbitrary bit patterns from a
  fixed seed. `fixtures/fp.golden` is what `sqlite3_mprintf` answered for
  each at three precisions, written by `tools/sqlite-oracle.c`, which
  links the C library. The test compares all three, so a difference in any
  digit fails it.
- The round trip: every rendering reads back as the double it was written
  from, which is the property seventeen digits are chosen for, and every
  double taken apart into digits and a power is put back together as
  itself.
- The edges by hand: the two infinities, a NaN, both zeros, the smallest
  and largest subnormal, the smallest normal, the largest finite, a
  precision below and above the range there is, a power of ten past either
  end of the table, and the rounding that carries into a digit that was
  not asked for.

### 6.6.80 Numbers out of text (`db-sqlite`)

D-142, document 16 step Q5. Which text is a number decides what `'1x' + 1`
answers and whether `'10' < '9'` compares as text or as numbers, so
`sqlite3AtoF` and `sqlite3Atoi64` are ported rather than approximated.

- The recorded oracle: `fixtures/num.corpus` holds two hundred and fifteen
  pieces of text, one per line as `x` and the bytes in hex so that a
  space, a tab or a NUL is a case like any other — every rule of the two
  routines by hand, both ways of writing a sign, an exponent with no
  digits after it, the words that are numbers in other languages and not
  in this one, the edge of the mantissa, the nineteen digits where an
  integer may or may not hold the number, digit strings of every length
  from one to twenty-five, and a point at every place of one.
  `fixtures/num.golden` is what the two routines answered, written by
  `tools/sqlite-oracle.c`. The test compares the code each returned, the
  double, and the integer, so a difference in any rule fails it.
- The round trip: every integer a reader would pick is written and read
  back as itself, the two ends of the range included.

### 6.6.81 What an expression answers (`db-sqlite`)

D-143, document 16 step Q5. Storage classes, affinity, collation and every
operator, checked against the engine rather than against a reading of
`docs/sqlite/datatype3.html`, because the order of the conversions is the
semantics and the prose does not give it.

- The recorded oracle: `fixtures/eval.corpus` holds seven thousand
  expressions, one per line — every binary operator between every pair of
  sixteen operands, so that `1 + '2'` and `'2' + 1` are both asked; every
  one-operand form and every cast over forty-eight values and nineteen
  type names; and the shapes no cross product writes, among them the
  overflows at both ends of an integer, division by zero in both classes,
  shifts past the width of a word and in the wrong direction, `BETWEEN`
  and `IN` with nothing on either side, and the three collations against
  each other. `fixtures/eval.golden` is what SQLite answered for each —
  `typeof` and `quote`, or the message it refused it with — written by
  `tools/sqlite-oracle.c`. The test compares the class and the value, so
  an answer that is right by accident in one class and wrong in another
  fails it.
- The conversions a column does, which no expression reaches, are unit
  tests beside it: what `applyAffinity` makes of each class under each of
  the six affinities, where a double becomes an integer and where it may
  not, and the two ends of what either holds.
- Refusals: a column, a function, the pattern operators, a variable, a
  row, and the four shapes of statement inside an expression each refuse
  by name rather than guessing, and a tree handed in from outside deeper
  than the walk goes is refused rather than followed.
- `sqlite_eval` is the fuzz target: no expression may panic, every value
  equals itself under every collation, and every number written down
  reads back as itself. The first thing it found is in the regression
  corpus — a zero is written without its sign, so a negative zero is the
  one double that does not come back as the bits it went out as.

### 6.6.82 The scalar functions (`db-sqlite`)

D-144, document 16 step Q5. Thirty-four names, and the pattern matching
`LIKE` and `GLOB` are, taken from the routines of `src/func.c` rather
than from the prose: the prose says `substr` counts characters, not that
it counts bytes for a blob, that it has four rules for a start before the
first character, and that an empty blob answers nothing because it has no
pointer to read from.

- The recorded oracle: `fixtures/eval.corpus` grows to seventeen thousand
  expressions. Every function of one argument over twelve values, every
  function of two over every pair of them, `substr`, `replace` and `iif`
  over every triple, and every pattern of thirty-one against every
  subject of fifteen under `LIKE`, `NOT LIKE`, `GLOB` and `LIKE` with an
  escape. `fixtures/eval.golden` is what SQLite answered, written by
  `tools/sqlite-oracle.c`, and it is read as bytes rather than as text
  because an expression may answer text that is not UTF-8.
- The type and the value are both compared through the port's own
  `typeof` and `quote`, so those two are checked against the C library
  along with everything they are asked about.
- The edges by hand: `abs` of the smallest integer, a hex literal one
  digit too long, an `ESCAPE` that is not one character, `likelihood`
  whose second argument is not a fraction, a call with the wrong number
  of arguments, and a name no function has — each refuses, and so does
  every function that reads a clock, a random source or the connection.
- The pattern length limit is what bounds how deep the comparison
  recurses, so both sides of it are tested: one byte over is refused, one
  byte under is answered, and a pattern that branches at every one of a
  thousand steps still answers.

### 6.6.83 The schema parser (`db-sqlite`)

D-145, document 16 step Q2, first half. `CREATE TABLE` and `CREATE
INDEX`, which is what a row of `sqlite_schema` holds and therefore what
a database has to be read through before anything in it can be.

- The recorded oracle: `fixtures/schema.corpus` is the shapes a schema is
  written in and the ways of writing each of them wrong, one statement
  per line — every column constraint and every table constraint, the
  four ways a type name is spelled, both ways of writing a generated
  column, `WITHOUT ROWID` and `STRICT` in either order, a table that
  takes its columns from a statement, and forty refusals.
  `fixtures/schema.golden` is what SQLite made of each, written by
  `tools/sqlite-oracle.c`: the object it created and what the pragmas
  then answer about it, or the message it refused it with.
- What a parser alone decides is the syntax errors, and those are what it
  is held to: a refusal the grammar itself made says `near "X": syntax
  error`, and every one of them is refused here. What SQLite refuses for
  a reason beyond syntax — a column named twice, a collation that is not
  there, `AUTOINCREMENT` on a column that is not an integer key — this
  parser accepts, and the count of those is a number the test holds down
  until the schema layer refuses them.
- The shapes by hand: where a type name ends and a constraint begins,
  which is what `GENERATED` turns on — it is a type name unless `ALWAYS
  AS` follows it; the text a `DEFAULT` was written as, which is what the
  schema stores and the pragma answers with; the comma between two table
  constraints, which may be left out and may not be left over; and
  `TEMP`, which is the word only before `TABLE` and a name everywhere
  else.
- `sqlite_expr` walks definitions too, so the fuzzer holds that every
  expression a schema carries — a default, a check, a generated column, a
  term of an index and the rows it covers — is a tree the arena holds.

### 6.6.84 The table a statement describes (`db-sqlite`)

D-145, document 16 step Q2, finished. The columns of a table, with the
affinity, collation, key and defaults each carries, built out of the
`CREATE TABLE` text because that text is all a file keeps.

- The oracle is the one `tests/definition.rs` reads: the same statements,
  and the rows `pragma_table_list` and `pragma_table_xinfo` answered for
  each. The test builds those rows out of the parsed statement and
  compares them field for field, so a name, a type, a `NOT NULL`, a
  default, a key place and a generated column are each checked against
  what SQLite says rather than against a reading of the documentation.
- The surprises are tested by hand, because the documentation does not
  have them: a type name of three letters or more that matches one of
  six is stored as that one in capitals and everything else verbatim; a
  primary key is the rowid only where the type is spelled `INTEGER` and
  not `INT`, is one column, and is not written backwards; `WITHOUT
  ROWID` and `STRICT` each make the key columns `NOT NULL`, which
  nothing in the statement said; `ANY` holds what it is given only in a
  strict table; and a string where a key names a column is a name, with
  or without a collation around it.
- The refusals: two columns of one name, a collation no engine has, a
  generated column in the key or a table of nothing but generated
  columns, two primary keys, `AUTOINCREMENT` anywhere but on a rowid
  key, `WITHOUT ROWID` with no key, `STRICT` with a type that is missing
  or is not one of the six, and a key term that is an expression. What
  is left over — a foreign key written with a collation, which SQLite
  allows only while reading a schema back off a file, and a table whose
  columns come from a statement, which a file never holds — is counted
  rather than asserted.
- `sqlite_expr` builds the table too, and holds what a built one must
  be: a key place inside the columns, a rowid that is one of them and
  not on a table without one, and no two columns of a name.

### 6.6.85 A statement answered from a file (`db-sqlite`)

D-146, document 16 step Q5, finished. A database is opened, its schema is
read, and a `SELECT` over one table is answered by walking that table's
tree: the first statement this port answers end to end.

- The recorded oracle: `fixtures/query.corpus` is a fixture and a
  statement per line, and `fixtures/query.golden` is the columns SQLite
  named and the rows it answered, each value quoted, written by
  `tools/sqlite-oracle.c`. The comparison is the name of every column
  and the value of every field, over two hundred and ninety-nine statements
  against fourteen files — every page size of the matrix, reserved space, a file that has
  been through write-ahead logging, both kinds of auto-vacuum, a tree
  with an interior page, a row on overflow pages, a key that is the
  rowid, a key written backwards, and computed columns.
- The names are checked as closely as the rows, because SQLite's rule
  for them is not obvious: an alias wins, a bare column answers under
  its own name, the rowid answers under the name of the column it is an
  alias for, and everything else answers under the text it was written
  as — `a + 1` with its spaces, `'lit'` with its quotes, and `a COLLATE
  NOCASE` whole, because a collation written around a column stops it
  being one.
- What the engine refuses by name is counted rather than asserted: a
  statement inside a `FROM`, and a `WITH`. The count is held down so
  that it can only fall.
- Text in UTF-16 is answered as well as text in UTF-8, both ways round,
  and the three places the stored encoding shows through are tested
  under both: `hex`, `octet_length` and a cast to a blob. So is the one
  place the order differs — `BINARY` compares the bytes as the file
  holds them, so with the little end first a `z` sorts after a character
  written as a pair beginning with a zero byte. `fixtures/wide16.db`
  holds the six pieces of text that show it.
- The refusals that are the engine's own: a file that is not a database,
  a statement that is not one, a table the schema does not name, an
  `ORDER BY` that counts past the answer, and a schema this crate cannot
  read — the last built by patching one byte of a fixture, because a
  file is not obliged to hold a schema anyone can read.
- A schema of five rows the shell would never write is built by hand: a
  row whose type is not text, one that is not a table, one whose root
  page is not a number, one with no statement, and one that calls itself
  a table and holds an index. The reader walks past all five.
- `sqlite_image` now opens every fuzzed file as a database and answers a
  statement over each table it finds. The first thing that found is in
  the regression corpus: a row whose length is a number no machine
  holds, which is now refused before the room for it is asked for.

### 6.6.86 Reading UTF-8 and UTF-16 alike (`db-sqlite`)

D-147. `crate::utf8` is the reader and writer of both, and the tests are
the text a file may hold that the shell will not write.

- The pairs: a surrogate joined with whatever follows it, a surrogate
  with nothing after it, an odd number of bytes, and a round trip both
  ways round for every string the crate holds.
- The sequences that are not: overlong, a surrogate written as three
  bytes, the two characters that are not characters, and a continuation
  byte with nothing in front of it — each read the way `sqlite3Utf8Read`
  reads it, which is leniently, because a reader that refuses where
  SQLite does not answers a different `length` and a different `substr`.
- The lengths: every character written in as few bytes as it takes, and
  a count that stops at a NUL as a C string does.

### 6.6.87 Grouping and the aggregates (`db-sqlite`)

D-148, document 16 step Q5. The seven aggregates, `GROUP BY` and
`HAVING`, tested through the same recorded oracle as the statements they
belong to.

- The accumulation: `count`, `sum`, `total`, `avg`, `min`, `max` and
  `group_concat` over every storage class a column holds, each of them
  also with `DISTINCT`, and each over no rows at all — where `count`
  answers zero, `total` answers `0.0` and the rest answer `NULL`.
- The three answers of `sum` that a port cannot guess: an integer while
  the integers hold it, a double once one of the values is one, and the
  refusal `integer overflow` where the integers stopped holding it and
  no double came after. `total` over the same column answers a double
  and never refuses, which is the pair that shows the difference.
- The sum is a Kahan-Babuska-Neumaier one once it is a double, so the
  cases include a value too large for a double to hold exactly, which is
  split before it is added, and a value of infinity, whose compensation
  term is not a number and is dropped rather than added back.
- The bare columns of a group come from a definite row: the first row of
  the group, unless a `min` or a `max` says otherwise, and both are
  tested beside each other — `SELECT b, max(a)`, `SELECT b, min(a)` and
  `SELECT a, count(*)` over the same three rows answer three different
  rows.
- What a `GROUP BY` term is: an expression, a whole number that counts
  the answered columns from one, or a name the statement answers under
  where the table has no column of that name. `GROUP BY 1` and `GROUP BY
  2-1` are different groupings and both are cases.
- The refusals: an aggregate in a `WHERE`, in a `GROUP BY`, in an
  `ORDER BY` of a statement that does not group, and inside another
  aggregate; a `HAVING` on a statement that groups nothing; a `DISTINCT`
  aggregate with more than one argument; and a `GROUP BY` that counts
  past the answer or before it.
- `sqlite_image` asks every table of every fuzzed file for the
  aggregates over its key, grouped, so that an accumulator reached from
  a file that lies is reached the same way a scan is.

### 6.6.88 Statements put together (`db-sqlite`)

D-149. `UNION`, `UNION ALL`, `INTERSECT`, `EXCEPT` and `VALUES`, tested
through the same recorded oracle.

- Which of two rows that compare equal but are not the same bytes comes
  out of a set operator is not a detail a port may leave to chance, and
  `fixtures/wide16.db` is where it is pinned: a `NOCASE` column holding
  `A`, `b` and `a`. Six cases hold the rule the merge of
  `multiSelectOrderBy` gives — the first of them inside one side, the
  right side's where a `UNION` finds one on both, and the left side's for
  an `INTERSECT` and an `EXCEPT`.
- The order: everything but `UNION ALL` answers sorted by every column,
  `UNION ALL` answers one side after the other, and an `ORDER BY` over
  the whole re-sorts what came out.
- The collation a column compares under is the first side that writes
  one, which is `multiSelectCollSeq`, so a `NOCASE` column on the left
  makes a literal on the right compare without case.
- The refusals: sides that answer different numbers of columns, an
  `ORDER BY` term that is an expression rather than a column of the
  answer, and a `VALUES` whose rows are not all the same width.
- `VALUES` is a core and not a statement, so an `ORDER BY` or a `LIMIT`
  after one is a syntax error — the parser now refuses it where SQLite
  does, and a `VALUES` inside a compound still takes the `ORDER BY` of
  the `SELECT` beside it.
- A table may be written with `main` in front of it and is the same
  table; any other schema names no table, which the column `main.t.a`
  tests as well as the table `main.t`.

### 6.6.89 Joins (`db-sqlite`)

D-150. `fixtures/joins.db` holds three tables — two sharing a column
named `x`, a third sharing one named `y` and collating it without case —
and fifty-one statements over them.

- Every way of writing one: a comma, `JOIN`, `INNER JOIN`, `CROSS JOIN`,
  `LEFT JOIN`, `RIGHT JOIN`, `FULL JOIN`, `ON`, `USING` and `NATURAL`,
  each against the same rows, with `NULL`s on both sides so that a key
  that matches nothing is tested as well as one that matches twice.
- A `RIGHT` or a `FULL` join answers the rows it matched nothing to
  after every row the nest answered, which is the order SQLite walks
  them in; the cases hold that order, and hold what a third table joined
  after one of them answers.
- A `USING` column of an outer join answers the first side holding
  something rather than the side written first, which is the `coalesce`
  `sqlite3ProcessJoin` writes around it: `SELECT a.x, b.x, x FROM a
  RIGHT JOIN b USING(x)` answers `NULL`, 4, 4 for one row.
- What a `*` answers, which is not the columns of the tables put
  together: a `USING` or a `NATURAL` answers the column it matched once,
  and `b.*` answers all of `b`'s. Two tables of one name make every
  column of them a name two tables answer, which SQLite refuses and so
  does this.
- A `USING` compares under the collation of the side written first, so
  `a JOIN c USING(y)` and `c JOIN a USING(y)` answer different rows over
  the same tables — both are cases.
- A bare name does not reach the side a `USING` matched, and a name two
  tables answer is refused rather than chosen between, `rowid` included.
- The refusals: a `NATURAL` with an `ON` or a `USING` written on it as
  well, and a `USING` naming a column the left or the right does not
  hold.
- `sqlite_image` joins every fuzzed table to itself with a `FULL JOIN`
  on a key that matches every row but one, so that the nest and the pass
  over what it matched nothing to are both walked over a file that lies.

### 6.6.90 Columns a record does not hold (`db-sqlite`)

D-151. A column computed and not stored takes no place in the record, so
the record is read by the storage place each column has and not by the
column's own place.

- `fixtures/generated.db` holds four tables: one with a stored computed
  column and one without, one whose computed column names a column
  computed after it, one giving each computed column a declared type,
  and one computing from the file's encoding and from the default
  collation.
- The order the passes settle in: a computed column that names a
  computed column is settled by the pass after that one. A column that
  names itself, and one that names a column no table has, both refuse —
  neither is a table the shell will write, so both are hand-built
  schemas whose table reads the schema page as its own rows.
- A computed column takes its declared affinity, so `b TEXT AS (a)` over
  an integer answers text.
- A `GROUP BY` that counts to a `*` counts the answered columns and not
  the result columns, so it reaches the table column the `*` stands for,
  a column a `USING` hides included.

### 6.6.91 The key's own tree (`db-sqlite`)

D-152. A table written `WITHOUT ROWID` keeps its rows in an index tree,
so the walk of one is a walk of the other kind and the record puts the
key's columns first.

- The walk: an index tree carries an entry on every page and not only on
  its leaves, so `fixtures/page512.db` holds four hundred rows in a key's
  own tree, whose root is an interior page. The test holds the keys to
  their sorted order, which is what descending, answering the entry
  between two subtrees, and descending again produces.
- The record: `fixtures/keys.db` holds one table whose key columns are
  out of the order they were declared in, one naming a key column twice,
  and one with a column the record does not hold, so the mapping from
  storage place to column is tested where it is not the identity.
- Such a table answers no `rowid`, `oid` or `_rowid_`, which SQLite
  refuses as a column no table has.
- The refusals a walk of an index tree makes: a root the file does not
  have, a root that leads to a table page, a tree deeper than the walk
  keeps frames for, an entry on a leaf whose cell reaches past its page,
  and an entry between two subtrees whose length does not end. The last
  two are pages laid out by hand.
- `sqlite_image` walks every root of every fuzzed file as a table tree
  and as an index tree, because a root is a number the file chooses.

### 6.6.92 A statement inside a statement (`db-sqlite`)

D-153. A side of a `FROM` carries a shape, so a statement written inside
the `FROM` and a `WITH` term are read the way a table is.

- The shapes the corpus puts to the engine: a statement with no `FROM`,
  a statement over a table, a statement inside a statement, a `VALUES`,
  and a compound. Each is read once as a bare side, once with an alias,
  and once joined to a table.
- What the shape carries is tested where it differs from the default:
  `SELECT * FROM (SELECT a FROM t) WHERE a='1'` answers a row only if
  the side keeps the column's integer affinity, and a `WHERE` and a
  `DISTINCT` over a side whose column was declared `COLLATE NOCASE`
  answer only if the side keeps that collation.
- A side with no name answers no qualified name, which
  `SELECT a.x FROM a, (SELECT 1)` reaches: the walk asks the unnamed
  side for `a.x` and it answers nothing.
- A side that is not a table answers no `rowid`, `oid` or `_rowid_`,
  which SQLite refuses as a column no table has.
- A `WITH` term is reached by its bare name and a name written with a
  schema in front of it is a table, so `WITH t(a) AS (SELECT 9)` over a
  file that holds a table `t` answers the term for `t` and the table for
  `main.t`.
- The refusals: a `WITH` term that writes more column names than its
  statement answers columns, two sides of one name under a `*` or under
  a `name.*`, a `name.*` naming no side, and a table-valued function.
- A `WITH` written `RECURSIVE` refuses, which is a unit test and not a
  corpus case, because SQLite answers a term that does not read itself.
- `sqlite_image` puts a `WITH` term and a statement inside the `FROM`
  over every fuzzed file, so the rows a side holds are rows the file
  decides.

### 6.6.93 A statement used as a value (`db-sqlite`)

D-154. An expression uses a statement in four shapes, and the corpus
puts each to the engine uncorrelated and correlated.

- `(SELECT ...)` as a value answers the first column of its first row,
  and `NULL` where it answers no row.
- `EXISTS` answers one or zero whatever the statement answers columns,
  so `EXISTS (SELECT a, b FROM t)` is not the refusal that
  `(SELECT a, b FROM t)` is.
- `IN` is three-valued: `NULL IN (SELECT a FROM t)` answers `NULL`,
  `NULL IN (SELECT a FROM t WHERE 0)` answers zero, and a `NOT IN` over
  a column holding one `NULL` answers no row at all.
- The comparison an `IN` makes converts under the affinity of the two
  sides together and compares under the collation written on the left,
  or the looked-in column's where the left writes none, which
  `SELECT 'A' IN (SELECT t FROM n)` reaches over a `NOCASE` column.
- `IN table` reads a table of one column, so `t IN s` over
  `fixtures/wide16.db` walks the table and `a IN t` over
  `fixtures/small.db` refuses as SQLite refuses.
- A correlated statement is answered once per row of the statement that
  encloses it: `SELECT a, (SELECT count(*) FROM t AS u WHERE u.a<t.a)
  FROM t` counts against the row the outer walk stands on.
- A `VALUES` and a `LIMIT` read one as well, which is the row a
  statement with no `FROM` stands on and not a row of nothing.
- `sqlite_image` puts a correlated count, an `EXISTS` and an `IN` over
  every fuzzed file in one statement.

### 6.6.94 The write-ahead log (`db-sqlite`)

D-155. A database in write-ahead logging keeps its newest pages in its
log, so a reader that does not follow the log reads a file that is
empty or stale.

- `fixtures/logged.db` is 4096 bytes and names no table; its log holds
  the schema, the rows, and three changes made after them — a text
  updated, a row deleted, a row inserted. The shell checkpoints a log
  away when the last connection closes, so the two files are copied
  while the connection is open.
- What the reader takes: the newest frame of each page up to the last
  commit frame, which is the row set the last commit left and not the
  three rows the first one wrote.
- Where the walk stops: a frame whose salt-1 or salt-2 is not the
  header's, and a frame whose checksums are not the running one. Either
  leaves the log holding nothing, because the frames after such a frame
  were written before the last reset.
- The refusals: a file shorter than the 32-byte header, a magic number
  that is neither `0x377f0682` nor `0x377f0683`, a page size that is not
  a power of two in 512..=65536, and a log whose page size is not the
  one the database header names.
- The shell writes only logs whose checksum is computed little-endian,
  so the big-endian half of section 4.2 is reached by logs the test
  builds itself, which also compute the checksum for themselves rather
  than calling the reader.
- A frame may name page zero; no reader may ask for it, because section
  1.2 numbers pages from one. `sqlite_wal` found this by asserting it.
- `sqlite_wal` reads arbitrary bytes as a log on their own and beside
  the fixture, and asserts the page size it answers, the length of every
  frame it hands back, and that page zero is in no log.

### 6.6.95 The rollback journal (`db-sqlite`)

D-156. A database caught between the sync of its journal and the sync of
its own pages holds a transaction half written, and the journal holds
what each page began with.

- `fixtures/rollback.db` holds four rows and the text `changed`;
  `rollback.db-journal` holds the two pages the transaction began with,
  so playing it back answers three rows and the text they started from.
- The pair is built by `sqlite-oracle journal`, not caught: SQLite
  writes the journal's magic only once its records are on disk, so a
  crash reachable from a script leaves a journal that is not hot. Forty
  attempts at killing the shell mid-transaction produced only journals
  whose magic was zeroed. `sh tools/sqlite-fixtures.sh` checks the built
  pair by opening it with the C library, which rolls it back and must
  answer the three rows.
- What the playback keeps: the first record of a page, because a page
  journaled twice was journaled first with the content the transaction
  started from. A page past the count the first header names is passed
  over, which is the truncation.
- Where the walk stops: a record naming page zero, a record whose
  checksum is not its own, a record cut short in its page number, its
  page or its checksum, and a header whose sector or page size is out of
  range or not a power of two.
- A journal whose magic is not the magic is not hot, and the database
  beside it reads back as it stands. So is one too short to hold a
  header and the sector it is padded to, which `readJournalHdr` answers
  `SQLITE_DONE` for.
- Two of the six journal modes leave the journal file behind when they
  commit: `persist` zeroes its header and `truncate` cuts it to no
  bytes. `fixtures/m-persist.db-journal` and
  `fixtures/m-truncate.db-journal` are what the shell left, and the
  databases beside them read back the three rows the matrix holds.
- A second header carries records of its own, at the next sector; the
  first header is the one that says how large the database was. Records
  that end exactly on a sector are followed by no padding.
- A count of `0xffffffff` is what a process working without sync writes,
  and the records are then however many the file holds.
- `sqlite_journal` reads arbitrary bytes as a journal, alone and beside
  the fixture, and asserts the page size it answers, the length of every
  record it hands back, that page zero is in no journal, and that no
  page past the truncation is restored.

### 6.6.96 The page a walk stands on (`db-sqlite`)

D-157. A walk parsed its page again for every cell of it; it holds the
page it stands on now.

- Nothing new is asserted: the walk answers the same rows, so every test
  of 6.6.1 onwards holds it to the same answers, and `sqlite_image`
  replays the same corpus. What changed is how often `Page::parse` runs,
  which no answer depends on.
- The two branches the hold adds are reached by the corpus as it stands:
  a leaf of two cells or more reads the page it already holds, and a
  descent or a climb reads one it does not.

### 6.6.97 The rowids a `WHERE` leaves a walk (`db-sqlite`)

D-158. A walk held to a range answers what a scan answers, so the
corpus holds it to the same rows and the timing is what changed.

- The shapes the corpus puts to the engine: each of `=`, `<`, `<=`, `>`
  and `>=`, each of them written the other way round, two terms that
  narrow the same end, two that narrow both, and two that leave an empty
  range.
- What must not be planned: a term under an `OR`, a term under a `NOT`,
  `<>`, a bound that is text or a real, a bound in a statement used as a
  value, a column that is not the rowid, a name two sides answer, a name
  written with a schema that is not `main`, and a name written with a
  table the statement does not have.
- What the rowid is: the three names it answers to, and a column
  declared `INTEGER PRIMARY KEY` which is another name for it. A column
  declared `INTEGER PRIMARY KEY DESC` is not, and a table that keeps its
  rows in the key's own tree answers no rowid at all, so neither is
  planned for.
- The bounds at the ends of the range: `rowid=0`, `rowid=-1`,
  `rowid=9223372036854775807`, `rowid>9223372036854775807` and
  `rowid<-9223372036854775808`, which are where a bound one past the
  written one does not exist.
- The joins: a comma join with a term on each side, a `LEFT JOIN` with a
  term on the left and one on the right, and a `RIGHT JOIN`, because a
  side held to a range is a side that answers fewer rows to match
  against.
- `Image::rows_between` is held against a scan of the same fixture
  filtered by the same bounds, over eleven pairs of bounds including
  reversed ones and ones outside the table, so the walk and the filter
  answer the same rowids.
- `sqlite_image` puts four such ranges over every fuzzed file and reads
  the count, the smallest rowid and the largest.

### 6.6.98 The index a `WHERE` reaches (`db-sqlite`)

D-159. An index answers where a row is, so the corpus holds a statement
read out of one to the rows a scan answers and the timing is what
changed.

- `fixtures/indexed.db` holds the indexes that must be used and the ones
  that must not: two over one column, one over two, one held backwards,
  one over an expression, one with a `WHERE`, one written with a
  collation the column does not have, one over a table that keeps its
  rows in the key's own tree, one whose entries run onto an overflow
  page, and one over a column holding each of the five storage classes.
- What the key is: the bound with the column's affinity applied, so
  `a='77'` over an `INTEGER` column reaches the entry `77`. A key of
  `NULL` reaches no entry, which is what `NULL = NULL` answering nothing
  means.
- What the collation is: the index's own, which is the column's unless
  the index writes another. `q='a'` over a column declared
  `COLLATE NOCASE` answers the rows `q='A'` answers.
- A file is not obliged to hold an index the shell would write. An entry
  the descent cannot read gives the index up and the table is scanned,
  which answers the same rows; an entry the walk cannot read refuses,
  because answering the rows found before it would be answering fewer
  rows than the table holds. Both are reached by damaging one byte of
  one cell: the cell the descent reads, and the cell after the ones the
  key reaches.
- The other damaged shapes: an entry whose record header claims more
  than the entry holds, an entry whose last value is not a rowid, a row
  an entry names whose record is broken, an index schema row whose root
  page is not a number, one that calls itself an index and holds a
  table, and one over a table the file does not have.
- `Image::entries_from` descends a tree of four hundred entries with
  interior pages, and is refused at the depth the walk keeps frames for
  where the tree never reaches a leaf.
- `sqlite_image` puts a `=` against the first two columns of every table
  of every fuzzed file, with an integer, a text and a blob, so the index
  a file describes is walked with keys of every class.

### 6.6.99 The math functions (`db-sqlite`)

D-160. `eval.corpus` gains sixty-seven cases, and what they hold is that
an answer is the same bits the C library answers.

- The oracle is built with `SQLITE_ENABLE_MATH_FUNCTIONS`, which the
  shell of `sh tools/sqlite.sh` is built with and the oracle was not: a
  library without it has no `pi` and no `ceil`, so the golden recorded a
  refusal the shell does not make. The two are the same library now.
- What each case reaches: a whole number and a fraction, both signs, a
  zero of each sign, the largest and smallest doubles, a number past
  2^53 where a double holds no fraction, text that is a number and text
  that is not, a blob, and `NULL`.
- `zeroblob(NULL)` answers a blob of no bytes rather than `NULL`,
  because `sqlite3_value_int64` of a `NULL` is nought. A count past
  `SQLITE_MAX_LENGTH` refuses rather than asking for the room.
- `ceil`, `floor` and `trunc` answer an integer argument as it stands,
  which is `ceilingFunc` reading the numeric type first.
- `mod` is `fmod`, whose answer is exact: the divisor is doubled up to
  the dividend and halved back down, and every subtraction is one the
  format rounds nothing in. The cases reach both signs of each operand,
  a divisor of nought, a dividend and a divisor at the ends of the range
  including the smallest subnormal against the largest normal, and an
  infinity, which has no remainder.
- `changes`, `total_changes` and `last_insert_rowid` answer nought,
  which is what a connection that has written nothing answers and what
  this engine will answer until Q7.

### 6.6.100 The engine under every configuration (`db-sqlite`)

Document 16, section 16.11. A configuration changes how the rows are
held and not what they are, so every configuration answers the same.

- Nineteen fixtures: five page sizes, three encodings, three reserved
  sizes, all six journal modes, and the three vacuum settings. Each
  holds the same three rows and the same index.
- Sixteen statements go to every one of them and the answers are held to
  each other: the text out of the encoding, the numbers out of the
  record, a walk of the table's tree and of the index's, a grouping, a
  sort, a join of the table to itself, a statement used as a value, and
  a `WHERE` that holds the walk to a rowid.
- `query.corpus` records what the C library answers for each of the
  seventeen statements under each of the nineteen fixtures, so the
  answers are held to SQLite as well as to each other.
- `hex`, `octet_length` and a cast to a blob are the three that answer
  the bytes as they are stored, so they are held to the difference the
  encoding makes rather than to sameness: three letters are three bytes
  in UTF-8 and six in either UTF-16.
- What the journal mode leaves in the file is the write version, which
  is two once the file has been in write-ahead logging and one
  otherwise. The journal itself is a second file, which 6.6.94 and
  6.6.95 read.

### 6.6.101 The format language and `unistr` (`db-sqlite`)

D-161. `eval.corpus` grows by six thousand eight hundred cases, and what
they hold is that a format writes the bytes the C library writes.

- The cases: a hundred and fourteen formats against twenty-seven values
  and against every pair of five, so each flag, width and precision
  meets every storage class; twelve formats whose width or precision is
  an argument of its own, against seven of them, two of which a cast to
  `int` turns around; and the shapes no cross product writes, among them
  a `%` at the end of a format, a flag with nothing after it, and `%,`
  after a width, which is a conversion character that does not exist.
- A format that writes nothing answers `NULL` rather than empty text,
  and `%n`, which writes no byte, answers empty text. What separates the
  two is whether a conversion wrote and not whether the answer is empty,
  so both are cases.
- Seventeen cases ask for a field wider than an answer holds. Each is
  refused by both engines and neither takes the memory it names: a width
  of two thousand million is refused before the buffer, and a precision
  of more than a hundred million digits is that many, which is
  `SQLITE_FP_PRECISION_LIMIT`. A width the C library does fill —
  `%999999900.5f` of a small number is a gigabyte of spaces — is left
  out, because a case is only worth what it costs to record.
- `sqlite_format` is the fuzz target: no format may panic, no format may
  answer more than a value holds, and a format with no conversion in it
  answers itself. It skips a format asking for more than a hundred
  thousand bytes, which the recorded cases above cover instead.
- What no statement reaches is a unit test beside the corpus: `%0f` of a
  NaN, which writes `null`, because a NaN is a `NULL` before it is a
  value of any statement.
- `unistr` has a case per form it reads — `\XXXX`, `\+XXXXXX`, `\uXXXX`,
  `\UXXXXXXXX` and `\\` — and a case per way of writing one wrong: too
  few digits for the form, a character that is no digit, a backslash at
  the end, and a code point past what a character is. A pair of
  surrogates is written as two characters of three bytes each, which is
  what the C library writes; `quote` of the answer is how the test sees
  it.
- `quote` and `unistr_quote` are held to the same corpus as everything
  else, because every case is compared through `quote`: the two are one
  conversion of the format language under two flags, so a difference
  between them is a difference in `%Q`.

### 6.6.102 A row as bytes (`db-sqlite`)

D-162, document 16 step Q7. `record::write` is `OP_MakeRecord`, and what
holds it is the bytes the C library wrote rather than a reading of
section 2.1.

- `records.db` is the fixture: every serial type in a column of no
  affinity, every affinity against five classes of value, a header of
  exactly 127 code bytes and one of 130, which are the two sides of the
  size varint counting itself, a table that keeps its rows in the key's
  own tree, and a rowid alias, whose column takes no place in the
  record.
- The test reads every row of eight fixtures, writes the values back and
  compares byte for byte: 978 rows, of which the matrix and the overflow
  fixtures contribute the rows a page does not hold whole.
- What no fixture reaches is a unit test: an integer in a column of real
  affinity that six bytes do not hold, which is stored as the double it
  stands for, and a file written before the fourth schema format, which
  has no serial type for a zero or a one.
- `sqlite_image` holds the other direction: the values of every record
  it reads are written back as a record, and reading that record answers
  the same values. A double is compared by its bits, because a file may
  hold a NaN and a NaN is equal to nothing.
- `put_varint` is held to `varint`: every value either side of every
  group boundary is written and read back, and the nine-byte form, whose
  ninth byte carries all eight of its bits, is the one the record layer
  never writes.

### 6.6.103 A cell and a page as bytes (`db-sqlite`)

D-163, document 16 step Q7. `page::write_cell` and `page::build`, held
to the pages the C library wrote.

- Every tree of eight fixtures is walked, root first and children after
  it, and every cell of every page is written back and compared against
  the bytes it lies in: 1663 cells, of all four shapes, with and without
  an overflow page.
- 56 of those pages are built whole and compared. A page qualifies where
  it carries no freeblock, no fragmented byte, and holds its cells from
  the end downward in the order the pointer array names them, which is
  the shape `rebuildPage` writes. The header, the pointer array and the
  content area are compared; the unallocated space between the array and
  the content is not, because the format says nothing about it and
  SQLite leaves in it whatever the page held before.
- What no fixture reaches is a unit test: a usable size past the page
  itself, a page shorter than the database header page 1 carries, cells
  whose bytes are more than the page holds, and cells that fit with no
  room left for the pointers naming them. Each is refused rather than
  written wrong.
- `sqlite_image` holds the statement arbitrary bytes allow: a page built
  from the cells of another page reads back as those cells. The bytes
  need not match, because a file may write a length as a varint longer
  than the value needs and this engine writes the shortest one.

### 6.6.104 A cell put on a page and taken off (`db-sqlite`)

D-164, document 16 step Q7. `Writer` is the part of `MemPage` one page
decides for itself, and what holds it is the pages the C library wrote.

- Every cell of every page of eight fixtures that carries no freeblock
  and no fragmented byte is taken off its page and put back: 1333 cells,
  and the page has to be the bytes it was. A page that already holds a
  freeblock is left out, because the cell comes back into a list that is
  not empty and what comes back is a slot of another size at another
  place; such a page is the balance's to rewrite.
- A run of four hundred inserts and removes over one page, sized so that
  the page fills, fragments, and is moved together again. The page is
  read back after every step and every cell compared against what went
  on.
- Two tests reach the two ways a page is moved together: one freeblock,
  which is closed by sliding the content over it, and three, which is
  one more than that path takes, so every cell is copied to the end
  instead. The second holds the order of the cells, which is what a
  copy in the wrong order would lose.
- A page with one byte of it changed, over every byte of the page and
  thirteen values, put to each of thirteen steps of a write on its own
  copy: every one of them refuses rather than writing outside the page,
  and the page still reads back afterwards.
- `sqlite_image` does the same against arbitrary bytes: a cell taken off
  a page a file decided the shape of, and a cell put on it.

### 6.6.105 A file written back (`db-sqlite`)

D-165, document 16 step Q7. `Header::written` and `image::write`, held
to the files the shell wrote.

- Twenty-seven files are taken apart into their pages, every b-tree page
  that was filled in one pass is built again out of its cells, and the
  file is written from the header and the pages. What comes out is the
  file that went in, byte for byte. 108 of the pages are built again
  rather than kept.
- The twenty-seven are the eight the other tests read and the nineteen
  of the configuration matrix, so a file is written back under five page
  sizes from 512 to 65536, under UTF-8 and both orders of UTF-16, with
  and without reserved bytes at the end of every page, after every
  journal mode, and with the file vacuuming itself or not.
- A page is built again only where the bytes between its pointer array
  and its content are noughts. A page SQLite rewrote holds what it held
  before in that space, which the format says nothing about and a writer
  that starts from nothing cannot reproduce; such a page is kept as it
  lies.
- Every fixture asserts the library version in its header against
  `header::LIBRARY_VERSION`, so a shell of another version is caught
  here rather than in the four bytes of a file this crate writes.

### 6.6.106 A database written from nothing (`db-sqlite`)

D-166, document 16 step Q7. `tree::Pages` and `tree::insert`, held to
the files the shell wrote rather than to a reading of section 1.6.

- Four databases are built here out of a schema and its rows and
  compared against the fixture byte for byte: `small.db`, whose table
  holds the four storage classes; `utf16.db`, whose text and whose
  schema are both in the encoding the file names; `joins.db`, which is
  three tables, each created and then filled, so the file counts six
  changes and three schemas; and `overflow.db`, whose one row is
  eighteen thousand letters, of which four thousand stay on the leaf and
  the rest is four overflow pages.
- What the comparison says is that the C library reads what this engine
  writes: the bytes are the bytes it wrote itself.
- The descent is tested on a tree built by hand, because nothing else
  makes an interior page yet: a row whose key an interior cell names
  goes on the child that cell points at, and a key above every cell goes
  on the page the right-most pointer names.
- `tall.db` is the fifth: four hundred rows over pages of five hundred
  and twelve bytes, which is thirteen leaves under one root. What it
  holds this crate to is the balance an append needs and the bytes
  `zeroPage` leaves behind — the root still holds the first leaf's rows
  in the space no cell is in, and so does the file this crate writes.
- The refusals: a page size the format does not allow, a reserved tail
  that leaves too little of a page, a tree of index pages, which holds
  no key a row belongs under, a tree of interior pages that never
  reaches a leaf, which the descent stops in rather than following
  forever, a schema of more than one page, and a tree whose dividers do
  not name the largest key of the page under them.

### 6.6.107 A row that lands in the middle of a tree (`db-sqlite`)

D-168, document 16 step Q7. `tree::balance_nonroot` and the balance that
runs up to the root, held to the files the shell wrote.

- `shuffled.db` puts four hundred rows in by a key that jumps about, so
  that every insert lands in the middle of a page and the page and its
  siblings are written again rather than appended to.
- `deep.db` puts four thousand in the same way, which fills the root,
  grows a third level under it, and then balances the interior pages of
  that level against each other.
- Each is the fixture byte for byte, and so was every one of the four
  thousand prefixes of `deep.db` while the port was written, which is
  what found the stale pointer `editPage` leaves and the noughts
  `copyNodeContent` does not copy over.
- The refusals: a balance whose siblings are not of one kind, a balance
  whose cells fit on fewer pages than they lie on, which frees a page
  and needs the free list, a page that gives back more cells than it
  holds, a page whose content area begins past the bytes the b-tree may
  use, a page smaller than the cells it is to hold, and a page whose
  free list says a block longer than the page.

### 6.6.108 The write path under every configuration (`db-sqlite`)

Document 16, section 16.11, over writing. Nine fixtures hold the same
four hundred rows, put in by a key that jumps about, under every page
size the format allows, every encoding, and the reserved tails the shell
writes.

- `w-utf8-512.db`, `w-utf8-1024.db`, `w-utf8-4096.db`,
  `w-utf8-8192.db` and `w-utf8-65536.db` are the five page sizes, which
  changes how many rows a leaf holds and, at 65536, how the header
  writes the size.
- `w-utf16le-512.db` and `w-utf16be-512.db` are the two UTF-16
  encodings, which changes the bytes of every row and of the schema.
- `w-reserved4.db` and `w-reserved32.db` leave four and thirty-two bytes
  at the end of every page, which changes what a payload holds locally
  and where a cell may lie.
- Each is built here from the schema and the rows and is the fixture
  byte for byte, so the configuration changes the file and not what the
  engine writes into it.

### 6.6.109 A row taken out again (`db-sqlite`)

D-169, document 16 step Q7. `tree::remove`, the free list, and the two
rules of the pager the file shows.

- `deleted.db` takes every third row out, which evens the leaves out and
  frees no page; `emptied.db` takes three in four out, which joins
  leaves and puts ten pages on the free list; `cleared.db` takes every
  row out, which leaves the root of the table a leaf with no cell.
- `unchained.db` holds forty rows whose payloads run onto overflow
  pages, two in three of them taken out again, so the chains go back on
  the free list.
- `reused.db` is three statements: four thousand rows, seven in eight
  taken out, and a thousand put in after that, so the pages the delete
  freed are the ones the insert takes. It is what shows the two rules
  the pager keeps — a page freed onto a trunk stays in the file as the
  transaction found it, and a page the free list gives back in a later
  transaction reads as noughts.
- Each is built here from the same statements and is the fixture byte
  for byte, and so was every one of the four hundred prefixes of
  `emptied.db` while the port was written.
- The refusals: a free list asked for page one or for a page the
  database does not hold, and an overflow chain that turns back on
  itself.

### 6.6.110 The journal a commit writes (`db-sqlite`)

D-170, document 16 step Q7. `journal::write` and `journal::committed`,
held to the journals the shell left beside its databases.

- `journalled.db-journal` is what the delete that makes `emptied.db`
  wrote: nine records, the pages the delete changed, in the order it
  opened them.
- `appended.db-journal` is what the insert that makes `shuffled.db`
  wrote: the leaf the rows went on and then page one, which the commit
  writes the change counter into.
- The nonce every checksum begins at comes from SQLite's random source,
  so each test reads the nonce out of the fixture's first record and the
  records after it are what say the nonce is right.
- Each journal is written here and is the fixture byte for byte, the
  noughts `persist` mode writes over the header included.
- The journal this crate writes is one it reads back: the pages
  `Journal::open` restores are the pages the transaction began with.
- The five journal modes: `delete`, `memory` and `off` leave no file,
  `truncate` an empty one, and `persist` the records under a header of
  noughts.

### 6.6.111 The log a commit writes (`db-sqlite`)

D-171, document 16 step Q7. `wal::Log`, held to the log the shell left
beside a database in write-ahead logging mode.

- `logging.db` is what `PRAGMA journal_mode=wal` left: one page, with
  the write and the read version two, and nothing after it, because the
  log was never checkpointed.
- `logging.db-wal` holds three transactions in thirty-two frames: the
  schema, four hundred rows, and a delete that frees no page.
- The delete is what shows the rule for page one: the count of pages
  does not change, so the log holds no page one for that transaction,
  where the two before it do.
- The two salts come from SQLite's random source, so the test reads them
  out of the fixture's header and every checksum of every frame is what
  says they are right.
- The log this crate writes is one it reads back, under either byte
  order of the checksum.

### 6.6.112 A statement run from its text (`db-sqlite`)

D-172, document 16 step Q8. `change::Writer`, held to the files the
shell wrote from the same statements.

- `small.db`, `utf16.db` and `joins.db` are built here by running the
  statements the shell was given, and each is the fixture byte for byte:
  the four storage classes, an encoding the text is stored in, and three
  tables each created and then filled.
- `stated.db` is the fourth: a table whose key is one of its columns,
  rows with the key given and rows without, columns named in another
  order, and rows read out of one table into another, which is
  `INSERT ... SELECT`.
- A statement is stored as its own text, so the space around it and the
  semicolon that ends it make no difference to the file.
- The refusals: a statement this crate does not write, a table the
  database does not hold, a column the table does not have, a row of
  another width, a key that is not a whole number, a table whose rows
  are kept in the key's own tree, and a schema that outgrows page one.

### 6.6.113 Rows taken out by a statement (`db-sqlite`)

D-173, document 16 step Q8. `DELETE` run from its text.

- `shuffled.db` is written here by one statement that names the key of
  every row, and `deleted.db`, `emptied.db` and `cleared.db` are the
  three deletes over it, each the fixture byte for byte.
- The `WHERE` is read against one row at a time: a column written with
  the table's name before it, one written without, the three names of
  the key, and a call that answers the bytes as they are stored, which
  is the encoding the file names.
- A statement that keeps no row writes no page, so the change counter
  stands where it stood and the file does not change at all.
- The refusals: a column the row does not have, a schema or a table the
  row does not belong to, a table the database does not hold, and a
  table whose rows are kept in the key's own tree.

### 6.6.114 Rows written over by a statement (`db-sqlite`)

D-174, document 16 step Q8. `UPDATE` run from its text.

- `updated.db` writes over rows that keep, shrink and grow their
  payloads, `overwritten.db` writes over a payload the length it was and
  over a cell the length it was, `moved.db` moves the key under both of
  its names and drops an overflow chain, and `keyed.db` writes over
  every row of a table that holds no column the key is another name
  for, each the fixture byte for byte.
- The three paths of `tree::update`: the payload that lies where it lay
  and keeps its chain, the cell that lies where it lay, and the cell
  dropped from the leaf and put in again.
- A key the table does not hold writes over no row, whether the key runs
  past the last row or falls between two rows.
- The refusals: a column the `SET` names that the table does not hold, a
  key written as text that is not a number, a `WITH` before the
  `UPDATE`, a `SET` without an `=`, and text after the statement.

### 6.6.115 The journal mode a statement commits under (`db-sqlite`)

D-175, document 16 section 16.11. The journal-mode dimension of the
matrix over the write path.

- `change::Writer` runs the same three statements under `delete`,
  `truncate`, `persist`, `memory` and `off`, and writes `emptied.db`
  under every one of them, so the mode changes what lies beside the file
  and not what is in it.
- What each mode leaves: nothing for `delete`, `memory` and `off`, an
  empty file for `truncate`, and `journalled.db-journal` byte for byte
  for `persist`.
- Under `wal` the file stays as `PRAGMA journal_mode=wal` left it, the
  three statements are three transactions of the log, and the log is
  `logging.db-wal` byte for byte.
- The counter a frame of page one carries is the counter the file
  carries and one, because no checkpoint writes the file, which is what
  the second and third transactions of the log show.

### 6.6.116 The pointer maps a file that vacuums itself keeps (`db-sqlite`)

D-177, document 16 section 16.11. The auto-vacuum dimension of the
matrix over the write path.

- `v-full.db` and `v-incremental.db` hold the same four hundred rows
  under the two settings, `v-chained.db` holds chains that cross the
  second pointer-map page, `v-freed.db` holds the free pages
  `incremental` keeps, and `v-moved.db` and `v-moved-chained.db` hold
  the pages `full` moves down and gives up at the commit, each the
  fixture byte for byte.
- Page two is the first map page and the first table takes page three;
  a page taken at the end of the file that falls where a map lies makes
  that map page and the caller takes the page after it.
- The kinds a map holds read back: a root, a page of a tree, the first
  page of a chain, a later page of one, and a page on the free list.
- The refusals: a page no map carries an entry for, an entry that says
  no kind of page, a map that says a root lies past the end the file is
  cut back to, and a map that asks the free list for more pages than it
  holds.

### 6.6.117 The covering array of the matrix (`db-sqlite`)

D-178, document 16 section 16.11. The five dimensions the write path
answers, put to it together.

- Thirty configurations, `x-01.db` to `x-30.db`, in which every value of
  the encoding, the page size, the reserved tail, the journal mode and
  the auto-vacuum setting appears, and every pair of values from two of
  them appears together at least once.
- Each holds the same four hundred rows and is the file the shell wrote,
  with the journal or the log the mode leaves beside it, byte for byte.
- The pair property is counted rather than asserted: the test names the
  hundred and fifty-six pairs and fails where a row no longer covers
  one.

### 6.6.118 The schema format a file was written under (`db-sqlite`)

D-179, document 16 section 16.11. The schema-format dimension of the
matrix.

- `format1.db` is written by the oracle through
  `SQLITE_DBCONFIG_LEGACY_FILE_FORMAT`, because no pragma the shell
  takes asks for a format below four. `format3.db` is the same file with
  two columns added after its rows, and `format4.db` is what the shell
  writes by default.
- What the formats differ in is what they store: format 4 stores the
  whole numbers 0 and 1 under serial types 8 and 9 with no payload, so
  its first row is five bytes where format 1 writes seven.
- What they answer is the same, which fourteen cases of `query.corpus`
  over the three files hold to the C library.
- A column the rows are short of answers what it falls back to, and
  nothing where it has none. `defaults.db` holds the same rule over the
  write path: a statement naming one of three columns writes the file
  the shell wrote, byte for byte.

### 6.6.119 SQLite's own test files (`db-sqlite`)

D-180, document 16 step Q9. `sh tools/xtask.sh sqlite-suite`.

- The part of `research/sqlite/test` that needs no TCL interpreter: a
  `do_execsql_test`, a `do_test` whose body is one `execsql`, and the
  `execsql` or `db eval` a file sets itself up with, each carrying no
  substitution. That is 15 364 cases in 1 171 files. Setup written as a
  quoted string carries one, so the file stops there, which D-194
  records. `db null` and
  `db nullvalue` say what a `NULL` prints as, which the answer a file
  writes is written under. `ifcapable !X` holds a block for a build
  without `X`, which is read past where this engine has `X`.
- A step this harness cannot run — a body that runs more than
  statements, or statements a substitution stands in — stops the file
  the way a refusal does, because the database is then short of what
  the cases after it read.
- The block of an `else` is read past and a case whose name repeats is
  dropped, because the interpreter runs one arm of a conditional, which
  D-192 and D-189 record.
- The database of a file is written at a page size of 1024, which is
  what `testfixture` is built with and what the answers the files write
  were recorded under.
- A statement reaches the connection that reads or the one that writes
  by its first word, read with the comments taken out, and a `WITH`
  clause stands in front of a statement that writes as well, so the
  words after it say which.
- A real is written with fifteen significant digits, which is what
  `tester.tcl` sets `tcl_precision` to and what the interpreter wrote
  into the answers the files hold, which D-212 records.
- A step this harness cannot run stops the file only where the text it
  stands for may have changed the database, which D-213 records.
- 2333 pass, 6 answer differently and 13 025 are refused or stopped.
  Document 16, section 16.23 groups the seven. Earlier runs answered
  14, then 27, then 17, then 13, then 16 differently; twenty-five were
  defects, which D-181, D-184, D-189, D-197 and D-205 record and
  twenty-eight cases of `query.corpus` now hold to the C library.
- `--why` counts what each refusal was for by the first two words of
  the statement, which is what says which missing feature stops the
  most files. It named a `PRAGMA` for sixty of them, and D-182 answers
  those.

### 6.6.120 The pragmas that configure a file (`db-sqlite`)

D-182 and D-190, document 16 step Q8.

- The six files of the auto-vacuum dimension are written a second time
  with `PRAGMA page_size`, `PRAGMA encoding` and `PRAGMA auto_vacuum`
  in place of the three calls, and each is the same fixture byte for
  byte, which is what says the two ways of configuring a file agree.
- What each pragma answers out of a header: the page size, the reserved
  tail, the encoding, the auto-vacuum setting, the journal mode, the
  page count, the free list count, the schema version, the user
  version, the application id and the schema format.
- `PRAGMA journal_mode=X` answers the mode it left the connection in,
  which is the one setting that answers a row. It is set after a table
  is there as well, and `wal` turns write-ahead logging on with the
  salts of D-190.
- A pragma that sets nothing is answered out of the connection, which
  holds the encoding a file with no table does not.
- The refusals: a pragma this crate does not answer, a value it does
  not name, a mode other than `wal` put to a connection that is
  logging, a setting that says how the first table is written after a
  table is there, a pragma that sets something put to a file being
  read, and a pragma the file does not hold read back.
- The checkout is not part of this repository, so this is never a step
  of `cargo xtask check`; `sh tools/sqlite.sh` brings it and
  `--file <name>` runs one file. `--show` prints each case that did not
  pass with what each side answered.

### 6.6.121 The index trees a statement writes (`db-sqlite`)

D-186, document 16 step Q8.

- Nine fixtures, `index-*.db`: an index over no row at all, over a few,
  over terms with a collation and an order of their own, over a key that
  runs onto a chain, over every storage class with two rows sharing a
  value, over enough rows to fill a second leaf, and over enough to need
  a page above the leaves. Each is the file the shell wrote, byte for
  byte.
- `index-kept.db` and `index-added.db` hold what an `INSERT` writes into
  an index that is already there, one of them landing in a leaf that is
  not the last, which is reached through a child pointer.
- `CREATE INDEX` sorts its entries and fills the left page before the
  right one is begun; an `INSERT` into an index that is already there
  evens the two out. The two write different files, which is what
  `index-deep.db` and `index-added.db` hold apart.
- The refusals: an index over an expression, a partial index, an index
  over a table the database does not hold, and an index tree deeper than
  the walk goes.

### 6.6.122 The entries a statement takes out of an index (`db-sqlite`)

D-187 and D-188, document 16 step Q8.

- Seven fixtures: `index-gone.db` takes a third of the entries off the
  leaves of a tree of two; `index-hollow.db` takes half out of a tree
  whose root holds entries as well, so an entry moves up from the leaf
  under the one that went; `index-emptied.db` takes every row of a
  table; `index-moved.db` and `index-rekeyed.db` are what an `UPDATE`
  writes again, one over a term and one over the key; `index-alias.db`
  is an index over the column the rowid is another name for, whose
  entries hold the key twice; `index-tall.db` is nine hundred rows whose
  text runs from four bytes to fifty-seven, so its index is three levels
  deep. Each is the file the shell wrote, byte for byte.
- `every_entry_taken_out_of_an_index_leaves_the_ones_beside_it` puts
  five hundred entries in a tree under eleven shapes of length, page
  size and removal order and takes every one of them out again, reading
  the rest back every sixteenth removal. This is what found the overflow
  pointer of D-188.
- `an_entry_moved_up_into_a_page_with_no_room_for_it_is_balanced_with_it`
  builds a root nearly full of short dividers over thirteen leaves and
  takes the first divider out, so the entry that moves up is one the
  root has no room for and the balance of the leaf reads it as a
  divider.
- The refusals: a tree deeper than the walk goes, on the descent to the
  entry and on the walk to the entry before it.

### 6.6.123 What a `DROP` takes away (`db-sqlite`)

D-191, document 16 step Q8.

- Six fixtures, `drop-*.db`: a table of two rows taken out beside one
  that stays, four hundred rows whose tree has a page above its leaves,
  two rows that run onto chains, a table an index is over, the index
  alone, and the table whose root is the last page of the file. Each is
  the file the shell wrote, byte for byte.
- The refusals: a name the database does not hold, and a table asked
  for as an index. `IF EXISTS` is what makes a name that is not there
  no refusal.
- A tree that names itself is one the walk that frees a tree stops in.
- `PRAGMA count_changes`: a connection that counts answers one row per
  statement that changes rows, which is how many it changed, and
  nought for a statement that changed none; a value that is not a truth
  is refused.

### 6.6.124 A transaction that spans statements (`db-sqlite`)

D-193, document 16 step Q8.

- `tx-one.db`: the statements between a `BEGIN` and a `COMMIT` write
  the file the same statements write on their own, and the change
  counter counts the transaction once.
- `tx-grown.db`: a transaction that frees pages and takes one of them
  back leaves the free list where the same statements leave it.
- `tx-back.db`: what a `ROLLBACK` leaves is `tx-kept.db` byte for byte,
  the rows are the ones the transaction began with, and the connection
  writes on from there.
- The refusals: a `BEGIN` inside a transaction, a `COMMIT` or a
  `ROLLBACK` outside one, and a `ROLLBACK TO`, which names a savepoint.
  `DEFERRED`, `IMMEDIATE`, `EXCLUSIVE` and the word `TRANSACTION` stand
  beside the three without changing what they do.

### 6.6.125 A view, which is a named statement (`db-sqlite`)

D-195, document 16 step Q8.

- `view-one.db`: a view over a table, whose rows are the ones its
  statement answers, read where the view is named; it stands in a join,
  under an aggregate, and reads a row written after the view was made.
- `view-named.db`: the names a definition writes for its columns are
  the names the view answers them under.
- `view-gone.db`: a `DROP VIEW` takes the row away and frees no page.
- The refusals: a `DROP VIEW` over a table, a `DROP TABLE` over a view,
  a name the schema does not hold, and a view that names itself, which
  the reader stops in.
- A view row whose statement the reader cannot read, and one that calls
  itself a view and holds a table, are walked past, which the crafted
  schema of `a_schema_row_that_is_not_a_table_is_passed_over` holds.

### 6.6.126 A column added to a table (`db-sqlite`)

D-196, document 16 step Q8.

- Three fixtures, `alter-*.db`: a column with a fallback, one with
  neither type nor fallback, and one added to a table that carries a
  constraint after its columns, which the column is written in front
  of. Each is the file the shell wrote, byte for byte.
- A row written before the column answers what the column falls back
  to; a row written after it holds a value of its own.
- The refusals: a table the database does not hold, a column that is
  `PRIMARY KEY` or `UNIQUE`, and one that may not be nothing and falls
  back to nothing. One that may not be nothing and falls back to
  something is allowed.
- The row the statement writes again is found by name and by kind, so
  the walk passes over a table of another name and over a row that is
  not a table.

### 6.6.127 The names a statement writes in quotes (`db-sqlite`)

D-197, document 16 step Q9.

- One column reached under all four quotes and bare: `a`, `"a"`,
  `[a]`, `` `a` ``, and `"t"."a"` over `FROM "t"`.
- The same column reached through a quoted name of every other kind:
  an `ORDER BY` term, an `ORDER BY` term that matches an alias, a
  `GROUP BY` term, a `COLLATE`, a function call and a `CAST` type.
- A quote doubled inside a name is one quote of the name, so the
  column `"""cb"""` is called `"cb"`: the star, the qualified star
  and the name itself each answer its value, and the answer names it
  with the two quotes it holds.
- `true` and `false` written bare and with no table in front of them
  are one and nought, under any case, and they add, compare and count
  as integers.
- A column of either name answers instead, and the same name in quotes
  is a column and never a number: `SELECT "false" FROM u` over a table
  with no such column is refused.
- The rows a second `RIGHT JOIN` answers: `t2` holds nothing, so the
  first join answers the two `t3` rows, each matches the one `t4` row,
  and the second join adds no row of its own.

### 6.6.128 A `WITH` term that reads itself (`db-sqlite`)

D-198, document 16 step Q8.

- The rows the walk answers, in the order it took them off the front:
  one core in front of the recursive one, two cores in front of it,
  and a `UNION` in front of it, which answers a row once.
- A `WITH` clause in front of an `INSERT` names the term the rows come
  from.
- A term of a `WITH` written `RECURSIVE` that reads nothing of its own
  is answered once, like any other.
- The refusals: a term whose first core reads it, a recursive core
  under `EXCEPT` or `INTERSECT`, and a recursive core that answers a
  different number of columns from the cores in front of it.
- A term that does not stop is refused at `RECURSION_ROWS` rows, which
  is where this crate answers rows into memory rather than handing one
  row on as SQLite does.

### 6.6.129 The bytes a seed draws (`db-sqlite`)

D-199, document 16 step Q8.

- A connection told a seed writes rows of `randomblob(400)`, and the
  two rows hold four hundred bytes each and hold different bytes,
  because each statement draws from where the connection stands.
- Two databases told the same seed answer the same bytes.
- `randomblob` answers one byte where the count is less than one, and
  a count above `SQLITE_MAX_LENGTH` is refused.
- An expression answered against no connection is refused with
  `Error::NoRandom`, which is what a generated column reads as.

### 6.6.130 The schema tree past one page (`db-sqlite`)

D-200, document 16 step Q8.

- `schema-deep.db` is eight tables at a page size of 512, whose schema
  tree is a root over four leaves. What this crate writes is that file
  byte for byte, the bytes of the blank record included.
- Twenty schema rows written into page one leave page one an interior
  page whose tree holds all twenty.

### 6.6.131 The pragmas a connection keeps (`db-sqlite`)

D-202, document 16 step Q8.

- What a connection told nothing answers for each of them, which is
  the shell's own answer for a database with nothing set.
- Setting one answers the value it was set to where that pragma
  answers a row, and nothing where it does not; what was set is what
  the connection answers after.
- `mmap_size` stands at nought and `data_version` at one, whatever a
  statement sets them to.
- The four shapes a value is written in: a whole number with the minus
  sign a cache size carries, a truth value, `normal` or `exclusive`,
  and the words `off`, `normal`, `full` and `extra`.
- The refusals: a value the pragma does not name, in each shape.
- A database being read answers what a connection told nothing
  answers.

### 6.6.132 A table made from a statement (`db-sqlite`)

D-203, document 16 step Q8.

- Eight fixtures, `as-*.db`. Each is the file the shell wrote, byte
  for byte, the statement the table is written as included.
- `as-typed.db` reaches every affinity a column type is written from;
  `as-quoted.db` reaches every reason a name is quoted, a name two
  columns share and a name that is `true`; `as-wider.db` is the
  layout of one column per line and a column place of two digits;
  `as-none.db` holds no row; `as-vacuum.db` is a file that vacuums
  itself.

### 6.6.133 A trigger (`db-sqlite`)

D-204, document 16 step Q8.

- Six fixtures, `trig-*.db`: one trigger, three that say which runs
  first, an `UPDATE OF`, a `DELETE`, a `WHEN`, and one taken away
  again. Each is the file the shell wrote, byte for byte, the row of
  `sqlite_schema` included.
- The row the body reads: `new` for an `INSERT`, `old` for a
  `DELETE`, both for an `UPDATE`, the key under the three names the
  rowid answers to, and no column for a name that is neither.
- `RAISE(ABORT)` refuses the statement and leaves the file as the
  statement found it; `RAISE(IGNORE)` passes the row over, in a
  `BEFORE INSERT`, a `BEFORE UPDATE` and a `BEFORE DELETE`.
- A trigger reaches itself once, and reaches as deep as
  `TRIGGER_DEPTH` where `PRAGMA recursive_triggers` is on.
- The refusals: a table the database does not hold, `INSTEAD OF`, a
  name the database already holds unless the statement allows it, and
  a name it does not hold unless the statement allows it.
- A trigger and an index may share a name, and a `DROP` takes away
  the one of its own kind; a `DROP TABLE` takes the triggers on the
  table with it.

### 6.6.134 The index a key carries (`db-sqlite`)

D-205, document 16 step Q8.

- Eight fixtures, `key-*.db`: one constraint of each kind, a rowid
  alias, two constraints over one table, a collation, a row taken out
  again, a key written over, an entry and a row that both run onto
  chains, and a file that vacuums itself. Each is the file the shell
  wrote, byte for byte, the row of `sqlite_schema` the index carries
  included.
- The entry of every index is written before the row, which is what
  says a row whose entry runs onto a chain takes the pages of the
  entry first.
- A row that shares a key: refused under `ABORT`, stopped under `FAIL`
  with the rows before it kept, passed over under `IGNORE`, written
  over under `REPLACE`, and the clause the index carries where the
  statement names none.
- A key one of whose columns is nothing constrains no row, and an
  index that is not unique constrains none.
- Thirteen statements name the indexes a table carries of its own,
  which the pinned shell answers: the rowid's own `PRIMARY KEY` gains
  none and a `UNIQUE` over that column gains one, a constraint over
  the columns of one already there gains none whatever order it writes
  them in, and the index there takes the clause of that constraint
  where it carries none.
- A `REPLACE` runs the `BEFORE DELETE` and `AFTER DELETE` triggers on
  the row it writes over, and a `RAISE(IGNORE)` in the `BEFORE` one
  passes the row over.
- A row of `sqlite_schema` that names an index of a table's own
  carries one only where the table and the constraint the name points
  at are both there.

### 6.6.135 A text that holds no statement (`db-sqlite`)

D-206, document 16 step Q8.

- `crate::parse::blank` answers true for a line comment and for a
  block comment with semicolons after it, and false for a statement
  written under a comment.
- The connection that reads answers no name and no row for one; the
  connection that writes answers no row and leaves every byte of the
  file as it found it.

### 6.6.136 What `ANALYZE` counts (`db-sqlite`)

D-207, document 16 step Q8.

- Ten fixtures, `stat-*.db`: two indexes over one table, a table with
  no index, a table with no row, an index a `PRIMARY KEY` carries, two
  tables, a second run over a table a first one counted, a run over
  one table, a run over one index, a collation, and a run over one
  table that keeps the rows of the other. Each is the file the shell
  wrote, byte for byte.
- The row of an index holds the number of rows and one count per
  prefix of its columns; the indexes are written with the one made
  last first, and the tables with the one made last first.
- A run takes out the rows a run before it wrote: every row for a
  whole database, the rows of the table for one table.
- `ANALYZE` makes `sqlite_stat1` where the database holds none, and
  counts no table whose name the word `sqlite_` begins; a name that
  begins with that word is a table it counts nothing of rather than a
  refusal.

### 6.6.137 Where a term is read and what names a key (`db-sqlite`)

D-208, document 16 step Q5.

- A join whose `ON` names the key of an index answers the rows a scan
  of the whole table answers: the key the other way round, a second
  term of the `AND` spine, a key of nothing under a `LEFT` join, and a
  `RIGHT` join.
- What names no key: an index held in another order, one held under
  another collation than its column compares under, a term that is not
  an equality, a term of two columns of the side itself, and a term
  over a column no index of the side is over.
- A term of every shape the walk answers, read on its own level: a
  `BETWEEN`, an `IN` over a list, a `LIKE`, a `CAST`, a `COLLATE`, a
  `CASE`, and an operator with one operand.
- A term that stays where the statement is answered: a function, a
  statement of its own, an `EXISTS`, an `IN` over a statement, a name
  no side answers, and a name two sides answer.
- A term above a `RIGHT` join is read on the level of that join.

### 6.6.138 The entries `REINDEX` writes again (`db-sqlite`)

D-209, document 16 step Q8.

- Five fixtures, `re-*.db`: every index of the schema, one index by
  name, the indexes of one table, the indexes of one collation, and
  the index a `PRIMARY KEY` carries. Each is the file the shell wrote,
  byte for byte.
- The refusals: a name neither a collation, nor a table, nor an index
  carries, and a word written after the name.
- A table with no index of its own, and a schema with no index at all,
  are left as they stand.

### 6.6.139 What the integrity check answers (`db-sqlite`)

D-210, document 16 step Q8.

- A file the check finds nothing in answers the one word `ok`: a
  table with a key and an index, a row taken out again, a row that
  runs onto a chain, a tree of more than one level, an index over a
  value of every class, a file that vacuums itself, and a file with a
  free list.
- A file written over answers what it holds: a page no tree names, a
  page two trees name, a column that may not be nothing holding
  nothing, a row no index holds an entry for, an index whose count of
  entries is not the count of rows, and two entries of a unique index
  that hold one value.
- A chain that runs onto itself and a free list that names its own
  trunk are each walked once round and no further.
- A hundred and one pages no tree names answer a hundred problems.

### 6.6.140 A name the schema already holds (`db-sqlite`)

D-211, document 16 step Q8.

- A table, an index and a view are one namespace: a `CREATE` of any
  of the three over a name any of the three holds is refused.
- A trigger is named in its own namespace: a trigger and an index may
  share a name.
- `IF NOT EXISTS` writes nothing rather than refusing, for all four
  and for a table made from a statement.

### 6.6.141 What a compound and a recursive term are ordered by (`db-sqlite`)

D-214 and D-215, document 16 step Q5.

- The `ORDER BY` of a compound compares under the `COLLATE` its term
  was written with, under the collation of the column where the term
  wrote none, and backwards where the term counts to a column and says
  `DESC`.
- A recursive term takes its rows in the order they were written where
  it wrote no `ORDER BY`, and smallest first where it wrote one.
- The `LIMIT` of a recursive term bounds the rows it answers, an
  `OFFSET` passes the first over, and a count below nought bounds
  nothing.

### 6.6.142 The schema as a table, and the keys that count up (`db-sqlite`)

D-216, D-217 and D-218, document 16 step Q8.

- `sqlite_master` answers the five columns of every row of the schema,
  under each of the four names it carries, and no statement writes it.
- A row of the schema holds the statement from the name on: neither
  `TEMP` nor `IF NOT EXISTS`, and `CREATE UNIQUE INDEX` whole.
- `sqlite_sequence` is made with the first table that counts its keys
  up and holds no row until one is written; a key given back by a
  `DELETE` is not given out again; a key the statement names that is
  smaller than the count leaves it; and a `DROP TABLE` takes the count
  away.

### 6.6.143 Window functions (`db-sqlite`)

D-219, document 16 steps Q4 and Q8.

- `FILTER (WHERE x)` and `OVER` follow the closing bracket of a call,
  and `WINDOW name AS (...)` follows a `HAVING`; each of the three
  words is a name anywhere else.
- A window names its partition, its order and its frame, and a window
  that names another takes that window's partition and order.
- A frame is counted in `ROWS`, in `RANGE` or in `GROUPS`, between two
  of the five bounds, with the rows one of the four `EXCLUDE` words
  leaves out taken away.
- A frame that ends before it begins is refused, and so is `UNBOUNDED
  FOLLOWING` where a frame begins and `UNBOUNDED PRECEDING` where one
  ends.
- The eleven built-in window functions and every aggregate answer over
  a window; `DISTINCT` is refused and `FILTER` is taken for an
  aggregate alone.
- A statement that writes no `ORDER BY` of its own answers its rows in
  the order the first window's terms sort them.
- 13 667 statements over one table, each written in both engines,
  answer the same rows: every function over every frame over six
  windows, and the refusals as well.

### 6.6.144 What a step the harness cannot run stands for (`db-sqlite`)

D-220, document 16 step Q9.

- A bracketed command the harness knows reads — `db eval`, `execsql`,
  `catchsql`, and the list and string commands — leaves the step
  reading alone, so the file runs on.
- A variable no longer stops a file on its own: the text it stands for
  was written by a step of the same file, and that step is what stops
  it.
- `do_execsql_test NAME { SQL }` with no answer after it expects no
  row, and an answer of one word needs no braces.
- A step whose text the harness read past says for itself whether it
  may have written, and `execsql` written as a quoted string stops the
  file whatever stands around it.

### 6.6.145 What a column of a compound converts (`db-sqlite`)

D-221, document 16 step Q5.

- A compound whose cores agree on a numeric affinity keeps it; one of a
  numeric core and a text core converts nothing.
- A compound whose first core has no affinity takes the affinity of the
  first core after it that has one.
- A table made from such a statement is written with the type the
  affinity is written as, and with none where the affinity converts
  nothing.
- A join against such a column compares a number against text as the
  classes sort, and a join against a view over one table converts as
  that table's column does.

### 6.6.146 SQLite's own test files under `tclsh` (`db-sqlite`)

D-223, document 17.

- Every command of a file runs, so a case is refused only where the
  engine refused a statement or where a command needs the C library's
  internals.
- `tools/suite/tester.tcl` answers `do_test`, `do_execsql_test` and
  `do_catchsql_test`, and compares an answer as SQLite's own tester
  does: `/RE/`, `~/RE/`, `#/A..B/`, `*GLOB*`, and otherwise the text.
- One database is held per path a connection opened, so a file that
  opens `db2` beside `db` reads what `db` wrote.
- Each file runs in a process of its own and is ended after sixty
  seconds, with the cases it ran counted.
- `sh tools/xtask.sh sqlite-suite` needs the `tclsh` of the machine and
  the checkout `sh tools/sqlite.sh` brings, so it is never a step of
  `cargo xtask check`.

### 6.6.147 The rows a foreign key holds (`db-sqlite`)

D-224, document 16 step Q8.

- A row whose key names no row of the table it points at is refused,
  and a key any column of which is null is refused by nothing.
- A key that names no columns points at the primary key of the table it
  points at, in the order that primary key was written.
- A key whose columns are neither the primary key nor a unique index of
  the table it points at is a mismatch, whatever rows either table
  holds.
- `ON DELETE` and `ON UPDATE` run `CASCADE`, `SET NULL`, `SET DEFAULT`,
  `RESTRICT` and `NO ACTION`, and a cascade reaches the rows that point
  at the rows it wrote.
- `PRAGMA foreign_key_list` answers the keys of a table newest first,
  and `PRAGMA foreign_key_check` answers a row per orphan whatever
  `PRAGMA foreign_keys` says.

### 6.6.148 The rows a statement compares (`db-sqlite`)

D-225, document 16 step Q8.

- A row compared against a row answers what the C library answers for
  every operator that takes one, nulls included.
- A row looked for among rows, and a row compared against a statement
  that answers as many columns, answer the same.
- A row written where one value belongs is refused before a row of a
  table is read, so a statement over a table of no rows refuses as
  well.
- A row misused in the body of a view is refused where a statement
  names the view.
- A statement used as one value compares under the affinity of the
  column it answers.

### 6.6.149 The column a `USING` names (`db-sqlite`)

D-226, document 16 step Q8.

- A `RIGHT JOIN` followed by another join answers the rows the C
  library answers, the column the `USING` names standing for the side
  that filled it.
- A row a `RIGHT JOIN` keeps whose own column is nothing matches no row
  of the side after it.

### 6.6.150 Tables inside brackets in a `FROM` (`db-sqlite`)

D-227, document 16 step Q8.

- A bracket holding a join reads that join first, and the join written
  after the bracket is against the rows it answers.
- A bracket holding one table answers the rows that table holds.
- `joinB.test` runs every one of its 512 cases.

### 6.6.151 The tables inside brackets answer under their own names (`db-sqlite`)

D-228, document 16 step Q8.

- A column written with the name of a table inside the brackets reaches
  that table, and the name the brackets carry reaches the same columns.
- A `*` answers the column a `USING` matched once, whatever brackets
  stand around the tables.
- A `GROUP BY` that counts to a column counts the ones a `*` answers.
- A statement written inside the `FROM` answers under its own name
  alone, and the tables it reads are not reachable through it.

### 6.6.152 The name a refusal carries (`db-sqlite`)

D-229, document 17 step T6.

- A statement that names a table the schema does not hold answers `no
  such table: NAME`, whatever the statement does with the table.
- A name no table answers to is `no such column: NAME`, with the table
  and the schema in front of it where the statement wrote them.
- A function no name matches is `no such function: NAME`, and one
  called with another number of arguments is `wrong number of
  arguments to function NAME()`.
- A `COLLATE` naming a collation the connection does not hold is `no
  such collation sequence: NAME`.

### 6.6.153 The constraints a row is held to (`db-sqlite`)

D-230, document 16 step Q8.

- A column written `NOT NULL` refuses a row that holds nothing there,
  whether an `INSERT` or an `UPDATE` wrote it.
- A `CHECK` that answers false refuses the row, under the name the
  `CONSTRAINT` gave it or the text of the expression; one that answers
  nothing holds.
- A key the table already holds refuses the row, and the message names
  the columns the key is over.
- `IGNORE` passes the row over, `REPLACE` writes what the column falls
  back to, and `FAIL` stops the statement and keeps the rows before it.

### 6.6.154 The pragmas a constraint is read under (`db-sqlite`)

D-231, document 16 step Q8.

- `PRAGMA ignore_check_constraints` leaves every `CHECK` unread while
  it is on, and reads them again when it is off.
- A name written in double quotes that no table answers carries the
  question the C library asks.

### 6.6.155 The date and time functions (`db-sqlite`)

D-232, document 16 step Q8.

- Every shape of moment the C library reads is read the same: a date, a
  time, a date and a time, a julian day number and a unix time.
- Every modifier answers what the C library answers, and a word that is
  near one but not it answers nothing.
- Every conversion of `strftime` writes what the C library writes.
- A call that asks for the clock answers nothing.

### 6.6.156 A connection over a database that is already written (`db-sqlite`)

D-233, document 16 step Q8.

- A connection that opens a written image writes rows into the tables it
  holds, and the indexes beside them answer those rows.
- A connection that opens such an image writes onto the free list the
  image carries.
- A file that keeps pointer maps keeps them when a connection opens it
  again.
- A header that counts no pages is read as the pages the image carries.
- An image that carries fewer pages than the header counts is refused,
  and so is one that is not a database.
- The encoding the image was written under is the encoding the new rows
  are written under.

### 6.6.157 A table that keeps its rows in the key's own tree (`db-sqlite`)

D-234, document 16 step Q8.

- A row written into such a table is read back, by the key and by a
  column the key does not name.
- A row is written again and taken out again, with the entries of the
  index beside it.
- A key a row already holds is refused, passed over or written over, by
  what the statement says.
- The file this crate writes for such a table is the file the shell
  wrote for it, byte for byte.

### 6.6.158 The JSON functions (`db-sqlite`)

D-235, document 16 step Q8.

- Every text the reader takes for JSON, and every form JSON5 adds to it,
  is written out again as the C library writes it.
- Every call of the twenty-six names and the four aggregates answers
  what the shell answers for the same arguments, over some seven hundred
  cases.
- A blob in the binary form answers the value the text it stands for
  answers, and a blob that holds no whole element is refused.
- Every escape a label of a blob holds is read when a path names that
  label.
- The fifteen cases of appendix A of RFC 7396 are patched as the
  document says.
- A path, a document and a patch that nest a thousand deep are each
  refused with their own message.

### 6.6.159 Savepoints (`db-sqlite`)

D-236, document 16 step Q8.

- A `SAVEPOINT` outside a transaction opens one, and the release of that
  savepoint writes the file.
- A `ROLLBACK TO` takes out what was written after the savepoint and
  leaves the savepoint open.
- A `COMMIT` and a `ROLLBACK` take every savepoint with them, so a
  `RELEASE` after one is refused.
- The innermost savepoint of a name is the one a `RELEASE` and a
  `ROLLBACK TO` name, and a name in quotes is the name without them.
- A name no open savepoint holds is refused.
- A statement of a transaction that refuses leaves the transaction where
  it stood, and one that stops where it stands keeps what it wrote.
- The file a savepoint block writes, and the file a refused statement of
  a transaction leaves, are the files the shell wrote for the same
  statements, byte for byte.

### 6.6.160 A table renamed (`db-sqlite`)

D-237, document 16 step Q8.

- Every statement of the schema that names the table is written again
  under the new name: the table's own, an index over it, a trigger on
  it, and a view that reads it.
- An index SQLite made for a key of the table carries the new name.
- The row of `sqlite_sequence` that counts the key up carries it too.
- The rows of the table are read under the new name, and the index over
  it answers them.
- A column, a string and a `WITH` term that read like the table are left
  as they are.
- A new name is written in double quotes, with a quote inside it
  doubled.
- A name that is no table of its own is refused: one the schema does not
  hold, a view, a table whose name begins `sqlite_`, and a new name the
  schema already holds.
- The file this crate writes for a rename is the file the shell wrote
  for the same statements, byte for byte.

### 6.6.161 A row that reaches an `ON CONFLICT` clause (`db-sqlite`)

D-238, document 16 step Q8.

- A clause that does nothing passes the row over, and a row that shares
  no key is written.
- A clause that writes writes the row the conflict found, under the
  columns of that row and of the row that was not written.
- The clause a conflict reaches is the one over the key the row shares,
  and a conflict on another key is refused.
- A clause that names the columns of no key is refused before the
  statement writes a row.
- A clause that writes holds the row to every constraint of the table,
  and the triggers of an `UPDATE` run over it.
- A clause that writes the key moves the row, and a key another row
  holds is refused.
- The key of a table takes a whole number and nothing else.

### 6.6.162 A statement written inside one that writes (`db-sqlite`)

D-239, document 16 step Q8.

- An `UPDATE` whose `SET` reads a statement writes what that statement
  answers, and one whose `WHERE` reads one writes the rows it names.
- A row that takes the key of a row the statement has not reached is
  refused, and the statement leaves every row as it was.
- Under `OR REPLACE` the row that held the key is taken out, and the
  statement passes it over when it reaches it.
- A table that keeps its rows in the key's own tree reads such
  statements the same way, and an entry that is no longer the row that
  was read is passed over.
- The walk of `PRAGMA integrity_check` answers `ok` after each of them.

### 6.6.163 A column dropped (`db-sqlite`)

D-240, document 16 step Q8.

- The column goes out of the text that made the table and out of every
  row, and the first column and the last take the comma beside them.
- An index over a column that stayed answers the rows it held, and the
  walk of `PRAGMA integrity_check` answers `ok`.
- A column no table holds, a column a key of its own is over, and the
  one column of a table are each refused.
- A statement of the schema that still names the column refuses the
  whole statement, and the file is what it was.

### 6.6.164 The rows a statement that writes answers (`db-sqlite`)

D-241, document 16 step Q8.

- An `INSERT`, an `UPDATE` and a `DELETE` answer one row per row they
  wrote, and a statement that wrote none answers none.
- `*` answers the columns of the table, and an expression beside it is
  read against the row.
- A `DO UPDATE` of an `ON CONFLICT` answers the row it wrote, and a `DO
  NOTHING` answers none.
- A table that keeps its rows in the key's own tree answers them the
  same way.
- `INSERT INTO t DEFAULT VALUES` writes one row of what every column
  falls back to.

### 6.6.165 A page that has no room for a cell (`db-sqlite`)

Document 16 step Q8.

- A row in the middle of a leaf, written longer than the page holds room
  for, leaves every page of the tree named once, which the walk of
  `PRAGMA integrity_check` answers `ok` for.
- The rows of such a table are read back whole, whichever row grew.
- A row written past the last of them goes on a page of its own, which
  is the one place the quick balance holds.

### 6.6.166 The words a statement that writes is refused with (`db-sqlite`)

Document 16 step Q8.

- An `INSERT` that names a column the table does not hold is refused
  `table t has no column named zz`.
- An `INSERT` that names columns and answers another count of values is
  refused `3 values for 2 columns`.
- An `INSERT` that names no column and answers another count of values
  than the table has columns is refused `table t has 2 columns but 3
  values were supplied`.
- An `UPDATE` that sets a column the table does not hold is refused `no
  such column: zz`.
- A `CREATE TABLE` of a name a table or a view holds is refused `table t
  already exists`, and of a name an index holds `there is already an
  index named ti`.
- A `CREATE INDEX` of a name an index holds is refused `index ti already
  exists`, and of a name a table or a view holds `there is already a
  table named t`.
- A `CREATE TABLE`, a `CREATE INDEX`, a `CREATE VIEW` and a `CREATE
  TRIGGER` of a name that begins `sqlite_` are refused `object name
  reserved for internal use: sqlite_x`.
- `sqlite_sequence` and `sqlite_stat1` are still written, because the
  crate and not a connection writes them.

### 6.6.167 An index over an expression and over fewer rows (`db-sqlite`)

Document 16 step Q8.

- An index over `substr(a,1,7)` holds one entry per row, and the walk of
  `PRAGMA integrity_check` answers `ok` after a row is written, changed
  and taken out.
- `REINDEX` writes the same entries again out of the rows.
- An index with a `WHERE` holds an entry only for the rows that clause
  answers true for, and a row that stops answering true loses its entry.
- `ANALYZE` counts the entries an index holds and not the rows the table
  has.
- A `UNIQUE` over an expression refuses a row with `UNIQUE constraint
  failed: index 'tu'`, and a `UNIQUE` with a `WHERE` names the column.
- An index term written as text names a column, so `CREATE INDEX tb ON
  t('b')` holds the column `b` and `ALTER TABLE t DROP COLUMN b` refuses
  `error in index tb after drop column: no such column: b`.
- A term that names a column under a table is refused `the "." operator
  prohibited in index expressions`; the `WHERE` of a partial index takes
  one.
- A statement over a table that carries only an index over an expression
  and a partial index answers every row, which a lookup in either index
  would not.

### 6.6.168 The key an `ON CONFLICT` clause names (`db-sqlite`)

Document 16 step Q8.

- Against `CREATE UNIQUE INDEX xyz1 ON xyz(d,c,b COLLATE nocase)`, the
  clauses `(b COLLATE nocase, c, d)`, `(b, c, d)`, `(b, c, d) WHERE
  a!=0` and the clause that names no column all name that key.
- `(b, c COLLATE nocase, d)` and `(b COLLATE nocase, c COLLATE nocase,
  d)` write a collation the key does not hold the column in, and
  `(d, c, c)` names one column twice and another not at all; each is
  refused `ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE
  constraint`.
- `(a)` names the key of the table, so the row reaches the resolution of
  the statement and is refused `UNIQUE constraint failed: xyz.d, xyz.c,
  xyz.b`.

### 6.6.169 `ALTER TABLE ... RENAME COLUMN` (`db-sqlite`)

Document 16 step Q8.

- The table's own statement, a table that points at the column, an index
  over the table, a trigger on it and a view that reads it are all
  written again under the new name, byte for byte as the C library's
  shell writes them.
- The rows of the table answer under the new name, the old name names
  nothing, and `PRAGMA integrity_check` answers `ok`.
- A statement over another table is left alone, including its own
  columns of the same name, its `CHECK`, its `DEFAULT`, its computed
  column, its keys, its index, its trigger and its view.
- A name written under another table names that table, and `old` alone
  names the row a trigger stands on.
- A `FROM` that names no table of the schema leaves a name written under
  nothing alone.
- A column the table does not hold is refused `no such column: "zz"`,
  and a table SQLite keeps for itself `table sqlite_master may not be
  altered`.
- A rename that leaves the table with two columns of one name is refused
  `error in table t after rename: duplicate column name: b`, and the
  schema stands as it was.
- A new name the statement wrote in quotes is written in quotes.

### 6.6.170 The order a row is held to the keys in (`db-sqlite`)

Document 16 step Q8.

Against `CREATE TABLE t1(a INTEGER PRIMARY KEY, b, c UNIQUE, d UNIQUE, e
UNIQUE)` holding the row `(1,2,3,4,5)`:

- A row that shares every key reaches the first clause, so
  `ON CONFLICT(a) ... ON CONFLICT(c)` writes what the clause for `a`
  writes.
- A row that shares `a` and `c` under `ON CONFLICT(c) ... ON CONFLICT(a)`
  writes what the clause for `c` writes, because the key of the table
  stands in the place of the clause that names it.
- A row that shares `a` and `d` under `ON CONFLICT(c) ... ON CONFLICT(d)
  ... ON CONFLICT(a)` writes what the clause for `d` writes.
- A row that shares `a`, `d` and `e` under `ON CONFLICT(c) ...
  ON CONFLICT(d) ... ON CONFLICT DO UPDATE` writes what the clause for
  `d` writes, because the key of the table stands after the named keys.
- `ON CONFLICT(c) ... ON CONFLICT(c)` reaches the first of the two.
- A clause whose term is not a column is refused `ON CONFLICT clause
  does not match any PRIMARY KEY or UNIQUE constraint`.

### 6.6.171 The constraints an `ALTER TABLE` writes (`db-sqlite`)

Document 16 step Q8.

- `DROP CONSTRAINT` cuts the named `CHECK` out of the statement, and
  leaves the `NOT NULL` beside it, for each of the eight shapes
  `altercons.test` writes.
- A name a constraint of no body carries, such as the `CONSTRAINT abc`
  of a generated column, is cut on its own.
- `ALTER COLUMN ... DROP NOT NULL` cuts the `NOT NULL` of that column
  alone, named or not, and writes the statement as it stands where the
  column holds none.
- `ALTER COLUMN ... SET NOT NULL` writes the words at the end of that
  column, keeping their spacing and their case.
- `ADD CONSTRAINT nn CHECK (...)` and `ADD CHECK (...)` write before the
  bracket that closes the columns, keeping the space before it.
- A name over a `UNIQUE` is refused `constraint may not be dropped:
  ccc`, and a name the statement does not hold `no such constraint:
  ddd`.
- A `SET NOT NULL` over a column a row holds nothing in, and an `ADD
  CHECK` a row does not hold to, are refused `constraint failed`, and
  the schema stands as it was.
- A name the statement already holds is refused `constraint abc already
  exists`.
- A statement whose tokens run out, and one that holds a byte no rule
  accepts, name no constraint.

### 6.6.172 What a foreign key is compared under (`db-sqlite`)

Document 16 step Q8.

- A child column written under `BINARY` that points at a parent column
  written under `NOCASE` is compared under `NOCASE`, so a `DELETE` of
  the parent row is refused `FOREIGN KEY constraint failed`.
- A unique index of the parent's own over the columns pointed at says
  they are a key.
- An index held in another collation than the column compares under, and
  an index that is not unique, do not; both are refused `foreign key
  mismatch - "u2" referencing "u1"`, which a row that holds nothing in
  the key reaches because the key is read where the statement is.
- A key that points at a table the schema does not hold is refused `no
  such table: main.missing`.
- `PRAGMA foreign_keys` set while a transaction is open changes nothing,
  and every other pragma the connection keeps is set there.

### 6.6.173 The years and the months `timediff` counts (`db-sqlite`)

Document 16 step Q8.

- `timediff('2000-03-01','2000-01-31')` answers `+0000-00-30`, because
  the walk takes the month back where the day of the month the second
  moment lands on runs past the first.
- `timediff('2024-02-29','2023-03-31')` answers `+0000-10-29`.
- `timediff('2000-12-15','2001-01-10')` answers `-0000-00-26`, where the
  walk crosses the end of a year going forward, and the two moments the
  other way round answer `+0000-00-26`, where it crosses going back.
- `timediff('1066-10-14 00:00:00','-4713-11-24 12:00:00')` answers
  `+5778-10-19 12:00:00.000`.

### 6.6.174 A foreign key over a table with no rowid (`db-sqlite`)

Document 16 step Q8.

- A row written into a table that points at a table with no rowid, where
  no row of that table holds the key, is refused `FOREIGN KEY constraint
  failed`.
- `ON DELETE CASCADE` takes the rows that point away with the row they
  pointed at, where both tables keep their rows in the key's own tree.
- `ON DELETE SET NULL` writes nothing into the columns that point, and
  the walk of `PRAGMA integrity_check` answers `ok` after it.

### 6.6.175 What a `CREATE` and a `DROP` name in a refusal (`db-sqlite`)

Document 16 step Q8.

- `CREATE TABLE "t"(y)` and `CREATE TABLE [t](y)` over a table named `t`
  are refused `table "t" already exists` and `table [t] already exists`,
  so the name carries the quotes the statement wrote.
- `CREATE INDEX "t" ON t(x)` is refused `there is already a table named
  t`, with the quotes off.
- `DROP TABLE`, `DROP INDEX`, `DROP VIEW` and `DROP TRIGGER` of a name
  the schema does not hold each name what the statement said it takes
  away.
- `CREATE TRIGGER` and `CREATE INDEX` over a table the schema does not
  hold answer `no such table: main.nosuch`, and `CREATE TEMP TRIGGER`
  answers `no such table: nosuch`.
- `INSTEAD OF` over a table, and `BEFORE` or `AFTER` over a view, are
  refused `cannot create WORD trigger on KIND: NAME`.
- A trigger over `sqlite_master` is refused `cannot create trigger on
  system table`.

### 6.6.176 The token a refused statement is named by (`db-sqlite`)

Document 16 step Q8.

- `SELECT FROM` is refused `near "FROM": syntax error`, and `SELECT *
  FROM` is refused `incomplete input`, so a token at the stop names it
  and tokens that ran out do not.
- `UPDATE t SET a = FROM` is refused `near "FROM": syntax error`, so the
  reading that took in most of the statement says where the parse
  stopped, and `UPDATE t SET a =` is refused `incomplete input`.
- `CREATE TABLE t AS SELECT 1 WITHOUT ROWID` is refused `near "ROWID":
  syntax error`.

### 6.6.177 A statement over a view (`db-sqlite`)

Document 16 step Q8.

- `INSERT INTO v`, `UPDATE v` and `DELETE FROM v` over a view run the
  `INSTEAD OF` triggers of the view and write no row of the view.
- A view that carries no `INSTEAD OF` trigger of that event refuses the
  statement `cannot modify v because it is a view`.
- A view column converts nothing, so `INSERT INTO v VALUES('5', 7)` puts
  text and an integer in `new`.
- A column an `INSERT` names no value for holds nothing, `DEFAULT
  VALUES` writes one row of nothing, and a value written for `rowid`
  goes nowhere.
- `UPDATE v SET rowid = 1` writes nowhere, and `UPDATE v SET zz = 1` is
  refused `no such column: zz`.
- `CREATE TRIGGER ... INSTEAD OF INSERT ON t` over a table is refused
  `cannot create INSTEAD OF trigger on table: t`.

### 6.6.178 The three counters of a connection (`db-sqlite`)

Document 16 step Q8.

- `changes()` answers the rows of the last statement that changed rows,
  `total_changes()` the rows of every statement, and
  `last_insert_rowid()` the key of the last `INSERT` into a table with a
  rowid.
- A `CREATE` leaves all three as it found them, and an `INSERT` into a
  table without a rowid leaves the rowid.
- A statement of a trigger's body sets `changes()` and
  `last_insert_rowid()` as it runs; the statement that fired the trigger
  sets `changes()` again when it ends and puts the rowid back.
- A statement the engine refuses under `OE_Abort` leaves `changes()` at
  nought, and one refused under `OE_Fail` leaves it at the rows it wrote
  before it stopped.
- A foreign key action counts toward `total_changes()`.
- Two connections over one file each carry their own three counters.

### 6.6.179 The fewest bytes a cell takes (`db-sqlite`)

Document 16 step Q8.

- `CREATE TABLE t1(x INTEGER PRIMARY KEY) WITHOUT ROWID` with the rows
  1 and 2 writes the page the shell writes for the same statements: the
  cell of `1` is three bytes and takes four, the byte after it held by
  no cell.
- A `DELETE` of the row under such a cell leaves a page whose free space
  counts up, and the rows beside it stand.

### 6.6.180 A clause over a table with no rowid (`db-sqlite`)

Document 16 step Q8.

- `ON CONFLICT(a) DO NOTHING` over a table that keeps its rows in the
  key's own tree passes the row over, and `DO UPDATE` writes the row the
  conflict found.
- A clause reaches the row an index over the table found as well as the
  row the key of the table found.
- A clause that writes the key writes the entry under the new key.
- The clause writes under `OE_Abort`, so `DO UPDATE SET a='y'` where `y`
  is another row's key is refused `UNIQUE constraint failed: k.a`
  although the key was written `ON CONFLICT IGNORE`.
- The `WHERE` of a clause passes the row over, a `BEFORE UPDATE` trigger
  that raises `IGNORE` leaves the row as it stands, the `UPDATE`
  triggers of the table run over the row the clause writes, and the
  `RETURNING` answers it.

### 6.6.181 The types a `STRICT` table holds (`db-sqlite`)

Document 16 step Q8.

- `CREATE TABLE t1(a) STRICT` is refused `missing datatype for t1.a`,
  and a type that is not one of the six is refused `unknown datatype for
  t1.f: "TEXT(50)"` with the type as the statement wrote it.
- A value is converted under the affinity of the column and then held to
  its type: `INSERT INTO t1(a) VALUES('xyz')` into a column of `INT` is
  refused `cannot store TEXT value in INT column t1.a`, and a column of
  `BLOB` refuses a number `cannot store INT value in BLOB column t1.c`.
- A column of `INT` holds the text `'3'` as the number 3, one of `REAL`
  holds the number 1 as 1.0, one of `TEXT` holds the number 4 as its
  text, and one of `ANY` holds whatever it is given.
- Every column holds nothing whatever its type says.
- `ALTER TABLE t4 ADD COLUMN d VARCHAR` over a `STRICT` table is refused
  `error in table t4 after add column: unknown datatype for t4.d:
  "VARCHAR"`, and the table stands as the statement found it.

### 6.6.182 The columns a statement computes (`db-sqlite`)

Document 16 step Q8.

- `CREATE TABLE t(c INT, a INT AS (c*3) VIRTUAL, d INT, b INT AS (c*2)
  STORED)` with `INSERT INTO t(c,d) VALUES(1,9)` answers `1|3|9|2`: the
  column computed where it is read takes no place in the record, so the
  column after it answers what the statement wrote.
- `UPDATE t SET c=5` computes both columns again, answering `5|15|9|10`.
- One computed column names another, in either direction.
- A `STRICT` table holds the value a column computes to the type of that
  column, so a column of `BLOB` that computes a number refuses the row.

### 6.6.183 The four percentile aggregates (`db-sqlite`)

Document 16 step Q8.

- Over the nine values `1,4,6,7,8,9,11,11,11`, `percentile(x,15)`
  answers 4.4, `percentile_cont(x,0.15)` the same, and
  `percentile_disc(x,0.15)` answers 4.0, which is a value the group
  holds.
- `median(x)` answers 8.0 and `median(DISTINCT x)` answers 7.0.
- `f(...) WITHIN GROUP (ORDER BY Y)` is `f(Y,...)`, so `percentile(15)
  WITHIN GROUP (ORDER BY x)` answers 4.4 and `median() WITHIN GROUP
  (ORDER BY x)` answers 8.0.
- A fraction that is not a number, or one out of range, is refused `the
  fraction argument to percentile() is not between 0.0 and 100.0`; one
  that changes between rows of a group is refused `... is not the same
  for all input rows`.
- A value that is neither nothing nor a number is refused `input to
  percentile() is not numeric`, and an infinity `Inf input to
  percentile()`.
- `percentile(DISTINCT 50) WITHIN GROUP (ORDER BY x)` is refused
  `DISTINCT not allowed on ordered-set aggregate percentile()`.
- An aggregate called with a number of arguments it does not take is
  refused `wrong number of arguments to function percentile()`.
- A group of no values answers nothing, and a group of one answers that
  value whatever the fraction says.

### 6.6.184 The digit separators a number is written with (`db-sqlite`)

Document 16 step Q8.

- `1_000` is the whole number 1000, `1.1_1` the real 1.11, and
  `0x1_e` the whole number 30, because a hex literal is a whole number
  whatever its digits are.
- A separator anywhere but between two digits makes the token one the
  tokenizer read as no token at all: `1_`, `1_.4`, `1e_4`, `12__34` and
  `12.34_` are each refused `unrecognized token: "..."`.
- A letter stuck to a number is one such token as well, so `123a456` is
  refused under its own text and `1.4e+_4` under `1.4e`.
- An `ORDER BY` or a `GROUP BY` term that counts to a column the answer
  does not have names its own place: `SELECT 1,2,3 ORDER BY 1,9` is
  refused `2nd ORDER BY term out of range - should be between 1 and 3`.

### 6.6.185 The tables an `UPDATE` reads beside its own (`db-sqlite`)

Document 16 step Q8.

- `UPDATE t1 SET z=v FROM d WHERE x=k` writes each row of `t1` with the
  row of `d` the `WHERE` holds for, and counts only the rows of `t1` it
  wrote.
- A row of the table the clause holds more than one row for takes the
  last of them.
- `WITH data(k,v) AS (VALUES ...) UPDATE t1 SET z=v FROM data WHERE x=k`
  reads the term of the `WITH` as a table of the clause.
- Two tables of a clause are one join, and a column names the table it
  came from.
- A source is a table under the one schema a file holds, a statement in
  brackets with or without a name of its own, or a term of the `WITH`.
- An `UPDATE ... FROM` of a trigger's body reads `new` and `old` through
  the row of the clause.
- An `INSTEAD OF UPDATE` trigger of a view runs once per row of the
  join, duplicates included.
- `UPDATE t1 SET z='z' FROM t1` and `... FROM d AS t1` are refused
  `target object/alias may not appear in FROM clause: t1`, and a table
  under another schema is another object.

### 6.6.186 The keys a transaction is held to at its end (`db-sqlite`)

Document 16 step Q8.

- A statement of its own commits at its end, so `INSERT INTO node
  VALUES(1, 0)` under a key written `DEFERRABLE INITIALLY DEFERRED` is
  refused `FOREIGN KEY constraint failed` and the row it wrote is gone
  with it.
- Inside a transaction the `COMMIT` holds the key: the statement that
  breaks it is written, the `COMMIT` is refused, the transaction stays
  open, and a statement that mends the row lets the next `COMMIT`
  write.
- A row that appears answers the rows waiting for it, and `PRAGMA
  defer_foreign_keys` holds every key at the end of the transaction.
- A row that goes stops being counted, and a `ROLLBACK TO` puts the
  count back where the savepoint found it.
- Releasing the savepoint that opened the transaction writes it, so the
  key is held there and the savepoint stays open.
- `ON DELETE RESTRICT` is held where the row is written whatever the key
  says, and `NO ACTION` counts the rows that lose their parent.

### 6.6.187 The words a window is refused with (`db-sqlite`)

Document 16 step Q8.

- A window function where no window was worked out is refused `misuse
  of window function sum()`, a scalar under an `OVER` `trim() may not be
  used as a window function`, and either called with a number of
  arguments it does not take `wrong number of arguments to function
  row_number()`.
- A `FILTER` on one of the eleven is refused `FILTER clause may only be
  used with aggregate window functions`.
- A window that names one no `WINDOW` clause defines is refused `no such
  window: nosuch`, and one that writes again what the window it builds
  on wrote `cannot override PARTITION clause of window: win`, the
  `ORDER BY` clause and the frame specification likewise, in that order.
- `ntile(0)` is refused `argument of ntile must be a positive integer`
  and `nth_value(a,0)` `second argument to nth_value must be a positive
  integer`.
- A frame counted in rows takes a whole number, `frame starting offset
  must be a non-negative integer`, and one counted in the values of its
  term takes any, `... must be a non-negative number`.

### 6.6.188 The words an aggregate out of place is refused with (`db-sqlite`)

Document 16 step Q8.

- A `*` for the arguments leaves a call with none, so `min(*)` is refused
  `wrong number of arguments to function min()` and `nosuch(*)` `no such
  function: nosuch`, while `count(*)` answers.
- An aggregate inside another, one in a `WHERE`, and one in the `FILTER`
  of another are refused `misuse of aggregate function min()`.
- An aggregate in a `GROUP BY` is refused `aggregate functions are not
  allowed in the GROUP BY clause`, and one in the `ORDER BY` of a
  statement that groups nothing `misuse of aggregate: min()`.
- An aggregate in an `INSERT`, an `UPDATE` or a `DELETE`, which groups
  nothing at all, is refused `misuse of aggregate function count()`.
- A name a statement answers an aggregate under, written inside another
  aggregate, is refused `misuse of aliased aggregate m`.
- `DISTINCT` before other than one argument of an aggregate is refused
  `DISTINCT aggregates must have exactly one argument` and before those
  of a window function `DISTINCT is not supported for window functions`,
  while a scalar reads it and drops it.

### 6.6.189 A name more than one side answers (`db-sqlite`)

Document 16 step Q8.

- A bare name two tables of the `FROM` hold is refused `ambiguous column
  name: a`, and so are the three names the rowid answers to.
- A name written with an alias two sides carry is refused `ambiguous
  column name: x.a`, naming it as it was written.
- An `ORDER BY` term is read the same way, so `ORDER BY a` over two
  tables that hold `a` is refused.
- A name one `USING` or one `NATURAL` matched is answered by the side
  before it and is no refusal.

### 6.6.190 The names a statement answers its columns under (`db-sqlite`)

Document 16 step Q8.

- A connection told nothing names a column by the column and every
  other expression by the text it was written as, so `SELECT t1 . a`
  answers `a` and `SELECT 1+2` answers `1+2`.
- With `short_column_names` off and `full_column_names` off, `SELECT
  t1 . a` answers `t1 . a`, the text as it was written.
- With `full_column_names` on, a column answers `t1.a` whichever name
  the statement calls its table by, and a `*` answers `x.a` under the
  name the statement calls the side, the second only where
  `short_column_names` is off.
- A statement inside a `FROM` that carries no alias is named after its
  own place, `(subquery-7).a`.
- A statement that stands as a table has the names of its columns made
  unique: `SELECT * FROM (SELECT a, a, a FROM t1)` answers `a`, `a:1`
  and `a:2`, and a name that already ends in a colon and digits loses
  them before it takes its own.
- The two pragmas belong to the connection, which answers them back and
  carries them for a caller that holds one writer per file; a
  connection opened again was told neither.
- A `*` written where the statement reads no table is refused `no
  tables specified`.

### 6.6.191 What a compound is refused with (`db-sqlite`)

Document 16 step Q8.

- Two cores that answer different numbers of columns are refused
  `SELECTs to the left and right of UNION do not have the same number of
  result columns`, naming the word that joins them, and two `VALUES`
  `all VALUES must have the same number of terms`.
- An `ORDER BY` or a `LIMIT` written on a core other than the last is
  refused `ORDER BY clause should come after UNION ALL not before`.
- A term of the `ORDER BY` counts to the column of the first core that
  answers one of that name, so `SELECT a FROM t1 UNION ALL SELECT n FROM
  t2 ORDER BY n` sorts by the column the core on the right answers.
- A term no core answers is refused `1st ORDER BY term does not match
  any column in the result set`, naming which term it is.

### 6.6.192 What an index is refused with (`db-sqlite`)

Document 16 step Q8.

- A `CREATE INDEX` over a table SQLite keeps for itself is refused
  `table sqlite_master may not be indexed`, over a view `views may not
  be indexed`, over a table that is not there `no such table:
  main.nosuch`, and over a column the table does not hold `no such
  column: nosuch`.
- `CREATE INDEX [i1]` over a name the schema holds is refused `index i1
  already exists`, with the quotes taken off, where `CREATE TABLE [t1]`
  keeps them.
- A `DROP INDEX` of an index that is not there is refused `no such
  index: nosuch`, and of one a `UNIQUE` or a `PRIMARY KEY` made `index
  associated with UNIQUE or PRIMARY KEY constraint cannot be dropped`.
- An entry holds the value the row holds, so a row written
  `INSERT INTO t1 VALUES('1.234e5',1)` into a column of integer affinity
  is found by `WHERE a=123400` and `PRAGMA integrity_check` answers
  `ok`.
- A constraint that says `ON CONFLICT ROLLBACK` undoes the transaction
  the statement runs in and ends it, where `ABORT` leaves it open.

### 6.6.193 When a foreign key is located (`db-sqlite`)

Document 16 step Q8.

- A key that points at no key of the table it names is refused where
  the statement is read, so `UPDATE c2 SET c=1, d=2`, `DELETE FROM p2`
  and `INSERT INTO p2 SELECT 1, 2` are refused `foreign key mismatch -
  "c2" referencing "p2"` although they reach no row.
- The columns a key points at are a key of the table it names in
  whatever order they were written, so `UNIQUE(y, x)` is the key
  `REFERENCES parent(x, y)` points at.
- A `CREATE TABLE` whose key names a different number of columns from
  the ones it points at is refused `number of columns in foreign key
  does not match the number of columns in the referenced table`, and one
  whose key names a column the table does not hold `unknown column "c"
  in foreign key definition`, both where the statement is read and
  whatever `PRAGMA foreign_keys` says.
- A statement over a view and one over a table the schema does not hold
  read no key of their own.

### 6.6.194 The functions an application defines (`db-sqlite`)

Document 16 step Q8.

- A statement that writes and one that reads both reach the functions
  the application defined, so `INSERT INTO t VALUES(twice(21))` and
  `SELECT twice(a) FROM t` answer through them.
- A name the application defined for another number of arguments is
  refused `no such function: twice`, and a connection told of no
  function reads every name as one the library holds.
- `PRAGMA max_page_count` holds the file to a count of pages: a
  statement that would grow it past that count is refused `database or
  disk is full` and the file stands where the statement found it.
- The suite's own harness defines `randstr(N,M)` of
  `src/test_func.c`, which `tkt2686.test` writes rows with until the
  file fills.

### 6.6.195 The journal mode of a connection (`db-sqlite`)

Document 16 step Q8.

- The mode a connection is in is the one it was last set to, which it
  answers whether the pragma names a schema or not, so `PRAGMA
  journal_mode=persist` is answered by `PRAGMA main.journal_mode`.
- A connection with a transaction open is left in the mode it had, so
  `PRAGMA journal_mode=truncate` there answers `persist`.
- A reader is told the mode of the connection that writes, and answers
  `delete` where it is told none and `wal` for a file whose header says
  so.
- A statement an `IN` looks in answers one column for a bare value and
  as many as a row of values holds, counted where the statement is read:
  `a IN (SELECT a, b FROM t3)` is refused `sub-select returns 2 columns
  - expected 1` although the statement reaches no row. A `*` among the
  columns is as wide as the tables it is over, which the count reads no
  table for.

### 6.6.196 Whether a text ends a statement (`db-sqlite`)

Document 16 step Q8.

- A text whose last token that carries meaning is a semicolon ends a
  statement, and one that holds no token at all ends none, so
  `SELECT 1;` and `-- a comment ;\n ;` end one and `This is a test`
  does not.
- A comment, a bracket or a quote the text never closes ends no
  statement, and a `--` comment that runs to the end of the text leaves
  the state where it stood.
- A `CREATE TRIGGER` ends at the `END` of its body and not at the
  semicolons inside it, whatever words stand between `CREATE` and
  `TRIGGER`.
- `NULLS FIRST` and `NULLS LAST` are written where a statement sorts
  rows and nowhere else: a `CREATE INDEX`, a `PRIMARY KEY`, a `UNIQUE`
  and an `ON CONFLICT` target are refused `unsupported use of NULLS
  LAST`.

### 6.6.197 The collations an application defines (`db-sqlite`)

Document 16 step Q8.

- A column declared `COLLATE BACKWARDS` orders under the collation the
  connection defines, and a `COLLATE` in the statement names another,
  so `ORDER BY a` answers `aa ba ab bb` where `ORDER BY a COLLATE
  BINARY` answers `aa ab ba bb`.
- An index over such a column holds that order, so `WHERE a > 'ba'`
  answers the rows the collation puts after `ba`.
- A reader is given the collations of the connection that wrote, and
  reads no schema where it is given none: `no such collation sequence:
  BACKWARDS`.
- A name is looked up without its case, which `sqlite3FindCollSeq`
  does, so `COLLATE backwards` names `BACKWARDS`.
- A statement that names a collation the connection does not define is
  refused where the statement is read, whether the name stands in a
  column of a `CREATE TABLE` or in a `COLLATE` of a statement.

### 6.6.198 A function of any number of arguments, and a `COLLATE` on a number (`db-sqlite`)

Document 16 step Q8.

- A function the application defined for no fixed number of arguments
  answers for every count, so `joined()`, `joined('x')` and
  `joined('x','y','z')` all reach it.
- Such a function is given the name it was called under, so one
  function answers for every name the application defined.
- A `COLLATE` on a whole number of an `ORDER BY` leaves the number
  counting the answered columns and names the collation the sort uses:
  `ORDER BY 1 COLLATE BACKWARDS` orders the first column under that
  collation, and `DESC` reverses it.
- A `GROUP BY 1 COLLATE BACKWARDS` counts to the first answered column
  too.
- A number past the columns is refused with the `COLLATE` taken off:
  `1st ORDER BY term out of range - should be between 1 and 1`.

### 6.6.199 The same statements under every page size and encoding (`db-sqlite`)

Document 16 step Q8.

- A `DROP TABLE`, `DROP INDEX`, `DROP VIEW` and `DROP TRIGGER` find the
  row of `sqlite_schema` that names the object under all three
  encodings, because the schema holds its text in the encoding of the
  file and the name a statement carries never is.
- A name the schema does not hold is refused `no such table: t2` under
  all three encodings.
- The same rows are written, read, compared and ordered under page
  sizes 512, 1024, 4096 and 65536 and all three encodings, and text
  comes back in UTF-8 whatever the file holds.
- `BINARY` compares the bytes the file holds, which
  `sqlite3MemCompare` does without reading them into another encoding,
  so a character above the basic plane sorts before every other under
  UTF-16 little endian and after them under UTF-8 and UTF-16 big
  endian.
- An index a `CREATE INDEX` fills holds its text in the encoding the
  file names, so a statement that reads a row through that index finds
  it: `WHERE a = 'b'` answers the row under every encoding.
- A `DO UPDATE` writes the row it found and the values it sets in the
  encoding the file names, over a table with a rowid and over one
  without, so `SET b='new'` reads back as `new` and a column the clause
  does not set keeps its text.
- The affinity of a column is applied to the text a statement wrote and
  not to the bytes the file holds, so `'03'` into an `INTEGER` column is
  the number 3 under every encoding, and `'1x'` stays text.
- A trigger reads `new` and `old` as the statement wrote them, so a
  value it carries into another table reads back as it was written.
- `PRAGMA integrity_check` holds the entries of an index against the
  rows of the table under every encoding, so a file the engine wrote
  answers `ok`.
- A schema of more rows than the page the header is on holds grows a
  tree under that page, so two hundred `CREATE TABLE` statements over
  512-byte pages leave a schema of two hundred rows that
  `PRAGMA integrity_check` answers `ok` for.
- The bytes a constraint reads the schema out of are taken again where
  the schema cookie moves, so a table written again under the same name
  is held to the `CHECK`, the generated column and the `DEFAULT` it
  carries now and not to the ones it carried before.

### 6.6.200 The words a join is written with, and what an `ON` may name (`db-sqlite`)

Document 16 step Q8.

- A combination of words no join is written with is refused naming the
  words: `INNER OUTER`, `INNER OUTER CROSS`, `OUTER NATURAL INNER`,
  `LEFT BOGUS` and `INNER BOGUS CROSS` each answer
  `unknown join type: <the words>`.
- The combinations a join is written with stand: `NATURAL LEFT OUTER`,
  `CROSS`, `LEFT`, `FULL OUTER` and `INNER`.
- An `ON` of an outer join that names a table read after it is refused
  `ON clause references tables to its right`, whether the name carries
  the table in front of it or not.
- An `ON` that names the sides read up to it stands.
- A `CREATE TRIGGER` whose text carries a variable is refused `trigger
  cannot use variables`, whether the variable stands in the `WHEN`, the
  body, a statement inside it, a `GROUP BY`, a `LIMIT`, an `ORDER BY`
  or a window; a variable inside a text is the text.
- A write of a trigger's body that names a schema is refused
  `qualified table names are not allowed on INSERT, UPDATE, and DELETE
  statements within triggers`, and one that names the table alone
  stands.
- `likelihood(X,Y)` takes a real written as one for `Y`, between nought
  and one: `1.000001`, `-0.000001`, `0.5+0.3`, `1`, `'0.5'` and `NULL`
  are each refused `second argument to likelihood() must be a constant
  between 0.0 and 1.0`, and `1.0` and `0.0` stand.
- `#1` is refused `near "#1": syntax error`, whether it stands among
  the columns, in a `WHERE` or in an `ORDER BY`.
- A column added to a table that points at a row of another falls back
  to nothing: `ADD COLUMN g REFERENCES t1 DEFAULT 4` is refused
  `Cannot add a REFERENCES column with non-NULL default value`, and
  `DEFAULT NULL` and a column that points at no row both stand.
- A `WITH` term reads a term written after it, so
  `WITH tmp2(x) AS (SELECT * FROM tmp1), tmp1(a) AS (SELECT * FROM t1)`
  answers the two rows of `t1`.
- Terms that read each other are refused `circular reference: X` naming
  the term the statement reads, and two terms under one name are refused
  `duplicate WITH table name: X`.
- A circle the statement never reads is left unanswered, so
  `WITH i(x) AS (SELECT * FROM j), j(x) AS (SELECT * FROM i) SELECT *
  FROM t1` answers the rows of `t1`.
- A term that reads a table the database does not hold is refused for
  that table and not for a circle.

## 6.7 CI pipeline

Full jrs acceptance additionally requires all tests in `docs/test-ext/test262`
(checkout `419d3e0a2273ba01a3bfcbec423f2801425b8e93`) under that checkout's
INTERPRETING.md execution rules. The checkout is not part of this repository:
`cargo xtask test-ext` brings it to that revision and `cargo xtask test-ext
--status` reports where it stands without the network. The revision is the
table `EXTERNAL_SUITES` of the xtask policy, and that subcommand is the only
one that uses the network, so it is never a step of `cargo xtask check`. A Test262 runner must preserve harness files,
flags/variants, negative phase/type semantics, module fixtures, async completion
and `$262` capabilities; an unsupported feature is not a passing test. Existing
ad hoc realm probes do not establish suite acceptance. Full backreference tests
also expose the explicit finite-automaton-only RegExp compatibility conflict.

The Test262 CLI diagnostic runner now tests selection/discovery, root confinement,
fixture exclusion, cache isolation, strict/non-strict/raw/module flags, CR metadata,
ordered original includes, harness errors, runtime-negative constructor names,
unsupported parse negatives, first-argument ToString print, missing/duplicate async
completion and fatal unsupported host APIs. Fixtures test runner transport only;
they are not conformance tests. Reusable compiled Script APIs have separate tests
for realm isolation, compilation/instantiation phases, limit equality, poisoning
and explicit GC. `jrs_source` fuzzes compiled persistent scripts with GC installed.
The optional evalScript callback now adds global Script execution from a running
VM callback: check strictness/this isolation, lexical and var publication, early
error atomicity, collision-before-definability order, exact thrown identities,
finally order, pending outer operands/locals through GC, nested compilation,
callback-created closures, async job deferral and combined frame/binding/fuel limits.
It is not direct/indirect eval. NewTarget in global arrows is rejected even when
the caller is a constructor. Object predicates cover Symbol keys, wrapper own
properties, omitted getters, error precedence, real prototype inheritance and
native Function property mutability. Original test directories remain unmodified.
Power tests cover NaN/zero ordering, negative zero/infinity parity, overflow and
subnormals, right associativity, unary-base grammar restrictions, compound
assignment references, super setters, await, GC and left-to-right numeric errors.
`crates/math` tests exact powers of two and bounded ULP differences against host
math; independent math_pow fuzzing uses raw bit pairs plus normalized and
near-one large-exponent inputs. Production uses no libm. Comma tests ensure
GetValue effects, receiver/reference loss and separation from list delimiters.
Basic String tests cover negative/NaN/infinite positions, UTF-16 versus code points,
overlapping forward/reverse matches, empty needles, Symbol.match overrides,
getter/conversion order, padding early exit, truncation of surrogate pairs,
repeat overflow, exact ECMAScript whitespace, lone-surrogate replacement,
arguments branding, mutable method metadata and GC inside hooks. Independent
`utf16_search` fuzzing and exhaustive small binary-alphabet tests compare KMP to
a naive search oracle. Repeated-prefix split/replace/indexOf regressions enforce
linear work budgets. The Unicode case/normalization/Intl features remain separate.
Dynamic Function tests validate separate parameter/body grammars (including
cross-fragment comments and delimiter injection), strict duplicate/lexical early
errors, conversion-before-policy ordering, global environment rather than caller
locals, anonymous name without self-binding, retained exact synthetic source,
Function subclasses, newTarget and ordinary metadata. Reflect apply/construct
tests preserve target validation order, array-like getter order without iteration,
throw identity and GC roots. Restricted Function accessors share ThrowTypeError
with strict arguments and cannot resurrect deleted properties. Fuzzing invokes
the real Function constructor on arbitrary body/parameter fragments separately.
Bound construction tests exercise bound-this exclusion, nested argument order,
newTarget identity substitution, distinct Reflect newTarget prototypes, super
constructors, native/script targets, nonconstructible arrows, explicit frame limits,
metadata getter order and GC during prototype/constructor callbacks. Long bound
chains are checked without Rust recursion or repeated prefix copies. hasInstance
tests distinguish the ordinary builtin from custom handlers, object targets,
primitive instances, inherited/null/noncallable handlers, bound target delegation,
non-writable descriptors and exception/GC behavior. Fuzz seeds cover both paths.
Function source tests compare exact text (not a native fallback) for declarations,
expressions, concise/block/async arrows, object/class methods and accessors,
computed keys, class constructors, comments and line endings. Source slices end
at the last grammar token, excluding trailing trivia. Test shared Rc backing,
later realm use, dynamic fragments, async closures, name changes, native metadata,
noncallable errors and output limits. Fuzzing invokes toString on resulting
functions and dynamically constructed values without invoking the generated code.
Numeric stack fast paths are checked directly against the generic binary
implementation for every optimized operator over IEEE special-value matrices
and 10,000 generated binary64 pairs. Non-NaN results must agree bitwise (including
signed zero); NaN payloads need not. Fallback tests assert no stack mutation,
and VM tests check instruction fuel, stack capacity, user conversion ordering and
async continuation. `tools/jrs-bench.sh` records seven sequential release samples
for three fixed workloads; performance reports must retain raw samples and avoid
claiming whole-engine competitiveness from these microbenchmarks alone.
AsyncFunction intrinsic tests check constructor/prototype identities across all
async forms, lack of a global binding, descriptors, mutable metadata, non-callable
prototype, nonconstructible instances, exact dynamic source, global environments,
bound/subclass/Reflect newTarget paths, conversion and prototype-getter order,
policy denial, default-parameter rejections, body/finally rejections, job ordering,
GC and compilation/reentry quotas. Dynamic-source fuzzing invokes both the normal
and async constructors; generated code still runs only under ordinary VM limits.
Reverse/lastIndexOf tests cover four hole-presence combinations, inherited indices,
lower-getter deletion before upper existence checks, partial writes before errors,
nonwritable/nonconfigurable elements, boxing, length snapshots, zero-length early
return, omitted versus undefined fromIndex and 53-bit generic indices. A sparse
vector model checks reverse and backward search together. Enumeration tests retain
key snapshots while getters delete or change later descriptors, skip Symbol keys,
and retain values across GC. Object constructor storage tests assignment/deletion,
integrity, prototype changes, Symbol properties and persistent-realm roots. Number
constant tests distinguish smallest subnormal from smallest normal binary64.
Fill/copyWithin tests cover relative/clamped 53-bit indices, fractional and
infinite arguments, boxed primitive receivers, length snapshots, empty-range
conversion order, overlapping copy direction, inherited getters/setters, source
holes deleting targets, immutable descriptors and partial mutation on failure.
Value/receiver roots survive GC inside callbacks; neither method touches species
or writes length directly. A sparse vector snapshot model validates both copy
directions and fill with per-statement assertions. Infinite scans are tested only
in jrs with fuel limits, never in unbounded reference-engine comparisons.
Unscopables tests verify the complete draft-required record, descriptor attributes,
null prototype, identity across realm turns/GC and non-recreation after deletion.

The independent regex-bt crate is explicitly allowed to backtrack and must not
be tested as if it had the NFA's linear bound. Regular-subset matches and capture
priority are compared against regex for all offsets/sticky/options combinations.
Dedicated cases check unmatched/forward backreferences, reverse-order lookbehind
captures, atomic lookaround, empty mandatory versus optional iterations and capture
clearing. Exact work limits, choice/assertion stack limits and register workspace
limits must return typed errors, not non-matches. `regex_bt` fuzzing checks arbitrary
UTF-16, bounds, capture ranges and differential results without external software.
Both engines are included in regex-check and the full project pipeline.
JSON native functions exercise extensibility, prototype/name/length mutation,
freeze, WeakMap identity and unchanged native behavior after property mutation.

Jobs run in this order; a failure stops the pipeline.

1. `lint` (fmt, clippy, SPDX headers)
2. `check-layering`, `check-deps`, `unsafe-budget`
3. `test --host` with coverage thresholds
4. `miri`
5. `doc`
6. `test --qemu`
7. `test --e2e`
8. `fuzz` regression corpus (short run)

CI runs on Linux runners with QEMU and its UEFI firmware from the
distribution package. A macOS runner job covers the build only. Docker is
not used; every step runs directly on the runner.

### 6.6.201 The key an `INSERT` takes at the largest key (`db-sqlite`)

Document 16 step Q8.

- A table whose largest key is 9223372036854775807 gives the next row a
  key drawn at random that is above nought and below that largest key.
- A hundred draws that all name a key a row holds refuse the write
  `database or disk is full`, and the table then holds the hundred rows
  the draws before them wrote.
- A key that counts up draws nothing, so a table whose
  `AUTOINCREMENT` counter reached the largest key an integer holds
  refuses the write.
- The state a writer answers draws the same words again, so two
  statements over one restored state write the same `randomblob`.

### 6.6.202 A varint written and read back (`db-sqlite`)

Document 16 step Q8.

- `bytes::varint_again` answers the value a varint carried, how many
  bytes the write took and how many the read took, and the three agree
  for 0, 127, 128, 16383, 16384 and the largest number a `u64` holds.

### 6.6.203 What a checkpoint answers (`db-sqlite`)

Document 16 step Q8.

- A file that is not logging answers `0 -1 -1`, because it holds no
  frame.
- A checkpoint over a log answers nought, then twice how many frames the
  log holds, and the file it wrote holds the rows the log held while the
  file before it named no table.
- The commit after a checkpoint begins the log again, so the checkpoint
  after that commit counts the frames of that commit alone.
- A checkpoint that follows one with no commit between them counts the
  same frames again.
- `RESTART` counts the frames it moved and leaves a log of no frame, so
  the checkpoint after it answers `0 0 0`.
- `TRUNCATE` answers `0 0 0` whatever the log held, and the rows stand in
  the file it wrote.

### 6.6.204 What a foreign key action is held to (`db-sqlite`)

Document 16 step Q8.

- A row an `ON UPDATE CASCADE` writes is held to the `CHECK` of the
  table it writes, so `UPDATE ab SET a=5` over a chain that reaches a
  `CHECK (e!=5)` is refused and the table keeps the key it had.
- A value the `CHECK` holds carries the chain to its end, so both tables
  after the one the statement named hold the new key.
- A row the chain wrote is a row other rows point at, so a `DELETE` that
  reaches a key naming no action is refused.
- A chain of 40 tables each pointing at the one before it is refused
  rather than reaching the end of the stack.

### 6.6.205 The moment `now` names (`db-sqlite`)

Document 16 step Q8.

- `datetime()` over a connection told the clock answers the moment the
  clock says, and so does `date('now')`, whatever case `now` is written
  in.
- A modifier after `now` moves that moment, so `datetime('NOW','+1 day')`
  answers the day after.
- `strftime` and `unixepoch` read the same clock where they name no
  moment.
- `CURRENT_TIME`, `CURRENT_DATE` and `CURRENT_TIMESTAMP` are `time`,
  `date` and `datetime` of the clock.
- A connection told no clock refuses the three literals and answers
  nothing for `now`.

### 6.6.206 What a `VACUUM` leaves (`db-sqlite`)

Document 16 step Q8.

- The rows, the indexes, the views and the triggers of a database stand
  after a `VACUUM`, and the file holds no free page and answers
  `PRAGMA integrity_check` with `ok`.
- A row the vacuum wrote fires no trigger, so a log table holds only the
  rows the statements before the vacuum wrote.
- A `PRAGMA page_size` written after the first table changes nothing
  until the vacuum runs, and the file the vacuum wrote holds that size.
- A table without a rowid keeps its rows under their keys, and a key that
  counts up keeps the counter it reached, so the next key is past the one
  a delete took away.
- A `VACUUM` inside a transaction, one that names a schema the connection
  does not hold, and `VACUUM INTO` are each refused.
- A pragma no version of the library holds answers no row when read and
  when set, and a name this crate holds nothing for at all is refused.

### 6.6.207 An aggregate the application defined (`db-sqlite`)

Document 16 step Q8.

- An aggregate the application defined reads the arguments of every row
  of its group, in the order the rows were read.
- A `GROUP BY` answers one row per group, each holding the rows of that
  group alone.
- A group of no row answers the aggregate over no row at all.
- `DISTINCT` puts the rows through one column before they are stepped.
- An `OVER` reads the aggregate over the frame, so each row answers the
  rows up to it.
- A name the connection was not told of is no function at all.

### 6.6.208 What `PRAGMA case_sensitive_like` sets (`db-sqlite`)

Document 16 step Q8.

- `LIKE` folds the twenty-six letters where the connection was told
  nothing.
- `PRAGMA case_sensitive_like=on` makes `LIKE` and the `like` function
  tell the letters apart.
- The pragma answers no row, set or read.
- A word that names no truth value turns the pragma off.
- `GLOB` tells the letters apart whatever the pragma says.
- A `DELETE` and an `UPDATE` read the pragma as a `SELECT` does.
- A database the caller opens without the pragma folds the letters.

### 6.6.209 What the header word holds of the cache size (`db-sqlite`)

Document 16 step Q8.

- `PRAGMA default_cache_size=-123` writes 123 into the word at offset 48
  and answers 123 for both cache pragmas.
- A reopen reads the word, and a `VACUUM` leaves it.
- A word of nought answers −2000 for `PRAGMA default_cache_size` and
  nought for `PRAGMA cache_size`.
- Text that names no number, and a number no signed word of 32 bits
  holds, write nought.
- `PRAGMA cache_size=-4321` stands over the word for that connection and
  writes no byte of the file.
- A word below nought answers its negation.

### 6.6.210 What a pragma writes of the header words (`db-sqlite`)

Document 16 step Q8.

- `PRAGMA schema_version`, `PRAGMA user_version` and `PRAGMA
  application_id` write their word, and a reopen reads it.
- Text that names no number writes nought, and a number below nought is
  kept as the bytes of a word of 32 bits.
- `PRAGMA freelist_count`, `PRAGMA page_count` and `PRAGMA data_version`
  read the file whatever stands after the equals sign.
- `PRAGMA encoding=bogus` is refused over a file that already holds a
  table.
- `PRAGMA synchronous=OFF` inside a transaction is refused and leaves the
  level as it was.

### 6.6.211 What the authorizer of a connection is asked (`db-sqlite`)

Document 16 step Q8.

- Every action carries the word `tclsqlite.c` writes it as, and no two
  actions carry one word.
- A connection told no function is asked nothing.
- `SQLITE_SELECT` denied refuses the statement; ignored answers no row.
- `SQLITE_READ` denied names the column; ignored answers a null for it
  under a `*`, under a `t.*`, under an alias, under a schema and in the
  `WHERE`.
- A bare `rowid` is asked under the column the key is another name for,
  and under `ROWID` where the table has none.
- A table no column of is read is asked for under the empty name.
- Every clause of a statement is read, and a statement written inside one
  is read as a statement of its own.
- `SQLITE_FUNCTION` denied names the function.
- Each `CREATE` and each `DROP` is asked for twice: for the write of the
  schema's own table and for what it makes or takes away.
- The four `ALTER TABLE` forms carry the schema, the table and, where one
  is named, the column.
- An `UPDATE` is asked once per column written, and an ignored column
  keeps the value it had.
- A `PRAGMA`, a `BEGIN`, a `SAVEPOINT`, an `ANALYZE` and a `REINDEX` are
  each asked for and each left undone where the function ignores it.
- A term of a `WITH` that reads its own name is asked for.
- A `VACUUM` is asked for nothing.

### 6.6.212 One statement written again with every literal as a `?` (`db-sqlite`)

Document 16 step Q8.

- Every literal, every variable and every blob is a `?`.
- A comment and a run of whitespace are dropped, and a space stands
  between two words that would otherwise run together.
- The right side of an `IN` becomes `?,?,?`, and one a statement opens
  stands as it was.
- A `NULL` after `IS` or `NOT` is a word of the language; every other one
  is a value.
- A name in double quotes is written bare where it is one identifier and
  quoted where it is not.
- A word in double quotes the schema carries no name for is a text; one
  in an `ATTACH` is a name.
- A statement that ends in no semicolon carries one.
- The lower-case form answers nothing for a byte no rule accepts.

### 6.6.213 The result code a refusal carries (`db-sqlite`)

Document 16 step Q8.

- A file whose header the format forbids carries `SQLITE_NOTADB` and the
  message `file is not a database`.
- A byte under the header that the format forbids carries
  `SQLITE_CORRUPT` and the message `database disk image is malformed`.
- A file held to a count of pages carries `SQLITE_FULL`.
- A row that broke a constraint carries `SQLITE_CONSTRAINT` and the
  extended code of the constraint it broke.
- A statement the authorizer denied carries `SQLITE_AUTH`.
- Every statement the engine could not read carries `SQLITE_ERROR`.

### 6.6.214 The type the schema declares for a column answered (`db-sqlite`)

Document 16 step Q8.

- A column of a table carries the type the schema declares, under its own
  name, under the name of its table and through a `*`.
- A column a statement written inside the `FROM` answers carries the type
  of the column it came from.
- A bare `rowid` of a table that keeps its rows under a key carries
  `INTEGER`.
- A column that came from an expression carries nothing.
- Every statement carries one entry per column it answers.

### 6.6.215 What `REGEXP` and `MATCH` reach (`db-sqlite`)

Document 16 step Q8.

- A connection told neither name is refused both.
- `x REGEXP y` calls `regexp(y, x)`, so the pattern is the first
  argument, and `x MATCH y` calls `match(y, x)`.
- `NOT REGEXP` answers the rows the function did not hold for.
- A row of nothing answers nothing either way.
- A statement that writes reaches the same function.

### 6.6.216 The statement after one the connection holds (`xtask`)

Document 17 step T5.

- The first name answers where the one given is `0` or nothing.
- The name after each one answers, and nothing after the last.
- A name the connection does not hold has nothing after it.
- A connection that holds no statement answers nothing for the first.

### 6.6.217 The parent a foreign key names under a rename (`db-sqlite`)

Document 16 step Q8.

- A column's own `REFERENCES` and a `FOREIGN KEY` clause of the table
  both carry the new name.
- A table that names itself is written again with its own new name.
- A key that names another table is left as it is.
- A table whose columns come out of a `SELECT` carries no key.

### 6.6.218 What databases one connection holds (`db-sqlite`)

Document 18 step A2.

- A bare name, a name in quotes and a name an expression answers are all
  the name the database answers to.
- `:memory:`, a file name of no bytes and a file of no bytes are each a
  database of one page.
- A name the connection already holds a database under is refused, which
  `main` and `temp` are two of.
- A file the opening function answers nothing for, a file whose bytes no
  header reads, and a file whose encoding is not the one of `main` are
  each refused.
- The eleventh database of a connection is refused.
- `DETACH main` is refused, and `DETACH temp` is no database at all.
- A connection told no opening function attaches a database of its own
  and no file.
- `SQLITE_ATTACH` and `SQLITE_DETACH` carry the text the statement wrote.

### 6.6.219 What a statement reads out of an attached database (`db-sqlite`)

Document 18 step A3.

- A bare name both databases hold is answered out of `main`.
- A bare name only the attached database holds is answered out of it,
  and so are its index, its view and its own schema table.
- `aux.t` is answered out of that database alone, and `main.u` where only
  `aux` holds a `u` names no table.
- A schema the connection holds no database under names no table.
- `aux.t.a` reaches the side that reads `aux.t`, and a schema the side
  does not read names no column.
- A join over two databases answers the rows of both.

### 6.6.220 What a statement writes into an attached database (`db-sqlite`)

Document 18 step A4.

- `CREATE`, `INSERT`, `UPDATE`, `DELETE` and `DROP` under a schema write
  that database and leave the one the connection was opened over as it
  was.
- `INSERT INTO aux.t SELECT ... FROM main.u` reads both databases.
- A bare name only the attached database holds names that database.
- A `CREATE` of a bare name makes the table in the database the
  connection writes, whatever the attached one holds.
- A schema the connection holds no database under is refused `no such
  table:` where the statement names a table and `unknown database` where
  it names the database alone.
- `attached_files` answers the file name of every attached database that
  names one.

### 6.6.221 The files the harness answers an `ATTACH` with (`xtask`)

Document 18 step A7.

- A path the session holds a writer under answers that writer's bytes.
- A path it holds none under answers no bytes, which is a database of one
  page.
- A text that holds no `attach` tells the files nothing, because writing
  them out costs O(n) in the pages of all of them.

### 6.6.222 What the temp schema holds (`db-sqlite`)

Document 18 step A6.

- A statement written `TEMP` writes schema place one, which no file of the
  client holds.
- A bare name both the temp schema and `main` hold is answered out of the
  temp schema, and a `DROP` of it takes the temp one away.
- `sqlite_temp_master` and `sqlite_temp_schema` read the temp schema, and
  `temp.sqlite_master` reads it as well.
- A name that holds the letters `temp` and is no word of its own opens no
  temp schema.
- The temp schema holds a place no `ATTACH` counts against the ten, and no
  `DETACH` takes it away.
- A `DETACH` leaves the databases after the one it took away one place
  lower.

### 6.6.223 What an added column that points may fall back to (`db-sqlite`)

Document 16 step Q8.

- A connection that holds its rows to no foreign key takes the column.
- A table that holds no row takes the column.
- A column that points and falls back to nothing is taken.
- A column that points at no table is taken whatever it falls back to.

### 6.6.224 One transaction over more than one database (`db-sqlite`)

Document 18 step A5.

- A `ROLLBACK` puts every database back where the `BEGIN` found it.
- A `COMMIT` writes every database the transaction wrote.
- A `ROLLBACK TO` puts every database back where the `SAVEPOINT` found it.
- A database an `ATTACH` added inside a transaction joins it.
- A `COMMIT` and a `ROLLBACK` outside a transaction carry a word each.

### 6.6.225 What the pragmas of the schema answer (`db-sqlite`)

Document 16 step Q8.

- `PRAGMA table_info` answers the place, the name, the declared type,
  whether the column refuses nothing, what it falls back to, and where it
  stands in the primary key.
- A computed column is out of `table_info` and in `table_xinfo`, with two
  for `VIRTUAL` and three for `STORED`.
- `PRAGMA index_info` answers -2 for a place over an expression.
- `PRAGMA index_xinfo` answers the rowid of a table that holds one and the
  primary key of a table that keeps its rows in the key's own tree.
- `PRAGMA index_list` answers the index made last first, with `c`, `pk` and
  `u` for where each came from.
- `PRAGMA collation_list` answers the three of the library and then the
  ones the application defined.
- A table or an index the schema does not hold answers no row.

### 6.6.226 What schema the authorizer is asked under (`db-sqlite`)

Document 16 step Q8.

- A `DROP` of a temporary table asks the action of the temp schema.
- A `CREATE TRIGGER` over a temporary table asks the action of the temp
  schema and `SQLITE_INSERT` of `sqlite_temp_master` after it.
- An index over a table of the temp schema asks the action of that schema,
  and one over a table no database holds names no table.
- A read the function denies of a column of another database carries the
  schema in front of the table.

### 6.6.227 What the schema a statement names writes (`db-sqlite`)

Document 16 step Q8.

- A statement written `temp.name` writes the temp schema, and one written
  `main.name` the database the connection writes.
- A statement over a table the named database does not hold is refused
  `no such table: schema.table`.
- A `DROP` of an index, a view or a trigger under a schema is not held to
  that schema.
- A `CREATE TEMPORARY TABLE` writes the temp schema, as one written
  `TEMP` does.

### 6.6.228 What a row that is written over is held to (`db-sqlite`)

Document 16 step Q8.

- The row a `REPLACE` writes over is refused where a row points at it
  under a key that names no action.
- The row a `REPLACE` writes over in a table that keeps its rows in the
  key's own tree is held to the same keys.
- The key a statement wrote is held against the rows the table holds
  where `rowid` names it and where a column of an `INTEGER PRIMARY KEY`
  does.
- The message of the refusal names `table.rowid` where the table has no
  column the key is another name for.
- A row that carries no key takes another where the body of a trigger
  before the row wrote the key it stood to take, and the counters of the
  connection name the key the row took.
- The keys held at the end of the transaction are counted back to what
  they were where a statement began wherever that statement leaves the
  file as it found it.

### 6.6.229 What columns a pragma answers (`db-sqlite`)

Document 16 step Q8.

- A pragma that answers columns of its own names each of them.
- A pragma that answers one value names the column after itself.
- A pragma written `TYPE: FLAG` names no column where a value follows
  the name.
- `PRAGMA case_sensitive_like` and `PRAGMA shrink_memory` name no column
  either way.
- A reader answers `PRAGMA table_info`, `table_xinfo`, `index_info`,
  `index_xinfo`, `index_list` and `collation_list` out of the database it
  was opened over.
- A pragma that wrote a schema in front of its name answers out of that
  database.
- `PRAGMA journal_mode = X` with no schema in front of it sets the mode
  of every database the connection holds, and one that names a schema
  sets that database alone.

### 6.6.230 What index a statement names (`db-sqlite`)

Document 16 step Q8.

- A `SELECT`, an `UPDATE` and a `DELETE` each take `INDEXED BY name` and
  `NOT INDEXED` after the name of the table.
- An index of another table is refused `no such index`.
- An `UPDATE` or a `DELETE` of a trigger's body that names an index is
  refused under the clause it wrote.
- `BEGIN`, `COMMIT` and `ROLLBACK` take a name after the word
  `TRANSACTION`.


### 6.6.231 The three hooks a connection is told (`db-sqlite`)

Document 16 step Q8.

- A connection told no function runs the same statements and is asked
  nothing.
- A statement of its own that writes a page asks the commit hook once,
  and a transaction asks it at its `COMMIT`.
- A commit hook that answers true refuses the statement `constraint
  failed` with the extended code 531, sends the transaction back and
  tells the rollback hook.
- A `ROLLBACK` tells the rollback hook whether or not the transaction
  wrote, and a statement of its own whose row broke a constraint tells
  it as well.
- The update hook is told the word of the action, the database, the table
  and the key of every row a statement wrote, including the rows a
  trigger's body wrote and the rows of an attached database.
- A row of a `WITHOUT ROWID` table and a row of `sqlite_sequence` tell
  the update hook nothing.

### 6.6.232 The preupdate hook a connection is told (`db-sqlite`)

Document 16 step Q8.

- An `INSERT`, an `UPDATE` and a `DELETE` each tell the action, the
  database, the table, the two keys, the depth and the rows.
- The column the key is another name for carries the key in both rows.
- The row a `REPLACE` writes over is told as a row taken away, which the
  update hook is not told of.
- A row of a table that keeps its rows in the key's own tree carries
  nought for both keys.
- A row a trigger's body writes carries how many triggers deep the
  statement stands.
- A row the triggers before it took away is told once, and the statement
  writes nothing over it.

### 6.6.233 The names a statement may not take away (`db-sqlite`)

Document 16 step Q8.

- A `DROP TABLE` and a `DROP VIEW` over a name that begins `sqlite_` are
  refused, and the refusal comes before the schema is read.
- `DROP TABLE sqlite_stat1` and `DROP TABLE sqlite_parameters` stand.
- An `INSERT` over a table that counts up writes the row of
  `sqlite_sequence` whether or not it wrote a row.
- The row of `sqlite_sequence` never counts back down.
- `AUTOINCREMENT` off an `INTEGER PRIMARY KEY` and over a `WITHOUT ROWID`
  table each carry the words `sqlite3AddPrimaryKey` writes.

### 6.6.234 The words a refusal carries (`db-sqlite`)

Document 16 step Q8.

- A duplicate column name, more than one primary key, a `WITHOUT ROWID`
  table with no primary key, a key naming a column the table does not
  hold and a key written as an expression each carry the words
  `sqlite3AddColumn`, `sqlite3AddPrimaryKey` and `sqlite3CreateIndex`
  write.
- A `NATURAL` join with a condition on it and a `USING` naming a column
  one side does not hold carry different words.
- A `WITH` term that writes another number of column names than its
  statement answers names the term and both counts.
- A hex literal too big to read names the literal as it was written, an
  `ESCAPE` of more than one character and an integer that overflows each
  carry their own words.

### 6.6.235 The columns an `ALTER TABLE` may not add (`db-sqlite`)

Document 16 step Q8.

- A `PRIMARY KEY` column and a `UNIQUE` column are refused whether or not
  the table holds a row.
- A `NOT NULL` column with no default other than nothing, and a column
  whose default is no value of its own, stand over a table that holds no
  row and are refused over one that does.
- A generated column reaches neither refusal.
- A view and a name the table already holds are refused before the column
  is read.
- `DEFAULT -'x'` stands; `DEFAULT (1+2)`, `DEFAULT (~3)` and
  `DEFAULT (-(1+2))` do not.
- A column that points at a row of another table is refused only where the
  connection holds the keys and the table holds a row.

### 6.6.236 What `PRAGMA integrity_check` is given (`db-sqlite`)

Document 16 step Q8.

- The count of an index comes before the rows missing from it.
- A number after the equals sign is how many problems the pragma answers,
  and nought stands for the hundred it answers by default.
- A word that is no number is the name of the one table the walk is over.
- A number in quotes is a name and not a count.
- A name no table, index or schema table carries is refused.
- A connection that writes reads the same argument as one that reads.

### 6.6.237 Every database the integrity check walks (`db-sqlite`)

Document 16 step Q8.

- The problems of the pages of one database are one row under the name of
  that database, and each line of it counts as one problem.
- A connection that holds several databases walks each in turn and spends
  one count of problems across all of them.
- A name after the equals sign is walked in the first database that holds
  a table of it, and no other database is read.
- A name no database holds a table of is refused, the name of an index
  among them.

### 6.6.238 What `VACUUM ... INTO` writes (`db-sqlite`)

Document 16 step Q8.

- The file holds the rows the database holds and joins the files the
  client writes back.
- The expression after `INTO` is answered against the database, so a
  column no table carries is refused `no such column`.
- A value that is no text is refused `non-text filename`.
- A name the client holds bytes for is refused `output file already
  exists`, and `:memory:` is written whatever the client holds.

### 6.6.239 The pragma that names the columns of no row (`db-sqlite`)

Document 16 step Q8.

- `PRAGMA empty_result_callbacks` answers the truth value the connection
  holds, and nought where no statement set it.
- Setting it answers no row.

### 6.6.240 The columns a view answers (`db-sqlite`)

Document 16 step Q8.

- `PRAGMA table_info` over a view answers the columns its statement
  answers, a column of a table carrying the declared type and a column of
  an expression carrying none.
- The names the column list wrote name the columns.
- A view whose statement does not run answers no row, and a quote in the
  name of a view is written twice where the pragma reads the view again.
- A column list of another width than the statement answers is refused
  where the view is read.
- A word after a name of the column list is refused naming that name.
- A `DROP TABLE` over a view and a `DROP VIEW` over a table each name the
  kind the schema holds.

### 6.6.241 Where a `RAISE` stands and what it says (`db-sqlite`)

Document 16 step Q8.

- A `RAISE` in the body of a trigger refuses the statement that fired the
  trigger with the message it carries.
- The refusal holds the action and the message as text, and carries
  `SQLITE_CONSTRAINT_TRIGGER`.
- `RAISE(IGNORE)` passes the row over and leaves the statement running.
- A message the engine cannot answer takes the place of the refusal.
- A `RAISE` outside the body of a trigger is refused where the statement
  is read.

### 6.6.242 The pages an incremental vacuum gives up (`db-sqlite`)

Document 16 step Q8.

- `PRAGMA incremental_vacuum` gives up the pages at the end of the file
  one at a time and answers one row of no column per page.
- How many pages the statement names is the number after it, a quoted
  number and a signed number among them.
- A number past what a signed word holds, a word that is no number, and
  a number at or below nought all name every page of the free list.
- A file that holds no free page and one that does not vacuum itself
  each give up none.
- A file that vacuums itself holds every page after a row with overflow
  pages is deleted, the chain of a long index key among them.
- `PRAGMA auto_vacuum` over a file that holds a table writes which of
  the two ways the file vacuums itself and turns the vacuuming neither
  on nor off.
- A word no way carries names none and changes nothing.
- A step moves the page at the end of the file into a free page below it,
  wherever the free list holds that page, and a file that ends on a
  pointer-map page gives that page up and moves nothing.
- A step refuses a file whose map names the last page the root of a tree
  and one whose free list holds no page at or below the end.
- `PRAGMA integrity_check` over a file with more than one pointer-map
  page names no page never used.

### 6.6.243 Where a script of the suite ends a statement (`db-sqlite`)

Document 17 step T5.

- The tester splits a script at the semicolons `sqlite3_complete` ends a
  statement at, so a body of a `CREATE TRIGGER` that holds a `CASE ...
  END` is one statement.
- `verify_ex_errcode` holds the extended code of the last refusal
  against the name of a code.
- `strftime` answers what the C library of the machine writes, `%F`
  among its fields.

### 6.6.244 Which database a statement writes (`db-sqlite`)

Document 16 step Q8.

- A statement that names `main` writes the rows and the indexes of that
  database where the temp schema holds a table of the same name.
- A statement that names no schema writes the table the temp schema
  holds.
- A trigger of the temp schema and one of `main` carry the same name,
  and a second trigger of that name in one database is refused.

### 6.6.245 The bytes of one value, read where they lie (`db-sqlite`)

Document 16 step Q8.

- A blob handle answers how many bytes the value holds, reads them from
  an offset, and writes over them.
- A value that runs onto a chain of overflow pages is read and written
  page by page, and the length it carries stands.
- A read or a write that reaches past the value is refused.
- A handle over a view, over a table written `WITHOUT ROWID`, over a
  column the table does not hold, over a key no row carries and over a
  value that is neither text nor bytes is each refused with the words
  the C library writes.
- A handle that writes a column an index or a foreign key holds is
  refused, and one that reads it is not.
- A handle names the database it reads, which the temp schema is one of.
- A row written before a column was added holds no value for it.

### 6.6.246 The bytes a page keeps back (`db-sqlite`)

Document 16 step Q8.

- `PRAGMA reserved_bytes = N` stands before the first table and is what
  the file is written under.
- A count no byte holds is refused, and one that leaves a page too
  little room is refused.
- `PRAGMA compile_options` answers no row.

### 6.6.247 What a command of a blob handle answers (`db-sqlite`)

Document 16 step Q8.

- `PRAGMA lock_status` answers one row per database of the connection,
  with `closed` for a temp schema no statement has opened.
- A handle names the database and the table together where the
  connection holds neither.

### 6.6.248 The bits of a binary64 number (`db-sqlite`)

Document 16 step Q8.

- `ieee754(X)` answers the mantissa and the exponent of two the number
  is, and the two-argument form answers the number they name.
- A nought, a nought that carries the sign, and a pair that names no
  number each answer what the C library answers.
- `ieee754_to_blob` and `ieee754_from_blob` write the eight bytes of a
  number and read them back, and every other value answers nothing.
- `sqlite_compileoption_used` answers nought and
  `sqlite_compileoption_get` answers nothing.

### 6.6.249 The regular expression matcher (`db-sqlite`)

Document 16 step Q8.

- Every operator of the grammar: `*`, `+`, `?`, `{m,n}`, `(X)`, `|`,
  `^`, `$`, `.`, a character class, and the classes a `\` names.
- A pattern that names a character by its value, as `\uXXXX` and
  `\xXX`, and the six characters written no other way.
- Bytes that are not UTF-8, which read as the replacement character.
- A step that reads past the last character of the text.
- The bytes every match begins with, which a match skips ahead to.
- A pattern the matcher refuses, once for each message.
- A pattern whose program asks for more steps than one holds.

### 6.6.250 The plan of an `OR` over several indexes (`db-sqlite`)

Document 16 step Q8.

- An `OR` whose every branch names a key answers the branches one after
  another, in the order the `WHERE` writes them.
- A row two branches name is answered once.
- An `OR` under an `AND` spine whose other terms name no key.
- A branch that names no key leaves the table scanned.
- A branch whose index the descent cannot read leaves the table scanned;
  one whose index the walk cannot read refuses.
- A term that carries a `COLLATE` names no key.

### 6.6.251 The steps and the sorts of a statement (`db-sqlite`)

Document 16 step Q8.

- A walk of a whole table counts one step per row after the first, and
  an empty table counts none.
- A walk held to a key, by an index, by a rowid range, by an `ON` or by
  the branches of an `OR`, counts no step.
- An `ORDER BY` the walk of the table's own tree answers needs no sort.
- An `ORDER BY` the walk of an index answers needs no sort, with a last
  term naming the rowid among them.
- A term written backwards, with its nulls moved, over an expression or
  under a collation of its own leaves the rows sorted.
- A join, a group, a window, a statement inside a `FROM` and a table
  that keeps its rows in the key's own tree each leave the rows sorted.
- An index over an expression, a partial index and an index under
  another collation answer no order.
- The `ORDER BY` of a compound statement counts one sort.

### 6.6.252 The bounds a walk of an index is held between (`db-sqlite`)

Document 16 step Q8.

- A term with `>=`, `>`, `<=` or `<` over the first column of an index
  holds the walk to a range of it, and two of them to both ends.
- A term with `=` over the first column and one with `>` over the second
  hold the key and the bounds together.
- An index written `DESC`, over an expression, under another collation,
  or over fewer rows than the table has reaches no key and no bound.
- A walk held to one value of the first column answers its entries in the
  order of the second, and one held to a range of the first in the order
  of the first and then the second.
- An entry that runs onto an overflow page is read whole before the
  bound is compared against it.
- A bound the affinity of the column changes is widened to hold the
  value itself.
- A column of real affinity reaches no key and no bound.

### 6.6.253 The bounds a pattern names (`db-sqlite`)

Document 16 step Q8.

- A `GLOB` over a column that compares under `BINARY` and a `LIKE` over
  one that compares under `NOCASE` each hold the column between two
  bounds.
- A `GLOB` over a column that compares under `NOCASE`, and a `LIKE` over
  one that compares under `BINARY`, hold it between none.
- A pattern that begins with a wildcard, one that begins with a
  character past the first 128, one written after `NOT`, one with an
  `ESCAPE`, and one that is no text of its own each name no bound.
- A prefix that reads as a number, a lone minus, and a prefix the value
  one past which reads as a number each name no bound where the column
  is of no text affinity.
- A branch of an `OR` that names the rowid, a range of rowids, a rowid
  compared against text, and a rowid of another side.
- A `BETWEEN` over the rowid and one over a column, and a `BETWEEN`
  written after `NOT`, over a value that is no column, or whose end is a
  column.

### 6.6.254 The file a transaction found (`db-sqlite`)

Document 16 step Q8.

- A transaction that writes a page the file already held, one that
  writes a page the file did not hold, and a page the transaction leaves
  alone.
- A connection in write-ahead logging answers the file the pragma left
  either way.

### 6.6.255 The entries of an index written `DESC` (`db-sqlite`)

Document 16 step Q8.

- An index written `DESC` beside one written `ASC` over the same column,
  read from the tree as the writer left it.
- A walk of an index held backwards between one bound, between two, and
  between none.
- An `ORDER BY` a place held backwards answers, and one it does not.

### 6.6.256 The walk of an index read backwards (`db-sqlite`)

Document 16 step Q8.

- An index read from its last entry to its first, over a tree of one
  leaf and over one that carries interior pages.
- An `ORDER BY` whose terms run in two directions, against an index
  whose places run in two directions.
- A term over the rowid at the end of the terms, running each way, and
  one the terms before it do not reach the last column for.

### 6.6.257 The walk between bounds read backwards (`db-sqlite`)

Document 16 step Q8.

- A walk held between one bound, between two, and held to a key, read
  from its last entry.
- An index held forwards and one held backwards, each read either way.
- A tree of one leaf and one that carries interior pages.

### 6.6.258 The collation a term of an `ORDER BY` compares under (`db-sqlite`)

Document 16 step Q8.

- A `COLLATE` naming the collation the index place holds, and one naming
  another.
- A term carrying no `COLLATE` over a column whose own collation the
  index place holds.
- A `COLLATE` over the rowid, and one naming a collation the connection
  does not define.

### 6.6.259 The plan an `EXPLAIN QUERY PLAN` names (`db-sqlite`)

Document 16 step Q8.

- A walk of a whole table, of a whole index, held to a key, held between
  bounds, and held to a range of rowids.
- A side under an alias, a side that reads what another statement
  answers, and a side read by one walk per branch of an `OR`.
- A sort the walk does not answer.

### 6.6.260 The column an `ORDER BY` term counts to (`db-sqlite`)

Document 16 step Q8.

- A whole number counting to one answered column and to several.
- A name one column is answered under, over a column of another name.
- A number written where the statement carries a `*`.

### 6.6.261 The row read out of a covering index (`db-sqlite`)

Document 16 step Q8.

- An index holding every column read, one leaving a column out, and one
  read together with the rowid.
- A walk held to a key and a walk of the whole index, each answering the
  rows the table holds.
- `EXPLAIN QUERY PLAN` saying `USING COVERING INDEX` for such a walk.

### 6.6.262 The searches a statement counts (`db-sqlite`)

Document 16 step Q8.

- A walk of a whole table, one held from a rowid up, and one held to
  one rowid.
- A walk of an index, with and without the row of the table each entry
  names.
- A sort, which counts one search back.

### 6.6.263 The walk a range of rowids is given up for (`db-sqlite`)

Document 16 step Q8.

- A range naming one rowid, a range with one end, and a range no index
  key stands beside.
- An `IS` over a value that is not null, and one over null.

### 6.6.264 The index a plan is built from (`db-sqlite`)

Document 16 step Q8.

- Two indexes the terms name one and two columns of.
- Two indexes the terms name as many columns of, made in either order.

### 6.6.265 The database a trigger's body writes (`db-sqlite`)

Document 16 step Q8.

- A trigger of the temp schema whose body names a table of `main` and
  one of the temp schema.
- A trigger of `main` whose body names a table two databases hold.
- An `INSERT`, an `UPDATE`, a `DELETE` and a `SELECT` in one body.

### 6.6.266 The pages a connection reads back (`db-sqlite`)

Document 16 step Q8.

- A connection in write-ahead logging reading a row its own commit put
  in the log.
- The same connection reading inside a transaction that took the row
  out, and after the rollback.
- The file the pragma left, which holds neither the table nor the row.

### 6.6.267 The trees a statement sorts its rows in (`db-sqlite`)

Document 16 step Q8.

- A statement that writes a `GROUP BY`, a `DISTINCT` and an `ORDER BY`
  together, and each of the three on its own.

### 6.6.268 The window function a `CREATE INDEX` may not call (`db-sqlite`)

Document 16 step Q8.

- A window function in a term of the index, under a call in a term, and
  in the `WHERE` of a partial index.
- A term that calls no window function, which the index holds.

### 6.6.269 The `ORDER BY` the groups answer (`db-sqlite`)

Document 16 step Q8.

- An `ORDER BY` naming one `GROUP BY` term and naming two.
- A term written backwards, one with its nulls moved, one over an
  expression, and a count of terms the `GROUP BY` does not name.
- A statement that groups nothing.

### 6.6.270 The groups a walk gathers (`db-sqlite`)

Document 16 step Q8.

- A `GROUP BY` one index answers the order of, and one no index does.
- The groups a walk gathers against the groups a sorter gathers, over a
  column that holds a null.

### 6.6.271 The key of a side the sides before it answer (`db-sqlite`)

Document 16 step Q8.

- A key of one column and one of two, against a column and against an
  expression.
- The rowid a side answers, over a key of an index and over a range of
  rowids.
- What names no key: a `COLLATE` on the other side, a column standing
  right of one that collates otherwise, a column of a statement written
  inside the `FROM`, an index over an expression, a partial index, an
  index under another collation, an index held backwards, a column of
  real affinity, and a statement holding a `RIGHT` join.
- A rowid a side answers as text that names no number, as a real between
  two whole numbers, and as a null.

### 6.6.272 The index a walk with no key reads (`db-sqlite`)

Document 16 step Q8.

- The narrowest of two indexes that cover, whichever was made last.
- What leaves the table scanned: a column no index holds, an index over
  an expression, a partial index, an index holding a place backwards, a
  table that keeps its rows in the key's own tree, and an `ORDER BY` the
  walk of the table answers.
- The searches a walk beginning at `OP_Rewind` counts.

### 6.6.273 The costs a key and a covering walk are held to (`db-sqlite`)

Document 16 step Q8.

- A term over a column of blob affinity against one asking for a number,
  which names no key.
- An index over the one column of a table, whose entry holds as many
  values as the row.

### 6.6.274 What a commit writes (`db-sqlite`)

Document 16 step Q8.

- The writes, the syncs and the removals of a commit that keeps a
  rollback journal, under each of the journal modes.
- The writes and the sync of a commit in write-ahead logging, and of the
  commit that begins the log again.
- A connection over a file beside a hot journal, over a file beside a
  log, and over three files of no bytes.
- A log holding a frame after its last commit frame that names a page no
  frame before it named.

### 6.6.275 The two journal syncs and the pragmas the table holds (`db-sqlite`)

Document 16 step Q8.

- The header a commit writes with no count of its records, and the count
  it writes after the first sync.
- A pragma name the table does not hold, with a value and without one,
  and one it holds that this crate does not write.

### 6.6.276 The root a drop moves (`db-sqlite`)

Document 16 step Q8.

- Two tables dropped in a row out of a file that keeps pointer maps,
  read back by `PRAGMA integrity_check`.

### 6.6.277 The page a new root takes (`db-sqlite`)

Document 16 step Q8.

- Three roots of a file that keeps pointer maps, with a chain lying where
  the second and the third take their pages.

### 6.6.278 The key an index over an expression names (`db-sqlite`)

Document 16 step Q8.

- A call, an operator and a `CAST` that say what the `CREATE INDEX` said,
  with the value on either side of the comparison.
- What says something else: a literal of another value, a column of
  another name, a call of another name, a call of another count of
  arguments, a node of another kind, a place under another collation, a
  term under a `COLLATE`, an expression that reads nothing of the row,
  one that reads a statement of its own, and one that reads two sides.

### 6.6.279 What a connection was told for fullfsync (`db-sqlite`)

Document 16 step Q8.

- `PRAGMA fullfsync` on and off, and a connection told neither.

### 6.6.280 The encoding a reader over a database of no schema holds (`db-sqlite`)

Document 16 step Q8.

- `PRAGMA encoding = 'UTF-16be'` before the first table, with `hex` read
  over a reader told the encoding and over one that is not.

### 6.6.281 The log a close writes back (`db-sqlite`)

Document 16 step Q8.

- A close over a file that keeps a log and one over a file that keeps
  none, with the rows read out of the file afterwards and the mode the
  file answers with.

### 6.6.282 Where a result column comes from (`db-sqlite`)

Document 16 step Q8.

- A column of a table, one of a table under an alias, a bare `rowid`,
  one read through a statement written inside a `FROM`, and one that
  comes out of an expression.

### 6.6.283 The log a commit makes again (`db-sqlite`)

Document 16 step Q8.

- A commit after a close that wrote the log back, with the rows the file
  holds and the mode it answers with.
- A header naming each schema format from one to four, and one naming
  five.

### 6.6.284 The temp schema of a connection (`db-sqlite`)

Document 16 step Q8.

- A connection with a temp table, the schema taken off it, the same
  schema handed back, and a schema of no bytes.

### 6.6.285 The row that points at itself (`db-sqlite`)

Document 16 step Q8.

- A row that points at itself written, changed and taken away, one that
  points at another row of the same table, and a table that keeps its
  rows in the key's own tree.

### 6.6.286 The zone localtime reads (`db-sqlite`)

Document 16 step Q8.

- A moment carried into local time and back, one outside the years the
  zone answers for, one the text says is UTC, a zone that fails, and a
  connection told no zone.

### 6.6.287 The rows a walk stops at (`db-sqlite`)

Document 16 step Q8.

- A `LIMIT` the rows of a `RIGHT` join fill, one an ordinary walk fills,
  one with an `OFFSET`, and one that names a column.

### 6.6.288 The file name written as a URI (`db-sqlite`)

Document 16 step Q8.

- A name that begins `file:` and one that does not, an authority that is
  empty, `localhost` and another, a `%HH` escape in the path, in the name
  of a parameter and in its value, a `%` that two digits do not follow,
  `%00`, a parameter of no name, one of no value, a fragment, and every
  `mode=` and `cache=` value the library holds and one it does not.

### 6.6.289 The options the build holds (`db-sqlite`)

Document 16 step Q8.

- `PRAGMA compile_options`, a name with the `SQLITE_` in front of it and
  one without, a name written in another case, a name that is the front
  of an option and one that is the front of an option and ends in `=`, a
  name the build holds no option under, the empty name, a place in the
  list, a place past it, and a place below nought.

### 6.6.290 The statement that stands as a row (`db-sqlite`)

Document 16 step Q8.

- A comparison of two statements, of a statement against a row, of a
  statement of one column against one value, a statement that answers no
  row, a `COLLATE` on either side, a `BETWEEN` over a statement, an `IN`
  over one, a statement of another width than the row, and a member of
  an `IN` list that holds another number of values than the row.

### 6.6.291 The columns one clause of a SET writes (`db-sqlite`)

Document 16 step Q8.

- A row of values, a statement, a statement that answers no row, one
  column in brackets, a clause of one column beside a clause of more, a
  statement whose width cannot be counted before it runs, and a clause
  that writes another number of columns than the value holds.

### 6.6.292 The operand of a CASE as a row (`db-sqlite`)

Document 16 step Q8.

- An operand that is a row and a `WHEN` that matches, one that does not,
  a statement as the operand, a `CASE` of one value, a `CASE` of no
  operand, and a `WHEN` of another width than the operand.

### 6.6.293 The words a result code names (`db-sqlite`)

Document 16 step Q8.

- Every primary code the table holds words for, the three codes that
  name words of their own, an extended code, a primary code the table
  holds no words for, and a code past every one the library holds.

### 6.6.294 The limits a connection holds (`db-sqlite`)

Document 16 step Q8.

- The number each limit carries, the value a connection opens with, a
  number no limit carries, a value the build takes, one above the hard
  limit, one below the smallest a limit takes, a value below nought, and
  an `ATTACH` past the databases the limit holds the connection to.

### 6.6.295 The length a value is held to (`db-sqlite`)

Document 16 step Q8.

- A blob of noughts longer than the limit, the text a quote grew, the
  text two values make, a literal, the text a replacement grew, the text
  a `group_concat` grew, a value the limit holds, a value that is no
  text, and a pattern longer than the limit of its own.

### 6.6.296 The columns a statement writes a value into (`db-sqlite`)

Document 16 step Q8.

- An `INSERT` of one value per column no expression computes, the count
  in the refusal, an `INSERT` that names a computed column, an `UPDATE`
  that writes one, and a table that keeps its rows in the key's own tree.

### 6.6.297 The order an aggregate reads its group in (`db-sqlite`)

Document 16 step Q8.

- One term, a term that sorts backwards, two terms, `NULLS FIRST` and
  `NULLS LAST`, one order per group, a `DISTINCT` beside the order, the
  row a `max` took, a call of no argument, an `ORDER BY` on a call that
  is no aggregate, and an aggregate inside the terms.

### 6.6.298 The file a rollback writes back (`db-sqlite`)

Document 16 step Q8.

- A transaction that vacuums the file and rolls back, the integrity of
  the file after every step of `incrvacuum3.test`, and the file read
  again from its bytes.

### 6.6.299 The database a trigger stands over (`db-sqlite`)

Document 16 step Q8.

- A schema in front of the table that names the database the trigger
  stands in, one that names another, a quoted name in the refusal, a
  trigger of the temp schema over a table of an attached database, the
  schema the `no such table:` names, and a trigger and an index over a
  table only an attached database holds.

### 6.6.300 The subtype a call answers (`db-sqlite`)

Document 16 step Q8.

- `coalesce`, `ifnull`, `iif` on either side of its condition, `nullif`,
  `min`, `max` and `likely` over a value `json()` answered, a sign before
  one, a sign that computes a number, and a call that answers none of its
  arguments.

### 6.6.301 The values an entry of an index holds (`db-sqlite`)

Document 16 step Q8.

- An expression over a column of a type that converted the text it was
  written, the `WHERE` of a partial index over the same column, and a
  unique index whose places hold nulls in more than one row.

### 6.6.302 The clock a value of the schema may not read (`db-sqlite`)

Document 16 step Q8.

- A row carrying `now` written against a `CHECK` constraint, against an
  index and against a generated column, a `localtime` and a `utc`
  modifier in an index, a `CREATE INDEX` over a row carrying `now`, and a
  modifier that reads neither the clock nor the zone.

### 6.6.303 The elementary functions (`db-sqlite`)

Document 16 step Q8.

- One statement per family of `func7.test`, against the text the C library
  writes: the logarithms with a base of ten, of two and of a number the
  call names, the exponential, the square root, the power, the three
  circular functions with their inverses, and the three hyperbolic
  functions with theirs.
- What answers nothing: an argument that is no number, a logarithm of
  nought or under it, a base of one, an arc sine outside its range, an
  inverse hyperbolic cosine under one, a negative base raised to a power
  that is no whole number, and a remainder by nought.
- The kernels themselves are compared with the host library in
  `crates/math`, which 6.6.304 records.

### 6.6.304 The kernels of the elementary functions (`audhsos-math`)

- Every function against the host library over a deterministic run of
  arguments at a tolerance of four units in the last place, with the
  square root compared bit for bit over the ranges where correct rounding
  is decided.
- The special values of each function: both zeros, both infinities, a
  NaN, the ends of the range, and the arguments outside the domain.
- `atanh` near one against the hyperbolic tangent of its own answer,
  because the host formula loses the digits of the argument there.
- A power of two and a power of ten are answered exactly by the logarithm
  of that base, which `format('%.30f', log10(100.0))` of `func7.test`
  reads.

### 6.6.305 The rows of a statement of no column (`db-sqlite`)

Document 17 step 55.

- `PRAGMA incremental_vacuum` answers one row per page it gives up and
  names no column, which the count of rows of the statement says and the
  freelist afterwards holds nothing of.

### 6.6.306 The mode a database is held under (`db-sqlite`)

Document 16 step Q8.

- `exclusive-1.*` of `exclusive.test` read against the writer: the
  default and each database of a connection, a pragma that names a
  schema, one that names none, a word that names neither mode, the temp
  schema, a database attached after the default was named, and one
  attached under `:memory:`.

### 6.6.307 The depth a trigger may reach (`db-sqlite`)

Document 16 step Q8.

- A trigger that writes its own table under `PRAGMA recursive_triggers`,
  a connection told a smaller `SQLITE_LIMIT_TRIGGER_DEPTH`, and a chain
  of forty keys each cascading into the next.

### 6.6.308 The temp schema under a rename (`db-sqlite`)

Document 16 step Q8.

- A view and a trigger of the temp schema over a table of `main`, a
  trigger written with the schema in front of that table, a view over
  another table, a temp table of the name being renamed, and a rename of
  a table of the temp schema itself.
- The tables one statement of the schema names, each with the schema
  written in front of it, for a trigger, a view, a statement in brackets
  and a statement the parser refuses.

### 6.6.309 The triggers a rename resolves (`db-sqlite`)

Document 16 step Q8.

- A trigger whose step writes a table no database holds, against a
  rename of a table and a rename of a column of another table, with the
  schema unchanged after each refusal, and the same rename taken once
  that table is there.
- A trigger of the temp schema whose step writes a table of `main`, and
  one whose step writes a table no database holds.
- The table each step of a trigger writes, for the three steps that
  write one, a step that writes none, and a statement the parser
  refuses.

### 6.6.310 What a value of the schema may name (`db-sqlite`)

Document 16 step Q8.

- A `CHECK`, a generated column, an index term and the `WHERE` of a
  partial index, each against: a column of the table, a column written
  under the table's name, a column written under another table, a name
  the table does not hold, a name written later in the statement, the key
  of the table, the key of a table written `WITHOUT ROWID`, a name in
  double quotes, and an index term written as a text.
- What a column falls back to: a literal, a call, a name, a variable, a
  statement in brackets, an `EXISTS`, a name in double quotes, and
  `false`.
