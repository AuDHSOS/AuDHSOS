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
  thresholds. Uncovered lines must be justified in review.
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
