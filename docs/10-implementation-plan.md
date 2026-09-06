# 10. Implementation Plan for Phases 1 to 11

This document tells an implementer, human or agent, exactly what to build in
each remaining phase of the [roadmap](08-roadmap.md): crates, modules,
types, functions, algorithms, tests, policy changes, and acceptance
commands. It assumes no knowledge of the conversation that produced the
plan. Read this document top to bottom before starting a phase, then read
the sections it points to.

## 10.0 Ground rules for the implementer

### 10.0.1 Read first

1. [docs/README.md](README.md) and every document it lists, in order. The
   [decision register](09-decisions.md) is binding; the
   [architecture](02-architecture.md) is the target; the
   [safety policy](04-safety-policy.md) and the
   [code organization](05-code-organization.md) are the rules; the
   [edge-case catalog](06-testing-strategy.md#66-edge-case-catalog) is the
   definition of done.
2. The existing code. Phase 0 is implemented and committed. Read every
   `README.md` and `lib.rs` under `crates/` and the policy tables in
   `crates/tools/xtask/src/policy.rs`.

### 10.0.2 What exists after Phase 0

| Crate | Path | Public API in one line |
|-------|------|------------------------|
| `audhsos-abi` | `crates/abi` | `Error` (stable codes, `error_codes!` table), `Rights` (bit set, `rights!` table), `ObjectType` with `rights_mask()` (`object_types!` table), `Handle` (64-bit: 32-bit index, 32-bit generation), `layout::*` constants (`PAGE_SIZE`, `USER_SPACE_START/END`, `KERNEL_SPACE_START`, `PHYS_WINDOW_BASE`, `KERNEL_BASE`, `ROOT_TASK_BASE`, `MAX_BOOT_REGIONS`, `MAX_MESSAGE_WORDS`, `MAX_MESSAGE_HANDLES`, `PRIORITY_COUNT`, `TICKS_PER_SECOND`, `DEFAULT_TIME_SLICE_TICKS`, `MAX_PAGES_PER_CALL`) |
| `kernel-types` | `crates/kernel/types` | `PhysAddr`, `PhysFrame`, `PhysFrameRange` (+ `MAX_PHYS_ADDR`, `MAX_FRAME_NUMBER`), `VirtAddr`, `Page`, `PageRange`, `Alignment`, `Error`; generators in `strategies` behind feature `test-strategies` |
| `kernel-hal-api` | `crates/kernel/hal-api` | traits `paging::{FrameAccess<T>, TlbControl, FrameSource}`, `console::DebugConsole`, `exit::{TestExit, ExitStatus}`, `timer::{Timer, TimerError}`, `interrupt::{InterruptController, InterruptLine, Vector, InterruptError}`, `port::PortAccess` (feature `port-io`), `platform::{Platform, MemoryRegion, MemoryRegionKind}`; doubles in `doubles` behind feature `test-doubles`: `MemoryFrameAccess<T>`, `RecordingTlb`, `CountingFrameSource`, `RecordingConsole`, `RecordingExit`, `FakeTimer`, `FakeInterruptController`, `RecordingPorts`, `ScriptedPlatform` |
| `audhsos-sync` | `crates/sync` | `Global<T>` with `init`, `borrow(&impl ExclusiveToken)`, `GlobalRef`; `UncontendedToken`; adapter crate with exactly two `unsafe` sites |
| `test-support` | `crates/support/testing` | `generators::{range, just, one_of, bool, pair, vec, bytes, option, Generator, BoxGen}`, `property::{check, check_with, Config, Failure, seed_for}`, `model::{ModelTest, run_model_test, run_model_test_with}`, `rng::Rng`, `tree::Tree` |
| `xtask` | `crates/tools/xtask` | `lint`, `check-layering`, `check-deps`, `unsafe-budget`, `test`, `coverage`, `miri`, `doc`, `fuzz`, `check`; policy tables in `src/policy.rs` |

### 10.0.3 Rules that apply to every phase

- **Command entry point.** Run everything through the wrapper scripts of
  [07 section 7.5](07-toolchain-and-environment.md#75-findings-about-the-development-machine):
  `sh tools/xtask.sh <subcommand>`, and `sh tools/xtask-check.sh` for the
  full check. They put rustup's Cargo proxy in front of the `PATH`; the
  xtask refuses to run under any other Cargo. The exit status is the
  xtask's own, and `sh tools/xtask-check.sh --quiet` leaves one line per
  step and the output of the step that failed. The full check must pass
  before a phase is complete.
- **No external code.** No dependency outside the workspace in any
  section of any `Cargo.toml`, ever. `check-deps` enforces it.
- **`unsafe` only in adapter crates** (`Kind::Adapter` in `policy.rs`).
  Every `unsafe` block: one operation, preceded by a `// SAFETY:` comment
  that names the precondition and who guarantees it. Logic crates start
  with `#![forbid(unsafe_code)]`. When an adapter crate is added, add it to
  `policy::CRATES` with `Kind::Adapter { unsafe_budget, asm_budget }` and to
  the allowlist in [04-safety-policy.md 4.3](04-safety-policy.md#43-allowlist);
  set the budgets to the real counts at the end of the phase.
- **Assembly** only as `asm!` or `naked_asm!` inside adapter crates. Never
  `global_asm!`, never `.S` files. Every site goes into the inventory in
  [04-safety-policy.md 4.5](04-safety-policy.md#45-inline-assembly-inventory).
- **New crate checklist:** directory under `crates/`, `Cargo.toml`
  inheriting `[workspace.package]` and `[lints] workspace = true`,
  `README.md` included by `#![doc = include_str!("../README.md")]`, SPDX
  header on every file, entry in the root `Cargo.toml` `members`, entry in
  `policy::CRATES` with the allowed `deps`, row in the catalog of
  [05-code-organization.md 5.2](05-code-organization.md#52-crate-catalog).
- **Lints.** The workspace lint set is strict. Use `checked_*`,
  `wrapping_*`, or `saturating_*` instead of bare arithmetic; `.get()`
  instead of indexing; `From`/`TryFrom` instead of `as` (or
  `#[expect(clippy::as_conversions, reason = "...")]` in `const fn`s); no
  `unwrap`/`expect`/`panic!` outside tests. A justified exception uses
  `#[expect(lint, reason = "...")]`, never `allow`.
- **Tests** live in `src/tests/<module>.rs`, declared by `#[cfg(test)]
  mod tests;` in the crate root. Product files contain no test code.
  Generators for a crate's types live in that crate behind the feature
  `test-strategies`. Name tests `<subject>_<condition>_<expected>`. Every
  applicable item of the edge-case catalog gets a test; a phase is not done
  before that.
- **Coverage thresholds** (90 percent lines, 85 percent branches) apply to
  every host-testable crate. Restructure product code to remove
  unreachable branches instead of lowering thresholds.
- **Target crates** (built for `x86_64-unknown-none` or
  `x86_64-unknown-uefi`) get a `target` field in `policy::Crate` (Phase 2
  adds it). The xtask excludes them from host tests and coverage, lints them
  with `--target`, and tests them in QEMU.
- **Documents follow the code.** When the implementation deviates from a
  document, change the document in the same commit. Documents state what
  is; they do not argue why. A decision that changes gets a new register
  entry `D-nn` that supersedes the old one.
- **Commits.** Conventional Commits, one logical change each, changelog
  entry in the same commit, footer `Decision: D-nn` when a decision applies.
  Never commit with failing checks.
- **Communication with the project owner** is in German, formal address
  (`Sie`), clear and to the point. Every artifact in the repository is in
  English.

### 10.0.4 Order of work inside a phase

1. Types and errors of the new crate (compiles, documented).
2. Pure logic with host tests and property tests against the catalog.
3. Test doubles for any new trait.
4. Adapters (only where the phase has hardware work), with QEMU tests.
5. Policy tables, documents, changelog.
6. `sh tools/xtask-check.sh` green, then commit.

## 10.1 Phase 1: Memory management logic

Goal: `kernel-mm` and `kernel-objects` complete and host-tested; boot image
header and boot information in `audhsos-abi`. No hardware, no QEMU.

### 10.1.1 `audhsos-abi` additions

`audhsos-abi` gains the feature `test-strategies` with the optional
`test-support` dependency, as `kernel-types` has it.

`src/boot_image.rs`:

```rust
pub const BOOT_IMAGE_MAGIC: [u8; 8] = *b"AUDHSOS\0";
pub const BOOT_IMAGE_VERSION: u32 = 1;
pub const BOOT_IMAGE_HEADER_LEN: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BootImageHeader { pub root_task_offset: u64, pub root_task_len: u64,
    pub archive_offset: u64, pub archive_len: u64, pub kernel_reserve_size: u64 }
impl BootImageHeader {
    /// Parses and validates the header against `image_len` (the length of
    /// the whole image) and `ram_bytes` (total usable RAM).
    pub fn parse(bytes: &[u8], image_len: u64, ram_bytes: u64) -> Result<Self, BootImageError>;
    /// Serializes into 64 bytes, little-endian, for the xtask image writer,
    /// always with the current version, the current header length, and zero
    /// flags.
    pub fn to_bytes(&self) -> [u8; BOOT_IMAGE_HEADER_LEN];
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BootImageError { TooShort, BadMagic, UnsupportedVersion(u32), HeaderLength(u32),
    RootTaskOffset, RootTaskLength, ArchiveOffset, ArchiveLength, Overlap, Flags(u64),
    ReserveSize(u64) }
```

Validation exactly as the table in
[03-target-platform.md 3.1.6](03-target-platform.md#316-boot-image-format).
`parse` reads the 64 bytes as sixteen little-endian `u32` words, joins the
halves of the `u64` fields, and checks the fields in the order in which they
appear in the image: magic, version, header length, root task offset, root
task length, archive offset, archive length, reserve size, flags. The one
cross-field check, the overlap of the archive with the root task, comes
last. Every offset sum uses `checked_add`. An archive of length zero never
overlaps.

`src/boot_info.rs`:

```rust
pub const BOOT_INFO_MAGIC: [u8; 8] = *b"AUDHBOOT";
pub const BOOT_INFO_VERSION: u32 = 1;
pub const BOOT_INFO_HEADER_LEN: usize = 104;
pub const BOOT_REGION_LEN: usize = 24;
pub const BOOT_INFO_PAGE_LEN: usize = 4096;

#[repr(u32)] pub enum BootRegionKind { Usable = 1, Reserved = 2, AcpiReclaimable = 3, AcpiNvs = 4, MmioReserved = 5 }
impl BootRegionKind { pub const ALL: &'static [BootRegionKind];
    pub const fn code(self) -> u32; pub const fn from_code(code: u32) -> Option<Self>; }

#[repr(C)] #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BootRegion { pub start: u64, pub len: u64, pub kind: u32, pub reserved: u32 }
impl BootRegion { pub const fn new(start: u64, len: u64, kind: BootRegionKind) -> Self;
    pub const fn end(self) -> Option<u64>;
    pub const fn contains_range(self, start: u64, len: u64) -> bool; }

#[repr(C)] #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BootInfoHeader { pub magic: [u8; 8], pub version: u32, pub size: u32,
    pub phys_window_base: u64, pub kernel_phys_start: u64, pub kernel_phys_len: u64,
    pub boot_image_phys_start: u64, pub boot_image_phys_len: u64,
    pub page_tables_phys_start: u64, pub page_tables_phys_len: u64,
    pub boot_stack_phys_start: u64, pub boot_stack_phys_len: u64,
    pub acpi_rsdp: u64, pub region_count: u32, pub reserved: u32 }

pub enum FixedRange { Kernel, BootImage, PageTables, BootStack }
impl FixedRange { pub const ALL: &'static [FixedRange]; pub const fn name(self) -> &'static str; }

pub enum BootInfoError { TooShort, BadMagic, UnsupportedVersion(u32), Size(u32),
    RegionCount(u32), RegionKind { index: usize, kind: u32 }, RegionLength(usize),
    RegionOverflow(usize), RangeEmpty(FixedRange), RangeOverflow(FixedRange),
    RangeOverlap(FixedRange, FixedRange), RangeOutsideRegions(FixedRange), AcpiPointer(u64) }

pub struct BootInfoView<'a> { header: BootInfoHeader, regions: &'a [u8] }
impl<'a> BootInfoView<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, BootInfoError>;
    pub const fn header(&self) -> BootInfoHeader;
    pub const fn fixed_range(&self, range: FixedRange) -> (u64, u64);
    pub const fn region_count(&self) -> usize;
    pub fn regions(&self) -> impl Iterator<Item = BootRegion> + '_;
    pub const fn phys_window_base(&self) -> u64;
    pub const fn acpi_rsdp(&self) -> Option<u64>;   // None for the address zero
}

pub struct BootInfoWriter;
impl BootInfoWriter {
    pub fn write(page: &mut [u8; BOOT_INFO_PAGE_LEN], header: &BootInfoHeader,
        regions: &[BootRegion]) -> Result<u32, BootInfoError>;
}
```

`BootInfoHeader` is 104 bytes, `BootRegion` is 24 bytes, and the whole
structure is `BOOT_INFO_HEADER_LEN + region_count * BOOT_REGION_LEN` bytes.
The `repr(C)` attributes record the ABI layout; no code reinterprets bytes
as these types.

`BootInfoView` keeps the region array as bytes and decodes one entry at a
time in `regions()`, because turning bytes into a `&[BootRegion]` would
need `unsafe`, which this crate does not have. `parse` decodes the fixed
part word by word with `u32::from_le_bytes` and validates, in this order:
magic; version; `BOOT_INFO_HEADER_LEN <= size <= BOOT_INFO_PAGE_LEN`;
`region_count <= MAX_BOOT_REGIONS`;
`size == BOOT_INFO_HEADER_LEN + region_count * BOOT_REGION_LEN`; then every
region, whose kind must be known, whose length must not be zero, and whose
end must be representable; then the four fixed ranges, each of which must
be non-empty, representable, inside one region, and disjoint from the other
three; and finally the ACPI pointer, which is either zero (absent) or
inside one region. `FixedRange` names the four ranges in the errors.

`BootInfoWriter::write` clears the page, then writes the magic, the
version, the size, and the region count from `regions`, not from `header`,
so that the writer always produces what the parser accepts; it returns the
number of bytes written. The kernel adapter (Phase 2) turns a raw pointer
into the byte slice; the parsing stays here, safe and fuzzable.

Every error type of both modules implements `Display`.

Tests: catalog 6.6.10 in full, one test per item, in
`src/tests/boot_image.rs` and `src/tests/boot_info.rs`; each test file
builds the raw bytes field by field, so that it can produce values the
serializer never writes. `src/strategies.rs` behind `test-strategies`
offers `any_boot_image_header()`, `image_len_for()`,
`any_boot_image_bytes()` (a serialized header with up to four bytes
replaced and sometimes truncated), `any_boot_info_bytes()`,
`any_region_kind_code()`, and `any_boot_region()`.

### 10.1.2 Crate `kernel-objects` (`crates/kernel/objects`)

Layer 2, `no_std`, `forbid(unsafe_code)`. The policy allows `kernel-types`,
`audhsos-abi`, and `test-support`; the manifest declares only what the code
uses, because `unused_crate_dependencies` is denied: `audhsos-abi` and the
optional `test-support` (feature `test-strategies`).

`src/pool.rs`:

```rust
pub struct ObjectId<T> { index: u32, generation: u32, _marker: PhantomData<fn() -> T> }
impl<T> ObjectId<T> {
    pub const fn new(index: u32, generation: u32) -> Self;
    pub const fn index(self) -> u32; pub const fn generation(self) -> u32;
}   // Clone, Copy, PartialEq, Eq, Hash, and Debug by hand, so that they do not need `T: Clone` and the like

pub struct Pool<T, const N: usize> { slots: [Slot<T>; N], free_head: Option<u32>, free_tail: Option<u32>, live: u32 }
struct Slot<T> { generation: u32, next_free: Option<u32>, occupant: Option<Occupant<T>> }
struct Occupant<T> { refs: u32, value: T }
pub enum PoolError { Exhausted, StaleId, RefOverflow }
impl<T, const N: usize> Pool<T, N> {
    pub fn new() -> Self;                                  // every slot free, generation 1, FIFO list 0..N
    pub fn capacity(&self) -> u32; pub const fn live(&self) -> u32; pub const fn is_empty(&self) -> bool;
    pub fn allocate(&mut self, value: T) -> Result<ObjectId<T>, PoolError>;   // refs = 1
    pub fn get(&self, id: ObjectId<T>) -> Result<&T, PoolError>;
    pub fn get_mut(&mut self, id: ObjectId<T>) -> Result<&mut T, PoolError>;
    pub fn references(&self, id: ObjectId<T>) -> Result<u32, PoolError>;
    pub fn retain(&mut self, id: ObjectId<T>) -> Result<(), PoolError>;       // refs += 1, checked
    pub fn release(&mut self, id: ObjectId<T>) -> Result<Option<T>, PoolError>; // refs -= 1; Some(value) when it reached 0: slot freed, generation += 1 (skipping 0), appended to the FIFO tail
}
impl From<PoolError> for audhsos_abi::Error { /* Exhausted -> PoolExhausted, StaleId -> InvalidHandle, RefOverflow -> QuotaExceeded */ }
```

A slot is free exactly when it has no occupant; the generation and the
free-list link live outside the occupant, so that no code path has to
distinguish a case that cannot occur. `[Slot<T>; N]` needs `T: Sized`;
build the array with `core::array::from_fn`. Phase 5 changes two details of
this so that an empty pool is all zeros and its constructor is `const`
(D-66, described in 10.5.2): the generation counts up in `allocate` instead
of starting at one, and the free list is implicit through a high-water mark
instead of being linked at construction. What the pool does is unchanged,
the FIFO reuse included. `N` is a const generic so that pool sizes come
from boot-time constants in `kernel-core`; sizes are not decided here.
`capacity` is not a `const fn` because `u32::try_from` is not const on the
pinned toolchain.

`src/quota.rs`:

```rust
pub struct QuotaExceeded { pub limit: u32, pub used: u32, pub requested: u32 }
pub struct Quota { limit: u32, used: u32 }   // Default is the quota with the limit zero
impl Quota {
    pub const fn new(limit: u32) -> Self;
    pub const fn limit(self) -> u32; pub const fn used(self) -> u32;
    pub const fn remaining(self) -> u32; pub const fn is_empty(self) -> bool;
    pub const fn charge(&mut self, amount: u32) -> Result<(), QuotaExceeded>;  // unchanged when it fails
    pub const fn refund(&mut self, amount: u32);                               // saturating
}
impl From<QuotaExceeded> for audhsos_abi::Error { /* QuotaExceeded */ }
```

`src/strategies.rs` (feature `test-strategies`): `PoolOp` with
`Allocate`, `Retain`, `Release`, `Get`, and `GetStale`, `any_pool_op()`,
and `any_quota_op()`.

Tests: catalog 6.6.6 pool and quota items (handle items come in Phase 5):
full pool, stale generation after free and reuse, generation wrap
(`u32::MAX` reuses, reached by setting the generation of a free slot
through the `#[cfg(test)]`-only helpers `set_generation`, `set_references`,
and `generation_of`), retain/release counts, reference overflow, FIFO reuse
order, quota exactly at limit and one above, refund restores. The step
that appends a released slot to the free list is the `pub(crate)` helper
`link_free`, so that the tests can call it with a tail outside the pool and
cover the branch that rejects it. Model-based test: random
`allocate`/`retain`/`release`/`get` against a
`HashMap<(index, generation), (value, refs)>` and the list of every id
handed out so far.

### 10.1.3 Crate `kernel-mm` (`crates/kernel/mm`)

Layer 2, `no_std`, `forbid(unsafe_code)`, deps: `kernel-types`,
`kernel-hal-api`, `audhsos-abi`, optional `test-support`; dev-deps
`test-support`, `kernel-hal-api` with `test-doubles`, `kernel-types` with
`test-strategies`.

Modules:

**`memory_map.rs`.** Input: `&[MemoryRegion]` from `Platform`. Output:

```rust
pub enum MapError { TooManyRegions, NoUsableMemory, ReserveDoesNotFit, Address(kernel_types::Error) }
pub struct NormalizedMap { spans: SpanList }
pub fn normalize(regions: &[MemoryRegion]) -> Result<NormalizedMap, MapError>;
impl NormalizedMap {
    pub const fn len(&self) -> usize; pub const fn is_empty(&self) -> bool;
    pub fn iter(&self) -> impl Iterator<Item = PhysFrameRange> + '_;
    pub fn total_frames(&self) -> u64; pub fn total_bytes(&self) -> u64;
    pub fn is_sorted_and_disjoint(&self) -> bool;
    pub(crate) fn without(&self, range: PhysFrameRange) -> Result<NormalizedMap, MapError>;
}
```

The map is held as a `SpanList`, a fixed array of `MAX_BOOT_REGIONS`
half-open frame-number spans plus a length. Frame numbers, not
`PhysFrameRange`, are the working representation, because a span of frame
numbers has a `Default` and needs no validated constructor while the list
is being edited; `iter()` converts on the way out.

Algorithm: (1) split input into usable (`MemoryRegionKind::Usable`) and
excluded (every other kind), skipping regions of length zero; (2) usable
regions shrink to frame boundaries (start rounded up, end rounded down and
clamped to the last representable frame; drop empty), excluded regions grow
to frame boundaries (start down, end up); (3) sort usable by start, merge
overlapping and touching; (4) subtract every excluded range from the usable
list (a range inside a usable region splits it into two); (5) sort and
merge again; fail with `TooManyRegions` if the count exceeds
`MAX_BOOT_REGIONS` at any point, so that a split that no longer fits is an
error and not a truncation; fail with `NoUsableMemory` if nothing remains;
fail with `Address(Overflow)` if a region reaches beyond the address space.
`is_sorted_and_disjoint()` is what the tests check after every operation.

**`reserve.rs`.**

```rust
pub const MIN_RESERVE_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_RESERVE_BYTES: u64 = 64 * 1024 * 1024;
pub const RESERVE_DIVISOR: u64 = 16;
pub const fn default_reserve_bytes(total_bytes: u64) -> u64;
pub fn select_reserve(map: &NormalizedMap, override_bytes: u64)
    -> Result<(PhysFrameRange, NormalizedMap), MapError>;
```

Size = `override_bytes` if non-zero (must be frame-aligned, else
`Address(Unaligned)`, and below the usable memory, else
`Address(Overflow)`), otherwise `default_reserve_bytes`, which clamps
`total_bytes / RESERVE_DIVISOR` to `MIN_RESERVE_BYTES` and
`MAX_RESERVE_BYTES` and rounds up to a frame. Take the first usable range
with at least that many frames, carve the reserve from its start, return
the reserve and the map without those frames; `ReserveDoesNotFit` if no
single range is large enough, which is also what happens when the machine
has less memory than the smallest reserve.

**`frame_allocator.rs`.**

```rust
pub const BITS_PER_WORD: u64 = 64;
pub const RESERVE_WORDS: usize = (64 * 1024 * 1024 / 4096) / 64;   // words for the largest reserve
pub const MAX_MANAGED_FRAMES: u64 = 64 * 1024 * 1024 / 4096;
pub struct BitmapFrameAllocator { base: PhysFrame, count: u64, bits: [u64; RESERVE_WORDS], free: u64 }
pub enum FrameError { OutOfFrames, NotManaged, NotAllocated, AlreadyAllocated, InvalidCount }
impl BitmapFrameAllocator {
    pub fn new(range: PhysFrameRange) -> Result<Self, FrameError>;     // InvalidCount if larger than the bitmap
    pub const fn capacity(&self) -> u64; pub const fn base(&self) -> PhysFrame;
    pub const fn free_count(&self) -> u64;
    pub fn range(&self) -> PhysFrameRange; pub fn manages(&self, frame: PhysFrame) -> bool;
    pub fn allocate(&mut self) -> Result<PhysFrame, FrameError>;        // first zero bit
    pub fn allocate_at(&mut self, frame: PhysFrame) -> Result<(), FrameError>;  // AlreadyAllocated when taken
    pub fn allocate_contiguous(&mut self, count: u64, alignment: Alignment) -> Result<PhysFrameRange, FrameError>; // first fit
    pub fn free(&mut self, frame: PhysFrame) -> Result<(), FrameError>;
    pub fn free_contiguous(&mut self, range: PhysFrameRange) -> Result<(), FrameError>;
}
impl FrameSource for BitmapFrameAllocator { /* allocate_frame = allocate().ok(), release_frame = free ignored on error */ }
impl From<FrameError> for audhsos_abi::Error { /* OutOfFrames -> OutOfKernelMemory, AlreadyAllocated -> AlreadyExists, the rest -> InvalidArgument */ }
```

A set bit means the frame is handed out. Every bit above `count` is set at
construction, so no frame outside the reserve is ever handed out and
`allocate` is a scan for the first clear bit. `allocate_at` claims one
named frame; the loader needs it to reserve frames it already used.
`free_contiguous` checks the whole range before it changes anything, so a
range that is only partly allocated leaves the bitmap untouched; an empty
range succeeds and frees nothing.

**`page_table.rs`.** Architecture-neutral interface plus the `x86_64`
format:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Permissions { pub write: bool, pub execute: bool, pub user: bool }
impl Permissions {
    pub const READ_ONLY: Permissions; pub const READ_WRITE: Permissions; pub const READ_EXECUTE: Permissions;
    pub const fn is_write_execute(self) -> bool; pub const fn for_user(self) -> Permissions;
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CachePolicy { #[default] WriteBack, Uncached }
pub enum EntryError { ReservedBits(u64), HugePage }
pub trait EntryFormat: Copy + Default + 'static {
    const LEVELS: usize; const INDEX_BITS: u32; const ENTRIES: usize; const EMPTY: Self;
    fn index(level: usize, page: Page) -> usize;             // 9-bit slices of the page number
    fn is_present(self) -> bool;
    fn frame(self) -> Option<PhysFrame>;                      // None when the entry is not present
    fn table(frame: PhysFrame) -> Self;                       // present | writable | user, pointing at a table
    fn leaf(frame: PhysFrame, perms: Permissions, cache: CachePolicy, global: bool) -> Self;
    fn permissions(self) -> Permissions;
    fn with_permissions(self, perms: Permissions) -> Self;
    fn validate(self) -> Result<(), EntryError>;              // reserved bits
}
#[repr(C, align(4096))] pub struct PageTable<F: EntryFormat> { pub entries: [F; 512] }
impl<F: EntryFormat> PageTable<F> {
    pub fn new() -> Self;                                     // Default is the same
    pub fn entry(&self, index: usize) -> F;                   // F::EMPTY outside the table
    pub fn set_entry(&mut self, index: usize, entry: F) -> bool;  // false outside the table
    pub fn is_unused(&self) -> bool;
}
pub const X86_PRESENT: u64; pub const X86_WRITABLE: u64; pub const X86_USER: u64;
pub const X86_WRITE_THROUGH: u64; pub const X86_NO_CACHE: u64; pub const X86_ACCESSED: u64;
pub const X86_DIRTY: u64; pub const X86_HUGE: u64; pub const X86_GLOBAL: u64;
pub const X86_NO_EXECUTE: u64; pub const X86_ADDRESS_MASK: u64; pub const X86_RESERVED_MASK: u64;
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)] pub struct X86Entry(u64);
impl X86Entry { pub const fn from_raw(raw: u64) -> Self; pub const fn as_u64(self) -> u64;
    pub const fn has(self, flag: u64) -> bool; }
```

`X86Entry` bits: PRESENT 0, WRITABLE 1, USER 2, WRITE_THROUGH 3, NO_CACHE 4,
ACCESSED 5, DIRTY 6, HUGE 7, GLOBAL 8, address bits 12..=51, NO_EXECUTE 63;
`validate` accepts every entry that is not present and otherwise rejects
bits 52..=62 and HUGE (large pages are not supported in the first release).
`Permissions { write: false, execute: false }` still maps readable; a
read-only entry has WRITABLE clear and NO_EXECUTE set unless `execute`.
`CachePolicy::Uncached` sets NO_CACHE and WRITE_THROUGH together.
`PageTable::entry` and `PageTable::set_entry` define what an index outside
the table does instead of failing, because `EntryFormat::index` never
produces one; the tests call both with such an index, so that the behavior
is covered rather than assumed.

**`mapper.rs`.**

```rust
pub struct Mapper<'a, F: EntryFormat, A: FrameAccess<PageTable<F>>, T: TlbControl, S: FrameSource> {
    root: PhysFrame, access: &'a mut A, tlb: &'a mut T, frames: &'a mut S, _format: PhantomData<fn() -> F> }
pub enum MapError { AlreadyMapped, NotMapped, OutOfKernelMemory, UnreachableFrame, Entry(EntryError) }
pub enum Progress { Done, Partial(u64) }
impl Mapper {
    pub fn new(root: PhysFrame, access: &'a mut A, tlb: &'a mut T, frames: &'a mut S) -> Self;
    pub const fn root(&self) -> PhysFrame;
    pub fn map(&mut self, page: Page, frame: PhysFrame, perms: Permissions, cache: CachePolicy) -> Result<(), MapError>;
    pub fn unmap(&mut self, page: Page) -> Result<PhysFrame, MapError>;
    pub fn protect(&mut self, page: Page, perms: Permissions) -> Result<(), MapError>;
    pub fn translate(&self, page: Page) -> Option<(PhysFrame, Permissions)>;
    pub fn map_range(&mut self, pages: PageRange, first_frame: PhysFrame, perms: Permissions, cache: CachePolicy, budget: u64) -> Result<Progress, MapError>;
    pub fn unmap_range(&mut self, pages: PageRange, budget: u64) -> Result<Progress, MapError>;
}
impl From<MapError> for audhsos_abi::Error { /* AlreadyMapped, NotMapped, OutOfKernelMemory; UnreachableFrame and Entry -> InvalidArgument */ }
```

`map` walks the levels `LEVELS - 1` down to 1 and then writes the leaf at
level 0: for a missing table, allocate a frame from `frames`, write
`PageTable::default()` through `access.table_mut(frame)`
(`UnreachableFrame` if `None`), install `F::table(frame)`. Keep the frames
allocated in this call in a fixed array of three entries, one per level
below the root; on any failure free them in reverse and clear the entries
they were installed in (rollback), then return the error. Installing the leaf into an occupied
entry is `AlreadyMapped`. After a successful change call
`tlb.flush_page(page)` exactly once; a call that fails flushes nothing.
`unmap` clears the leaf, then frees each intermediate table whose 512
entries are all empty, from the lowest level upward, never the root. Range
operations stop after `budget` pages and return `Partial(done)`; `budget`
is `MAX_PAGES_PER_CALL` in the kernel, and the pages a range operation
processed before an error stay processed. The `global` bit is set when the
page is a kernel page.

**`address_space.rs`.**

```rust
pub enum RegionError { Empty, Unaligned, Overflow, OutsideUserSpace, AddressInUse, NotMapped, QuotaExceeded }
pub fn user_range(start: u64, len: u64) -> Result<PageRange, RegionError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region<B: Copy> { pub pages: PageRange, pub backing: B, pub offset: u64, pub perms: Permissions }
pub struct Removed<B: Copy, const M: usize> { items: [Option<Region<B>>; M], len: usize, remaining: Option<PageRange> }
impl<B: Copy, const M: usize> Removed<B, M> {
    pub fn iter(&self) -> impl Iterator<Item = &Region<B>>;
    pub const fn len(&self) -> usize; pub const fn is_empty(&self) -> bool;
    pub const fn remaining(&self) -> Option<PageRange>;
}
pub struct RegionTable<B: Copy, const N: usize> { regions: [Option<Region<B>>; N], len: usize, user: bool } // Phase 5 inverts the flag to `kernel: bool` (D-66)
impl RegionTable {
    pub const fn new() -> Self;                                                     // user address space
    pub const fn kernel() -> Self;                                                  // the half check relaxed
    pub const fn len(&self) -> usize; pub const fn is_empty(&self) -> bool;
    pub const fn free_slots(&self) -> usize;
    pub fn insert(&mut self, region: Region<B>) -> Result<(), RegionError>;         // sorted insert, overlap check
    pub fn remove<const M: usize>(&mut self, pages: PageRange) -> Result<Removed<B, M>, RegionError>; // may split one region into two; needs one free slot
    pub fn protect(&mut self, pages: PageRange, perms: Permissions) -> Result<(), RegionError>; // may split into three; needs two free slots
    pub fn find(&self, page: Page) -> Option<&Region<B>>;
    pub fn iter(&self) -> impl Iterator<Item = &Region<B>>;
    pub fn check_invariants(&self) -> bool;
}
impl From<RegionError> for audhsos_abi::Error { /* Unaligned, AddressInUse, NotMapped, QuotaExceeded; the rest -> InvalidArgument */ }
```

Every range must satisfy: user half, start at or above
`USER_SPACE_START`, non-empty. The kernel address space uses the same type
with the check relaxed through the constructor `RegionTable::kernel()`.
`user_range` turns a raw address and a length from a system call into a
`PageRange` and is the place where `Empty`, `Unaligned`, `Overflow`, and
`OutsideUserSpace` are told apart.

`remove` is bounded like the mapper's range operations: it removes at most
`M` pieces per call and reports the untouched rest of the request in
`Removed::remaining()`, so that a caller with a long range loops instead of
running unbounded inside the kernel. A request that overlaps no region at
all is `NotMapped`. `protect` works on the region that holds the first page
of the request and reports `NotMapped` when the request reaches beyond that
region. A split that would need a slot the table does not have is
`QuotaExceeded` and changes nothing. Splitting moves the offset into the
backing object with the pages.

**`strategies.rs`** (feature `test-strategies`): `any_memory_region()` and
`any_memory_regions()` (lists of 0..=64 regions with random kinds and
lengths, including overlapping and unaligned ones),
`any_populated_memory_regions()` (the same with one usable region added, so
that normalization can succeed), `any_permissions()`,
`any_user_page_range()`, `RegionOp`, and `any_region_table_op()`.

Tests: catalog 6.6.2, 6.6.3, 6.6.4 (including the model-based test with
`MemoryFrameAccess<PageTable<X86Entry>>`, `RecordingTlb`, and
`CountingFrameSource` from `kernel-hal-api::doubles`), 6.6.5. For the
model test of the mapper, extend `MemoryFrameAccess` with
`fn with_lazy_tables(ram: PhysFrameRange) -> Self` that materializes a
default table on first `table_mut` for any frame inside `ram`, so that
freshly allocated frames are reachable like in the kernel. The private
helpers that index the fixed arrays (`SpanList`, `RegionTable`, the
bitmap, `Region::clipped`, `remaining_from`) are `pub(crate)`, so that the
crate's own tests reach the branches that reject an index outside the array
or a range that does not overlap; without them those branches would be
unreachable and the branch coverage would report a gap that no test can
close.

### 10.1.4 Policy and documents

- `policy::CRATES`: add `kernel-objects` (deps `kernel-types`,
  `audhsos-abi`, `test-support`) and `kernel-mm` (deps `kernel-types`,
  `kernel-hal-api`, `audhsos-abi`, `test-support`); `audhsos-abi` gains
  `test-support` for its `test-strategies` feature. The policy lists what a
  crate may depend on; a manifest declares only what its code uses.
- The layering test of the xtask uses `kernel-hal-api` as the example of a
  crate that may reach `test-support` only as a dev-dependency, because
  `audhsos-abi` may now reach it as a normal one.
- Catalog rows in 05 for both crates; changelog entries.
- `kernel-types` gains `PhysFrame::ZERO` and `PhysFrameRange::EMPTY`, the
  fillers the fixed-size arrays of `kernel-mm` need.
- `MemoryFrameAccess::with_lazy_tables` bounds the `FrameAccess`
  implementation of that double to `T: Default`; 06 records the behavior.

### 10.1.5 Acceptance

`sh tools/xtask-check.sh` passes; the coverage table shows both new
crates above the thresholds; every item of 6.6.2 to 6.6.6 (pool items) and
6.6.10 has a test whose name identifies the item.

## 10.2 Phase 2: Loader, boot, and test harness

Goal: `sh tools/xtask.sh test --qemu` boots test kernels through the
project's own UEFI loader and reports over the serial line.

### 10.2.0 `audhsos-abi` additions

Extend `BootInfoHeader` with the framebuffer fields of
[03-target-platform.md 3.1.5](03-target-platform.md#315-boot-information-structure):
`framebuffer_phys_start: u64`, `framebuffer_len: u64`,
`framebuffer_width: u32`, `framebuffer_height: u32`,
`framebuffer_stride: u32`, `framebuffer_format: u32`, placed after
`acpi_rsdp` and before `region_count`; `BOOT_INFO_HEADER_LEN` becomes
136. Add `enum FramebufferFormat { Rgbx8888 = 1, Bgrx8888 = 2 }` with
`code` and `from_code`, and `struct Framebuffer { phys_start, len, width,
height, stride, format }` returned by `BootInfo::framebuffer() ->
Option<Framebuffer>`. The parser applies the framebuffer rules of 3.1.5,
including the overlap check against `Usable` regions and the enclosing
`MmioReserved` region; the writer takes `Option<Framebuffer>`; the
generators produce present and absent framebuffers. The version stays
`1` (D-32).

Tests: catalog 6.6.24 boot information items.

### 10.2.1 Crate `audhsos-elf` (`crates/elf`)

Layer 0, `no_std`, `forbid(unsafe_code)`, no deps except optional
`test-support`.

```rust
pub const MAX_SEGMENTS: usize = 16;
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct Segment { pub file_offset: u64, pub file_size: u64, pub vaddr: u64, pub mem_size: u64, pub align: u64, pub write: bool, pub execute: bool }
pub struct Image<'a> { pub bytes: &'a [u8], pub entry: u64, segments: [Option<Segment>; MAX_SEGMENTS], count: usize }
pub struct Constraints { pub lowest_vaddr: u64, pub highest_vaddr: u64 }   // inclusive bounds of the allowed half
pub fn parse(bytes: &[u8], constraints: Constraints) -> Result<Image<'_>, ElfError>;
pub enum ElfError { TooShort, BadMagic, NotClass64, NotLittleEndian, NotExecutable, WrongMachine, HeaderSize,
    ProgramHeaderSize, ProgramHeaderTable, NoSegments, TooManySegments, SegmentFileRange, SegmentMemorySize,
    SegmentAlignment, SegmentOutsideBounds, SegmentsOverlap, WritableAndExecutable, EntryNotExecutable, Overflow }
```

Modules: `error.rs` (the error enum with `Display`), `image.rs` (the
constants `EHDR_LEN` and `PHDR_LEN`, `Segment` with `end`, `contains`, and
`overlaps`, `Constraints::allows`, `Image` with `segments`,
`segment_count`, `segment_bytes`, and `highest_address`, and `parse`), and
`strategies.rs`. The errors that name one segment carry its index in the
program header table.

Decode the header as sixteen little-endian `u32` words and each program
header entry as fourteen, joining the halves of the `u64` fields; every
field of both structures is `u32`-aligned inside its structure, so no
byte-wise reader is needed. Checks: magic `7F 45 4C 46`, `EI_CLASS == 2`, `EI_DATA == 1`,
`e_type == 2`, `e_machine == 0x3E`, `e_ehsize >= 64`, `e_phentsize >= 56`,
`e_phoff + e_phnum * e_phentsize <= len` (checked arithmetic), at least one
`PT_LOAD` (`p_type == 1`), `p_filesz <= p_memsz`, `p_offset + p_filesz <=
len`, `p_align` zero or a power of two and `p_vaddr % p_align == p_offset %
p_align` when non-zero, segment inside the constraints, no two load
segments overlap in memory, never both `PF_W` (2) and `PF_X` (1), entry
inside an executable segment. Segments are returned sorted by `vaddr`.

Tests: catalog 6.6.13 ELF items. `strategies.rs`, behind
`test-strategies`, holds `ProgramHeader` and `ElfBuilder`, which assemble
an image byte by byte with every field open to a test, `any_elf_image()`
for well-formed images, and `any_elf_bytes()`, which replaces up to eight
bytes of one and sometimes truncates it. The builder lives there rather
than in the tests so that the unit tests, the property tests, and the fuzz
corpus of Phase 7 share one description of a well-formed file.

### 10.2.2 Crate `audhsos-uefi` (`crates/uefi`)

Layer 0, `no_std`, `forbid(unsafe_code)`. Only `#[repr(C)]` structures,
function-pointer types, constants, and pure helpers. No calls.

Define, with the field order of the UEFI 2.10 specification (consult the
specification for every offset and write a layout test for each structure):
`Status` (`usize`; `SUCCESS = 0`; error bit `1 << 63`), `Handle`
(`*mut c_void`), `Guid { data1: u32, data2: u16, data3: u16, data4: [u8; 8] }`,
`TableHeader` (24 bytes), `SystemTable`, `BootServices` (all 44 function
slots in specification order, typed as `unsafe extern "efiapi" fn`
pointers with the exact signatures for the eleven services the loader
calls, `LocateProtocol` included, and as `usize` for the rest),
`ConfigurationTable`, `MemoryDescriptor`
(40 bytes: `type_: u32`, `physical_start: u64`, `virtual_start: u64`,
`pages: u64`, `attribute: u64`), `MemoryType` (`from_u32` with the 16
standard values), `AllocateType`, `LoadedImageProtocol`,
`SimpleFileSystemProtocol`, `FileProtocol`, `FileInfo`,
`SimpleTextOutputProtocol`, `GraphicsOutputProtocol` (`query_mode`,
`set_mode`, `blt` as `usize`; `mode: *const GraphicsOutputProtocolMode`),
`GraphicsOutputProtocolMode { max_mode: u32, mode: u32, info: *const
GraphicsOutputModeInformation, size_of_info: usize, frame_buffer_base:
u64, frame_buffer_size: usize }`, `GraphicsOutputModeInformation {
version: u32, horizontal_resolution: u32, vertical_resolution: u32,
pixel_format: u32, pixel_information: [u32; 4], pixels_per_scan_line:
u32 }`, `GraphicsPixelFormat::from_u32` with the four specification
values, and the GUIDs `ACPI_20_TABLE`, `LOADED_IMAGE_PROTOCOL`,
`SIMPLE_FILE_SYSTEM_PROTOCOL`, `FILE_INFO`, `GRAPHICS_OUTPUT_PROTOCOL`
(`9042A9DE-23DC-4A38-96FB-7ADED080516A`).

Pure helpers: `memory_map::descriptors(buffer: &[u8], descriptor_size:
usize) -> impl Iterator<Item = MemoryDescriptor>` (honors the stride),
`memory_map::to_boot_regions(iter, out: &mut [BootRegion; MAX_BOOT_REGIONS])
-> Result<usize, ConversionError>` with the type mapping: `ConventionalMemory`,
`LoaderCode`, `LoaderData`, `BootServicesCode`, `BootServicesData` →
`Usable`; `AcpiReclaimMemory` → `AcpiReclaimable`; `AcpiMemoryNvs` →
`AcpiNvs`; `MemoryMappedIo`, `MemoryMappedIoPortSpace` → `MmioReserved`;
everything else → `Reserved`; merge adjacent regions of equal kind; error
if more than `MAX_BOOT_REGIONS` remain. `utf16::encode(ascii: &str, out:
&mut [u16]) -> Result<&[u16], _>` for file names.
`graphics::to_framebuffer(base: u64, size: usize, info:
&GraphicsOutputModeInformation) -> Option<Framebuffer>`: pixel formats
`0` and `1` map to `Rgbx8888` and `Bgrx8888`; the other formats, a zero
base, a zero resolution, or a `size` too small for `height * stride * 4`
yield `None`; the length is `size` rounded up to a frame multiple.

Modules: `status.rs`, `types.rs`, `tables.rs`, `protocols.rs`,
`memory_map.rs`, `graphics.rs`, `utf16.rs`. `Status` is
`repr(transparent)` over `usize` with the named codes the loader meets and
`ok(value)`, which turns a status into a `Result`. The services and
protocol entry points the loader calls are typed `unsafe extern "efiapi"
fn` pointers; every other slot is a `usize`, so that the tables keep their
size and every offset stays right. An `unsafe` function pointer type is
not an unsafe site: it says that calling the pointer is unsafe, and the
loader pays for that. `memory_map::descriptors` yields nothing when the
reported stride is below the structure, because such a buffer cannot hold
descriptors at all; `MemoryDescriptor::decode` reads one descriptor out of
bytes, since turning bytes into a `&[MemoryDescriptor]` would need
`unsafe`. In `to_boot_regions` the limit applies before merging, so that
the array never overflows, and regions of different kinds may touch: only
shared bytes are `ConversionError::Overlap`. `graphics::framebuffer_format`
is the pixel-format mapping on its own, so that a test names it directly.

Tests: catalog 6.6.14 structure and conversion items, 6.6.24 layout and
conversion items.

### 10.2.3 Crate `driver-uart16550` (`crates/drivers/uart16550`)

Layer 1, `no_std`, `forbid(unsafe_code)`, no deps.

```rust
#[derive(Clone, Copy)] pub enum Register { Data = 0, InterruptEnable = 1, FifoControl = 2, LineControl = 3, ModemControl = 4, LineStatus = 5, ModemStatus = 6, Scratch = 7 }
pub trait Registers { fn read(&mut self, register: Register) -> u8; fn write(&mut self, register: Register, value: u8); }
pub struct Uart16550<R: Registers> { registers: R }
pub enum UartError { Timeout, WouldBlock }
impl<R: Registers> Uart16550<R> {
    pub fn init(registers: R) -> Self;   // IER=0; LCR=0x80; Data=0x01 (divisor low), IER=0x00 (divisor high): 115200 baud; LCR=0x03; FCR=0xC7; MCR=0x0B
    pub fn write_byte(&mut self, byte: u8) -> Result<(), UartError>;  // poll LineStatus bit 5 at most POLL_LIMIT times
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), UartError>;
    pub fn read_byte(&mut self) -> Result<u8, UartError>;             // LineStatus bit 0 else WouldBlock
    pub fn enable_receive_interrupt(&mut self);                       // IER = 0x01
    pub fn interrupt_pending(&mut self) -> bool;                      // InterruptId bit 0 clear
}
pub const POLL_LIMIT: u32 = 100_000;
```

`Uart16550::new` takes the register block over without touching it, for a
controller the firmware already programmed; `into_registers` gives it back.
`enable_interrupts(receive, transmit)` and `disable_interrupts` cover the
sources the enable register carries, and `Register::{ALL, offset, index}`
lets a double index a register file. Every bit the crate writes or tests
has a named constant, so that no test repeats a magic number the product
code computes.

`doubles.rs`, behind `#[cfg(any(test, feature = "test-doubles"))]`, holds
`RecordingRegisters`: a register file that answers a read from a script
first, then from the last written value, and records every access, so that
a test can make the transmitter ready after a given number of polls.

Tests: catalog 6.6.17.

### 10.2.4 Crate `kernel-hal-x86_64` (`crates/kernel/hal-x86_64`)

Adapter crate, target `x86_64-unknown-none`, `#![no_std]`,
`#![allow(unsafe_code)]`, `#![feature(abi_x86_interrupt)]`. Deps:
`kernel-hal-api`, `kernel-types`, `audhsos-abi`, `driver-uart16550`,
`audhsos-sync`. Features `debug-uart`, `test-exit`, `port-io`.

Modules and their `unsafe`/`asm!` content:

- `instructions.rs`: one `asm!` per wrapper: `cli`, `sti`, `hlt`,
  `read_cr2`, `read_cr3`, `write_cr3`, `invlpg`, `lgdt`, `lidt`, `ltr`,
  `rdmsr`, `wrmsr`, `inb/inw/inl`, `outb/outw/outl`, `read_rflags`
  (`pushfq; pop`). Each is a safe-looking `pub fn` only where the operation
  is harmless (`read_rflags`, `hlt`); the rest are `pub unsafe fn` with
  `# Safety` docs.
- The descriptor encodings are not in the adapter at all: they are the
  logic crate `kernel-x86-tables` (`crates/kernel/x86-tables`, layer 1,
  deps none), so that they are tested on the host instead of only in QEMU.
  Modules `gdt.rs` (the four segment descriptors, `Selector` with index and
  requested privilege level, the five named selectors, `tss_descriptor`
  with its two decoders, and `build_gdt`), `idt.rs` (`gate` with the four
  decoders `gate_handler`, `gate_selector`, `gate_ist`, and
  `gate_attributes`, plus `gate_present` and `gate_privilege`), and
  `tss.rs` (`TaskStateSegment` with `with_kernel_stack`,
  `with_interrupt_stack`, and `to_bytes`). The adapter depends on the
  crate and only loads what it returns. Encodings: GDT `0x00AF9A000000FFFF` kernel code,
  `0x00CF92000000FFFF` kernel data, `0x00CFF2000000FFFF` user data,
  `0x00AFFA000000FFFF` user code, 16-byte system descriptor type `0x9` for
  the TSS with base split into bits 16..39, 56..63, and the high 32 bits in
  the second quadword. Selectors: kernel code `0x08`, kernel data `0x10`,
  user data `0x18 | 3`, user code `0x20 | 3`, TSS `0x28`. IDT entry: offset
  bits 0..15, selector, IST index in bits 0..2 of byte 4, type byte `0x8E`
  (interrupt gate, DPL 0) or `0xEE` (DPL 3), offset bits 16..31, offset bits
  32..63, reserved. TSS: 104 bytes, `rsp0` at offset 4, `ist1` at offset 36,
  `iomap_base` at offset 102 set to 104.
- `gdt.rs`, `idt.rs`, `tss.rs`: static tables in `Global<T>` cells or
  `static` with interior mutability through `audhsos-sync`; loading with
  `lgdt`/`ltr`/`lidt`; reloading segment registers after `lgdt` (one `asm!`
  with `mov` to `ds/es/ss` and a far return for `cs`).
- `traps.rs`: `extern "x86-interrupt" fn` handlers for vectors 0..=31 with
  error codes where the CPU pushes them (8, 10, 11, 12, 13, 14, 17, 21, 29,
  30), double fault on IST 1 with a dedicated 16 KiB stack, and one
  generic handler per device vector. Handlers call
  `kernel_core::trap::on_exception(Exception { vector, error_code, ip, sp,
  cr2 })` through a function pointer registered at init; in Phase 2 the
  core prints the exception and exits with failure.
- `bootinfo.rs`: `Platform` adapter: `unsafe fn from_pointer(ptr: *const
  u8) -> Result<X86Platform, BootInfoError>` reads one page as `&[u8;
  4096]` (one `unsafe` block, precondition: the loader mapped the page
  read-only at that address) and parses it with `BootInfoView::parse`.
  Converts `BootRegion` to `MemoryRegion` and appends the four fixed ranges
  with kinds `Kernel`, `BootImage`, `PageTables`, `BootStack`, plus the boot
  information page as `BootInfo`.
- `console.rs` (feature `debug-uart`): `PortRegisters { base: u16 }`
  implementing `driver_uart16550::Registers` over `inb`/`outb`;
  `SerialConsole` implementing `DebugConsole`.
- `exit.rs` (feature `test-exit`): `QemuExit` implementing `TestExit` by
  `outl(0xF4, 0x10)` for success and `0x11` for failure, followed by `hlt`
  in a loop.
- `qemu_coverage.rs`: `pub const QEMU_TESTS: &[(&str, &str)]` mapping every
  public function of the crate to a QEMU test name; `check-layering`
  verifies both sides exist (function by text search in the crate, test by
  text search under `crates/kernel/bin/tests`).

Budget after the phase: set `unsafe_budget` and `asm_budget` to the
measured counts.

### 10.2.5 Crate `kernel-test-harness` (`crates/kernel/test-harness`)

Target crate, `no_std`, `forbid(unsafe_code)`, deps `kernel-hal-api`.

```rust
pub trait Testable { fn run(&self); fn name(&self) -> &'static str; }
impl<F: Fn()> Testable for F { /* name = core::any::type_name::<F>() */ }
pub struct Harness<C: DebugConsole, E: TestExit> { console: C, exit: E, should_panic: bool, passed: u32, running: bool }
impl Harness {
    pub const fn new(console: C, exit: E) -> Self;
    pub const fn expecting_panic(console: C, exit: E) -> Self;
    pub fn run(&mut self, tests: &[&dyn Testable]);
    pub fn begin(&mut self, name: &str); pub fn end(&mut self);
    pub fn fail(&mut self, message: fmt::Arguments<'_>);
    pub fn fail_at(&mut self, message: fmt::Arguments<'_>, location: Option<&Location<'_>>);
    pub fn on_panic(&mut self, info: &PanicInfo<'_>);
}
```

The runner returns instead of diverging, and the image halts after it; a
`-> !` would need a loop that no host test could leave, and the whole point
of the crate is that the protocol is checked on the host. For the same
reason the crate is built and tested for the host, not only for
`x86_64-unknown-none`: it holds no hardware access. `begin` and `end` open
and close a line for an image that runs its tests itself; `fail_at` is what
`on_panic` calls, so that the reporting path is testable without a
`PanicInfo`, which a test cannot build. The `x86_64` implementations of
`DebugConsole` and `TestExit` are what makes the difference between a host
run and a QEMU run.

Output lines exactly as in
[03-target-platform.md 3.1.7](03-target-platform.md#317-test-exit-protocol):
`[test] <crate>::<name> ... ok` per test, `[summary] passed=<n> failed=<m>`,
then `exit(Success)`. A panic inside a test prints `[test] ... FAILED:
<message>`, then the summary with `failed=1`, then `exit(Failure)`. When
`should_panic` is set (by the test kernel before running), `on_panic`
prints `ok` and the summary and exits with success; reaching the end of a
`should_panic` kernel without a panic is a failure.

### 10.2.6 Crate `kernel-core` (`crates/kernel/core`)

Layer 4, `no_std`, `forbid(unsafe_code)`. Phase 2 content:

- `print`: a `core::fmt::Write` over a `DebugConsole` and the `println!`
  macro the kernel writes with. A formatting error is dropped: a console
  that cannot take the bytes must not stop the kernel.
- `boot::run(platform, console) -> Result<(), BootError>`: prints the
  banner, the physical window, the ACPI pointer, and every memory region,
  then checks what the kernel cannot run without. A window somewhere else
  than `PHYS_WINDOW_BASE` and a machine without usable memory are errors;
  `boot::abort` reports one and exits with a failure, `boot::finish`
  reports the end of the boot and exits with a success. `run` returns
  rather than diverging, so that the whole sequence runs in a host test
  against the recording doubles.
- `trap::Exception { vector, error_code, ip, sp, cr2 }` with `name` and
  `has_error_code`, and `trap::on_exception`, which reports the exception,
  counts it in the kernel state, and exits with a failure. The error code
  is reported only for the vectors that push one, the faulting address
  only for a page fault.
- `state::KERNEL`, the `Global<KernelState>` cell. Phase 2 keeps the number
  of reported traps and nothing else.

### 10.2.7 Crate `audhsos-kernel` (`crates/kernel/bin`)

Target crate, `#![no_std]`, `#![no_main]`, `forbid(unsafe_code)` except
for the two attributes the entry point needs: `#[unsafe(no_mangle)]
pub extern "C" fn kernel_entry(boot_info: *const u8) -> !` (an unsafe
attribute counts as one `unsafe` site; make the binary an adapter crate
with budget 1 and `asm_budget` 0, or move the entry function into
`kernel-hal-x86_64` and keep the binary a logic crate: choose the second).
The binary: calls `kernel_hal_x86_64::entry::start(boot_info)` which
validates the boot information, initializes GDT/TSS/IDT, the debug console,
and hands over to `kernel_core::boot::run`. `#[panic_handler]` prints the
message through the console and exits with failure.

`build.rs` emits `cargo:rustc-link-arg-bins=-T<absolute path to
crates/kernel/bin/kernel.ld>` and `cargo:rerun-if-changed=kernel.ld`. The
linker script: `ENTRY(kernel_entry)`, `. = KERNEL_BASE` (write the constant
`0xFFFFFFFF80000000` in the script and add a test in `xtask` that it equals
`audhsos_abi::layout::KERNEL_BASE`), sections `.text` (R X), `.rodata`
(R), `.data` and `.bss` (RW), each starting on a 4 KiB boundary
(`ALIGN(4096)`), `.eh_frame` discarded. `.cargo/config.toml` gets
`[target.x86_64-unknown-none] rustflags = ["-C",
"relocation-model=static"]`; the runner is not written there but passed
through the environment by `test --qemu` (see 10.2.9).

Test kernels under `crates/kernel/bin/tests/`: each file uses
`#![feature(custom_test_frameworks)]`,
`#![test_runner(kernel_hal_x86_64::testing::run_tests)]`,
`#![reexport_test_harness_main = "test_main"]`, the wiring macro
`kernel_hal_x86_64::test_kernel!()`, and `#[test_case]` functions. Each
file needs a `[[test]]` entry in the manifest and `use audhsos_abi as _;
use kernel_core as _;`, because a test image uses neither crate while the
kernel image uses both. `build.rs` emits `cargo:rustc-link-arg-tests` next
to `cargo:rustc-link-arg-bins`, so that the test images get the same
linker script.

The wiring lives in `kernel-hal-x86_64::testing`, not in
`kernel-test-harness`: the entry point needs `#[unsafe(no_mangle)]`, which
a logic crate with `#![forbid(unsafe_code)]` may not carry. That module
holds the harness over the debug console and the exit device, the trap
hook a test image registers, and the instructions that raise the
exceptions the trap tests expect (`int3`, `ud2`, `div` by zero, a segment
selector beyond the descriptor table, a read from an unmapped address).
The line of a test is written when the test is over, so that a test may
write on the console without breaking the protocol; the name of the
running test is kept in a cell and is what a panic or a trap reports.

Phase 2 kernels: `boot.rs` (reaches the harness, the loader reported
memory), `console.rs` (writes a marker string the runner asserts),
`descriptors.rs` (a second load of the tables is refused),
`breakpoint.rs` (the handler sees vector 3 and the test goes on),
`divide_error.rs`, `invalid_opcode.rs`, `general_protection.rs`,
`page_fault.rs` (the fault at `0xdead_beef` reports that address through
the trap hook, which then ends the machine, because a fault cannot be
resumed), `double_fault.rs` (infinite recursion overflows the kernel
stack; the handler runs on IST 1 and reports vector eight, which it could
not do without its own stack), and `panic.rs` (`should_panic`: a panic
reaches the panic handler, which names the running test). The three loader
failure images are built by the xtask, not checked in.

`BOOT_STACK_PAGES` is 64, not 16: the unoptimized build of a test image
needs well over 64 KiB of stack before it reaches the harness, and the
sixteen-page stack ran into its guard page inside `descriptors::install`.

### 10.2.8 Crate `boot-uefi-x86_64` (`crates/boot/uefi-x86_64`)

Adapter crate, target `x86_64-unknown-uefi`, `#![no_std]`, `#![no_main]`,
`#![allow(unsafe_code)]`, `panic = "abort"` (the target's own strategy), no
`alloc`. Deps: `audhsos-abi`, `audhsos-elf`, `audhsos-uefi`,
`kernel-types`, `kernel-mm`, `kernel-hal-api`.

Entry: `#[unsafe(no_mangle)] pub extern "efiapi" fn efi_main(image: Handle,
system_table: *const SystemTable) -> Status`. Modules:

- `firmware.rs`: `Firmware<'a> { image: Handle, table: &'a SystemTable,
  boot: &'a BootServices, console: Option<&'a SimpleTextOutputProtocol> }`,
  built by an `unsafe fn new` that checks both table signatures, with one
  method per service, each containing exactly one `unsafe` block that
  calls the function pointer: `allocate_pages(count) ->
  Result<PhysFrameRange, Status>`, `free_pages`, `memory_map(buffer: &mut
  [u8]) -> Result<MemoryMapInfo { size, key, descriptor_size }, Status>`,
  `handle_protocol<T>(handle, guid) -> Result<&T, Status>`,
  `locate_protocol<T>(guid) -> Result<&T, Status>`,
  `exit_boot_services(key)`, `output_string(&str)` (encoded through
  `audhsos_uefi::utf16`), `configuration_table(guid) -> Option<u64>`. The
  memory map buffer starts at 16 firmware-allocated pages and is doubled
  up to 256 pages while the firmware reports `BUFFER_TOO_SMALL`.
- `memory.rs`: the one conversion of a physical range into a byte slice,
  `unsafe fn bytes_mut(range, len) -> Option<&mut [u8]>` with a single
  `unsafe` block; precondition: the range is a firmware allocation, nobody
  else borrows it, and the identity mapping is active. Files, the kernel
  image, the memory map buffer, and the boot information page go through
  it, so that no other module builds a pointer from an address.
- `graphics.rs`: `locate_protocol::<GraphicsOutputProtocol>` with
  `GRAPHICS_OUTPUT_PROTOCOL`; two `unsafe` blocks turn the `mode` and
  `info` pointers into references (precondition: the firmware owns both
  while boot services run, and the loader reads them before
  `exit_boot_services`); the result goes through
  `audhsos_uefi::graphics::to_framebuffer`. A missing protocol or an
  unsupported format yields `None` and the loader continues. The loader
  never writes to the framebuffer.
- `files.rs`: open the loaded image's device (`LoadedImageProtocol::device_handle`
  → `SimpleFileSystemProtocol::open_volume`), `read_file(name: &str) ->
  Result<LoadedFile { frames, len }, LoadError>`: `open` with mode read,
  `get_info` with the `FILE_INFO` GUID into a stack buffer to learn the
  size, allocate pages, `read` until the size is reached, `close`. A
  protocol pointer is derived from the reference the firmware reported
  (`ptr::from_ref(..).cast_mut()`), so that each call is one `unsafe`
  block and no module keeps a raw pointer.
- `placement.rs` (pure where possible): from the parsed kernel ELF compute
  the page-aligned span of all segments, allocate that span as **one**
  frame range (the boot information reports the kernel image as a single
  physical range), zero it, copy `file_size` bytes of each segment to its
  offset in the span, and record `(PageRange, PhysFrameRange,
  Permissions)` per segment. A segment whose `vaddr` is not page aligned
  is rejected, because two segments would then share a page.
- `paging.rs`: `IdentityAccess` implementing `FrameAccess<PageTable<X86Entry>>`
  by casting the frame address to `&mut PageTable` (one `unsafe` block per
  direction; precondition: the firmware identity-maps all memory while
  boot services run and the loader keeps that mapping), `PoolFrames`
  implementing `FrameSource` over one contiguous firmware allocation (so
  that `page_tables_phys_start` and `page_tables_phys_len` describe one
  range), `NoTlb` implementing `TlbControl` as a no-op, and `pool_frames`,
  which sizes that allocation from the amount of memory the map reports.
  Build the tables with `kernel_mm::Mapper`: (1) the physical window: `0`
  up to the first byte above the highest region the memory map reports as
  memory (`Usable`, `AcpiReclaimable`, `AcpiNvs`), mapped at
  `PHYS_WINDOW_BASE + phys`, read/write, no-execute, global. Device
  apertures are left out: the map of the reference machine reaches to a
  terabyte, and the window covers memory, not the address space, so that
  the kernel finds every frame it may own behind one constant offset.
  (2) an identity mapping of the same memory at `phys` (so that the loader
  keeps running after the `CR3` switch), read/write/execute; (3) the kernel
  segments at their `vaddr` with their permissions, global; (4) the boot
  stack (`BOOT_STACK_PAGES` pages plus one unmapped guard page below) with
  its top at `BOOT_STACK_TOP` (`KERNEL_BASE - 0x100_0000`, a constant in
  `audhsos-abi`), read/write, no-execute; (5) the boot information page at
  `BOOT_INFO_VADDR` (constant in `audhsos-abi`), read-only.
- `bootinfo.rs`: fill a page with `BootInfoWriter` after
  `exit_boot_services`; the memory map used is the final one obtained
  immediately before the call; loader code and data are reported as
  `Usable`; the framebuffer, if present, is written into the header and
  its range is inserted, sorted by start address, as one `MmioReserved`
  region unless the final memory map already covers it with an
  `MmioReserved` region. A framebuffer that overlaps a `Usable` region, or
  that no longer fits into the region array, is dropped instead of being
  reported, so that the loader never writes a structure the kernel's own
  parser rejects. An ACPI root pointer that lies outside every region is
  reported as absent for the same reason.
- `loader.rs`: the order of the work. Everything the firmware has to
  supply is asked for before the boot services end: the two files, the
  configuration table, the first memory map (for the extent of the window
  and the size of the table pool), the kernel image frames, the boot stack, the boot information
  page, the table pool, the tables themselves, and the graphics mode. Then
  the memory map is read once more and `ExitBootServices` is called with
  its key, retried up to three times with a fresh key. A failure after
  that call can only end the machine, because nothing is left to report
  through.
- `entry.rs`: one naked function `enter_kernel(cr3: u64, stack_top: u64,
  boot_info: u64, entry: u64) -> !`: `cli; mov cr3, rdi; mov rsp, rsi; mov
  rdi, rdx; jmp rcx`. The function is `extern "sysv64"`, not `extern "C"`:
  on the UEFI target `extern "C"` is the Microsoft ABI, and the kernel
  entry point on `x86_64-unknown-none` takes its argument in `RDI`.
- `exit.rs`: on any error, `output_string` a diagnostic built in a
  fixed-size `fmt::Write` buffer and `outl(0xF4, 0x12)` (one `asm!`), then
  spin forever. The same buffer writes the one progress line the loader
  emits before it builds the tables: the amount of memory and the kernel
  entry point.

The loader does not use the physical memory window or any kernel address
before the jump; it runs on firmware-provided identity mappings and its own
identity mapping afterwards.

### 10.2.9 `xtask` additions

- `policy::Crate` gets `target: Target` with `Target::{Host, X86_64None,
  X86_64Uefi}`; host commands (`test --host`, `coverage`, `miri`) use
  `--exclude` for non-host crates; `lint` runs clippy per target group with
  `--target`.
- `build`: `cargo build -p boot-uefi-x86_64 --target x86_64-unknown-uefi`
  and `cargo build -p audhsos-kernel --target x86_64-unknown-none`
  (`--release` passes through).
- `image::boot_image`: header (`BootImageHeader::to_bytes`) + root task +
  ustar archive (empty in Phase 2: a header with zero-length root task is
  rejected by the kernel, so Phase 2 ships a placeholder root task of one
  page of `0xF4` bytes (`hlt`) that the kernel does not start yet).
- `image::disk`: `crc32.rs` (IEEE, reflected polynomial `0xEDB88320`, table
  of 256 entries computed at first use), `gpt.rs` (protective MBR at LBA 0:
  entry type `0xEE`, start LBA 1, size `min(total - 1, 0xFFFF_FFFF)`,
  signature `0x55AA`; primary header at LBA 1: signature `EFI PART`,
  revision `0x0001_0000`, header size 92, CRC over the 92 bytes with the
  CRC field zeroed, current LBA 1, backup LBA `last`, first usable 34, last
  usable `last - 34`, fixed disk GUID, partition entry LBA 2, 128 entries
  of 128 bytes, entry-array CRC; the array at LBA 2..=33; one entry: type
  GUID `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`, fixed unique GUID, first LBA
  2048, last LBA `last - 34`, attributes 0, name `AUDHSOS ESP` in UTF-16LE;
  backup array at `last - 32 ..= last - 1`, backup header at `last` with
  swapped current and backup LBAs, so that the last usable sector is
  `last - 33`), `fat32.rs` (512-byte sectors, 1 sector per
  cluster, 32 reserved sectors, 2 FATs, FSInfo at sector 1, backup boot
  sector at 6, root directory cluster 2, media `0xF8`, end-of-chain
  `0x0FFF_FFFF`, cluster count at least 65525, deterministic timestamps
  `2026-01-01 00:00:00`, 8.3 names uppercase, directories `EFI`, `EFI/BOOT`,
  `AUDHSOS` with `.` and `..` entries), plus a reader used only by the
  tests, behind `#[cfg(test)]`, so that the product only writes. Default
  image size 64 MiB, grown in whole mebibytes when the files need more.
  `xtask` gains `audhsos-abi` and `kernel-test-harness` as its two
  dependencies, so that the boot image header, the layout constants, and
  the serial protocol grammar are written down once; the policy table
  records both.
- `image [--release]`: writes `target/boot.img` and `target/audhsos.img`
  from the built loader and kernel.
- `qemu.rs`: locate `qemu-system-x86_64` (`AUDHSOS_QEMU` or `PATH`) and the
  firmware (`AUDHSOS_OVMF` or `<qemu dir>/../share/qemu/edk2-x86_64-code.fd`);
  the command line of
  [03-target-platform.md 3.1.1](03-target-platform.md#311-reference-machine-configuration)
  with `-serial stdio -display none -no-reboot`; timeout 60 s
  (`AUDHSOS_QEMU_TIMEOUT`) implemented with a thread that kills the child;
  capture stdout; parse the protocol; map exit status 33/35/37; print the
  captured output on failure.
- `qemu-runner <elf>`: write a disk image with the given kernel ELF, the
  loader `build` left in `target/`, and the boot image, run QEMU, and exit
  0 on success. The runner starts no Cargo of its own: it runs inside
  `cargo test`, which holds the lock on the build directory.
- `test --qemu`: `build`, then `cargo test -p audhsos-kernel --target
  x86_64-unknown-none` (Cargo invokes the runner for every test kernel),
  then the three loader failure images. The runner is passed through the
  environment variable `CARGO_TARGET_X86_64_UNKNOWN_NONE_RUNNER`, set to
  the path of the running xtask binary, and the workspace root through
  `AUDHSOS_ROOT`; `.cargo/config.toml` carries no `runner`, because a
  runner that started `cargo run -p xtask` would wait for the build
  directory lock the enclosing `cargo test` holds. The three loader images
  are a kernel file whose ELF magic is broken, a volume without
  `AUDHSOS/KERNEL.ELF`, and a volume without `AUDHSOS/BOOT.IMG`; each has
  to end with the loader failure status and a `[loader] ` diagnostic. A
  table in `commands.rs` names the marker a test image has to write beyond
  the protocol, so that the console image proves the debug UART carries
  more than the protocol lines.
- `run [--release] [--display]`: `build`, `image`, then QEMU without a
  time limit and with the streams attached to the terminal; `--display`
  replaces `-display none` with the platform's backend.
- Tests: catalog 6.6.15 and the runner items of 6.6.20.

### 10.2.10 Policy and documents

`policy::CRATES` entries: `audhsos-elf` (Logic, deps `test-support` behind `test-strategies`),
`audhsos-uefi` (Logic, deps `audhsos-abi`), `kernel-x86-tables` (Logic,
deps none), `driver-uart16550` (Logic, deps none), `kernel-core` (Logic,
deps `kernel-types`, `kernel-hal-api`, `kernel-mm`, `kernel-objects`,
`audhsos-abi`, `audhsos-sync`), `kernel-hal-x86_64` (Adapter,
`X86_64None`, deps `kernel-hal-api`, `kernel-types`, `audhsos-abi`,
`driver-uart16550`, `audhsos-sync`, `kernel-x86-tables`, `kernel-mm`),
`kernel-test-harness` (Logic, `Host`, deps `kernel-hal-api`: the runner is
pure and is tested on the host, so the kernel images link it but the
coverage gate still applies), `audhsos-kernel` (Adapter, `X86_64None`,
deps `kernel-core`, `kernel-hal-x86_64`, `audhsos-abi`; the budget of
three counts the `#[unsafe(no_mangle)]` attribute of the entry point, the
call into the adapter, and the one call a test image makes),
`boot-uefi-x86_64` (Adapter, `X86_64Uefi`, deps as in 10.2.8; the budget
counts the three graphics blocks of `firmware.rs` and `graphics.rs`).
`xtask` (Host, deps `audhsos-abi`, `kernel-test-harness`).
Update the catalog in 05, the allowlist and inventory in 04,
`rust-toolchain.toml` already lists both targets. Record `BOOT_STACK_TOP`,
`BOOT_STACK_PAGES`, and `BOOT_INFO_VADDR` in the kernel address space
table of 02 and in 03.

### 10.2.11 Acceptance

`sh tools/xtask-check.sh` (now including `test --qemu`) passes; at least
eight test kernels and three loader images run; `sh tools/xtask.sh run`
shows the banner on the terminal.

## 10.3 Phase 3: Kernel memory bring-up

Goal: the kernel owns its memory after boot. Implemented.

- `audhsos-abi::layout`: `KERNEL_STACKS_BASE`, `KERNEL_STACK_PAGES` (4),
  `KERNEL_STACK_SLOT_PAGES` (5, the guard page included),
  `KERNEL_STACK_SLOTS` (1024), and `MAX_PHYS_WINDOW_BYTES`, the memory the
  window covers before it would reach the stack area.
- `kernel-hal-x86_64::window`: `PhysicalWindow { base: VirtAddr }` with
  `frame_bytes_mut(&mut self, frame) -> Option<&mut [u8; 4096]>` and
  `impl<T> FrameAccess<T>`, three `unsafe` blocks, one per reference it
  hands out; the constructor carries the precondition that the window maps
  all of memory read and write and that the kernel is the only writer.
  `kernel-hal-x86_64::paging` keeps `LocalTlb` (`invlpg` per page,
  `write_cr3(read_cr3())` for `flush_all`) and adds `active_root()`, the
  frame `CR3` names, and `activate(root)`, which writes `CR3`. It
  re-exports `X86Entry`, so that a caller of the mapper need not name
  `kernel-mm` for the entry format.
- `kernel-hal-x86_64::memory`: `translate(address)`, the walk of the
  active tables through the window with `NoFrames` as the frame source;
  `register_boot_info(platform)`, which appends the boot information page
  as a region with the physical address that walk gives; and
  `read_frame(address)`, a copy of one frame out of the window, which the
  kernel uses to read the boot image header.
- `kernel-mm::frame_allocator`: `NoFrames`, a frame source for a walk that
  creates no table.
- `kernel-mm::stack`: `KernelStack { pages, index }` with `top` and
  `guard`, `slot_pages(index, slots)` for the geometry, and
  `StackPool<N>`, a bitmap of `N * 64` slots. `allocate(mapper)` takes the
  lowest free slot and maps `KERNEL_STACK_PAGES` frames above its guard
  page; an allocation that runs out of frames leaves no slot taken and no
  page mapped. `release(mapper, stack)` unmaps the pages, gives the frames
  back, and frees the slot. `Mapper::frames_mut` hands the frame source
  out for it.
- `kernel-core::config`: `PROCESSES = 256`, `THREADS = 1024`,
  `MEMORY_OBJECTS = 4096`, `ENDPOINTS = 1024`, `NOTIFICATIONS = 1024`,
  `REPLIES = 1024`, `INTERRUPTS = 64`, `IO_PORT_RANGES = 64`,
  `KERNEL_STACKS = 1024`, `REGIONS_PER_PROCESS = 64`,
  `HANDLES_PER_PROCESS = 1 << 16`, plus `KERNEL_REGIONS = 64` and
  `KERNEL_IMAGE_MAX_PAGES = 8192`, how far above `KERNEL_BASE` the walk
  looks for mapped pages. The pools of the object types are `static`
  `Global<Pool<..>>` cells in Phase 5, with the types they hold; Phase 3
  builds the cells the memory bring-up fills.
- `kernel-core::memory`: the bring-up, over the HAL traits and therefore
  the same code on the host and in QEMU.
  - `reserve(platform, override_bytes)`: `normalize`, then
    `select_reserve`, then `BitmapFrameAllocator::new`; it also reports the
    number of pages the window maps and refuses a machine with more memory
    than `MAX_PHYS_WINDOW_BYTES`.
  - `requested_reserve(platform, header)` reads `kernel_reserve_size` out
    of the boot image header the kernel copied through the window; a
    header that does not parse asks for the default.
  - `adopt(mapper, window_pages)` walks the four fixed ranges of the
    kernel half with `Mapper::translate` and registers every maximal run
    of mapped pages in a `RegionTable<KernelBacking, KERNEL_REGIONS>`:
    `Image`, `Window`, `BootStack`, `BootInfo`. It also returns the frame
    the boot information page is mapped to.
  - `drop_identity(mapper, pages)` removes every translation below
    `USER_SPACE_END` that exists, in steps of `MAX_PAGES_PER_CALL` pages,
    and gives no frame back: those frames are the memory the window maps.
    Table frames the mapper collects go to the allocator, which refuses
    them because they lie outside the reserve.
  - `bring_up` composes the three and returns `KernelMemory` with the
    allocator, the free memory, the region table, the kernel stack pool,
    the root frame, and the boot information frame; `initialize` stores it
    in the `MEMORY` cell, `with_memory` borrows it, `report` writes it.
- `kernel-hal-x86_64::bootinfo`: `X86Platform::from_page` pushed the
  address of the page it read as the `MemoryRegionKind::BootInfo` region.
  That address is `BOOT_INFO_VADDR`, a virtual one, so `PhysAddr::new`
  rejected it and the region was silently dropped: no boot report ever
  showed a `boot-info` line. Phase 3 removes that push and adds
  `push_boot_info(start)`, which the entry point calls with the frame the
  walk of the loader's tables gives, before the report runs.
- `KernelMemory::allocate_stack` and `release_stack` build the mapper out
  of the root frame and the reserve and drive the pool. A kernel stack is
  not a row of the region table: the area is one constant range, the pool
  bitmap says which of its 1024 slots are taken, and the table has 64 rows.
- QEMU tests: `memory.rs` (the reserve leaves every frame free; every
  reserve frame is handed out and taken back; the identity mapping is
  gone; the boot information page is reported with the physical address
  the walk gives; a frame the kernel maps itself carries what the window
  wrote), `memory_fault.rs` (the read after the unmap faults at the
  address of the mapping), and `kernel_stack.rs` (every page of a stack
  carries what the kernel writes; a released stack is gone and its slot
  comes back; a write to the guard page faults at the guard address). A
  fault ends the machine, so each of the two fault tests needs an image of
  its own. Host tests in `kernel-core::tests::memory` and
  `kernel-mm::tests::stack` run the same code against a page-table image
  built with the doubles; only the machine shows that a guard page
  faults.

Acceptance: `check` green; catalog 6.6.21 memory items covered.

## 10.4 Phase 4: Interrupts and timer

Phase 4 is implemented. What follows is what stands, with the deviations
from the draft of this section named where they are.

- `kernel-acpi` (`crates/kernel/acpi`, layer 1, deps `kernel-types`) holds
  every parser: `parse_rsdp(bytes: &[u8; 36]) -> Result<Rsdp, AcpiError>`
  (signature `RSD PTR `, checksum over 20 bytes, revision 0 or 2, and for
  revision 2 a length that reaches the extended checksum without leaving
  the structure and a second checksum over that many bytes);
  `SdtHeader::parse(bytes) -> Result<SdtHeader, AcpiError>` (signature,
  length at least 36 and covered by the bytes, checksum over `length`) with
  `require(signature)` and `announced_length`, which reads the length field
  and checks nothing, because an adapter has to know how much to copy
  before it can hand the whole table over; `RootTable::parse` walks an
  `RSDT` or an `XSDT`, the signature deciding whether the entries are four
  or eight bytes wide; `madt::parse(bytes) -> Result<Madt, AcpiError>` with
  `Madt { lapic_address: PhysAddr, flags: u32, processors: usize,
  io_apics: [Option<IoApic>; 4], overrides: [Option<Override>; 16] }`,
  entry types 0 (processor local APIC: counted and nothing else), 1 (I/O
  APIC), 2 (interrupt source override), 5 (local APIC address override);
  entry length zero is an error, an entry that leaves the table is an
  error, an entry of a type the parser reads with the wrong length is an
  error, unknown types are skipped by their length.
  `Madt::route_isa(irq)` answers where a line goes and how it is taken,
  which is the one piece of routing that is logic rather than register
  writes. The addresses are `PhysAddr` rather than `u64`, which is what
  makes the dependency on `kernel-types` a real one and rejects an address
  the machine cannot have.
- `kernel-x86-tables` holds the register layouts, not the adapter:
  `lapic::{ID = 0x20, VERSION = 0x30, TPR = 0x80, EOI = 0xB0, SVR = 0xF0,
  LVT_TIMER = 0x320, LVT_LINT0 = 0x350, LVT_LINT1 = 0x360, LVT_ERROR =
  0x370, TIMER_INITIAL = 0x380, TIMER_CURRENT = 0x390, TIMER_DIVIDE =
  0x3E0}` with `lvt`, `spurious`, and `count_for_rate`;
  `ioapic::{IOREGSEL = 0x00, IOWIN = 0x10, ID = 0, VERSION = 1,
  REDIRECTION_BASE = 0x10}` with the redirection entry encoding and its
  decoders; `pic::remap(master, slave)`, the ten writes that move the two
  legacy controllers and then mask every line of both; and `vectors`, the
  vector plan. All of it has host tests, which the adapter crate could not
  have. `kernel_hal_x86_64::vectors` is a re-export of that module, so the
  path the draft named still works.
- `kernel-hal-x86_64::acpi` reads the tables through the window: the RSDP,
  then the XSDT or the RSDT, then each table it names until one carries the
  `APIC` signature. No range is read before it has been checked against the
  memory the firmware reported, which is what the window maps.
  `PhysicalWindow::bytes` is the one new `unsafe` read, and it is an
  `unsafe fn` because the caller and not the window knows the bound.
- `kernel-hal-x86_64::apic`: `LocalApic` over the window with one `unsafe`
  block per volatile read and per volatile write, enabled through the
  `IA32_APIC_BASE` model-specific register and the enable bit of the
  spurious vector register with vector `0xFF`; `IoApic` over its two
  registers, where reading is also a write and therefore needs the
  exclusive borrow. `Apics` implements `InterruptController` — `route`
  writes the redirection entry masked, with polarity and trigger from the
  overrides, and refuses a line that is already routed — and `Timer`.
- Legacy PIC: remapped to `0x20..0x2F` and masked, only on a machine whose
  MADT sets the `PCAT_COMPAT` flag.
- Vector plan (`kernel_x86_tables::vectors`): exceptions `0..=31`, legacy
  controllers `0x20..=0x2F`, LAPIC timer `0x30`, I/O APIC line `gsi` at
  `0x40 + gsi` for 24 lines, system call `0x80`, LAPIC spurious `0xFF`.
  `traps::fill` installs a gate for every one of them except `0x80`, which
  needs the ring three gate of Phase 5.
- Timer: the local APIC timer measured against channel two of the interval
  timer. Divide by 16, initial count `u32::MAX`, channel two armed one-shot
  for 10 ms (gate low, command `0xB2` to `0x43`, count 11932 to `0x42` low
  then high, gate high, then poll `0x61` bit 5 with a bound), read the
  count that is left, divide, then periodic mode with the vector and
  `TIMER_INITIAL = ticks_per_ms * 1000 / TICKS_PER_SECOND`. Every poll is
  bounded, so a machine whose channel two does not run reports instead of
  hanging.
- `kernel-hal-x86_64::interrupts` is the bring-up and the cell the handler
  reaches: the register windows are mapped by a closure the caller
  supplies, because only the kernel knows its own address space, and
  `KernelMemory::map_device` is what the kernel supplies (D-60).
- `instructions::InterruptGuard` turns interrupts off for as long as a
  borrow of the controller lasts. Without it a timer interrupt that
  arrived while the kernel held the controller found the cell busy,
  returned without acknowledging, and the local APIC delivered nothing
  afterwards. The QEMU test image found this.
- `kernel-core::tick` counts the tick and gives the scheduler its turn,
  which is nothing until Phase 5. `KernelState` counts ticks, device
  interrupts, and spurious interrupts, and `with_state` hands the state out
  the way `with_memory` hands out the memory.
- There are two tick counts, not one. The draft said `Timer::ticks` counts
  in `kernel-core`, which the layering does not allow: `Timer` is
  implemented by the adapter, and the adapter may not depend on
  `kernel-core`. The adapter therefore counts what the hardware delivered,
  in an atomic the handler of the timer vector raises, and that is what
  `Timer::ticks` reads; `KernelState.ticks` counts what the kernel
  processed. In this phase the two are equal, and they are two numbers
  because they answer two questions.
- QEMU tests (`interrupts.rs`, six of them): the tables name the hardware
  and the unit is on; a second tick arrives after the end-of-interrupt; the
  tick counter grows while the kernel does nothing; a masked timer delivers
  nothing and unmasking starts it again; a vector raised from software
  reaches the handler of that vector, for `0x30`, `0x40`, and `0xFF`; a
  routed line carries the vector and the wiring the table names, comes up
  masked, follows `mask` and `unmask`, and refuses a second routing. The
  last one reads the redirection entry back out of the I/O APIC rather
  than trusting what the kernel meant to write, which is what makes it the
  test of the line half of catalog item 6.6.21; the timer test is the
  delivery half. None of them ends the machine, so one image holds all
  six; each one begins by making sure the bring-up has happened, because
  the order of the tests is not fixed.

  Raising a vector from software needs the vector as an immediate, which an
  inline constant supplies:

  ```rust
  pub fn raise_interrupt<const VECTOR: u8>() {
      unsafe {
          asm!("int {vector}", vector = const VECTOR, options(nomem, nostack));
      }
  }
  ```

  The handler of a vector raised this way must not acknowledge: the local
  APIC never delivered it, so an end-of-interrupt would acknowledge
  whatever is actually in service. The test image sets a flag around the
  instruction and its handler reads it.
- Fuzz target `madt` under `fuzz/`, with fifteen seeds under
  `fuzz/corpus/madt/`. The target reads the bytes twice: as they are, so
  that signature, length, and checksum are exercised, and once with those
  three repaired, so that the fuzzer reaches the walk over the entries
  without having to guess a checksum.

Acceptance: `check` green; catalog 6.6.11, 6.6.16 APIC items, 6.6.21
interrupt items.

## 10.5 Phase 5: Objects, threads, user mode, system calls

### 10.5.0 What the kernel reserve carries, and what the image carries

Phase 3 built the reserve and the kernel stack pool and measured what they
cost. The numbers that follow were settled before the object pools were
written, in D-57.

**The object pools live in the `.bss` of the kernel image**, not in the
reserve. A `Pool<T, N>` is `[Slot<T>; N]`, a typed array; putting one into
raw frames means a pointer cast to `&mut Pool<T, N>`, which is `unsafe` in
`kernel-objects`, a logic crate with `#![forbid(unsafe_code)]`. The loader
maps the image from its ELF and zero-fills what the file does not carry, so
a large `.bss` needs nothing new. `KERNEL_IMAGE_MAX_PAGES` (32 MiB) is the
ceiling: `kernel_core::memory::adopt` looks that far above `KERNEL_BASE`
for mapped pages of the image and registers no run beyond it.

**The reserve holds what the count of is not known before boot**: page
tables, kernel stacks, and the IPC buffer of every thread. The default
reserve is a sixteenth of the usable memory, and `BitmapFrameAllocator`
manages at most `MAX_MANAGED_FRAMES = 16384` frames, so the reserve never
exceeds 64 MiB whatever the machine has:

| Machine | Default reserve | Frames |
|---------|-----------------|--------|
| 256 MiB (the reference machine) | 15.8 MiB | 4048 |
| 512 MiB | 31.8 MiB | 8144 |
| 1 GiB and above | 64 MiB (the cap) | 16384 |

**Every thread costs nine frames of the reserve**: eight for its kernel
stack and one for its IPC buffer. It was five when this was written, and
D-73 raised the stack to eight pages after the system call tests measured
what the unoptimized build needs: twenty-three kilobytes on the deepest
path, which is `process_create`. `THREADS` and `KERNEL_STACKS` are 256, not
1024, and `KERNEL_STACK_SLOTS` in `audhsos-abi` follows them. At 1024 the
demand would be 9216 frames against the 4048 the reference machine has; at
256 it is 2304, which leaves 1744 frames — about 7 MiB — for the page
tables of the address spaces, an order of magnitude more than the handful
of processes of Phase 7 need. Raising the machine to 512 MiB would have
made 1024 fit, but it treats the symptom: the number was never derived
from what the system runs, and the larger machine costs the test suite
seven seconds per image that brings the memory up.

**The handle slots are one shared arena**, not a table inside every
`Process`. A table inside the object would mean `PROCESSES` multiplied by
`HANDLES_PER_PROCESS` multiplied by the size of an entry, whether it is
used or not. An entry is `AnyObjectId` plus `Rights` (a `u32`) plus a `u64`
badge, so 32 bytes with the padding:

| Shape | `.bss` | Enough for the memory server? |
|-------|--------|-------------------------------|
| 256 × 65536 inline (the first plan) | 512 MiB | yes |
| 64 × 4096 inline | 8 MiB | yes |
| 64 × 1024 inline | 2 MiB | no |
| arena of 16384, ceiling 4096 (chosen) | 0.5 MiB | yes |

The demand that decides it is `server-memory`, not the root task. The
server owns the RAM memory objects and holds one handle per object it hands
out; `memory_split` makes two objects out of one and
[2.4.2](02-architecture.md#242-memory-objects-and-mappings) says the kernel
never merges them, so its handle count rises with the fragmentation of the
memory up to `MEMORY_OBJECTS`, which is 4096. The root task, with one
memory object per boot region (`MAX_BOOT_REGIONS` is 128), its servers, and
their endpoints, is an order of magnitude below that.

`HANDLE_ENTRIES` is 16384, two per object the machine can hold, because a
handle may be duplicated and transferred. `HANDLES_PER_PROCESS` is 4096 and
becomes the ceiling the quota enforces against the arena, not memory set
aside per process; `Handle` carries 32 index bits, so no number here is an
ABI constraint. A handle table that grows out of the reserve frame by frame
stays the answer if the arena is ever too small; it needs a
`FrameAccess`-shaped path into `kernel-objects` and is not built now.

**What the bring-up costs in QEMU**, measured with the `memory` image on
the reference machine: 5.6 seconds, of which the firmware accounts for 2.2.
On a 512 MiB machine it would be 12.9 against 2.6, because
`kernel_core::memory::adopt` walks the window page by page and
`drop_identity` unmaps twice as many pages. Both are bounded by the memory
of the machine and stay far below the time limit of a run. If a later phase
does raise the machine, `adopt` is the lever: the window is one contiguous
run by construction and does not have to be walked page by page.

### 10.5.1 `audhsos-abi` additions

- `syscall.rs`: the table macro. Every entry: number, name, argument
  count, first-argument object type (or none):

```rust
syscalls! {
    ProcessCreate = 1 (5 args, Process), ProcessInstallHandle = 2 (3, Process), ProcessSetFaultHandler = 3 (2, Process),
    ProcessKill = 4 (1, Process), ThreadCreate = 5 (6, Process), ThreadStart = 6 (1, Thread), ThreadSuspend = 7 (1, Thread),
    ThreadResume = 8 (1, Thread), ThreadKill = 9 (1, Thread), ThreadSetPriority = 10 (2, Thread), ThreadInfo = 11 (1, Thread),
    ThreadExit = 12 (0, None), ThreadYield = 13 (0, None), MemorySplit = 14 (2, MemoryObject), MemoryMap = 15 (6, Process),
    MemoryUnmap = 16 (3, Process), MemoryProtect = 17 (4, Process), MemoryInfo = 18 (1, MemoryObject),
    HandleDuplicate = 19 (2, Any), HandleClose = 20 (1, Any), EndpointCreate = 21 (0, None), EndpointBadge = 22 (2, Endpoint),
    IpcCall = 23 (1, Endpoint), IpcSend = 24 (1, Endpoint), IpcRecv = 25 (1, Endpoint), IpcTryRecv = 26 (1, Endpoint),
    IpcReply = 27 (1, Reply), IpcReplyRecv = 28 (2, Reply), NotificationCreate = 29 (0, None),
    NotificationSignal = 30 (2, Notification), NotificationWait = 31 (1, Notification), NotificationPoll = 32 (1, Notification),
    InterruptCreate = 33 (2, SystemControl), InterruptBind = 34 (3, Interrupt), InterruptAck = 35 (1, Interrupt),
    IoPortCreate = 36 (3, SystemControl), IoPortRead = 37 (3, IoPortRange), IoPortWrite = 38 (4, IoPortRange),
    MemoryCreateDevice = 39 (3, SystemControl), SystemInfo = 40 (1, SystemControl), DebugLog = 41 (0, None),
}
```

  The macro generates `enum Syscall`, `Syscall::from_number`,
  `Syscall::name`, `Syscall::argument_count`, and
  `Syscall::first_argument() -> FirstArgument`, which is `Nothing`, `Any`,
  or an object type; `Syscall::object_type()` and `takes_handle()` read it.
  There is no `dispatch!` helper: a `match` over an exhaustive enum is
  already checked for completeness by the compiler, and a macro around it
  would only make the errors worse. The userland wrappers in Phase 7
  consume the same table.
- The status word (`ipc_buffer::Status`) carries the error code in its low
  half, where zero is success, and the partial flag in bit 32. Partial
  progress is a success and cannot be an error code, and a caller reads one
  word; a word that claims both, or names an error the table does not have,
  is refused when it is decoded.
- `ipc_buffer.rs`: the fixed layout of the 4096-byte buffer as offsets:
  `SYSCALL_NUMBER = 0`, `ARGS = 8` (six words), `STATUS = 56`, `RETURN =
  64` (two words), `MESSAGE = 128`: `LABEL = 128`, `WORD_COUNT = 136`,
  `HANDLE_COUNT = 144`, `HANDLES = 152` (four words), `WORDS = 184` (480
  words, ends at 4024). Safe accessors over `&[u8; 4096]` and
  `&mut [u8; 4096]` with `u64::from_le_bytes`; `Message` view with
  validation (`word_count <= 480`, `handle_count <= 4`).
- `thread.rs`: `ThreadState` codes for `thread_info`, `FaultKind`.
- `Error` gets `Partial` moved out: `Partial` is a success status in the
  return word, not an error. Add `InvalidState`, `NotRunnable`.

### 10.5.2 `kernel-objects` additions

`handle_table.rs`: one arena for the whole machine, not a table inside
every process. `HandleArena<const N: usize>` holds `Slot { generation: u32,
owner: Option<ProcessId>, occupant: Option<Entry> }` with `Entry { object:
AnyObjectId, rights: Rights, badge: u64 }`, where `AnyObjectId` is an enum
over the typed ids, plus the free list of the arena and, per slot, the link
that chains the slots of one owner. A process holds `HandleList { head:
Option<u32>, count: u32, capacity: u32 }`: what it has and what its creator
granted it, not memory set aside for it.

`insert(process, entry)` takes a slot from the free list, stamps the owner,
chains it, and refuses with `Error::QuotaExceeded` when `count` has reached
`capacity` or the arena is full. `lookup(process, handle) -> Result<&Entry,
Error::InvalidHandle>` checks the index, the generation, **and the owner**;
without the owner check one process could name a slot of another.
`duplicate(process, handle, rights)`, `close(process, handle)`, FIFO slot
reuse, generation increment skipping 0. `close_all(process)` walks the
owner chain, so that destroying a process does not scan the arena.

The index of a handle is the index of the arena slot, which is what makes
every operation constant time. A process therefore sees slot numbers that
say roughly how many handles the machine holds; that is the price of the
shared arena and it reveals nothing about what those handles name.

`HANDLE_ENTRIES` sizes the arena and `HANDLES_PER_PROCESS` is the ceiling a
creator may grant, which is what
[2.3.2](02-architecture.md#232-handles-and-rights) means by a capacity
fixed at process creation within the creator's quota (D-58).

Object structs: `Process { root: PhysFrame, regions: RegionTable<MemoryObjectId,
REGIONS_PER_PROCESS>, handles: HandleList, threads: [Option<ThreadId>; 64],
quota: Quota, fault_handler: Option<EndpointId>, kernel_object_quota: Quota
}`, `Thread { process, state: ThreadState, priority, max_priority,
time_slice, kernel_stack, ipc_buffer: PhysFrame, ipc_state, queue_links:
Links, context: VirtAddr }`, `MemoryObject { frames: PhysFrameRange, kind:
MemoryKind, cache }`.

The address space of a process is the root and the region table inside the
`Process`; there is no address-space object, no pool for one, and no
`AddressSpaceId` (D-65). `context` is one word, the kernel stack pointer of
the thread while it is not running, so no type parameter for the machine
context reaches this crate or the crates above it (D-67); the dependencies
stay `kernel-types` and `audhsos-abi`.

A new root copies the kernel half from the kernel's own root: the two
page-map level four entries the kernel occupies, 256 for the window and 511
for the stacks, the boot information page, and the image. Both exist when
the memory bring-up is done, and a kernel stack allocated later changes
only tables below entry 511, so it needs no second copy. A QEMU test holds
that no further kernel entry appears after boot; if one ever did, every
address space created before it would be missing it.

Every pool and the handle arena are `const`-constructible and all zeros
when empty (D-66): the generation of a slot counts up in `allocate` instead
of starting at one, and the free list is implicit through a high-water mark
while released slots keep the FIFO chain that 6.6.6 requires. `RegionTable`
carries `kernel: bool` instead of `user: bool` for the same reason.

They live in one `Objects` structure, which is parameterized by its four
sizes rather than reading them from `config` directly: the kernel uses the
alias `MachineObjects`, and a test holds an `Objects<2, 4, 4, 8>` it can
keep on its stack. The structure measures 1 224 280 bytes with the numbers
of `config`, so a test that built one would overflow its stack — which is
what happened when one did, and is the same failure D-66 predicts for the
boot stack one level down. The counts themselves moved from
`kernel-core::config` to `kernel-objects::config`, where the structure can
name them; `kernel-core::config` re-exports them and keeps the numbers of
the memory bring-up.

`kernel_core::machine::Machine` holds the objects and the scheduler, and
`MACHINE` is the `Preset` cell holding it: the second cell type of
`audhsos-sync`, `const`-initialized and without the `Option`, because
`Some(value)` in a `static` carries a non-zero discriminant and would move
the whole structure out of the `.bss` into the image file. `KernelState`
keeps its counters and does not hold the pools.

`kernel-objects` depends on `kernel-mm`, because `Process` holds the real
`RegionTable` and not a count of one, and `CachePolicy` moved from
`kernel_mm::page_table` to `kernel-types`, where a memory object reaches it
without depending on the page tables; the old path stays as a re-export.

### 10.5.3 `kernel-sched` (`crates/kernel/sched`)

Layer 2, deps `kernel-objects`, `audhsos-abi`. `Scheduler { queues:
[Queue; 32], ready_bitmap: u32, current: Option<ThreadId>, idle: ThreadId
}`, queues as intrusive doubly linked lists through `Links { next, prev }`
in the thread entries (the scheduler receives `&mut Pool<Thread, N>` in
every call). Operations from
[02-architecture.md 2.5.3](02-architecture.md#253-scheduler); the state
transition table as a `const` array of `(from, event, to)` with an
exhaustive test. `tick` decrements the slice and requests a reschedule at
zero.

### 10.5.4 HAL additions (`kernel-hal-api` trait, `kernel-hal-x86_64` adapter)

- The frame a new thread starts through is `kernel_x86_tables::context`,
  not a HAL trait: writing it is arithmetic over `u64` values, so it lives
  with the other pure tables and is host-tested there, at full coverage.
  `prepare_user` writes thirteen words into the top of a kernel stack and
  returns the index the first switch loads. The thirteenth is the one a
  thread cannot ask for: the address of its own IPC buffer, which the
  trampoline pops into the first argument register before `iretq`. The
  kernel maps that buffer at `ipc_buffer_address(slot)`, the top pages of
  the address space of the process.
- The adapter holds the two naked functions, `switch` and
  `enter_user_trampoline`, and `switch_to`, which is `switch` with the two
  words named as what they are; a `VirtAddr` is `repr(transparent)` so
  that the pointer the switch writes through names exactly that word.
- `trait AddressSpaceControl { fn activate(&mut self, root: PhysFrame); fn active(&self) -> PhysFrame; }`
  with a recording double, replacing the free `unsafe fn activate` the
  adapter carries today (D-65). `kernel-core` calls it before a switch only
  when the incoming thread belongs to another process; threads of one
  process switch without touching `CR3`.
- Adapter: the saved context is the kernel stack pointer. A new thread's
  kernel stack is prepared in safe Rust as `u64` values: callee-saved
  registers (six zeros), the address of `enter_user_trampoline`, then the
  interrupt frame `rip = entry`, `cs = 0x23`, `rflags = 0x202`, `rsp =
  stack`, `ss = 0x1B`. `switch` is the naked function: push `rbx rbp r12
  r13 r14 r15`, store `rsp` into `*from`, load `rsp` from `to`, pop the
  six, `ret`. `enter_user_trampoline` is a second naked function: `iretq`
  (it runs with `rsp` at the synthesized frame after `switch` returns into
  it). Update the assembly inventory: the naked user-entry function is a
  new site.
- On every switch, `tss.rsp0 = kernel_stack_top` of the incoming thread.
- Vector `0x80` handler (`extern "x86-interrupt"`, DPL 3): reads the current
  thread's IPC buffer through the window, calls
  `kernel_core::syscall::dispatch`, writes the status and return words,
  returns; a reschedule request set during the call is honored before the
  return by calling `switch`.
- Timer handler: `kernel_core::tick`, then `switch` if requested, then EOI.

### 10.5.5 `kernel-syscall` (`crates/kernel/syscall`)

Layer 3. `dispatch(machine: &mut Machine<'_, E, ..>, caller: ThreadId,
buffer: &mut [u8; 4096]) -> Outcome` implementing the validation order of
[02-architecture.md 2.8](02-architecture.md#28-system-call-interface):
number → argument count → handle → type → rights → arguments → quota. The
`Machine` is the objects, the scheduler, and an `Environment`; the return
value says whether the caller should switch threads before it returns to
user mode. Then one function per system call in `calls/*.rs`; every error
path tested on the host.

`KernelState` does not reach this layer, which is in `kernel-core` above
it. What the calls need beyond the objects is the trait `Environment`:
address spaces, mappings, kernel stacks, frames, and the debug console.
The kernel implements it over its memory bring-up, a test with a recording
double, and that is what makes every error path reachable on the host.

The argument count is checked without a count in the buffer: the words
above what the call reads must be zero, which is what a caller built
against another version of the table looks like. The argument words of
every call are a table in the module documentation of `calls`.

`reaper::reap` gives back what a thread that has ended held. A thread that
ends itself — `thread_exit`, or a `process_kill` of its own process — is
still standing on its kernel stack while the kernel writes its answer, so
the stack, the IPC buffer, and the pool slot stay until the kernel has
switched away from it; the state `Exited` is the whole record of what is
left to do. The kernel calls `reap` after a switch.

Phase 5 implements twenty of the forty-one calls: `process_create`,
`process_install_handle`, `process_kill`, the nine thread calls
(`thread_create` through `thread_yield`), the five memory calls
(`memory_split` through `memory_info`), `handle_duplicate`,
`handle_close`, and `debug_log`. The other twenty-one belong to Phase 6
and return `Unsupported` until then, `process_set_fault_handler` among
them: it is a process call by name and an IPC call by nature, because the
endpoint it names is a Phase 6 object. A test covers every one of the
twenty-one, so that the boundary is a fact of the build and not of this
paragraph.

### 10.5.6 User-mode test programs

`crates/user/sys-x86_64` (adapter, target `X86_64None`): the `entry!`
macro, which puts `_start` into `.text.entry` so that it is the first byte
of a flat binary, `syscall()` with one `asm!("int 0x80")`, `buffer()` over
the address the kernel handed the thread, and `call()`, which fills the
buffer, makes the call, and reads the answer back.

`crates/user/test-programs` with one binary per scenario, linked at
`0x40_0000` with a linker script whose base the xtask checks, converted to
flat binaries by `build-user-tests` (using `llvm-objcopy -O binary`) into
`target/user-tests/<name>.bin`. Test kernels embed them with
`include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/<name>.bin"))`;
the xtask sets the variable when it builds test kernels.

`tests/support` builds a process the way the root task will be built in
Phase 7 — an address space that carries the kernel half, the program
mapped read and execute, a stack, an IPC buffer, a thread whose kernel
stack carries the frame — switches into it, and runs the system call gate
and the trap handler over it. The test images share it and differ in what
they watch: `tests/user.rs` the lifecycle of a thread, `tests/isolation.rs`
what a user thread cannot do, `tests/concurrency.rs` two threads of one
process taking turns, `tests/preemption.rs` what the timer does to threads
that never ask for anything, and `tests/syscalls.rs` the whole table of
calls made from ring three. The programs are `thread_exit`,
`count_and_exit`, `read_kernel_memory`, `hlt_in_user`, `two_threads`,
`spin`, and `every_syscall`.

Acceptance: `check` green; catalog 6.6.6 handle and pool items, 6.6.7,
6.6.9 for the calls this phase implements, the frame item of 6.6.16, the
cell item of 6.6.18, 6.6.21 address space, thread, and system call items
and the isolation item that does not name a handler.

## 10.6 Phase 6: IPC and interrupt forwarding

### 10.6.0 What the phase settles before the first line

Four things reach across the crates of this phase, and each of them is
cheap to write down now and expensive to change halfway through.

**The queues a thread waits in.** A `Thread` carries two link pairs. The
run queues of `kernel-sched` use `queue_links`, the wait queues of
`kernel-ipc` use `wait_links`, and a thread is in at most one of the two
at a time, because a thread in a run queue is `Ready` and a thread in a
wait queue is blocked. They are separate fields and not one, because
`Scheduler::dequeue` unlinks through `queue_links` and then corrects the
head, the tail, and the ready bitmap of the run queue of the thread's
priority: handed a thread whose links point into an endpoint queue it
would splice that queue and leave the endpoint's own head and tail naming
a thread that is no longer in it. Beside the links the thread carries
`wait: Wait`, which names what it waits on, so that a cancellation finds
the queue without searching every endpoint (D-74). This is the `ipc_state`
that 10.5.2 already sketched and Phase 5 had nothing to put in.

**Who writes the result of a call that blocked.** A blocked call has no
result yet, so `dispatch` must write none: `Reply` gains `blocked: bool`
and `Reply::BLOCKED`, and for that reply the dispatcher leaves the status
word and the return words of the caller's buffer untouched. The thread
that completes the rendezvous writes them into the buffer of the thread it
wakes, through the seam of 10.6.4. Every wake-up therefore carries a
status: the rendezvous writes `Status::OK`, a destroyed object writes
`ObjectDestroyed`, a dropped reply object writes `ReplyDropped`, and a
cancelled operation writes `Cancelled`.

**When an object is destroyed.**
[2.3.4](02-architecture.md#234-lifetime) is implemented in this phase and
amended in one point: a thread blocked on an object holds no reference to
it (D-75). References come from handles, from mappings, and from
bindings; closing the last handle to an endpoint destroys it and wakes
everyone who waited. With a blocked thread holding a reference the wake-up
that [6.6.8](06-testing-strategy.md#668-ipc-kernel-ipc) asks for could
never be reached from user mode.

**Where the new pools live.** The five pools of this phase — endpoints,
notifications, replies, interrupts, port ranges — take their sizes from
`kernel_objects::config` directly and are not const parameters of
`Objects`. Together they stay under 200 KiB at the sizes of `config`,
against the 1.2 MiB the four parameterized pools reach, so a host test can
hold a machine with all of them at full size on its stack. The four
parameters stay four, and the signature of everything that touches the
machine — every call function, `dispatch`, `reap`, `handle_syscall`, the
test images — stays what it is instead of growing to nine.

### 10.6.1 `audhsos-abi` additions

- `ipc_buffer.rs`: the label range the kernel keeps for itself.
  `KERNEL_LABEL_BASE = 0xFFFF_FFFF_FFFF_FF00` is the first reserved label;
  `FAULT_LABEL_BASE = KERNEL_LABEL_BASE` and the label of a fault message
  is `FAULT_LABEL_BASE + FaultKind::code()`, so the six kinds occupy
  `..FF01` to `..FF06` and 250 labels are left for the kernel messages of
  later phases. `Message::is_kernel_label` reads it. A `send` or a `call`
  whose label is at or above the base is refused with
  `Error::InvalidArgument` before anything is copied, which is what makes
  the range reserved rather than merely documented.
- `error.rs`: `Cancelled = 25 => "the operation was cancelled before it
  completed"`. It is what a thread finds in its status word when
  `thread_suspend` took it out of a wait queue, which the transition table
  allows out of every blocked state.
- The result convention for a call whose answer does not fit into two
  return words: the rest goes into the message area of the caller's own
  buffer, with label zero and handle count zero, and the first return word
  says how many words were written. `thread_info` and `system_info` use
  it and nothing else does.
- `thread_info` becomes: return word 0 the state code, return word 1 the
  code of `FaultKind` or zero when the thread carries no fault; message
  words 0 to 2 the faulting address, the instruction pointer, and the
  error code, with word count 3 when there is a fault and 0 otherwise.
  This replaces the boolean Phase 5 wrote into return word 1 and is what
  [2.8](02-architecture.md#28-system-call-interface) means by state and
  fault information.
- `system_info` writes twenty words: for each of the eight pools its
  capacity and its live count, in the order processes, threads, memory
  objects, endpoints, notifications, replies, interrupts, port ranges,
  then the capacity and the live count of the handle arena, then
  `TICKS_PER_SECOND`, then the physical address of the RSDP, which is zero
  when the platform named none. Return word 0 is 20.
- A port width is a plain word of 1, 2, or 4 bytes; every other value is
  `Error::InvalidArgument`. There is no width enum in the ABI: the value
  crosses the interface as an argument word, and an enum over three
  numbers would be encoded on one side and decoded on the other for
  nothing.

### 10.6.2 `kernel-objects` additions

The six object structures:

```rust
pub struct Endpoint { pub senders: WaitQueue, pub receivers: WaitQueue }
pub struct WaitQueue {
    head: Option<ThreadId>, tail: Option<ThreadId>, len: u32,
}
pub struct Reply { pub caller: ThreadId, pub consumed: bool }
pub struct Notification {
    pub word: u64,
    pub waiter: Option<ThreadId>,
    pub bound_interrupt: Option<InterruptId>,
}
pub struct Interrupt {
    pub line: u8,
    pub vector: u8,
    pub notification: Option<(NotificationId, u8)>,
    pub masked: bool,
}
pub struct IoPortRange { pub first: u16, pub count: u16 }
pub struct SystemControl;
```

`SystemControl` holds nothing: it is the right to create interrupts, port
ranges, and device memory, and a capability to it is the whole of that
right. It needs no pool. The four calls that take one check the type the
handle names and never look an object up, so its `AnyObjectId` carries an
index and a generation that nothing reads; the root task receives the one
handle to it at boot.

`Thread` gains three fields: `wait_links: Links` beside the `queue_links`
that Phase 5 called `links`, `wait: Wait`, and `fault: Option<Fault>`.

```rust
pub enum Wait {
    Nothing,
    Endpoint { endpoint: EndpointId, queue: Queue, badge: u64 },
    Reply { reply: ReplyId },
    Notification { notification: NotificationId },
}
pub enum Queue { Senders, Callers, Receivers }
```

`Queue` has three variants and not two because the rendezvous is completed
by whichever side arrives second, so the record has to say what the side
that arrived first asked for: a thread that used `ipc_send` is done when its
message is taken, and one that used `ipc_call` waits for the answer
afterwards. Both wait in the senders queue and a cancellation treats them
alike. The badge beside them is the badge of the capability a queued sender
sent through, which is what the receiver that meets it later sees; a
receiver carries none, having no capability of anyone else's in its hand.

The operations of `WaitQueue` — `enqueue`, `dequeue_front`, `unlink`, and
the `requeue` that puts a peer back where a meeting took it from — live in
`kernel-objects` beside the pool their links thread through, because a
method has to live in the crate that defines the type and the queue is a
field of `Endpoint`. `kernel-ipc` is the rendezvous that uses them.

`Process::fault_handler` becomes `Option<EndpointId>`, which is what
10.5.2 sketched and Phase 5 could not name.

`store.rs`: five pools sized from `config`, `MachineObjects` unchanged in
its parameters, and `Objects::counts` beside a new `Objects::capacities`,
each nine numbers — the eight pools and the handle arena — which is what
`system_info` reports. `Objects::destroy(id)` is the one
place that turns a reference count of zero into the destruction of the
object it names; what destruction does to waiters belongs to `kernel-ipc`
and is passed back to the caller rather than done here. It comes back as
`Destroyed`, which carries the object away with it: the pool slot is free by
the time the caller sees it, and the queues are the only record of who has
to be woken. Beside it, `Pool::force_release` frees a slot whatever its
count, which is what a `process_kill` and the reaper of a thread do — a
process ends when it is killed and a thread when what it held has been given
back, whatever handle still names either of them.

`handle_table.rs`: `close_all` is replaced by `close_next(list) ->
Option<Entry>`, so that every entry a dying process held passes through
the caller's release path one at a time, in the shape the reaper already
uses. A count is not enough once a handle is a reference.

### 10.6.3 `kernel-ipc` (`crates/kernel/ipc`, layer 3)

Deps `kernel-objects`, `kernel-sched`, `audhsos-abi`; `test-support` as a
dev-dependency. The crate is the rendezvous state machine over the pools
and the scheduler. Every operation takes the objects and the scheduler and
returns `Result<Outcome, Error>`, where the outcome says whether the
caller blocks, which threads became ready, and what is to be written into
whose buffer; the syscall layer applies it. Nothing here reaches a frame
or an address space.

**The queues.** `WaitQueue::enqueue` inserts by priority and then FIFO:
the thread goes behind the last thread of a priority at least its own, so
a walk of the queue is bounded by its length and no second structure
carries the order. The position is fixed at that moment; a later
`thread_set_priority` does not move a thread that already waits.
`WaitQueue::dequeue_front` and `WaitQueue::unlink(thread)` work through
`wait_links` and correct head, tail, and length.

**`cancel(objects, scheduler, thread)`** takes a thread out of whatever it
waits on, reading `Wait` to find it, and clears the record. `thread_kill`,
`process_kill`, and `thread_suspend` call it before the scheduler
operation; for suspend the thread's buffer is answered with `Cancelled`,
for the two that end a thread nothing is written.

**The nine operations.** `call`, `send`, `recv`, `try_recv`, `reply`,
`reply_recv`, `signal`, `wait`, `poll`, with the semantics of
[2.6.1](02-architecture.md#261-endpoint-operations) and
[2.6.3](02-architecture.md#263-notifications). A sender that finds a
waiting receiver transfers at once and makes it ready: `send` returns and
`call` blocks the caller in `BlockedReply` on a fresh `Reply` object whose
handle the receiver gets. A sender that finds no receiver blocks in
`BlockedSend`, `recv` with no sender blocks in `BlockedRecv`, `try_recv`
answers `WouldBlock` and leaves both queues as they were. `reply` writes
into the caller's buffer, wakes it, and consumes the reply object; a
consumed one answers `InvalidState`, and one whose caller is gone answers
`InvalidState` without touching memory. `reply_recv` is the two in that
order, and a failing reply does not become a receive.

**`transfer`.** The one function that moves a message:

```rust
pub fn transfer<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    from: &[u8; SIZE],
    to: &mut [u8; SIZE],
    objects: &mut Objects<NP, NT, NM, NH>,
    sender: ProcessId,
    receiver: ProcessId,
) -> Result<Transferred, Error>;

pub struct Transferred { pub words: u16, pub handles: u8, pub truncated: bool }
```

It takes the store and not two handle lists and the arena, for two reasons:
a handle that arrives retains a reference to the object it names, which
needs the pools; and the two processes may be one process, so two mutable
borrows of one list would be two views of the same thing and whichever was
written back last would win. The list is read out of the store and written
back instead.

The order is what makes a refused message change nothing: the header is
validated first (`word_count <= 480`, `handle_count <= 4`), then all four
handles are resolved in the sender's list and checked for `TRANSFER`, and
only then are the words copied and the handles installed. A handle without
`TRANSFER` therefore fails before any other handle of the same message is
installed. The reserved label range is not checked here but at the entry
of `ipc_send` and `ipc_call`, where the message is one a user thread
wrote: the fault message of 10.6.4 is the kernel's own and carries a
reserved label by construction, and a check inside `transfer` would refuse
exactly the message it exists for. Installation stops when
the receiver's list is full: the message is delivered, `handle_count` in
the receiver's buffer says how many arrived, `truncated` is set, and the
receiver's status word carries `Status::PARTIAL`, which is the error flag
[2.6.2](02-architecture.md#262-message-layout) asks for and not an error.
A badged capability carries `SEND` alone and therefore no `BADGE` right,
so the ordinary rights check refuses a second badging with `AccessDenied`
and `endpoint_badge` needs no case of its own.

**Destruction.** `destroy_endpoint` wakes both queues with
`ObjectDestroyed`, `destroy_notification` wakes its waiter with the same,
`destroy_reply` wakes its caller with `ReplyDropped`. Each returns the
threads that became ready.

**Interrupt delivery.** `deliver(objects, scheduler, vector)` finds the
interrupt object the vector names by walking the pool of 64 slots — a scan
of that length in an interrupt handler is cheaper than a second table to
keep in step with the pool — marks the line masked, signals the bound bit
into the notification word, and wakes its waiter. `acknowledge(interrupt)`
clears the mark. The masking, the end-of-interrupt, and the unmasking
themselves are the adapter's, in the order
[2.7](02-architecture.md#27-interrupts-and-devices) gives: mask at the I/O
APIC, end-of-interrupt at the local APIC, then signal.

### 10.6.4 `kernel-syscall` additions

**The seam to a second buffer.** `Environment` gains

```rust
fn with_buffer<R>(
    &mut self,
    frame: PhysFrame,
    body: impl FnOnce(&mut [u8; SIZE]) -> R,
) -> Result<R, Error>;
fn read_port(&mut self, port: u16, width: u8) -> Result<u64, Error>;
fn write_port(&mut self, port: u16, width: u8, value: u64) -> Result<(), Error>;
fn interrupt_vector(&self, line: u8) -> Option<u8>;
fn route_interrupt(&mut self, line: u8, vector: u8) -> Result<(), Error>;
fn mask_interrupt(&mut self, line: u8);
fn unmask_interrupt(&mut self, line: u8);
fn meets_ram(&self, frames: PhysFrameRange) -> bool;
fn acpi_pointer(&self) -> u64;
```

`interrupt_vector` is a question and not a table of this crate: the vector
plan is the architecture's, so `kernel-hal-api` gains
`InterruptController::vector_of` and the environment passes the question on.
`meets_ram` is the other question only the architecture layer can answer,
and it is what `memory_create_device` refuses an aperture with.

`dispatch` holds the buffer of the calling thread and nothing else, and
every transfer has the caller on one side: a send copies out of the
caller's buffer into the buffer `with_buffer` reaches, a receive copies
the other way. A transfer whose two frames are one frame is refused with
`InvalidArgument`; two threads never share an IPC buffer, so that is a
check on the argument and not a state the kernel can reach. The interrupt
and port functions carry plain integers, because this crate does not
depend on `kernel-hal-api` and will not start to.

**The twenty-one calls.** Their arguments:

| Call | Arguments |
|------|-----------|
| `process_set_fault_handler` | process handle, endpoint handle (zero clears it) |
| `endpoint_create`, `notification_create` | none |
| `endpoint_badge` | endpoint handle, badge (non-zero) |
| `ipc_call`, `ipc_send`, `ipc_recv`, `ipc_try_recv` | endpoint handle |
| `ipc_reply` | reply handle |
| `ipc_reply_recv` | reply handle, endpoint handle |
| `notification_signal` | notification handle, bits |
| `notification_wait`, `notification_poll` | notification handle |
| `interrupt_create` | system control handle, line |
| `interrupt_bind` | interrupt handle, notification handle, bit index |
| `interrupt_ack` | interrupt handle |
| `ioport_create` | system control handle, first port, count |
| `ioport_read` | port range handle, port, width |
| `ioport_write` | port range handle, port, width, value |
| `memory_create_device` | system control handle, first frame, frame count |
| `system_info` | system control handle |

`interrupt_create` takes no vector: the vector of a line is fixed by the
plan of `kernel_x86_tables::vectors`, so the kernel derives it and refuses
a line that has none with `InvalidArgument`, and a line an interrupt
object already names with `AlreadyExists`. `ioport_create` refuses a count
of zero, a range that runs past `0xFFFF`, and a range overlapping one that
exists, the last with `AlreadyExists`. `ioport_read` and `ioport_write`
refuse a port outside the range, a port whose width runs past its end, and
a width that is not 1, 2, or 4. `memory_create_device` refuses a range
that meets RAM: a device object is an aperture, and the frames of the
memory map belong to the memory server. `ipc_recv` returns the badge in
return word 0 and the reply handle, or zero, in return word 1;
`notification_wait` and `notification_poll` return the word that was
present in return word 0.

**Fault delivery.** `fault::stop` becomes

```rust
pub fn deliver(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    thread: ThreadId,
    fault: Fault,
    buffer: &mut [u8; SIZE],
) -> Outcome;
```

`buffer` is the IPC buffer of the faulting thread, which the caller reaches
the way the system call gate does. The message is built there and copied
from there, and a copy needs both buffers at once, which one `with_buffer`
cannot give.

It records the fault in the thread, builds the message in the faulting
thread's own IPC buffer — label `FAULT_LABEL_BASE + kind`, word count 3,
handle count 0, words address, instruction pointer, error code — and
performs `call` on the fault handler endpoint of the thread's process.
The thread is then blocked as any caller is, and the reply resumes it at
the instruction it faulted on. A process with no handler, a handler
endpoint that is gone, and a reply pool with no slot left all end the same
way, in `ThreadState::Faulted`, which is what Phase 5 does for every
fault today and stays the answer for a fault nobody takes.

`process_set_fault_handler` retains the endpoint, releases the one it
replaces, and clears the handler when its second argument is zero.

**Handle lifetime.** `handle_close` and the close of a dying process's
list release the reference the entry holds; `process_install_handle`,
`handle_duplicate`, and the handle installation of `transfer` retain one.
`memory_map` and `memory_unmap` keep the references they already take, so
a memory object that is mapped survives the close of its last handle.
When the last reference is gone the object is destroyed, and for the three
object types with waiters that is the operation of 10.6.3.

### 10.6.5 HAL and `kernel-core` additions

- `kernel-hal-api` gains `paging::FrameBytes`, which reaches a frame as
  `&[u8; PAGE_SIZE]` and `&mut [u8; PAGE_SIZE]`, with the double
  `MemoryFrameBytes`. `PhysicalWindow` has the method already
  (`frame_bytes_mut`) and gains the `impl`.
- `kernel-hal-api` gains `device::Devices: InterruptController +
  PortAccess`, the one bound `KernelEnvironment` needs for the two
  hardware seams, with the double `RecordingDevices` over the existing
  `FakeInterruptController` and `RecordingPorts`. The feature `port-io`
  stops being unused: `kernel-hal-x86_64` and `kernel-core` enable it.
- `kernel-hal-x86_64` gains `Ports`, the `PortAccess` implementation, and
  `instructions` gains `read_port_u16`, `write_port_u16`, and
  `read_port_u32` beside the three that exist. `DeviceAccess<'_>` joins
  `&mut Apics` and `Ports` into the `Devices` the kernel environment
  takes. Three new `asm!` sites — `read_port_u16`, `write_port_u16`,
  `read_port_u32` — and the `unsafe` blocks of the six methods that reach
  them; the budgets in `policy::CRATES` and the inventories in
  [4.3](04-safety-policy.md#43-allowlist) and
  [4.5](04-safety-policy.md#45-inline-assembly-inventory) are set to the
  real counts at the end of the phase.
- `KernelEnvironment` gains the parameter `D: Devices`, the field it is
  held in, and the ACPI pointer the bring-up read, which is what
  `system_info` reports and the only thing of the firmware the kernel
  keeps. `KernelState` stays the counter structure it is (D-66).
- `trap.rs` gains `Exception::fault() -> Option<Fault>`, the mapping from
  vector to `FaultKind`: 14 to `PageFault` with `cr2` as the address, 13
  to `GeneralProtection`, 6 to `InvalidOpcode`, 0 to `DivideError`, 3 to
  `Breakpoint`, 17 to `AlignmentCheck`, and `None` for every other vector,
  which stops the thread without a message as Phase 5 does.
- The device interrupt handler counts the interrupt, masks the line at the
  I/O APIC, acknowledges at the local APIC, hands the vector to
  `kernel_ipc::deliver` through the machine, writes the word into the buffer
  of the thread it woke, and switches if that thread outranks the running
  one. That is the order
  [2.7](02-architecture.md#27-interrupts-and-devices) gives, and the switch
  is the last of it. The kernel binary has the first four and not the
  switch, because it starts no user thread and there is nobody to switch
  to; the test kernel its images share has all five, and the root task of
  Phase 7 brings the same shape to the boot kernel. Interrupts arrive
  through interrupt gates, so no device interrupt reaches a kernel that
  holds the machine borrow; the vector that names no interrupt object is
  acknowledged and nothing else.

### 10.6.6 User-mode test programs and QEMU tests

New programs in `user-test-programs`: `ipc_client` and `ipc_server`, which
are a call and a reply with a badge, four words, and a handle;
`fault_handler`, which receives fault messages and answers them;
`write_unmapped`, whose fault a handler can repair; and
`notification_waiter`, which programs the interval timer through a port
range, binds its line, and waits for the bit. What a program observes goes
into a page the kernel shares with it and not into its own IPC buffer: a
message of 480 words fills the message area, and a log kept there would be
overwritten by the message it is about. What each of them needs
beyond its own endpoint the test installs into its process with
`support::install`, the way the root task will in Phase 7: the memory
object of the transfer test, the `SystemControl` capability of
`notification_waiter`, and for `fault_handler` a handle to the faulting
process carrying `MANAGE` and `MAP` and a memory object to map with.

`tests/ipc.rs` runs, over two processes:

- call and reply between two user threads, with the badge and the words
  the server saw and the client got back;
- handle transfer: the client sends a handle to a memory object, the
  server maps it and writes, the client reads what the server wrote;
- a message with zero words, with 480 words, and one with 481, which is
  refused before anything is copied;
- a sender killed while blocked, and an endpoint destroyed under a waiter.
  The two queues of one endpoint cannot both hold a waiter at once, because
  whichever side arrives second meets the first, so the image shows them one
  at a time and the host test of `kernel-ipc` shows them together;
- `try_recv` on an empty endpoint;
- the interrupt: `notification_waiter` creates an interrupt object for ISA
  line 0, binds it to bit 3 of a notification, programs the interval timer
  through an `IoPortRange` over ports `0x40` to `0x43`, waits, and finds
  its bit; `interrupt_ack` lets the second one arrive.

The interval timer and not the local APIC timer: the APIC timer is the
kernel's own, it carries vector `0x30`, and `vectors::gsi_of` gives it no
global system interrupt, so no interrupt object can name it. ISA line 0
reaches the I/O APIC through the interrupt source override the MADT
carries, which `Apics::gsi_of` already reads, and programming it is three
port writes — which makes the one test cover the interrupt path, the port
path, and the notification path from ring three.

`tests/isolation.rs` gains the item
[6.6.21](06-testing-strategy.md#6621-kernel-integration-tests-in-qemu)
marks from Phase 6: the process of `read_kernel_memory` and of
`hlt_in_user` names a fault handler, and the handler receives the message
with the reserved label, the kind, and the address. It answers the first
one; the thread resumes at the instruction it faulted on and faults again,
which is what a reply to those two programs must do, and the handler
answers the second by killing the process. A third program writes to an
unmapped address of its own half: there the handler maps a frame and
replies, and the thread runs on and exits, which is the reply that
resumes.

### 10.6.7 Policy and documents

- `policy::CRATES` gains `kernel-ipc` (`Kind::Logic`, deps
  `kernel-objects`, `kernel-sched`, `audhsos-abi`, coverage gate on, host
  target); `kernel-syscall` gains `kernel-ipc`; the budgets of
  `kernel-hal-x86_64`, `audhsos-kernel`, `user-sys-x86_64`, and
  `user-test-programs` rise to their measured counts. Root `Cargo.toml` members, `README.md` for the new crate, and
  the `[[test]]` and `[[bin]]` entries for the new images and programs.
- [05-code-organization.md 5.2](05-code-organization.md#52-crate-catalog)
  already carries the row for `kernel-ipc`; the deviations this phase
  writes down are the two decisions D-74 and D-75, and
  [2.3.4](02-architecture.md#234-lifetime) changes with the second of
  them.
- The system call table of
  [2.8](02-architecture.md#28-system-call-interface) is complete after
  this phase, and the sentence in
  `kernel_syscall::calls` that names the calls Phase 6 owns goes with the
  list it describes.

Acceptance: `check` green; catalog 6.6.8, the IPC items of 6.6.21 and the
isolation item marked from Phase 6; the system call items of 6.6.21 and
6.6.9 now cover every call in the table, with no call left answering
`Unsupported`.

## 10.7 Phase 7: Userland foundation

### 10.7.1 `user-rt` (`crates/user/rt`, logic, layer u0)

Everything a program works out for itself, with no system call in it, so
that all of it runs on the host under test (D-89).

- Typed handles: `ProcessHandle`, `ThreadHandle`, `MemoryHandle`,
  `EndpointHandle`, `ReplyHandle`, `NotificationHandle`,
  `InterruptHandle`, `IoPortHandle`, `SystemControlHandle`, each a newtype
  over `Handle` behind the trait `Typed`, which names the object type. They
  are `#[must_use]` and do not close themselves; `handle_close` needs the
  buffer of the calling thread, which `Drop` is not given (D-89).
- `heap.rs`: `Allocator<BLOCKS, EXTENTS>` over an arena named by its
  length. One free list, first fit, coalescing on release, and a table of
  live blocks so that a release names only its offset. Not size classes:
  6.6.12 wants two released neighbours to become one block large enough
  for their sum, and a class list cannot do that. `grow(new_len)` extends
  the arena; every failure is an `AllocError` and nothing is changed by
  one.
- `message.rs`: `Writer` and `Reader` over the message area. A byte string
  is a word with its length and then the bytes, eight to a word,
  little-endian.
- `startup.rs`: the startup message as named fields, over the pairs
  `audhsos_abi::startup` defines.
- `report.rs`: `Line<N>`, a `core::fmt::Write` into a fixed array, which is
  what a program with no heap formats a log line or a panic report into.

### 10.7.2 `user-sys-x86_64` (layer u1, adapter)

`_start` receives the address of the thread's IPC buffer in the first
argument register, which is where the kernel put it, and hands it to the
program's `main` together with the startup message it read from the
buffer. `Gate { buffer: u64 }` holds that address and carries the
forty-two system call wrappers, written out one by one; each writes the
call number and its arguments into the buffer, executes `int 0x80`, and
turns the status word into a `Result`. A constant assertion holds them to
the table: `COVERED` lists what exists, `Syscall::ALL` lists what must, and
a build fails when they differ (D-92). The panic handler formats a `user_rt::Line` and
sends it to the log endpoint.

### 10.7.3 `user-proto` (`crates/user/proto`)

Name protocol: `Register { name: [u8; 32], endpoint handle }`,
`Lookup { name } -> endpoint`; console protocol: `Write { bytes in words }`,
`Read { max } -> bytes`; memory protocol: `Allocate { len, align } ->
memory handle`, `Release { handle }`; each message a struct with
`encode(&self, &mut Message)` and `decode(&Message) -> Result`, labels as
constants, versions in the label's high 16 bits.

### 10.7.4 `user-loader` (`crates/user/loader`)

`tar.rs`: a ustar reader and a ustar writer over `&[u8]`. The reader takes
512-byte headers, the octal size field, the checksum (the sum of the header
with the checksum field as spaces), the `ustar\0` magic, and the prefix
field for long names; it refuses `..` components and absolute paths, and it
ends the walk at the end marker whatever follows it. The writer is there so
that the archive of the boot image and the archives the tests read are made
by one piece of code, and so that what the crate reads can be checked
against what it writes.

`program.rs`: `plan(bytes)`, which reads a user ELF through `audhsos-elf`
under the constraints of user space — above the lowest mappable page, below
the buffers of the process — and answers with the whole pages that have to
be mapped, the entry point, and where the stack of sixteen pages goes, with
one unmapped page between the stack and the program.

It is a plan and not an action. Carrying it out — allocate, map into the
loader, copy, unmap, map into the child, then the stack, the buffer, the
handles, the startup message, and the start — needs capabilities, and that
is the wiring of the root task in `user-programs`.

### 10.7.5 The kernel starts the root task

[2.9](02-architecture.md#29-boot-sequence) steps 12 to 14 have always been
part of this phase and were written down nowhere in it. They are what makes
the difference between a kernel that boots and a system that runs.

`kernel-core` gains `root.rs`, which is architecture-neutral and host-tested
over the doubles the memory and system call tests already use: it creates
the address space, reads the root task as an ELF and maps every segment with
the permissions its header names (D-92), maps a stack of
sixteen pages with one unmapped page between it and the program, allocates a
kernel stack and an IPC buffer, makes the thread, installs the handles —
`SystemControl`, the process itself, the boot image, and one `Ram` memory
object per free region — and writes the startup message into the buffer.
The root task runs at the highest priority, because it is the fault handler
of everything it starts.

`audhsos-kernel` gains `task.rs`, which is the rest: the frame a new thread
returns through (`context::prepare_user`), the switch of the stacks, the
task state segment, the idle thread on the boot stack, and the loop the
kernel becomes once the root task runs — switch to whatever the scheduler
picked, halt when there is nothing. The system call gate is armed with
`traps::set_syscall_handler`, and the trap handler now tells a fault of the
kernel from a fault of a user thread: the first ends the machine, the second
becomes a message to the fault handler of that process. The device interrupt
handler gains the switch that was the missing third step of
[2.7](02-architecture.md#27-interrupts-and-devices), because there is now a
thread to switch to.

`kernel_hal_x86_64::memory::physical_slice` is what reads the root task out
of the boot image: the window maps physical memory contiguously, so a range
of frames is a range of bytes.

### 10.7.6 Servers and the application

- `server-init`: parse the boot image header again (root task side), read
  the archive, start `server-memory` first with the RAM memory objects,
  then `server-name`, `server-console` with the `IoPortRange`
  `0x3F8..0x400` and the `Interrupt` for line 4, then `app-hello`; act as
  the fault handler for every child and report faults on the log endpoint
  (the console).
- `server-name`: registry of up to 64 names; ownership by badge.
- `server-console`: `Uart16550` over a `Registers` implementation that
  uses `ioport_read`/`ioport_write`; receive interrupt bound to a
  notification; the serial protocol lines of the tests pass through here.
- `server-memory`: allocation policy over memory objects with the zeroing
  rules of D-12 (zero before hand-out and immediately after return), adjacency
  bookkeeping, per-client accounting by badge.
- `app-hello`: looks up `console`, writes `hello from userland`, reads a
  line back and says it again, then reports to its parent and exits.
- Root task as an ELF at `ROOT_TASK_BASE`, with every section on a page of
  its own so that no two segments share one set of permissions (D-92).
- The xtask `image` writes the real boot image: header, root task, ustar
  archive of the server and application ELFs.
- The kernel owns COM1 until userland takes it: `ioport_create` over the
  range of the port makes the kernel give the line up, and after that it
  writes nothing, `debug_log` included. So `debug-uart` stays in both
  profiles — there is no silent window while the first servers come up,
  and no interleaving afterwards, because the handover and not a feature
  flag decides who writes.
- The run ends through the userland: `app-hello` reports `Finished` to the
  root task, and the root task writes to the exit device through its
  `SystemControl` (D-94). `sh tools/xtask.sh run --release` shows the
  greeting through the userland driver and ends when a line is typed;
  `test --e2e --release` is the same run without a person at it.

### 10.7.7 Fuzzing

`crates/support/fuzz` (`fuzz-support`, adapter, host) is the fuzzing
engine and the macro `fuzz_target!(|bytes: &[u8]| { ... })` that writes a
target's two entry points. `fuzz/` is a workspace of its own and holds one
binary crate per target; the root workspace excludes it, because it is
built with flags the checks do not use.

The engine is this project's own, ported from libFuzzer (D-63). `sancov`
holds the callbacks the compiler emits calls to and `counters` holds the
ranges it registers; above them are the mutator, the table of values the
target was seen comparing against, the corpus and its features, and the
loop. Nothing is linked in from outside the workspace, so nothing depends
on the platform's clang carrying a fuzzer runtime, which Apple's does not.

`xtask fuzz` builds `--release` with `--cfg fuzzing`, which is what makes
a target's `main` the loop instead of the replay. The coverage
instrumentation (`-Cpasses=sancov-module` and the `-sanitizer-coverage-*`
LLVM arguments) is not in `RUSTFLAGS` but per package in
`fuzz/Cargo.toml`, with `fuzz/.cargo/config.toml` turning on the Cargo
feature that allows it: it goes on the code under test and the targets,
and not on the engine. Instrumenting the engine costs about three runs in
four, because its counters are the greater part of a target's and never
say anything about the input. There is no `-Zsanitizer=fuzzer`: `fuzzer`
is not one of the values rustc accepts there.

`xtask fuzz --regression` builds the same sources without those flags,
which makes every target an ordinary program that replays the files of its
corpus, and runs it over `fuzz/corpus/<target>/`. That is the step `check`
runs, and it is uninstrumented and therefore quick. Every crash becomes a regression test in the
parser's crate and its input a file in the corpus. Register the targets in
`policy::FUZZ_TARGETS`.

### 10.7.8 Acceptance

`check` green — it runs `test --e2e` as its tenth step, so the end-to-end
run is part of it — with the userland programs reporting through the
console driver and the root task ending the machine; catalog 6.6.12,
6.6.13 tar items, 6.6.22, 6.6.23, 6.6.56; fuzz targets run for 60 seconds
each without findings.

## 10.8 Phase 8: Consolidation and release 0.1.0

1. Measurement: add `read_tsc` (`rdtsc`, one `asm!`) to the adapter and
   a QEMU test kernel `bench.rs` that measures the median of 10 000 system
   call round trips (`thread_yield`) and IPC round trips (call/reply)
   in TSC ticks, printed in the `[bench]` line format; record the numbers
   in `docs/08-roadmap.md` 8.10.
2. Decide the `syscall` instruction path from the numbers with a decision
   register entry D-29 (either "kept as `int 0x80`" or "implemented").
3. Unsafe budget review: every site listed in 04 4.5 with its crate,
   counts equal to the budgets.
4. Documents: read every document against the code and fix every
   difference.
5. `CHANGELOG.md` section `[0.1.0]`, tag `v0.1.0`.

## 10.9 Phase 9: Framebuffer output

### 10.9.1 Crate `gfx` (`crates/gfx`)

Layer 1 logic crate, `no_std`, `forbid(unsafe_code)`, deps `audhsos-abi`
(`FramebufferFormat`), `test-support` behind `test-strategies`. Modules:

- `format.rs`: `PixelFormat` from `FramebufferFormat`; `Color { r, g, b }`
  with `encode(format) -> [u8; 4]` and `decode(format, [u8; 4])`.
- `rect.rs`: `Rect { x, y, w, h }` in `u32` with `intersect`, `union`,
  `is_empty`; `Damage`: a fixed array of 16 rectangles that collapses to
  the bounding rectangle when full.
- `surface.rs`: `Surface<'a> { bytes: &'a mut [u8], width, height, stride,
  format }`; `new` validates `bytes.len() >= height * stride * 4`;
  `fill(rect, color)`, `blit(&Surface, src: Rect, dst_x, dst_y)`, both
  clipping to the surface and recording damage; every byte access goes
  through `get`/`get_mut` with checked arithmetic.
- `font.rs`: `const GLYPHS: [[u8; 16]; 95]` for `' '..='~'`, 8 by 16
  pixels, one bit per pixel, authored in this repository; `glyph(c: char)
  -> &'static [u8; 16]` with the replacement glyph for other characters;
  `draw_text(surface, x, y, text, fg, bg)`.
- `present.rs`: `present(back: &Surface, target: &mut impl PixelSink,
  damage: &Damage)` copies the damaged rectangles; `PixelSink` is
  implemented by `Surface` (the framebuffer mapping) and by a recording
  double in the tests.

Tests: catalog 6.6.26; the glyph test renders every glyph and compares it
against a checksum table in the test file.

### 10.9.2 Display protocol (`user-proto`)

Label range `DISPLAY`. Messages: `Info -> { width, height, format }`;
`CreateSurface { width, height } -> { surface id, memory handle }` (the
display server allocates the backing store from the memory server and
transfers a handle with `READ | WRITE | MAP`); `Present { surface id,
damage: up to 16 rects }`; `DestroySurface { surface id }`; `SetCursor {
x, y, visible }`. One full-screen surface per client in this phase; the
client with the most recent `Present` owns the screen. Errors: `NotFound`
when no framebuffer exists, `InvalidArgument` for a surface larger than
the screen, `PermissionDenied` for a surface of another badge.

### 10.9.3 `server-display` (`crates/user/servers/display`)

Startup message: the framebuffer `Device` memory handle and its
description, the memory server endpoint, the name server endpoint. Maps
the framebuffer read/write, no-execute, `Uncached`. Keeps one `Surface`
per client keyed by badge; `present` goes through `gfx::present`; the
cursor sprite (16 by 16 pixels, project data) is drawn after each present
and the background restored before the next. Registers as `display`. The
logic lives in `state.rs` and is tested on the host with the recording
double; the process loop in `main.rs` only moves messages.

### 10.9.4 Root task and kernel

`system_info` returns the framebuffer description as six result words
(`0` throughout when absent). `server-init` calls
`memory_create_device(start, len, Uncached)` for it and hands the handle
to the display server; without a framebuffer the display server starts
and answers `NotFound`. The kernel accepts a device range that overlaps
no `Usable` region and lies inside an `MmioReserved` region of the boot
information.

### 10.9.5 `xtask` additions

- `qmp.rs`: `Qmp::connect(socket path, timeout)` reads the greeting and
  negotiates `qmp_capabilities`; `execute(command, arguments) ->
  Result<Value, QmpError>`; `screendump(path) -> Result<Image, QmpError>`.
- `json.rs`: `Value` with a parser and a writer for the subset of catalog
  6.6.28. `ppm.rs`: `Image { width, height, rgb: Vec<u8> }`, `pixel(x, y)`,
  `checksum(rect)`.
- The QEMU command line for `test --e2e` gets `-qmp unix:<scratch
  dir>/qmp.sock,server,nowait`; `run --display` replaces `-display none`
  with `-display cocoa` on macOS and `-display gtk` elsewhere.
- Loader test image `no_vga`: run with `-vga none`; expected: the kernel
  reaches the harness and prints `[info] framebuffer=absent`.
- `policy::CRATES` entries `gfx` (Logic, deps `audhsos-abi`) and
  `server-display` (Logic, `X86_64None`, deps `user-rt`, `user-proto`,
  `gfx`); loader budget updated; catalog rows in 05.

### 10.9.6 Acceptance

`check` green; catalog 6.6.24, 6.6.26, 6.6.27 display items, 6.6.28,
6.6.29 output items; the e2e test `display_fill_and_text` passes.

## 10.10 Phase 10: PS/2 input

### 10.10.1 Crate `driver-i8042` (`crates/drivers/i8042`)

Layer 1 logic crate, `no_std`, `forbid(unsafe_code)`, no workspace
dependencies, following the pattern of `driver-uart16550`: a trait
`Ports { read_data, read_status, write_data, write_command }` that the
input server implements over `ioport_read` and `ioport_write`, and a
scripted double `ScriptedPorts` behind the feature `test-doubles` that
replays status and data bytes and records writes. Modules:

- `controller.rs`: constants (data port `0x60`; status and command port
  `0x64`; status bits `OUTPUT_FULL = 0x01`, `INPUT_FULL = 0x02`, `AUX =
  0x20`; commands `READ_CONFIG = 0x20`, `WRITE_CONFIG = 0x60`,
  `DISABLE_AUX = 0xA7`, `ENABLE_AUX = 0xA8`, `TEST_AUX = 0xA9`, `SELF_TEST
  = 0xAA`, `TEST_KBD = 0xAB`, `DISABLE_KBD = 0xAD`, `ENABLE_KBD = 0xAE`,
  `WRITE_AUX = 0xD4`); `Controller<P: Ports>::init(&mut self) ->
  Result<Devices, Error>` runs: disable both ports, flush the output
  buffer, read the configuration byte, clear the interrupt and translation
  bits, self-test expecting `0x55`, port tests expecting `0x00`, enable the
  ports, reset the devices, set the interrupt bits. Every wait is bounded
  by `MAX_POLLS` iterations and yields `Error::Timeout`.
- `keyboard.rs`: `Decoder` for scancode set 2 with the states `Idle`,
  `Extended`, `Release`, `ExtendedRelease`, `Pause(n)`; `feed(byte) ->
  Option<KeyEvent>`; the table from set 2 codes to `KeyCode`.
- `mouse.rs`: `Decoder` with the packet length from the device id;
  `feed(byte) -> Option<PointerEvent>`; sync check, resynchronization,
  sign extension, overflow clamping.
- `device.rs`: command sequences with ACK handling: keyboard reset (`0xFF`
  answered by `0xFA`, `0xAA`), scancode set 2 (`0xF0 0x02`), enable
  scanning (`0xF4`); mouse reset, the sample rate sequence `200, 100, 80`,
  `GET_ID` (`0xF2`) selecting the 4-byte packet on id `3`, enable data
  reporting (`0xF4`).

Tests: catalog 6.6.25 with `ScriptedPorts`; fuzz targets `scancode` and
`mouse_packet` registered in `policy::FUZZ_TARGETS`.

### 10.10.2 Input protocol (`user-proto`)

`KeyCode`: an exhaustive enum of the keys of a 105-key layout with stable
numeric codes; `KeyEvent { code, pressed: bool }`; `PointerEvent { dx:
i16, dy: i16, wheel: i8, buttons: u8 }`; `Event` as a 16-byte record with
a kind byte; `Ring` header `{ write_seq: u64, read_seq: u64, overflow:
u32, capacity: u32 }` followed by the records in a shared memory object
of one page (254 records); `Subscribe { ring memory handle, notification
handle }` registers a client; `Unsubscribe`. `Keyboard` (client side):
tracks modifiers, maps `(KeyCode, modifiers)` through the layout tables
`us` and `de` to a `char`.

### 10.10.3 `server-input` (`crates/user/servers/input`)

Startup message: the `IoPortRange` for `0x60..=0x64`, the `Interrupt`
objects for lines 1 and 12, one notification with bits 0 and 1 bound to
them, the name server endpoint. Loop: wait on the notification; while the
status register shows a full output buffer, read a byte and route it by
the `AUX` bit; feed the decoders; append events to every subscriber's
ring and signal its notification; acknowledge both interrupts. Registers
as `input`. `server-init` creates the capabilities with `ioport_create`,
`interrupt_create`, and `interrupt_bind`. The logic lives in `state.rs`
and is tested on the host with the ring in a `Vec` and a recording
signal double.

### 10.10.4 `xtask` additions

`Qmp::send_key(qcode, pressed)`, `Qmp::move_pointer(dx, dy)`,
`Qmp::button(index, pressed)` over `input-send-event`. A user test program
`input_echo` prints `[input] key <code> <pressed>` and `[input] pointer
<dx> <dy> <wheel> <buttons>` lines through the console driver; the e2e
tests inject a sequence and compare the lines. `policy::CRATES` entries
`driver-i8042` (Logic, deps none) and `server-input` (Logic,
`X86_64None`, deps `user-rt`, `user-proto`, `driver-i8042`).

### 10.10.5 Acceptance

`check` green; catalog 6.6.25, 6.6.27 input items, 6.6.29 input items;
both fuzz targets run for 60 seconds without findings.

## 10.11 Phase 11: Graphical demonstration

- `app-canvas` (`crates/user/apps/canvas`): subscribes to `input`,
  creates a full-screen surface on `display`, keeps a cursor position
  clamped to the screen, draws a line segment for every pointer event
  while button 0 is held, renders typed characters from the `us` layout at
  a text cursor with the bitmap font, clears the screen on `Escape`, and
  presents with damage rectangles. It sends `SetCursor` on every pointer
  event. The drawing state is a host-tested module over `gfx::Surface`.
- e2e tests: `canvas_cursor` (a pointer path moves the cursor sprite;
  screendump before and after), `canvas_stroke` (press, move, release;
  the pixels along the path carry the pen color), `canvas_text` (a typed
  string appears at the text cursor pixel for pixel).
- `cargo xtask run --display` starts the canvas; the root task starts it
  after the servers when the boot image contains it.
- `policy::CRATES` entry `app-canvas` (Logic, `X86_64None`, deps
  `user-rt`, `user-proto`, `gfx`).

Acceptance: `check` green; catalog 6.6.29 combined items; the three e2e
tests pass in CI without a display window.
