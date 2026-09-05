// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The memory bring-up: the kernel takes its reserve out of the map the
//! loader reported, adopts the page tables the loader built, and drops the
//! identity mapping the loader needed and the kernel does not.
//!
//! Invariants: the bring-up touches the machine only through the traits it
//! is given, so that it runs unchanged on the host and in QEMU; every frame
//! the reserve holds is either free in the allocator or handed out by it;
//! the identity mapping is gone before the bring-up reports success, and no
//! frame it named is given back, because those frames belong to the memory
//! the window maps.

use core::fmt;

use audhsos_abi::boot_image::BootImageHeader;
use audhsos_abi::layout::{
    BOOT_INFO_VADDR, BOOT_STACK_PAGES, BOOT_STACK_TOP, KERNEL_BASE, MAX_PAGES_PER_CALL,
    MAX_PHYS_WINDOW_BYTES, PAGE_SIZE, PHYS_WINDOW_BASE, USER_SPACE_END,
};
use audhsos_sync::{Global, UncontendedToken};
use kernel_hal_api::console::DebugConsole;
use kernel_hal_api::paging::{FrameAccess, FrameSource, TlbControl};
use kernel_hal_api::platform::{MemoryRegionKind, Platform};
use kernel_mm::address_space::{Region, RegionError, RegionTable};
use kernel_mm::frame_allocator::{BitmapFrameAllocator, FrameError};
use kernel_mm::mapper::{MapError, Mapper, Progress};
use kernel_mm::memory_map::{MapError as MemoryMapError, NormalizedMap, normalize};
use kernel_mm::page_table::{EntryFormat, PageTable};
use kernel_mm::reserve::select_reserve;
use kernel_mm::stack::{KernelStack, StackError, StackPool};
use kernel_types::{Page, PageRange, PhysAddr, PhysFrame, VirtAddr};

use crate::config::{KERNEL_IMAGE_MAX_PAGES, KERNEL_REGIONS, KERNEL_STACK_WORDS};
use crate::println;

/// What a range of the kernel address space holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KernelBacking {
    /// A run of mapped pages of the kernel image.
    Image,
    /// The window every physical frame is reachable through.
    Window,
    /// The stack the loader set up and the kernel boots on.
    BootStack,
    /// The page the loader wrote the boot information into.
    BootInfo,
}

/// The regions of the kernel address space.
pub type KernelRegions = RegionTable<KernelBacking, KERNEL_REGIONS>;

/// The pool of kernel stack slots.
pub type KernelStacks = StackPool<KERNEL_STACK_WORDS>;

/// Why the memory bring-up could not finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MemoryError {
    /// The firmware map could not be normalized, or no reserve fits.
    Map(MemoryMapError),
    /// The reserve could not be turned into an allocator.
    Frames(FrameError),
    /// A region of the kernel address space could not be registered.
    Region(RegionError),
    /// A page table could not be read or written.
    Paging(MapError),
    /// A range of the kernel address space is not addressable.
    Address,
    /// The machine has more memory than the window can map.
    WindowTooLarge(u64),
    /// The boot information page has no translation, so the loader did not
    /// map it where the layout says.
    NoBootInfo,
    /// The bring-up has already run.
    AlreadyDone,
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryError::Map(error) => write!(f, "{error}"),
            MemoryError::Frames(error) => write!(f, "{error}"),
            MemoryError::Region(error) => write!(f, "{error}"),
            MemoryError::Paging(error) => write!(f, "{error}"),
            MemoryError::Address => f.write_str("a kernel range is not addressable"),
            MemoryError::WindowTooLarge(bytes) => write!(
                f,
                "the machine has {bytes} bytes of memory, more than the window maps"
            ),
            MemoryError::NoBootInfo => {
                write!(f, "nothing is mapped at {BOOT_INFO_VADDR:#x}")
            }
            MemoryError::AlreadyDone => f.write_str("the memory bring-up has already run"),
        }
    }
}

impl From<MemoryMapError> for MemoryError {
    fn from(error: MemoryMapError) -> Self {
        MemoryError::Map(error)
    }
}

impl From<FrameError> for MemoryError {
    fn from(error: FrameError) -> Self {
        MemoryError::Frames(error)
    }
}

impl From<RegionError> for MemoryError {
    fn from(error: RegionError) -> Self {
        MemoryError::Region(error)
    }
}

impl From<MapError> for MemoryError {
    fn from(error: MapError) -> Self {
        MemoryError::Paging(error)
    }
}

/// What the kernel owns after the bring-up.
#[derive(Debug)]
pub struct KernelMemory {
    root: PhysFrame,
    frames: BitmapFrameAllocator,
    free: NormalizedMap,
    regions: KernelRegions,
    stacks: KernelStacks,
    boot_info: PhysFrame,
    identity_pages: u64,
}

impl KernelMemory {
    /// The frame the kernel page tables are rooted in.
    #[must_use]
    pub const fn root(&self) -> PhysFrame {
        self.root
    }

    /// The allocator over the kernel reserve.
    #[must_use]
    pub const fn frames(&self) -> &BitmapFrameAllocator {
        &self.frames
    }

    /// The allocator over the kernel reserve, for modification.
    pub const fn frames_mut(&mut self) -> &mut BitmapFrameAllocator {
        &mut self.frames
    }

    /// The usable memory the reserve left, which becomes memory objects of
    /// the root task in Phase 5.
    #[must_use]
    pub const fn free(&self) -> &NormalizedMap {
        &self.free
    }

    /// The regions of the kernel address space.
    #[must_use]
    pub const fn regions(&self) -> &KernelRegions {
        &self.regions
    }

    /// The regions of the kernel address space, for modification.
    pub const fn regions_mut(&mut self) -> &mut KernelRegions {
        &mut self.regions
    }

    /// The pool of kernel stack slots, for modification.
    pub const fn stacks_mut(&mut self) -> &mut KernelStacks {
        &mut self.stacks
    }

    /// Takes a slot of the kernel stack area and maps its pages with
    /// frames of the reserve.
    ///
    /// The stack area is one constant range of the kernel address space
    /// and the pool answers which of its slots are taken, so a stack is
    /// not a region of the region table: there are more slots than the
    /// table has rows.
    ///
    /// # Errors
    ///
    /// The errors of [`StackPool::allocate`].
    pub fn allocate_stack<F, A, T>(
        &mut self,
        access: &mut A,
        tlb: &mut T,
    ) -> Result<KernelStack, StackError>
    where
        F: EntryFormat,
        A: FrameAccess<PageTable<F>>,
        T: TlbControl,
    {
        let root = self.root;
        let mut mapper =
            Mapper::<'_, F, A, T, BitmapFrameAllocator>::new(root, access, tlb, &mut self.frames);
        self.stacks.allocate(&mut mapper)
    }

    /// Unmaps the pages of `stack`, gives their frames back, and frees the
    /// slot.
    ///
    /// # Errors
    ///
    /// The errors of [`StackPool::release`].
    pub fn release_stack<F, A, T>(
        &mut self,
        access: &mut A,
        tlb: &mut T,
        stack: KernelStack,
    ) -> Result<(), StackError>
    where
        F: EntryFormat,
        A: FrameAccess<PageTable<F>>,
        T: TlbControl,
    {
        let root = self.root;
        let mut mapper =
            Mapper::<'_, F, A, T, BitmapFrameAllocator>::new(root, access, tlb, &mut self.frames);
        self.stacks.release(&mut mapper, stack)
    }

    /// The frame the boot information page is mapped to.
    #[must_use]
    pub const fn boot_info(&self) -> PhysFrame {
        self.boot_info
    }

    /// The number of pages the bring-up unmapped when it dropped the
    /// identity mapping.
    #[must_use]
    pub const fn identity_pages(&self) -> u64 {
        self.identity_pages
    }
}

/// The one cell holding the kernel memory.
pub static MEMORY: Global<KernelMemory> = Global::new();

/// The reserve and what is left of the usable memory.
#[derive(Debug)]
pub struct Reserve {
    /// The allocator over the reserve.
    pub frames: BitmapFrameAllocator,
    /// The usable memory the reserve was taken out of.
    pub free: NormalizedMap,
    /// The number of pages the physical window maps.
    pub window_pages: u64,
}

/// Normalizes the reported regions, takes the kernel reserve out of them,
/// and builds the allocator over it. `override_bytes` of zero selects the
/// default reserve size.
///
/// # Errors
///
/// [`MemoryError::Map`] if the map does not normalize or no usable range
/// holds the reserve; [`MemoryError::Frames`] if the reserve is larger than
/// the allocator manages; [`MemoryError::WindowTooLarge`] if the machine
/// has more memory than the window covers.
pub fn reserve(platform: &impl Platform, override_bytes: u64) -> Result<Reserve, MemoryError> {
    let regions = platform.memory_regions();
    let map = normalize(regions)?;
    let window_bytes = window_bytes(platform)?;
    let (range, free) = select_reserve(&map, override_bytes)?;
    let frames = BitmapFrameAllocator::new(range)?;
    Ok(Reserve {
        frames,
        free,
        window_pages: window_bytes.wrapping_div(PAGE_SIZE),
    })
}

/// The number of bytes the physical window has to map: the first byte above
/// the highest region the loader reported that is memory rather than a
/// device aperture, which is what the loader sized the window for.
///
/// # Errors
///
/// [`MemoryError::WindowTooLarge`] if that is more than the window covers.
fn window_bytes(platform: &impl Platform) -> Result<u64, MemoryError> {
    let end = platform
        .memory_regions()
        .iter()
        .filter(|region| is_memory(region.kind))
        .filter_map(|region| region.start.checked_add(region.len))
        .fold(0u64, |highest, end| highest.max(end.as_u64()));
    if end > MAX_PHYS_WINDOW_BYTES {
        return Err(MemoryError::WindowTooLarge(end));
    }
    Ok(end)
}

/// `true` if the region describes memory rather than a device aperture.
/// The loader sizes the window over the same set of kinds.
const fn is_memory(kind: MemoryRegionKind) -> bool {
    matches!(
        kind,
        MemoryRegionKind::Usable
            | MemoryRegionKind::AcpiReclaimable
            | MemoryRegionKind::AcpiNvs
            | MemoryRegionKind::Kernel
            | MemoryRegionKind::BootImage
            | MemoryRegionKind::PageTables
            | MemoryRegionKind::BootStack
            | MemoryRegionKind::BootInfo
    )
}

/// The reserve size the boot image header asks for, or `None` when the
/// header at the start of the boot image does not parse. A `Some(0)` asks
/// for the default size, which is also what the caller uses for `None`.
#[must_use]
pub fn reserve_override(bytes: &[u8], image_len: u64, ram_bytes: u64) -> Option<u64> {
    BootImageHeader::parse(bytes, image_len, ram_bytes)
        .ok()
        .map(|header| header.kernel_reserve_size)
}

/// The reserve size the boot image asks for, from the first bytes of the
/// image and the map the loader reported. Zero selects the default, which
/// is also what a boot image the kernel cannot read gets.
#[must_use]
pub fn requested_reserve(platform: &impl Platform, header: &[u8]) -> u64 {
    let Some((_, image_len)) = boot_image(platform) else {
        return 0;
    };
    let ram = platform
        .memory_regions()
        .iter()
        .filter(|region| region.kind == MemoryRegionKind::Usable)
        .fold(0u64, |total, region| total.saturating_add(region.len));
    reserve_override(header, image_len, ram).unwrap_or(0)
}

/// The physical start and the length of the boot image, if the loader
/// reported one.
#[must_use]
pub fn boot_image(platform: &impl Platform) -> Option<(PhysAddr, u64)> {
    platform
        .memory_regions()
        .iter()
        .find(|region| region.kind == MemoryRegionKind::BootImage)
        .map(|region| (region.start, region.len))
}

/// What the walk of the loader's tables found.
#[derive(Debug)]
pub struct Adopted {
    /// The regions of the kernel address space.
    pub regions: KernelRegions,
    /// The frame the boot information page is mapped to.
    pub boot_info: PhysFrame,
}

/// Registers the kernel image, the physical window, the boot stack, and the
/// boot information page in a fresh kernel region table, reading the
/// translations out of the tables the loader built.
///
/// # Errors
///
/// [`MemoryError::Address`] if one of the four ranges is not addressable;
/// [`MemoryError::NoBootInfo`] if nothing is mapped at
/// [`BOOT_INFO_VADDR`]; [`MemoryError::Region`] if the table has no slot
/// left.
pub fn adopt<F, A, T, S>(
    mapper: &Mapper<'_, F, A, T, S>,
    window_pages: u64,
) -> Result<Adopted, MemoryError>
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    S: FrameSource,
{
    let mut regions = KernelRegions::kernel();
    register_runs(
        mapper,
        &mut regions,
        KERNEL_BASE,
        KERNEL_IMAGE_MAX_PAGES,
        KernelBacking::Image,
    )?;
    register_runs(
        mapper,
        &mut regions,
        PHYS_WINDOW_BASE,
        window_pages,
        KernelBacking::Window,
    )?;
    let stack_bottom = BOOT_STACK_TOP
        .checked_sub(BOOT_STACK_PAGES.wrapping_mul(PAGE_SIZE))
        .ok_or(MemoryError::Address)?;
    register_runs(
        mapper,
        &mut regions,
        stack_bottom,
        BOOT_STACK_PAGES,
        KernelBacking::BootStack,
    )?;
    register_runs(
        mapper,
        &mut regions,
        BOOT_INFO_VADDR,
        1,
        KernelBacking::BootInfo,
    )?;
    let boot_info = mapper
        .translate(page_at(BOOT_INFO_VADDR)?)
        .map(|(frame, _)| frame)
        .ok_or(MemoryError::NoBootInfo)?;
    Ok(Adopted { regions, boot_info })
}

/// The page starting at `address`.
fn page_at(address: u64) -> Result<Page, MemoryError> {
    VirtAddr::new(address)
        .and_then(Page::from_start)
        .map_err(|_| MemoryError::Address)
}

/// Registers every maximal run of mapped pages in the `count` pages above
/// `base` as one region backed by `backing`.
fn register_runs<F, A, T, S>(
    mapper: &Mapper<'_, F, A, T, S>,
    regions: &mut KernelRegions,
    base: u64,
    count: u64,
    backing: KernelBacking,
) -> Result<(), MemoryError>
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    S: FrameSource,
{
    let start = page_at(base)?;
    let mut offset = 0u64;
    while offset < count {
        let page = start.checked_add(offset).ok_or(MemoryError::Address)?;
        let Some((_, perms)) = mapper.translate(page) else {
            offset = offset.saturating_add(1);
            continue;
        };
        let length = run_length(mapper, page, count.saturating_sub(offset));
        let pages = PageRange::new(page, length).map_err(|_| MemoryError::Address)?;
        regions.insert(Region {
            pages,
            backing,
            offset: 0,
            perms,
        })?;
        offset = offset.saturating_add(length);
    }
    Ok(())
}

/// The number of mapped pages from `start`, at most `limit`.
fn run_length<F, A, T, S>(mapper: &Mapper<'_, F, A, T, S>, start: Page, limit: u64) -> u64
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    S: FrameSource,
{
    let mut length = 0u64;
    while length < limit {
        let Some(page) = start.checked_add(length) else {
            break;
        };
        if mapper.translate(page).is_none() {
            break;
        }
        length = length.saturating_add(1);
    }
    length
}

/// The highest page the identity mapping can reach: everything below the
/// user half, bounded by the memory the window covers.
///
/// # Errors
///
/// [`MemoryError::Address`] if the range is not addressable.
pub fn identity_range(window_pages: u64) -> Result<PageRange, MemoryError> {
    let limit = USER_SPACE_END.wrapping_div(PAGE_SIZE);
    let count = window_pages.min(limit);
    PageRange::new(page_at(0)?, count).map_err(|_| MemoryError::Address)
}

/// Removes every translation of `pages` that exists and returns how many
/// there were. The frames stay where they are: they are the memory the
/// window maps, not frames the kernel handed out.
///
/// The work is done in steps of [`MAX_PAGES_PER_CALL`] pages, the same
/// bound a range system call uses.
///
/// # Errors
///
/// The errors of [`Mapper::unmap`].
pub fn drop_identity<F, A, T, S>(
    mapper: &mut Mapper<'_, F, A, T, S>,
    pages: PageRange,
) -> Result<u64, MemoryError>
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    S: FrameSource,
{
    let mut removed = 0u64;
    let mut offset = 0u64;
    let count = pages.count();
    while offset < count {
        let page = pages
            .start()
            .checked_add(offset)
            .ok_or(MemoryError::Address)?;
        if mapper.translate(page).is_none() {
            offset = offset.saturating_add(1);
            continue;
        }
        let length = run_length(mapper, page, count.saturating_sub(offset));
        let mut rest = PageRange::new(page, length).map_err(|_| MemoryError::Address)?;
        while !rest.is_empty() {
            match mapper.unmap_range(rest, MAX_PAGES_PER_CALL)? {
                Progress::Done => {
                    removed = removed.saturating_add(rest.count());
                    break;
                }
                Progress::Partial(done) => {
                    removed = removed.saturating_add(done);
                    rest = advance(rest, done)?;
                }
            }
        }
        offset = offset.saturating_add(length);
    }
    Ok(removed)
}

/// The part of `pages` beyond its first `done` pages.
fn advance(pages: PageRange, done: u64) -> Result<PageRange, MemoryError> {
    let start = pages
        .start()
        .checked_add(done)
        .ok_or(MemoryError::Address)?;
    let count = pages.count().saturating_sub(done);
    PageRange::new(start, count).map_err(|_| MemoryError::Address)
}

/// Runs the whole bring-up over the tables rooted in `root` and returns
/// what the kernel owns afterwards.
///
/// # Errors
///
/// The errors of [`reserve`], [`adopt`], and [`drop_identity`].
pub fn bring_up<F, P, A, T>(
    platform: &P,
    root: PhysFrame,
    access: &mut A,
    tlb: &mut T,
    override_bytes: u64,
) -> Result<KernelMemory, MemoryError>
where
    F: EntryFormat,
    P: Platform,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
{
    let Reserve {
        mut frames,
        free,
        window_pages,
    } = reserve(platform, override_bytes)?;
    let identity = identity_range(window_pages)?;
    let mut mapper =
        Mapper::<'_, F, A, T, BitmapFrameAllocator>::new(root, access, tlb, &mut frames);
    let Adopted { regions, boot_info } = adopt(&mapper, window_pages)?;
    let identity_pages = drop_identity(&mut mapper, identity)?;
    Ok(KernelMemory {
        root,
        frames,
        free,
        regions,
        stacks: KernelStacks::new(),
        boot_info,
        identity_pages,
    })
}

/// Runs the bring-up and stores the result in [`MEMORY`].
///
/// # Errors
///
/// The errors of [`bring_up`]; [`MemoryError::AlreadyDone`] if the cell is
/// already filled.
pub fn initialize<F, P, A, T>(
    platform: &P,
    root: PhysFrame,
    access: &mut A,
    tlb: &mut T,
    override_bytes: u64,
) -> Result<(), MemoryError>
where
    F: EntryFormat,
    P: Platform,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
{
    let memory = bring_up::<F, P, A, T>(platform, root, access, tlb, override_bytes)?;
    MEMORY.init(memory).map_err(|_| MemoryError::AlreadyDone)
}

/// Runs `body` with the kernel memory, if the bring-up has run and nothing
/// else is holding it.
pub fn with_memory<R>(body: impl FnOnce(&mut KernelMemory) -> R) -> Option<R> {
    let mut memory = MEMORY.borrow(&UncontendedToken).ok()?;
    Some(body(&mut memory))
}

/// Reports the reserve, the regions of the kernel address space, and the
/// identity mapping the bring-up dropped.
pub fn report(memory: &KernelMemory, console: &mut impl DebugConsole) {
    let range = memory.frames.range();
    println!(
        console,
        "reserve {}-{} ({} frames)",
        range.start().start(),
        range
            .start()
            .start()
            .checked_add(range.bytes())
            .unwrap_or(PhysAddr::MAX),
        range.count()
    );
    println!(
        console,
        "identity mapping dropped, {} pages", memory.identity_pages
    );
    println!(console, "{} kernel regions", memory.regions.len());
    for region in memory.regions.iter() {
        println!(
            console,
            "  {}+{} {}",
            region.pages.start().start(),
            region.pages.count(),
            backing_name(region.backing)
        );
    }
    println!(
        console,
        "boot information page at {}",
        memory.boot_info.start()
    );
}

/// The name a report uses for what backs a kernel region.
#[must_use]
pub const fn backing_name(backing: KernelBacking) -> &'static str {
    match backing {
        KernelBacking::Image => "kernel-image",
        KernelBacking::Window => "physical-window",
        KernelBacking::BootStack => "boot-stack",
        KernelBacking::BootInfo => "boot-info",
    }
}
