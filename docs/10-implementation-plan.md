# 10. Implementation Plan for Phases 1 to 8

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

- **Command entry point.** Run everything through rustup's Cargo proxy:
  `~/.cargo/bin/cargo xtask <subcommand>`. The xtask refuses to run under
  any other Cargo. `~/.cargo/bin/cargo xtask check` must pass before a
  phase is complete.
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
6. `~/.cargo/bin/cargo xtask check` green, then commit.

## 10.1 Phase 1: Memory management logic

Goal: `kernel-mm` and `kernel-objects` complete and host-tested; boot image
header and boot information in `audhsos-abi`. No hardware, no QEMU.

### 10.1.1 `audhsos-abi` additions

`src/boot_image.rs`:

```rust
pub const BOOT_IMAGE_MAGIC: [u8; 8] = *b"AUDHSOS\0";
pub const BOOT_IMAGE_HEADER_LEN: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootImageHeader { pub root_task_offset: u64, pub root_task_len: u64,
    pub archive_offset: u64, pub archive_len: u64, pub kernel_reserve_size: u64 }
impl BootImageHeader {
    /// Parses and validates the header against `image_len` (the length of
    /// the whole image) and `ram_bytes` (total usable RAM).
    pub fn parse(bytes: &[u8], image_len: u64, ram_bytes: u64) -> Result<Self, BootImageError>;
    /// Serializes into 64 bytes, little-endian, for the xtask image writer.
    pub fn to_bytes(&self) -> [u8; BOOT_IMAGE_HEADER_LEN];
}
pub enum BootImageError { TooShort, BadMagic, UnsupportedVersion(u32), HeaderLength(u32),
    RootTaskOffset, RootTaskLength, ArchiveOffset, ArchiveLength, Overlap, Flags(u64),
    ReserveSize(u64) }
```

Validation exactly as the table in
[03-target-platform.md 3.1.6](03-target-platform.md#316-boot-image-format);
every check uses `checked_add`. Field order and offsets as in that table.

`src/boot_info.rs`:

```rust
pub const BOOT_INFO_MAGIC: [u8; 8] = *b"AUDHBOOT";
pub const BOOT_INFO_VERSION: u32 = 1;
#[repr(u32)] pub enum BootRegionKind { Usable = 1, Reserved = 2, AcpiReclaimable = 3, AcpiNvs = 4, MmioReserved = 5 }
#[repr(C)] #[derive(Clone, Copy)] pub struct BootRegion { pub start: u64, pub len: u64, pub kind: u32, pub reserved: u32 }
#[repr(C)] pub struct BootInfoHeader { pub magic: [u8; 8], pub version: u32, pub size: u32,
    pub phys_window_base: u64, pub kernel_phys_start: u64, pub kernel_phys_len: u64,
    pub boot_image_phys_start: u64, pub boot_image_phys_len: u64,
    pub page_tables_phys_start: u64, pub page_tables_phys_len: u64,
    pub boot_stack_phys_start: u64, pub boot_stack_phys_len: u64,
    pub acpi_rsdp: u64, pub region_count: u32, pub reserved: u32 }
```

Provide a safe, allocation-free view: `BootInfoView<'a> { header:
BootInfoHeader, regions: &'a [BootRegion] }` built by
`BootInfoView::parse(bytes: &[u8]) -> Result<BootInfoView<'_>, BootInfoError>`
that decodes the header field by field with `u64::from_le_bytes` and
validates: magic, version, `size == header size + region_count * 16`,
`size <= PAGE_SIZE`, `region_count <= MAX_BOOT_REGIONS`, every region kind
known and `len != 0`, the four fixed ranges pairwise disjoint and each
inside some region. Also a writer `BootInfoWriter` that fills a
`&mut [u8; 4096]` from a header and a region slice (used by the loader).
The kernel adapter (Phase 2) turns a raw pointer into the byte slice; the
parsing stays here, safe and fuzzable.

Tests: catalog 6.6.10 in full, one test per item, in
`src/tests/boot_image.rs` and `src/tests/boot_info.rs`. Add a generator
`any_boot_image_header()` behind `test-strategies` (add the feature and the
optional `test-support` dependency to `audhsos-abi` as in `kernel-types`).

### 10.1.2 Crate `kernel-objects` (`crates/kernel/objects`)

Layer 2, `no_std`, `forbid(unsafe_code)`, deps: `kernel-types`,
`audhsos-abi`, optional `test-support` (feature `test-strategies`).

`src/pool.rs`:

```rust
pub struct ObjectId<T> { index: u32, generation: u32, _marker: PhantomData<fn() -> T> }
pub struct Pool<T, const N: usize> { slots: [Slot<T>; N], free_head: Option<u32>, free_tail: Option<u32>, live: u32 }
enum Slot<T> { Free { generation: u32, next_free: Option<u32> }, Occupied { generation: u32, refs: u32, value: T } }
pub enum PoolError { Exhausted, StaleId, RefOverflow }
impl<T, const N: usize> Pool<T, N> {
    pub fn new() -> Self;                                  // every slot free, generation 1, FIFO list 0..N
    pub fn allocate(&mut self, value: T) -> Result<ObjectId<T>, PoolError>;   // refs = 1
    pub fn get(&self, id: ObjectId<T>) -> Result<&T, PoolError>;
    pub fn get_mut(&mut self, id: ObjectId<T>) -> Result<&mut T, PoolError>;
    pub fn retain(&mut self, id: ObjectId<T>) -> Result<(), PoolError>;       // refs += 1, checked
    pub fn release(&mut self, id: ObjectId<T>) -> Result<Option<T>, PoolError>; // refs -= 1; Some(value) when it reached 0: slot freed, generation += 1 (skipping 0), appended to the FIFO tail
    pub fn live(&self) -> u32; pub const fn capacity(&self) -> u32;
}
```

`[Slot<T>; N]` needs `T: Sized`; build the array with
`core::array::from_fn`. `N` is a const generic so that pool sizes come from
boot-time constants in `kernel-core`; sizes are not decided here.

`src/quota.rs`: `Quota { limit: u32, used: u32 }` with
`charge(n) -> Result<(), QuotaExceeded>`, `refund(n)` (saturating),
`remaining()`.

Tests: catalog 6.6.6 pool and quota items (handle items come in Phase 5):
full pool, stale generation after free and reuse, generation wrap
(`u32::MAX` reuses; test with a pool of one slot by setting the generation
through a `#[cfg(test)]`-only constructor), retain/release counts, FIFO
reuse order, quota exactly at limit and one above, refund restores.
Model-based test: random `allocate`/`retain`/`release`/`get` against a
`HashMap<u32, (generation, refs)>`.

### 10.1.3 Crate `kernel-mm` (`crates/kernel/mm`)

Layer 2, `no_std`, `forbid(unsafe_code)`, deps: `kernel-types`,
`kernel-hal-api`, `audhsos-abi`, optional `test-support`; dev-deps
`test-support`, `kernel-hal-api` with `test-doubles`, `kernel-types` with
`test-strategies`.

Modules:

**`memory_map.rs`.** Input: `&[MemoryRegion]` from `Platform`. Output:

```rust
pub struct NormalizedMap { usable: [PhysFrameRange; MAX_BOOT_REGIONS], count: usize }
pub fn normalize(regions: &[MemoryRegion]) -> Result<NormalizedMap, MapError>;
pub enum MapError { TooManyRegions, NoUsableMemory, ReserveDoesNotFit, Address(kernel_types::Error) }
```

Algorithm: (1) split input into usable (`MemoryRegionKind::Usable`) and
excluded (every other kind); (2) usable regions shrink to frame boundaries
(start rounded up, end rounded down; drop empty), excluded regions grow to
frame boundaries (start down, end up); (3) sort usable by start, merge
overlapping and touching; (4) subtract every excluded range from the usable
list (a range inside a usable region splits it into two); (5) sort and
merge again; fail with `TooManyRegions` if the count exceeds
`MAX_BOOT_REGIONS` at any point; fail with `NoUsableMemory` if nothing
remains. `NormalizedMap` offers `iter()`, `total_frames()`, `is_sorted_and_disjoint()`
(used by tests and `debug_assert!`).

**`reserve.rs`.** `select_reserve(map: &NormalizedMap, override_bytes: u64)
-> Result<(PhysFrameRange, NormalizedMap), MapError>`: size = `override_bytes`
if non-zero (must be frame-aligned and below total RAM, else
`Address`), otherwise `clamp(total_ram / 16, 4 MiB, 64 MiB)` rounded up to
a frame; take the first usable region with at least that many frames,
carve the reserve from its start, return the reserve and the map without
those frames; `ReserveDoesNotFit` if no region is large enough.

**`frame_allocator.rs`.**

```rust
pub struct BitmapFrameAllocator { base: PhysFrame, count: u64, bits: [u64; RESERVE_WORDS], free: u64 }
pub const RESERVE_WORDS: usize = (64 * 1024 * 1024 / 4096) / 64;   // words for the largest reserve
pub enum FrameError { OutOfFrames, NotManaged, NotAllocated, AlreadyAllocated, InvalidCount }
impl BitmapFrameAllocator {
    pub fn new(range: PhysFrameRange) -> Result<Self, FrameError>;     // InvalidCount if larger than the bitmap
    pub fn allocate(&mut self) -> Result<PhysFrame, FrameError>;        // first zero bit
    pub fn allocate_contiguous(&mut self, count: u64, alignment: Alignment) -> Result<PhysFrameRange, FrameError>; // first fit
    pub fn free(&mut self, frame: PhysFrame) -> Result<(), FrameError>;
    pub fn free_contiguous(&mut self, range: PhysFrameRange) -> Result<(), FrameError>;
    pub fn free_count(&self) -> u64; pub fn capacity(&self) -> u64;
}
impl FrameSource for BitmapFrameAllocator { /* allocate_frame = allocate().ok(), release_frame = free ignored on error */ }
```

**`page_table.rs`.** Architecture-neutral interface plus the `x86_64`
format:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Permissions { pub write: bool, pub execute: bool, pub user: bool }
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum CachePolicy { WriteBack, Uncached }
pub trait EntryFormat: Copy + Default + 'static {
    const LEVELS: usize; const INDEX_BITS: u32; const ENTRIES: usize;
    fn index(level: usize, page: Page) -> usize;             // 9-bit slices of the page number
    fn is_present(self) -> bool;
    fn frame(self) -> Option<PhysFrame>;
    fn table(frame: PhysFrame) -> Self;                       // present | writable | user, pointing at a table
    fn leaf(frame: PhysFrame, perms: Permissions, cache: CachePolicy, global: bool) -> Self;
    fn permissions(self) -> Permissions;
    fn with_permissions(self, perms: Permissions) -> Self;
    fn validate(self) -> Result<(), EntryError>;              // reserved bits
    const EMPTY: Self;
}
#[repr(C, align(4096))] pub struct PageTable<F: EntryFormat> { pub entries: [F; 512] }
#[derive(Clone, Copy, Default, PartialEq, Eq)] pub struct X86Entry(u64);
```

`X86Entry` bits: PRESENT 0, WRITABLE 1, USER 2, WRITE_THROUGH 3, NO_CACHE 4,
ACCESSED 5, DIRTY 6, HUGE 7, GLOBAL 8, address bits 12..=51, NO_EXECUTE 63;
`validate` rejects bits 52..=62 set on a present entry and rejects HUGE (not
supported in the first release). `Permissions { write: false, execute: false }`
still maps readable; a read-only entry has WRITABLE clear and NO_EXECUTE set
unless `execute`.

**`mapper.rs`.**

```rust
pub struct Mapper<'a, F: EntryFormat, A: FrameAccess<PageTable<F>>, T: TlbControl, S: FrameSource> {
    root: PhysFrame, access: &'a mut A, tlb: &'a mut T, frames: &'a mut S }
pub enum MapError { AlreadyMapped, NotMapped, OutOfKernelMemory, UnreachableFrame, Entry(EntryError) }
pub enum Progress { Done, Partial(u64) }
impl Mapper {
    pub fn map(&mut self, page: Page, frame: PhysFrame, perms: Permissions, cache: CachePolicy) -> Result<(), MapError>;
    pub fn unmap(&mut self, page: Page) -> Result<PhysFrame, MapError>;
    pub fn protect(&mut self, page: Page, perms: Permissions) -> Result<(), MapError>;
    pub fn translate(&self, page: Page) -> Option<(PhysFrame, Permissions)>;
    pub fn map_range(&mut self, pages: PageRange, first_frame: PhysFrame, perms: Permissions, cache: CachePolicy, budget: u64) -> Result<Progress, MapError>;
    pub fn unmap_range(&mut self, pages: PageRange, budget: u64) -> Result<Progress, MapError>;
}
```

`map` walks levels 3..0: for a missing table, allocate a frame from
`frames`, write `PageTable::default()` through `access.table_mut(frame)`
(`UnreachableFrame` if `None`), install `F::table(frame)`. Keep the frames
allocated in this call in a fixed array; on any failure free them in
reverse and clear the entries they were installed in (rollback), then
return the error. Installing the leaf into an occupied entry is
`AlreadyMapped`. After a successful change call `tlb.flush_page(page)`
exactly once. `unmap` clears the leaf, then frees each intermediate table
whose 512 entries are all empty, from the lowest level upward. Range
operations stop after `budget` pages and return `Partial(done)`; `budget`
is `MAX_PAGES_PER_CALL` in the kernel. The kernel-half `global` bit is set
when the page is a kernel page.

**`address_space.rs`.**

```rust
pub struct Region<B: Copy> { pub pages: PageRange, pub backing: B, pub offset: u64, pub perms: Permissions }
pub struct RegionTable<B: Copy, const N: usize> { regions: [Option<Region<B>>; N], len: usize }
pub enum RegionError { AddressInUse, NotMapped, QuotaExceeded, OutsideUserSpace, Unaligned, Overflow }
impl RegionTable {
    pub fn insert(&mut self, region: Region<B>) -> Result<(), RegionError>;         // sorted insert, overlap check against neighbors
    pub fn remove(&mut self, pages: PageRange) -> Result<Removed<B, 3>, RegionError>; // may split one region into two; needs one free slot
    pub fn protect(&mut self, pages: PageRange, perms: Permissions) -> Result<(), RegionError>; // may split into three; needs two free slots
    pub fn find(&self, page: Page) -> Option<&Region<B>>;
    pub fn iter(&self) -> impl Iterator<Item = &Region<B>>;
    pub fn check_invariants(&self) -> bool;
}
```

Every range must satisfy: user half, start at or above
`USER_SPACE_START`, non-empty. The kernel address space uses the same type
with the check relaxed through a constructor flag `RegionTable::kernel()`.

**`strategies.rs`** (feature `test-strategies`): `any_memory_regions()`
(lists of 0..=64 regions with random kinds and lengths, including
overlapping and unaligned ones), `any_permissions()`, `any_region_table_op()`.

Tests: catalog 6.6.2, 6.6.3, 6.6.4 (including the model-based test with
`MemoryFrameAccess<PageTable<X86Entry>>`, `RecordingTlb`, and
`CountingFrameSource` from `kernel-hal-api::doubles`), 6.6.5. For the
model test of the mapper, extend `MemoryFrameAccess` with
`fn with_lazy_tables(ram: PhysFrameRange) -> Self` that materializes a
default table on first `table_mut` for any frame inside `ram`, so that
freshly allocated frames are reachable like in the kernel.

### 10.1.4 Policy and documents

- `policy::CRATES`: add `kernel-objects` (deps `kernel-types`,
  `audhsos-abi`, `test-support`) and `kernel-mm` (deps `kernel-types`,
  `kernel-hal-api`, `audhsos-abi`, `test-support`).
- Catalog rows in 05 for both crates; changelog entries.

### 10.1.5 Acceptance

`~/.cargo/bin/cargo xtask check` passes; the coverage table shows both new
crates above the thresholds; every item of 6.6.2 to 6.6.6 (pool items) and
6.6.10 has a test whose name identifies the item.

## 10.2 Phase 2: Loader, boot, and test harness

Goal: `~/.cargo/bin/cargo xtask test --qemu` boots test kernels through the
project's own UEFI loader and reports over the serial line.

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

Decode with `u16::from_le_bytes`/`u32`/`u64` from slices obtained with
`get(..)`. Checks: magic `7F 45 4C 46`, `EI_CLASS == 2`, `EI_DATA == 1`,
`e_type == 2`, `e_machine == 0x3E`, `e_ehsize >= 64`, `e_phentsize >= 56`,
`e_phoff + e_phnum * e_phentsize <= len` (checked arithmetic), at least one
`PT_LOAD` (`p_type == 1`), `p_filesz <= p_memsz`, `p_offset + p_filesz <=
len`, `p_align` zero or a power of two and `p_vaddr % p_align == p_offset %
p_align` when non-zero, segment inside the constraints, no two load
segments overlap in memory, never both `PF_W` (2) and `PF_X` (1), entry
inside an executable segment. Segments are returned sorted by `vaddr`.

Tests: catalog 6.6.13 ELF items with an `ElfBuilder` in the tests that
assembles headers byte by byte; a generator `any_elf_bytes()` behind
`test-strategies` that mutates valid images.

### 10.2.2 Crate `audhsos-uefi` (`crates/uefi`)

Layer 0, `no_std`, `forbid(unsafe_code)`. Only `#[repr(C)]` structures,
function-pointer types, constants, and pure helpers. No calls.

Define, with the field order of the UEFI 2.10 specification (consult the
specification for every offset and write a layout test for each structure):
`Status` (`usize`; `SUCCESS = 0`; error bit `1 << 63`), `Handle`
(`*mut c_void`), `Guid { data1: u32, data2: u16, data3: u16, data4: [u8; 8] }`,
`TableHeader` (24 bytes), `SystemTable`, `BootServices` (all 44 function
slots in specification order, typed as `unsafe extern "efiapi" fn`
pointers with the exact signatures for the ten services the loader calls
and as `usize` for the rest), `ConfigurationTable`, `MemoryDescriptor`
(40 bytes: `type_: u32`, `physical_start: u64`, `virtual_start: u64`,
`pages: u64`, `attribute: u64`), `MemoryType` (`from_u32` with the 16
standard values), `AllocateType`, `LoadedImageProtocol`,
`SimpleFileSystemProtocol`, `FileProtocol`, `FileInfo`,
`SimpleTextOutputProtocol`, and the GUIDs `ACPI_20_TABLE`,
`LOADED_IMAGE_PROTOCOL`, `SIMPLE_FILE_SYSTEM_PROTOCOL`, `FILE_INFO`.

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

Tests: catalog 6.6.14 structure and conversion items.

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

Include `RecordingRegisters` behind `#[cfg(any(test, feature = "test-doubles"))]`.
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
- `descriptors.rs` (pure, host-testable through `cfg(not(target_os =
  "none"))`? No: keep it `no_std` and test it in QEMU plus a duplicate-free
  host test by making the module a separate logic crate
  `kernel-x86-tables` (`crates/kernel/x86-tables`, layer 1, deps none): GDT
  entry encoding, IDT entry encoding, TSS layout. Then the adapter depends
  on it.) Encodings: GDT `0x00AF9A000000FFFF` kernel code,
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
pub struct Harness<C: DebugConsole, E: TestExit> { console: C, exit: E, should_panic: bool }
impl Harness { pub fn run(&mut self, tests: &[&dyn Testable]) -> !; pub fn on_panic(&mut self, info: &PanicInfo) -> !; }
```

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
`boot::run<P: Platform, C: DebugConsole, E: TestExit>(platform: &P,
console: &mut C, exit: &mut E) -> !`: prints the banner and the memory
regions, then halts (in test builds the test kernel calls the harness
instead). `trap::Exception` and `trap::on_exception` printing the
exception and calling `exit(Failure)`. The `Global<KernelState>` cell is
declared here (empty state in Phase 2).

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
`[target.x86_64-unknown-none] rustflags = ["-C", "relocation-model=static"]`
and `runner = "cargo xtask qemu-runner"`.

Test kernels under `crates/kernel/bin/tests/`: each file uses
`#![feature(custom_test_frameworks)]`, `#![test_runner(run_tests)]`,
`#![reexport_test_harness_main = "test_main"]`, its own `kernel_entry`
wiring (through a shared `kernel_test_harness::entry!` macro so that the
wiring is written once), and `#[test_case]` functions. Phase 2 kernels:
`boot.rs` (banner, exit success), `console.rs` (writes a marker string the
runner asserts), `exceptions.rs` (breakpoint returns; page fault at
`0xdead_beef` reports that address through a handler hook; divide error;
invalid opcode; general protection), `double_fault.rs` (`should_panic`:
infinite recursion overflows the kernel stack; the double-fault handler
runs on IST 1 and panics), `bad_kernel/` and `missing_image/` loader
images built by the xtask from a corrupt ELF and an empty volume.

### 10.2.8 Crate `boot-uefi-x86_64` (`crates/boot/uefi-x86_64`)

Adapter crate, target `x86_64-unknown-uefi`, `#![no_std]`, `#![no_main]`,
`#![allow(unsafe_code)]`, `panic = "abort"`, no `alloc`. Deps:
`audhsos-abi`, `audhsos-elf`, `audhsos-uefi`, `kernel-types`, `kernel-mm`,
`kernel-hal-api`.

Entry: `#[unsafe(no_mangle)] pub extern "efiapi" fn efi_main(image: Handle,
system_table: *const SystemTable) -> Status`. Modules:

- `firmware.rs`: `Firmware<'a> { image: Handle, table: &'a SystemTable }`
  with one method per service, each containing exactly one `unsafe` block
  that calls the function pointer: `allocate_pages(count) ->
  Result<PhysFrameRange, Status>`, `free_pages`, `memory_map(buffer: &mut
  [u8]) -> Result<MemoryMapInfo { size, key, descriptor_size }, Status>`
  (retry with a larger buffer on `BUFFER_TOO_SMALL`; the buffer is one
  firmware-allocated region of 16 pages), `handle_protocol<T>(handle,
  guid) -> Result<&T, Status>`, `exit_boot_services(key)`,
  `output_string(&[u16])`, `configuration_table(guid) -> Option<PhysAddr>`.
- `files.rs`: open the loaded image's device (`LoadedImageProtocol::device_handle`
  → `SimpleFileSystemProtocol::open_volume`), `read_file(name: &str) ->
  Result<&'static [u8], LoadError>`: `open` with mode read, `get_info`
  with the `FILE_INFO` GUID to learn the size, allocate pages, `read`
  until the size is reached, `close`. Buffers become slices through one
  `unsafe` block each.
- `placement.rs` (pure where possible): from the parsed kernel ELF compute
  for each segment the frame count, allocate frames, copy `file_size`
  bytes, zero the rest (`slice::fill`), record `(PageRange, PhysFrameRange,
  Permissions)` per segment.
- `paging.rs`: `IdentityAccess` implementing `FrameAccess<PageTable<X86Entry>>`
  by casting the frame address to `&mut PageTable` (one `unsafe` block;
  precondition: the firmware identity-maps all memory while boot services
  run), `FirmwareFrames` implementing `FrameSource` over `allocate_pages(1)`,
  `NoTlb` implementing `TlbControl` as a no-op. Build the tables with
  `kernel_mm::Mapper`: (1) the physical window: every byte of every region
  in the memory map from `0` to the highest region end, mapped at
  `PHYS_WINDOW_BASE + phys`, read/write, no-execute, global; (2) the kernel
  segments at their `vaddr` with their permissions, global; (3) the boot
  stack (16 pages plus one unmapped guard page below) at
  `KERNEL_BASE - 0x100_0000` (a fixed constant `BOOT_STACK_TOP` in
  `audhsos-abi`), read/write, no-execute; (4) the boot information page at
  `BOOT_INFO_VADDR` (constant in `audhsos-abi`), read-only; (5) an identity
  mapping of the same memory as (1) at `phys` (so that the loader keeps
  running after the `CR3` switch), read/write/execute.
- `bootinfo.rs`: fill a page with `BootInfoWriter` after
  `exit_boot_services`; the memory map used is the final one obtained
  immediately before the call; loader code and data are reported as
  `Usable`.
- `entry.rs`: one naked function `enter_kernel(cr3: u64, stack_top: u64,
  boot_info: u64, entry: u64) -> !`: `mov cr3, rdi; mov rsp, rsi; mov rdi,
  rdx; jmp rcx`.
- `exit.rs`: on any error, `output_string` a diagnostic and `outl(0xF4,
  0x12)` (one `asm!`), then loop on `hlt`.

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
  backup array at `last - 33 ..= last - 1`, backup header at `last` with
  swapped current/backup LBAs), `fat32.rs` (512-byte sectors, 1 sector per
  cluster, 32 reserved sectors, 2 FATs, FSInfo at sector 1, backup boot
  sector at 6, root directory cluster 2, media `0xF8`, end-of-chain
  `0x0FFF_FFFF`, cluster count at least 65525, deterministic timestamps
  `2026-01-01 00:00:00`, 8.3 names uppercase, directories `EFI`, `EFI/BOOT`,
  `AUDHSOS` with `.` and `..` entries), plus a reader used only by the
  tests. Default image size 64 MiB, sparse file.
- `qemu.rs`: locate `qemu-system-x86_64` (`AUDHSOS_QEMU` or `PATH`) and the
  firmware (`AUDHSOS_OVMF` or `<qemu dir>/../share/qemu/edk2-x86_64-code.fd`);
  the command line of
  [03-target-platform.md 3.1.1](03-target-platform.md#311-reference-machine-configuration)
  with `-serial stdio -display none -no-reboot`; timeout 60 s
  (`AUDHSOS_QEMU_TIMEOUT`) implemented with a thread that kills the child;
  capture stdout; parse the protocol; map exit status 33/35/37; print the
  captured output on failure.
- `qemu-runner <elf>`: build the loader if needed, write a disk image with
  the given kernel ELF and the boot image, run QEMU, exit 0 on success.
- `test --qemu`: `cargo test -p audhsos-kernel --target x86_64-unknown-none`
  (Cargo invokes the runner for every test kernel) plus the loader failure
  images.
- `run`: `build`, `image`, then QEMU without timeout and with the serial
  console attached to the terminal.
- Tests: catalog 6.6.15 and the runner items of 6.6.20.

### 10.2.10 Policy and documents

`policy::CRATES` entries: `audhsos-elf` (Logic, deps none),
`audhsos-uefi` (Logic, deps `audhsos-abi`), `kernel-x86-tables` (Logic,
deps none), `driver-uart16550` (Logic, deps none), `kernel-core` (Logic,
deps `kernel-types`, `kernel-hal-api`, `kernel-mm`, `kernel-objects`,
`audhsos-abi`, `audhsos-sync`), `kernel-hal-x86_64` (Adapter,
`X86_64None`, deps `kernel-hal-api`, `kernel-types`, `audhsos-abi`,
`driver-uart16550`, `audhsos-sync`, `kernel-x86-tables`, `kernel-mm`),
`kernel-test-harness` (Logic, `X86_64None`, deps `kernel-hal-api`),
`audhsos-kernel` (Logic, `X86_64None`, deps `kernel-core`,
`kernel-hal-x86_64`, `kernel-test-harness`, `audhsos-abi`),
`boot-uefi-x86_64` (Adapter, `X86_64Uefi`, deps as in 10.2.8). Update the
catalog in 05, the allowlist and inventory in 04, `rust-toolchain.toml`
already lists both targets. Record `BOOT_STACK_TOP` and `BOOT_INFO_VADDR`
in 02 and 03.

### 10.2.11 Acceptance

`~/.cargo/bin/cargo xtask check` (now including `test --qemu`) passes; at
least eight test kernels and three loader images run; `~/.cargo/bin/cargo
xtask run` shows the banner on the terminal.

## 10.3 Phase 3: Kernel memory bring-up

Goal: the kernel owns its memory after boot.

- `kernel-hal-x86_64::window`: `PhysicalWindow { base: VirtAddr }` with
  `frame_bytes_mut(&mut self, frame: PhysFrame) -> &mut [u8; 4096]` (one
  `unsafe` block; precondition: the window maps all RAM read/write and the
  kernel is single-threaded) and `impl FrameAccess<PageTable<X86Entry>>`
  built on it (a second `unsafe` block for the typed reference; alignment
  holds because frames are 4 KiB aligned). `TlbControl` adapter:
  `invlpg` per page, `write_cr3(read_cr3())` for `flush_all`.
  `activate(root: PhysFrame)` writes `CR3`.
- `kernel-core::memory`: from the platform regions: `normalize`,
  `select_reserve` (override from the boot image header, which the kernel
  reads through the window at `boot_image_phys_start`),
  `BitmapFrameAllocator` over the reserve, then the object pools sized by
  constants in `kernel-core::config` (`PROCESSES = 256`, `THREADS = 1024`,
  `MEMORY_OBJECTS = 4096`, `ENDPOINTS = 1024`, `NOTIFICATIONS = 1024`,
  `REPLIES = 1024`, `INTERRUPTS = 64`, `IO_PORT_RANGES = 64`, `KERNEL_STACKS
  = 1024`, regions per process 64, handles per process up to `1 << 16`).
  Pools are `static` `Global<Pool<..>>` cells, initialized in place.
- Adopt the loader's tables: `root = read_cr3()`; walk the kernel half
  with the mapper's `translate` to register the kernel image, the window,
  the boot stack, and the boot information page in the kernel
  `RegionTable`; unmap the identity range (every page below
  `USER_SPACE_END` that is present) with `unmap_range` in `MAX_PAGES_PER_CALL`
  steps and free nothing (the frames belong to the window).
- Kernel stacks: `StackPool` in `kernel-mm`: `allocate() ->
  Result<KernelStack { pages: PageRange }, _>` mapping 4 frames from the
  reserve below a guard page at `KERNEL_STACKS_BASE` (constant) + index ×
  5 pages; `release`.
- QEMU tests (`memory.rs`): allocate every reserve frame and free them;
  map a frame at a user page, write through the window, read through the
  mapping, unmap, and verify that a read faults (handler hook records the
  address); the identity mapping is gone (`translate` of page 0x1000 is
  `None`).

Acceptance: `check` green; catalog 6.6.21 memory items covered.

## 10.4 Phase 4: Interrupts and timer

- `kernel-hal-x86_64::acpi` as a logic module in a new crate
  `kernel-acpi` (`crates/kernel/acpi`, layer 1, deps `kernel-types`):
  `parse_rsdp(bytes: &[u8; 36]) -> Result<Rsdp, AcpiError>` (signature
  `RSD PTR `, checksum over 20 bytes, revision 2 adds length, XSDT address,
  extended checksum over `length` bytes), `Sdt::parse(bytes) ->
  Result<SdtHeader, _>` (signature, length ≥ 36, checksum over `length`),
  `madt::parse(bytes) -> Result<Madt, _>` with `Madt { lapic_address: u64,
  io_apics: [Option<IoApic>; 4], overrides: [Option<Override>; 16] }`, entry
  types 0 (processor local APIC: ignored beyond counting), 1 (I/O APIC: id,
  address, GSI base), 2 (interrupt source override: bus, source IRQ, GSI,
  flags), 5 (local APIC address override); entry length zero is an error;
  unknown types skipped by their length. The adapter reads the tables
  through the window: RSDP page, then XSDT/RSDT, then each entry until the
  `APIC` signature.
- `kernel-hal-x86_64::apic`: register layouts in `kernel-x86-tables`
  (`lapic::{ID = 0x20, VERSION = 0x30, TPR = 0x80, EOI = 0xB0, SVR = 0xF0,
  LVT_TIMER = 0x320, LVT_LINT0 = 0x350, LVT_LINT1 = 0x360, LVT_ERROR = 0x370,
  TIMER_INITIAL = 0x380, TIMER_CURRENT = 0x390, TIMER_DIVIDE = 0x3E0}`,
  `ioapic::{IOREGSEL = 0x00, IOWIN = 0x10, ID = 0, VERSION = 1,
  REDIRECTION_BASE = 0x10}` and the redirection entry encoding with tests).
  `LocalApic` over the window (volatile reads and writes through one
  `unsafe` block each for `read_register`/`write_register`): enable with
  SVR bit 8 and spurious vector `0xFF`, `end_of_interrupt` writes `0` to
  EOI. `IoApic`: `route(line, vector)` writes the redirection entry (vector,
  fixed delivery, physical destination, level/polarity from the override
  flags, masked), `mask`/`unmask` toggle bit 16; `InterruptController`
  implementation resolves ISA lines through the overrides.
- Legacy PIC: remap to vectors `0x20..0x2F` (ICW1 `0x11` to `0x20`/`0xA0`,
  offsets `0x20`/`0x28` to `0x21`/`0xA1`, cascade `0x04`/`0x02`, ICW4
  `0x01`), then mask everything (`0xFF` to `0x21` and `0xA1`).
- Vector plan (constants in `kernel-hal-x86_64::vectors`): exceptions
  `0..=31`, PIC spurious `0x20..=0x2F`, LAPIC timer `0x30`, I/O APIC lines
  `0x40 + gsi`, system call `0x80`, LAPIC spurious `0xFF`.
- Timer: PIT calibration of the LAPIC timer: divide by 16
  (`TIMER_DIVIDE = 0x3`), initial count `u32::MAX`, PIT channel 2 one-shot
  for 10 ms (`0x61` gate bit 0 set and bit 1 clear, command `0xB2` to
  `0x43`, count 11932 to `0x42` low then high, poll `0x61` bit 5), read
  the current count, compute ticks per millisecond, program periodic mode
  (LVT bit 17) with the vector and `TIMER_INITIAL = ticks_per_ms *
  1000 / TICKS_PER_SECOND`. `Timer::ticks` counts in `kernel-core`.
- `kernel-core::tick`: increments the tick counter and calls the scheduler
  hook (empty until Phase 5).
- QEMU tests (`interrupts.rs`): the timer tick counter increases; a second
  tick arrives after EOI; masking the timer stops it; the spurious vector
  handler runs when triggered by software (`int 0xFF`... not possible
  without asm: instead assert that the spurious handler is installed and
  that a software-triggered `int 0x30` reaches the timer handler).
- Fuzz target `madt` under `fuzz/` with `fuzz-support` (see 10.7.6 for
  the fuzz crate layout; create it in this phase).

Acceptance: `check` green; catalog 6.6.11, 6.6.16 APIC items, 6.6.21
interrupt items.

## 10.5 Phase 5: Objects, threads, user mode, system calls

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
  `Syscall::argument_count`, `Syscall::object_type() -> Option<ObjectType>`
  (`Any` means a handle of any type), and a `dispatch!` helper the kernel
  uses to build its `match`. The userland wrappers in Phase 7 consume the
  same table.
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

`handle_table.rs`: `HandleTable<const N: usize>` with `Entry { object:
AnyObjectId, rights: Rights, badge: u64 }` where `AnyObjectId` is an enum
over the typed ids; `insert`, `lookup(handle) -> Result<&Entry,
Error::InvalidHandle>` checking index and generation, `duplicate(handle,
rights)`, `close(handle)`, FIFO slot reuse, generation increment skipping
0. Object structs: `Process { address_space: AddressSpaceId, handles:
HandleTable, threads: [Option<ThreadId>; 64], quota: Quota, fault_handler:
Option<EndpointId>, kernel_object_quota: Quota }`, `Thread { process,
state: ThreadState, priority, max_priority, time_slice, kernel_stack,
ipc_buffer: PhysFrame, ipc_state, queue_links: Links, context: ArchContext
}` where `ArchContext` is an associated type of the HAL `Context` trait,
`MemoryObject { frames: PhysFrameRange, kind: MemoryKind, cache }`.

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

- `trait Context { type Saved; fn initial_user(entry: VirtAddr, stack: VirtAddr, kernel_stack_top: VirtAddr) -> Self::Saved; fn switch(from: &mut Self::Saved, to: &Self::Saved); }`.
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

Layer 3. `dispatch(state: &mut KernelState, caller: ThreadId, buffer: &mut
[u8; 4096]) -> ()` implementing the validation order of
[02-architecture.md 2.8](02-architecture.md#28-system-call-interface):
number → argument count → handle → type → rights → arguments → quota;
then one function per system call in `calls/*.rs`; every error path
tested on the host with `KernelState` built from doubles. Phase 5
implements the process, thread, memory, handle, and `debug_log` calls;
Phase 6 the rest (unimplemented ones return `Unsupported` until then).

### 10.5.6 User-mode test programs

`crates/user/sys-x86_64` (adapter, target `X86_64None`): `_start`
(`extern "C"`, no runtime, calls `main`), `syscall()` with one
`asm!("int 0x80")`, and nothing else in this phase. `crates/user/test-programs`
with one binary per scenario (`thread_exit`, `two_threads`, `preempt`,
`read_kernel_memory`, `hlt_in_user`, `every_syscall_error`), linked at
`0x40_0000` with a linker script, converted to flat binaries by the xtask
(`build-user-tests`, using `llvm-objcopy -O binary`) into
`target/user-tests/<name>.bin`. Test kernels embed them with
`include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/<name>.bin"))`;
the xtask sets the variable when it builds test kernels. A test kernel
creates a process, maps the flat binary, a stack, and an IPC buffer, starts
the thread, and observes the outcome through the kernel state.

Acceptance: `check` green; catalog 6.6.6 handle items, 6.6.7, 6.6.9,
6.6.21 thread, isolation, and system call items.

## 10.6 Phase 6: IPC and interrupt forwarding

- `kernel-objects`: `Endpoint { senders: Queue, receivers: Queue }`,
  `Reply { caller: ThreadId, consumed: bool }`, `Notification { word: u64,
  waiter: Option<ThreadId>, bound_interrupt: Option<InterruptId> }`,
  `Interrupt { line, vector, notification: Option<(NotificationId, u8)>,
  masked: bool }`, `IoPortRange { first: u16, count: u16 }`,
  `SystemControl`.
- `kernel-ipc` (`crates/kernel/ipc`, layer 3): the rendezvous state
  machine over the pools and the scheduler: `call`, `send`, `recv`,
  `try_recv`, `reply`, `reply_recv`, `signal`, `wait`, `poll`; message
  transfer `transfer(from: &[u8; 4096], to: &mut [u8; 4096], sender_table,
  receiver_table) -> TransferResult` copying `word_count` words and
  installing up to four handles (rights check `TRANSFER`, receiver table
  space, truncation flag); queue ordering by priority then FIFO; cancellation
  on kill; wake-ups on object destruction with `ObjectDestroyed`. Every
  operation takes the kernel state and returns `Result<Outcome, Error>`
  where `Outcome` says whether the caller blocks and which threads became
  ready; the syscall layer applies it.
- Fault delivery: the exception handler for user-mode faults builds the
  fault message in the faulting thread's IPC buffer (label
  `FAULT_LABEL_BASE + kind`, words: address, ip, error code) and performs
  `call` on the process's fault handler endpoint; `reply` resumes. No
  handler: state `Faulted`.
- Interrupt objects: the device-vector handler looks up the interrupt by
  vector, masks the line, sends EOI, signals the bound notification bit;
  `interrupt_ack` unmasks.
- `SystemControl` calls: `interrupt_create`, `ioport_create`,
  `memory_create_device`, `system_info` (pool capacities and usage, tick
  frequency, RSDP).
- QEMU tests (`ipc.rs`): call/reply between two user threads (user test
  programs `ipc_client`, `ipc_server`), handle transfer, notification from
  the timer interrupt bound to a notification, fault message delivery for
  a kernel-address read, `hlt` in user mode.

Acceptance: `check` green; catalog 6.6.8, 6.6.21 IPC items.

## 10.7 Phase 7: Userland foundation

### 10.7.1 `user-sys-x86_64` (complete)

`_start` receives the initial `rsp` from the kernel with the IPC buffer
address in the startup message; `syscall(buffer: &mut IpcBuffer)`
(`int 0x80`); `GlobalAlloc` adapter `HeapAdapter` over `user_rt::heap::Allocator`
with `wrapping_add` on the base pointer obtained when the heap memory
object is mapped; the panic handler delegates to `user_rt::panic::report`.

### 10.7.2 `user-rt` (`crates/user/rt`, logic)

- Typed handles: `ProcessHandle`, `ThreadHandle`, `MemoryHandle`,
  `EndpointHandle`, `ReplyHandle`, `NotificationHandle`,
  `InterruptHandle`, `IoPortHandle`, `SystemControlHandle`, each a newtype
  over `Handle` with `Drop` calling `handle_close` and methods generated
  from the `syscalls!` table by a second macro `wrappers!`.
- `heap.rs`: the offset-based allocator: `Allocator { arena_len, classes:
  [FreeList; 8] (16, 32, 64, 128, 256, 512, 1024, 2048 bytes), large:
  fixed table of free extents }`; blocks larger than 2048 bytes come from
  the extent table with first-fit and coalescing on release; the
  bookkeeping is a fixed table of `MAX_BLOCKS = 4096` entries; failure is
  `Err(OutOfMemory)`; `grow(new_len)` extends the arena.
- `message.rs`: builder and parser over the IPC buffer layout;
  `startup.rs`: the startup message (`label = STARTUP`, words: handle
  numbers by role); `log.rs`: `log!` macro sending to the log endpoint;
  `panic.rs`: report and `thread_exit`.

### 10.7.3 `user-proto` (`crates/user/proto`)

Name protocol: `Register { name: [u8; 32], endpoint handle }`,
`Lookup { name } -> endpoint`; console protocol: `Write { bytes in words }`,
`Read { max } -> bytes`; memory protocol: `Allocate { len, align } ->
memory handle`, `Release { handle }`; each message a struct with
`encode(&self, &mut Message)` and `decode(&Message) -> Result`, labels as
constants, versions in the label's high 16 bits.

### 10.7.4 `user-loader` (`crates/user/loader`)

`tar.rs`: ustar reader over `&[u8]`: 512-byte headers, octal size field,
checksum (sum of the header with the checksum field as spaces), `ustar\0`
magic, prefix field for long names, reject `..` components and absolute
paths, iterate entries; `process.rs`: create a process from an ELF using
`audhsos-elf` with user constraints and the memory server: for each
segment allocate, map into the loader, copy, unmap, map into the child;
allocate stack (16 pages) and IPC buffer; install handles; write the
startup message; start.

### 10.7.5 Servers and the application

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
- `app-hello`: looks up `console`, writes `hello from userland`, exits.
- Root task as flat binary: linker script at `ROOT_TASK_BASE` with `.bss`
  inside the file; the xtask converts with `llvm-objcopy -O binary`.
- The xtask `image` writes the real boot image: header, root task, ustar
  archive of the server and application ELFs.
- Release build: `debug-uart` and `test-exit` off; `~/.cargo/bin/cargo
  xtask run --release` shows the greeting through the userland driver.

### 10.7.6 Fuzzing

`crates/support/fuzz` (`fuzz-support`, adapter, host): defines
`#[unsafe(no_mangle)] pub extern "C" fn LLVMFuzzerTestOneInput(data: *const u8,
len: usize) -> i32` once through a macro `fuzz_target!(|bytes: &[u8]| { ...
})` that builds the slice in one `unsafe` block. `fuzz/` holds one binary
crate per target (`elf`, `tar`, `madt`, `boot_image_header`, `boot_info`,
`message`), built by `xtask fuzz` with `RUSTFLAGS=-Zsanitizer=fuzzer` and
`--release`; corpora under `fuzz/corpus/<target>/`; every crash becomes a
regression test in the parser's crate. Register the targets in
`policy::FUZZ_TARGETS`.

### 10.7.7 Acceptance

`check` green; `test --e2e` passes with the userland test programs
reporting through the console driver; catalog 6.6.12, 6.6.13 tar items,
6.6.22, 6.6.23; fuzz targets run for 60 seconds each without findings.

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
