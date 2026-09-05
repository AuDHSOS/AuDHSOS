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
| L7 Miri | host-executable `unsafe` in adapter crates | host | `cargo miri test` | every push |
| L8 Static | lints, layering, external code, unsafe budget, documentation | host | `cargo xtask check` | every push |

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
    results after every step, shrinking the sequence on failure.

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
- A hang is a failure. There are no retries; a test that fails once without
  a code change is a bug to fix.

## 6.4 Coverage

- Host coverage: `cargo xtask coverage` builds host tests with
  `-C instrument-coverage -Z coverage-options=branch`, merges the profiles
  with `llvm-profdata`, and exports LCOV with `llvm-cov`, both from the
  `llvm-tools-preview` component. Files under `src/tests/` are excluded, so
  the thresholds apply to product code only. Thresholds: 90 percent of
  lines and 85 percent of branches per gated crate (every crate except
  `xtask`, which is reported only). CI fails below the thresholds.
  Uncovered lines must be justified in review.
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

### 6.6.8 IPC (`kernel-ipc`)

- `call` with no receiver blocks the caller; a later `recv` completes the
  rendezvous; the receiver sees the badge and gets a reply object.
- `recv` with no sender blocks; a later `call` completes.
- `reply` on a consumed reply object fails; on a reply object whose caller
  was killed fails without touching memory; dropping a reply object wakes the
  caller with `ReplyDropped`.
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
  second concurrent waiter gets `Busy`; the bound interrupt sets exactly its
  bit.
- Fault message: has the reserved label range, carries fault kind, address,
  instruction pointer, and error code; the reply resumes the thread; the
  handler killing the process ends the wait cleanly.

### 6.6.9 System call decoding and dispatch (`kernel-syscall`)

- Unknown system call number; number above the table; every known number is
  in the table (table-driven).
- Argument count mismatch is rejected before any handle lookup.
- Validation order is observable: invalid handle beats wrong type beats
  missing right beats invalid argument beats quota (tested pairwise).
- IPC buffer with the result area at the last bytes of the page: no
  out-of-page access.
- Every error variant of every system call in its documentation table has a
  test that produces it.
- Round-trip encode/decode of every request and result layout, including
  maximum values of every field.

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

### 6.6.11 ACPI MADT parser (`kernel-hal-x86_64`, pure module)

- Missing RSDP; RSDP with a bad checksum; revision 0 versus 2 (RSDT versus
  XSDT); table length shorter than the header; entry length zero (must not
  loop forever); entry length beyond the table; unknown entry types skipped;
  local APIC address override; zero I/O APICs; more I/O APICs than the fixed
  capacity (error, not truncation); interrupt source overrides for IRQ 0 and
  IRQ 4.
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

- CRC32: the standard check value for the string `123456789`; empty input;
  a single byte; inputs crossing internal block boundaries.
- Protective MBR: signature bytes, one entry of type `0xEE` starting at
  sector 1 and covering the disk (capped at the maximum representable
  size).
- GPT: primary header at sector 1 and backup header at the last sector
  reference each other; header CRC32 and partition array CRC32 verify after
  writing; partition array has 128 entries of 128 bytes with exactly one in
  use; the entry carries the EFI system partition type GUID and the fixed
  unique GUID; first and last usable sector enclose the partition; a disk
  too small for the GPT structures is rejected.
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
- Miri: the tests above run under Miri with the cell holding a type with a
  destructor.

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
- `cargo tree` parser: nested depth prefixes; a crate appearing twice;
  workspace members versus the toolchain's own crates.
- `check-deps`: a lock file with a non-workspace package fails; a manifest
  with a `git`, `version`, or registry dependency fails; a path dependency
  outside the workspace fails.
- SPDX check: missing header; header on the second line; wrong license
  identifier.
- Runner: QEMU exit status mapping for 33, 35, 37, 0, 1, and a killed
  process; timeout produces a crash report with the captured output.

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
- Kernel stacks: every page of an allocated stack carries what the kernel
  writes into it; a released stack is unmapped and its frames are back in
  the reserve; the slot is handed out again with the same pages; a write to
  the guard page below a stack raises a page fault at the guard address.
- Threads: create a user thread that executes `thread_exit`; two threads of
  equal priority alternate (observed through a shared counter); a
  higher-priority thread preempts.
- Isolation: a user thread that reads a kernel address faults and the fault
  handler receives the message; a user thread that executes `hlt` faults.
- System calls: every system call has at least one success and one failure
  test issued from user mode.
- IPC: call and reply between two user threads; handle transfer; notification
  from a timer-bound interrupt to a user thread.

### 6.6.22 End-to-end tests in QEMU

- The root task starts, parses the archive, and starts the name server, the
  console driver, and the memory server.
- `app-hello` looks up the console by name, writes a line, and the line
  appears on the serial port through the userland driver while the kernel
  debug UART is disabled.
- Name lookup of a missing name returns `NotFound`.
- Two clients write interleaved lines; no line is torn.
- A client that faults is reported by the root task and the system keeps
  running.
- Console input: the runner sends bytes over the serial port and a test
  program echoes them.
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
  object for a request of their combined size.
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
- Loader in QEMU: booting with `-vga none` reports an absent framebuffer
  and the kernel reaches the harness.

### 6.6.25 i8042 controller and PS/2 decoding (`driver-i8042`)

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

- Event ring: a full ring drops the newest event and sets the overflow
  flag; the reader clears the flag; sequence numbers are contiguous
  otherwise; a subscriber whose notification cannot be signalled is
  removed.
- Input: the modifier state follows press and release; a release without
  a press is delivered as a release; pointer button state is tracked
  across packets; a wheel delta is delivered as its own event; both
  interrupts are acknowledged after the output buffer is drained.
- Display: `present` with damage rectangles copies exactly those pixels
  from the surface to the framebuffer (recording double); the cursor
  sprite saves and restores the background; the cursor is clamped to the
  screen; a surface larger than the screen is rejected; a client
  presenting a surface it does not own is rejected by badge; a client
  that goes away releases its surface.

### 6.6.28 QMP client and screendump reader (`xtask`)

- The greeting is parsed and `qmp_capabilities` is negotiated before the
  first command; an error response becomes an error value, not a panic;
  asynchronous event lines interleaved with responses are skipped.
- JSON subset: objects, arrays, strings with escapes, integers, booleans,
  `null`; nesting depth is bounded; malformed input is an error; the
  writer output parses back to the same value (property).
- `input-send-event` for a key press and release by `qcode`, for relative
  pointer motion, and for a button press and release.
- `screendump`: the PPM file is parsed (`P6`, comments, `maxval` 255); a
  truncated file is an error; a pixel and a rectangle checksum are read at
  given coordinates; coordinates outside the image are an error.
- A socket that never answers hits the timeout.

### 6.6.29 Graphical end-to-end tests in QEMU

- Output: a filled rectangle appears in the screendump with the expected
  color at its corners and the untouched color outside; a rendered string
  matches the glyph table pixel for pixel; the resolution used by the test
  is read from the boot information, never assumed.
- Input: a key sequence injected through QMP is echoed as `[input]` lines
  through the console driver; a pointer path produces motion events whose
  sum equals the injected path; a button press and release arrive in
  order.
- Combined: the cursor pixels move with the pointer; a stroke drawn while
  the button is held changes the pixels along the path; typed text appears
  at the text cursor.
- Absent hardware: with `-vga none` the input tests still pass and the
  display server reports `NotFound` to its clients.

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
  back; the canonical encoding rejects values at or above the prime.
- X25519 against RFC 7748 §5.2 and §6.1, including the iterated test at
  one thousand rounds and, behind a slow test, at one million; a peer
  value that produces an all-zero shared secret is rejected; non-canonical
  peer encodings are handled as the RFC prescribes.
- Ed25519 verification against RFC 8032 §7.1; rejection of `S >= L`, of
  non-canonical point encodings, of small-order public keys, and of a
  signature over a modified message.
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
- With `test-signing`: signing then verifying round-trips for both
  algorithms; the deterministic ECDSA nonce matches the RFC 6979 example.

### 6.6.34 Random generator (`crypto-rng`)

- `ChaChaRng` produces the expected stream for a fixed seed; requests of
  zero, one, block-sized, and block-crossing lengths are contiguous.
- The generator rekeys after each request: the state after a request never
  reproduces the bytes just returned.
- The reseed budget triggers exactly one `Entropy` call at the boundary; a
  failing entropy source surfaces as an error and never yields bytes.
- `ScriptedRng` returns the scripted bytes and reports exhaustion instead
  of repeating.

### 6.6.35 DER reader (`audhsos-der`)

- Header parsing: single-byte and multi-byte lengths; a non-minimal
  length, an indefinite length, a length beyond the input, and a length
  whose encoding is longer than needed are all rejected.
- Integers: a leading zero that is not required, a negative value where
  unsigned is expected, and the empty integer are rejected.
- Bit strings with a non-zero unused-bit count where zero is required;
  octet strings of length zero; object identifiers compared as bytes.
- Times: `UTCTime` and `GeneralizedTime` in the forms RFC 5280 allows,
  and rejection of the forms it forbids; leap years, the two-digit year
  window, the day-of-month bounds per month, and out-of-range fields.
- Nesting deeper than `MAX_DEPTH` is rejected without recursion beyond the
  limit; trailing bytes after the outermost value are rejected.
- Property: no input causes a panic and every accepted value re-encodes to
  the input bytes. Fuzz target `der`.

### 6.6.36 Certificates and path validation (`audhsos-x509`)

- Parsing: a minimal valid certificate; missing mandatory fields; an
  unknown critical extension is rejected; an unknown non-critical
  extension is ignored; duplicate extensions are rejected; a key algorithm
  outside the supported set is rejected.
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
- Names: an exact `dNSName` match; a wildcard in the leftmost label; a
  wildcard elsewhere, a partial-label wildcard, and a wildcard matching
  more than one label are rejected; a common name that would match is
  ignored when no matching SAN exists; case is compared case-insensitively
  for ASCII; a trailing dot is handled; an IP address matches only an
  `iPAddress` entry.
- Property: mutating any byte of a valid chain makes verification fail or
  parsing fail, never succeed. Fuzz target `x509`.

### 6.6.37 TLS record layer and key schedule (`audhsos-tls`)

- Records: the maximum plaintext and ciphertext lengths are accepted, one
  byte more is `RecordOverflow`; a header with an unexpected content type
  before the handshake is rejected; `change_cipher_spec` records are
  dropped in the compatibility window and rejected outside it; a record
  spanning two `read_tls` calls is reassembled; a zero-length inner
  plaintext without a content type byte is rejected.
- Padding: trailing zeros are stripped, an all-zero inner plaintext is
  rejected, padding of the maximum length is accepted.
- Sequence numbers: the nonce is the IV xor the sequence number; exhaustion
  of the sequence space is an error rather than a wrap.
- Key schedule: the secrets, keys, and IVs of the RFC 8448 traces for all
  three cipher suites; `hkdf_expand_label` against the label examples of
  RFC 8446 §7.1; `key_update` in both directions produces the documented
  successor keys.
- Transcript: the hash after each message of the trace; the `message_hash`
  substitution after a `HelloRetryRequest` reproduces the documented
  value.

### 6.6.38 TLS handshake and connection (`audhsos-tls`)

- The full RFC 8448 §3 trace: with a scripted generator and a fixed clock,
  every byte the client writes matches the document, and every secret and
  key matches; certificate verification is stubbed for this test because
  the trace uses an RSA certificate.
- A second full handshake against a project-generated ECDSA chain and a
  third against an Ed25519 chain, with certificate verification on.
- `HelloRetryRequest`: a server that asks for the other group completes;
  a second retry is rejected.
- Rejections: a `ServerHello` negotiating anything other than 1.3; the
  TLS 1.2 downgrade sentinel in the server random; a cipher suite not
  offered; a key share group not offered; a missing `key_share`; a
  `Finished` with a wrong verify data; a `CertificateVerify` over the
  wrong context string; an empty certificate list; an unexpected message
  in every state of the machine.
- After any fatal error the connection is poisoned: every further call
  returns the same error and no further bytes are produced.
- Buffers: a buffer below the documented minimum is rejected at
  construction; a handshake message larger than the reassembly buffer is
  `HandshakeTooLarge`; `write_tls` into a short output buffer makes
  progress across calls without losing bytes.
- Application data: `send` and `recv` round-trip through a paired client
  and a scripted peer; `close` emits `close_notify` and a peer
  `close_notify` surfaces as `PeerClosed`.
- Property: arbitrary byte streams fed to `read_tls` never panic and
  always end in an error or a consistent state. Fuzz targets `tls_record`
  and `tls_handshake`.

### 6.6.39 Time and calendar (`audhsos-time`)

- `civil_from_days` and `days_from_civil` round-trip for every day from
  1601-01-01 to 9999-12-31 (property) and agree with hand-computed values
  at the epoch, at 2000-02-29, at 1900-03-01, and at 2100-03-01.
- Leap years: 1900 and 2100 are common, 2000 and 2400 are leap; February
  has 28 or 29 days accordingly; day 0 and day 32 of any month are
  rejected.
- Field ranges: month 0 and 13, hour 24, minute 60, second 60, and a
  negative year are rejected; second 59 and hour 23 are accepted.
- `UnixTime`: the epoch is zero; negative values represent times before
  1970 and convert back; `checked_add` and `checked_sub` at the extremes
  of `i64` return `None` rather than wrapping.
- `Instant` and `Duration`: addition saturates at the maximum;
  `saturating_duration_since` of an earlier instant is zero; ordering is
  total.

### 6.6.40 Encodings (`audhsos-encoding`)

- Base64 against the RFC 4648 §10 vectors for lengths zero to six;
  encoding into a buffer one byte too small is an error and writes
  nothing.
- Base64 decoding rejects a missing pad, an excess pad, a pad in the
  middle, a character outside the alphabet, whitespace, and non-zero
  trailing bits in the final quantum.
- Hex: round trip for arbitrary input (property); an odd length, an
  upper-case and a lower-case digit pair, and a non-hex character are
  handled as specified.
- PEM against RFC 7468: a minimal certificate block; a label mismatch
  between the begin and end line, a missing end line, a line longer than
  64 characters other than the last, data after the end line, and an
  empty payload are rejected; a block preceded by explanatory text is
  accepted, as the RFC's lax parsing permits, and the text is not
  returned.
- Property: no input causes a panic, and every accepted block re-encodes
  to a canonical form that decodes to the same bytes. Fuzz target `pem`.

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
- Model tests: every container against `Vec`, `VecDeque`, and `BTreeMap`
  under generated operation sequences; no operation panics for any
  sequence.

### 6.6.42 Wire primitives (`net-wire`)

- Addresses: `MacAddr` and `Ipv4Addr` parse from and format to their
  canonical forms; the broadcast, unspecified, loopback, and multicast
  predicates; an `Ipv4Cidr` with prefix length 0, 32, and 33, the last
  rejected; `contains` at both ends of a range.
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

### 6.6.43 Ethernet and ARP (`net-eth`)

- Frames: a minimum-length frame, a maximum-length frame, one byte too
  short, and an unregistered ether type; the destination filter accepts
  the interface address and the broadcast address and drops others.
- ARP encoding against the RFC 826 field layout; a request for the
  interface address produces exactly one reply; a request for another
  address produces none.
- Cache: an entry moves `Incomplete` to `Reachable` on a reply and
  `Reachable` to `Stale` at the age boundary; an entry evicted at
  capacity is the least recently used; a second packet for a destination
  with a pending request replaces the first rather than queueing two.
- Retransmission: probes are emitted at the scheduled instants and stop
  after the configured count, after which the pending packet is dropped
  and the caller sees an unreachable result.
- Gratuitous ARP refreshes a reachable entry with the same address and
  does not replace one with a different address.

### 6.6.44 IPv4 and ICMP (`net-ip`)

- Header: a minimum header, a header with options that are skipped, a
  wrong version, a header length below five words, a total length beyond
  the frame, and a bad checksum are each handled as specified.
- Fragmentation: a datagram exactly at the MTU is not fragmented, one
  byte more produces two fragments whose reassembly equals the original;
  the don't-fragment bit turns an oversized datagram into an error.
- Reassembly: fragments in order, in reverse order, and with a duplicate;
  an overlapping fragment discards the datagram; a missing fragment
  expires at the deadline and frees its buffer; more concurrent datagrams
  than buffers evicts the oldest.
- ICMP: an echo request produces a reply with the payload copied; a
  destination-unreachable message is delivered to the upper layer with
  the embedded header parsed; generated errors stop at the token-bucket
  limit and resume after it refills; no error is generated for an
  incoming error, for a broadcast, or for a non-initial fragment.
- Routing: longest-prefix match with a host route, a subnet route, and
  the default route; a destination with no route is an error; an on-link
  destination resolves through ARP, an off-link one through the gateway.

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

- Encoding: a question, an answer with an `A` record, a `CNAME` chain of
  depth two, and a response with no answers.
- Names: a label of 63 characters, one of 64 rejected, a name of 255
  bytes, one of 256 rejected, an empty name, and a name with a
  compression pointer to an earlier label.
- Compression: a pointer loop, a forward pointer, and a pointer chain
  longer than the jump limit are rejected without unbounded work.
- Resolver: a matching response is accepted; responses with a wrong
  transaction id, a wrong question section, a wrong source address, or a
  wrong source port are ignored; the query is retried at the scheduled
  instants and rotates servers; exhaustion is an error.
- `CNAME` chains longer than eight and a chain that loops are rejected.
  Fuzz target `dns_message`.

### 6.6.48 DHCP (`net-dhcp`)

- The four-message exchange produces a bound lease with address, mask,
  router, and DNS servers taken from the options.
- Options: an unknown option is skipped; a truncated option is rejected;
  the end marker is required; padding is accepted; a missing message
  type is rejected.
- An offer with a foreign transaction id is ignored; a NAK returns the
  machine to the start; two offers select the first and ignore the
  second.
- Lease timers: renewal at T1 through unicast, rebinding at T2 through
  broadcast, and expiry that clears the address; a renewal answered late
  keeps the lease; backoff grows exponentially and stays inside the
  jitter bounds.

### 6.6.49 HTTP/1.1 client (`net-http`)

- Requests: the request line and headers for a minimal `GET`, with a
  host header always present; a header value with a control character is
  rejected at encoding time.
- Responses: a minimal response, a response with a body of declared
  length, a chunked body in one and in several reads, a chunk with an
  extension, the terminating zero chunk with and without trailers.
- Rejections: a status line that is too long, more headers than the
  limit, a header longer than the limit, obsolete line folding, both
  `Content-Length` and `Transfer-Encoding` present, two `Content-Length`
  headers that disagree, and a non-numeric length.
- Framing: a response split across arbitrary read boundaries produces the
  same result as one read (property); a body larger than the caller's
  buffer is delivered in parts without loss.
- A redirect status is reported with its location and is not followed.
  Fuzz target `http_response`.

### 6.6.50 Interface and demultiplexing (`net-stack`)

- Demultiplexing: an ARP frame, an IPv4 frame for a bound UDP socket,
  one for a TCP connection, one for an unbound port, and one for a
  foreign address each reach the right layer or are dropped.
- Sockets: handles are generation-checked, a handle from a closed socket
  is rejected, and the table reports exhaustion rather than reusing a
  live slot.
- `poll_at` returns the earliest deadline of every layer; after `poll`
  at that instant the deadline has advanced; an idle stack reports no
  deadline.
- `poll` drains the outgoing work into a transmit buffer that is too
  small across several calls without losing or reordering a frame.
- An interface without an address answers no ARP request and produces no
  IP traffic; configuring an address through DHCP makes both work.

### 6.6.51 Virtqueue logic (`virtio-queue`)

- Descriptors: a single-descriptor request, a chain of three, a chain
  that exhausts the free list, and a chain whose length exceeds the queue
  size are handled as specified.
- Rings: the available index wraps at 2^16 while the queue size is not a
  power-of-two divisor of it; a used element that names an unknown
  descriptor is rejected; the used index moving backwards is rejected.
- Notification suppression: with the no-notify flag set the driver emits
  no notification; clearing it resumes them.
- Initialization: the status sequence reset, acknowledge, driver,
  features-ok, driver-ok; a device that clears features-ok leaves the
  machine in failure; an operation on a queue before driver-ok is
  rejected; a device that sets `DEVICE_NEEDS_RESET` refuses every further
  operation.
- Model test: descriptors are allocated and freed against a reference
  free-list model over generated sequences; no sequence leaks a
  descriptor or hands out one twice.

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
  stored corpus file; Miri covers the glue.
- Symbol table: an address inside a function, at its first byte, at its
  last byte, and one past it; an address in no function; a file with no
  symbol table.
- Line program: a DWARF version 4 and a version 5 program; the standard
  opcodes, a special opcode sequence, and an end-of-sequence marker; an
  address before the first row and after the last; a truncated program is
  rejected without panic.
- Property: no input file causes a panic and every lookup either yields a
  location inside the file's ranges or reports none.

## 6.7 CI pipeline

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
