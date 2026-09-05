// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The order the loader does its work in.
//!
//! Invariants: everything the firmware has to supply is asked for before
//! the boot services end; the memory map the boot information reports is
//! the one the loader read immediately before it left them; after that
//! call the loader talks to nothing but its own memory and the exit
//! device.

use core::convert::Infallible;
use core::fmt;

use audhsos_abi::boot_image::BOOT_IMAGE_HEADER_LEN;
use audhsos_abi::boot_info::{BOOT_INFO_PAGE_LEN, BootRegionKind, Framebuffer};
use audhsos_abi::layout::{
    BOOT_INFO_VADDR, BOOT_STACK_PAGES, BOOT_STACK_TOP, KERNEL_SPACE_START, PAGE_SHIFT, PAGE_SIZE,
    PHYS_WINDOW_BASE,
};
use audhsos_elf::{Constraints, ElfError};
use audhsos_uefi::memory_map::{MemoryDescriptor, descriptors};
use audhsos_uefi::protocols::ACPI_20_TABLE;
use audhsos_uefi::status::Status;
use kernel_hal_api::paging::{FrameAccess, FrameSource};
use kernel_mm::mapper::{MapError, Mapper};
use kernel_mm::page_table::{CachePolicy, PageTable, Permissions, X86Entry};
use kernel_types::{Page, PageRange, PhysAddr, PhysFrame, PhysFrameRange, VirtAddr};

use crate::bootinfo::{self, Range, Ranges};
use crate::files::{self, LoadError, Volume};
use crate::firmware::{Firmware, MemoryMapInfo};
use crate::memory;
use crate::paging::{IdentityAccess, NoTlb, PoolFrames, pool_frames};
use crate::placement::{self, PlaceError, Placement};
use crate::{entry, exit, graphics};

/// Path of the kernel on the boot volume.
const KERNEL_PATH: &str = "AUDHSOS\\KERNEL.ELF";

/// Path of the boot image on the boot volume.
const BOOT_IMAGE_PATH: &str = "AUDHSOS\\BOOT.IMG";

/// Frames the first attempt at the memory map buffer asks for.
const MAP_BUFFER_PAGES: u64 = 16;

/// Frames the last attempt at the memory map buffer asks for.
const MAX_MAP_BUFFER_PAGES: u64 = 256;

/// How often the loader retries `ExitBootServices` with a fresh map key.
const EXIT_ATTEMPTS: u32 = 3;

/// The bounds the kernel image has to lie in.
const KERNEL_BOUNDS: Constraints = Constraints {
    lowest_vaddr: KERNEL_SPACE_START,
    highest_vaddr: u64::MAX,
};

/// What the loader may allow a mapping of the physical window.
const WINDOW_PERMISSIONS: Permissions = Permissions::READ_WRITE;

/// What the loader has to allow itself so that it keeps running after the
/// address space switch.
const IDENTITY_PERMISSIONS: Permissions = Permissions {
    write: true,
    execute: true,
    user: false,
};

/// Why the loader could not start the kernel.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Failure {
    /// A file could not be read.
    File(&'static str, LoadError),
    /// The kernel is not an image the loader accepts.
    Elf(ElfError),
    /// The boot image does not even hold a header.
    BootImage(u64),
    /// The kernel could not be placed.
    Place(PlaceError),
    /// A mapping could not be built.
    Map(MapError),
    /// The address space layout and the machine disagree.
    Address(&'static str),
    /// A firmware service failed.
    Firmware(&'static str, Status),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::File(name, error) => write!(f, "{name}: {error}"),
            Failure::Elf(error) => write!(f, "the kernel image: {error}"),
            Failure::BootImage(len) => write!(f, "the boot image is {len} bytes long"),
            Failure::Place(error) => write!(f, "{error}"),
            Failure::Map(error) => write!(f, "the page tables: {error}"),
            Failure::Address(what) => write!(f, "{what} is not an address the loader can map"),
            Failure::Firmware(what, status) => write!(f, "{what}: {status}"),
        }
    }
}

impl From<MapError> for Failure {
    fn from(error: MapError) -> Self {
        Failure::Map(error)
    }
}

/// Loads the kernel and enters it. Every return is a failure.
pub(crate) fn run(firmware: &Firmware<'_>) -> Failure {
    match load(firmware) {
        Ok(never) => match never {},
        Err(failure) => failure,
    }
}

/// The whole sequence, from the boot volume to the jump.
fn load(firmware: &Firmware<'_>) -> Result<Infallible, Failure> {
    let volume: Volume<'_> =
        files::open_volume(firmware).map_err(|error| Failure::File("the boot volume", error))?;
    let kernel_file = volume
        .read_file(firmware, KERNEL_PATH)
        .map_err(|error| Failure::File(KERNEL_PATH, error))?;
    let boot_image = volume
        .read_file(firmware, BOOT_IMAGE_PATH)
        .map_err(|error| Failure::File(BOOT_IMAGE_PATH, error))?;
    if boot_image.len < image_header_len() {
        return Err(Failure::BootImage(boot_image.len));
    }
    let rsdp = firmware.configuration_table(ACPI_20_TABLE).unwrap_or(0);

    // SAFETY: the frames hold the file the loader has just read, nothing
    // else borrows them, and the identity mapping is active.
    let kernel_bytes = unsafe { memory::bytes_mut(kernel_file.frames, kernel_file.len) }
        .ok_or(Failure::File(KERNEL_PATH, LoadError::TooLarge))?;
    let image = audhsos_elf::parse(kernel_bytes, KERNEL_BOUNDS).map_err(Failure::Elf)?;

    let (map_frames, first_map) = map_buffer(firmware)?;
    let ram_end = ram_end(map_frames, first_map)?;

    let placement = placement::place(firmware, &image).map_err(Failure::Place)?;
    let stack = allocate(firmware, "the boot stack", BOOT_STACK_PAGES)?;
    let info_page = allocate(firmware, "the boot information page", 1)?;
    let pool = allocate(firmware, "the page tables", pool_frames(ram_end))?;
    exit::report(
        firmware,
        format_args!(
            "{} KiB of memory, kernel at {:#x}",
            ram_end >> 10,
            placement.entry
        ),
    );

    let root = build_tables(ram_end, &placement, stack, info_page, pool)?;
    let framebuffer = graphics::framebuffer(firmware);

    let final_map = leave_boot_services(firmware, map_frames)?;
    write_boot_info(
        info_page,
        map_frames,
        final_map,
        &Ranges {
            kernel: range_of(placement.frames),
            boot_image: Range {
                start: memory::start_of(boot_image.frames).as_u64(),
                len: boot_image.len,
            },
            page_tables: range_of(pool),
            boot_stack: range_of(stack),
            rsdp,
        },
        framebuffer,
    );

    // SAFETY: the tables rooted in `root` map the loader's own code
    // identically, the kernel at its entry point, the boot stack below
    // `BOOT_STACK_TOP`, and the boot information page at
    // `BOOT_INFO_VADDR`, which is what the kernel entry point expects.
    unsafe {
        entry::enter_kernel(
            memory::start_of(root).as_u64(),
            BOOT_STACK_TOP,
            BOOT_INFO_VADDR,
            placement.entry,
        )
    }
}

/// The length of a boot image header, as a `u64`.
fn image_header_len() -> u64 {
    u64::try_from(BOOT_IMAGE_HEADER_LEN).unwrap_or(u64::MAX)
}

/// The physical range of an allocation, as the boot information reports it.
const fn range_of(frames: PhysFrameRange) -> Range {
    Range {
        start: memory::start_of(frames).as_u64(),
        len: frames.bytes(),
    }
}

/// Frames for the caller, or the failure the firmware reported.
fn allocate(
    firmware: &Firmware<'_>,
    what: &'static str,
    count: u64,
) -> Result<PhysFrameRange, Failure> {
    firmware
        .allocate_pages(count)
        .map_err(|status| Failure::Firmware(what, status))
}

/// A buffer the memory map fits into, together with the map that was read
/// into it.
fn map_buffer(firmware: &Firmware<'_>) -> Result<(PhysFrameRange, MemoryMapInfo), Failure> {
    let mut pages = MAP_BUFFER_PAGES;
    loop {
        let frames = allocate(firmware, "the memory map buffer", pages)?;
        let outcome = read_map(firmware, frames, frames.bytes());
        match outcome {
            Ok(info) => return Ok((frames, info)),
            Err(status) if status == Status::BUFFER_TOO_SMALL && pages < MAX_MAP_BUFFER_PAGES => {
                firmware.free_pages(frames);
                pages = pages.saturating_mul(2);
            }
            Err(status) => {
                firmware.free_pages(frames);
                return Err(Failure::Firmware("the memory map", status));
            }
        }
    }
}

/// Reads the memory map into the first `len` bytes of `frames`.
fn read_map(
    firmware: &Firmware<'_>,
    frames: PhysFrameRange,
    len: u64,
) -> Result<MemoryMapInfo, Status> {
    // SAFETY: the frames were allocated for the memory map, nothing else
    // borrows them, and the identity mapping is active.
    let buffer = unsafe { memory::bytes_mut(frames, len) }.ok_or(Status::OUT_OF_RESOURCES)?;
    firmware.memory_map(buffer)
}

/// Runs `body` for every descriptor of the map in `frames`.
fn with_descriptors<T>(
    frames: PhysFrameRange,
    map: MemoryMapInfo,
    body: impl FnOnce(&mut dyn Iterator<Item = MemoryDescriptor>) -> T,
) -> Option<T> {
    let len = u64::try_from(map.size).ok()?;
    // SAFETY: the frames hold the memory map the firmware wrote, nothing
    // else borrows them, and the identity mapping is active.
    let buffer = unsafe { memory::bytes_mut(frames, len) }?;
    let mut iterator = descriptors(buffer, map.descriptor_size);
    Some(body(&mut iterator))
}

/// The first byte above the highest region of memory the firmware
/// reports. Device apertures are left out: on this machine the memory map
/// reaches to a terabyte, and the window has to cover memory, not the
/// address space.
fn ram_end(frames: PhysFrameRange, map: MemoryMapInfo) -> Result<u64, Failure> {
    let end = with_descriptors(frames, map, |entries| {
        entries
            .filter(|descriptor| is_memory(*descriptor))
            .fold(0u64, |highest, descriptor| {
                highest.max(descriptor.end().unwrap_or(0))
            })
    })
    .unwrap_or(0);
    if end == 0 {
        return Err(Failure::Firmware(
            "the memory map",
            Status::INVALID_PARAMETER,
        ));
    }
    Ok(end)
}

/// `true` if the descriptor describes memory rather than a device.
const fn is_memory(descriptor: MemoryDescriptor) -> bool {
    matches!(
        descriptor.region_kind(),
        BootRegionKind::Usable | BootRegionKind::AcpiReclaimable | BootRegionKind::AcpiNvs
    )
}

/// Builds the address space the kernel starts in and returns its root.
fn build_tables(
    ram_end: u64,
    placement: &Placement,
    stack: PhysFrameRange,
    info_page: PhysFrameRange,
    pool: PhysFrameRange,
) -> Result<PhysFrameRange, Failure> {
    let mut frames = PoolFrames::new(pool);
    let mut access = IdentityAccess::new();
    let mut tlb = NoTlb::new();
    let root = frames
        .allocate_frame()
        .ok_or(Failure::Map(MapError::OutOfKernelMemory))?;
    let table: &mut PageTable<X86Entry> = access
        .table_mut(root)
        .ok_or(Failure::Map(MapError::UnreachableFrame))?;
    *table = PageTable::default();
    let mut mapper = Mapper::new(root, &mut access, &mut tlb, &mut frames);

    let pages = ram_end >> PAGE_SHIFT;
    map_physical(&mut mapper, 0, pages, PHYS_WINDOW_BASE, WINDOW_PERMISSIONS)?;
    map_physical(&mut mapper, 0, pages, 0, IDENTITY_PERMISSIONS)?;

    for segment in placement.segments() {
        mapper.map_range(
            segment.pages,
            segment.frames.start(),
            segment.perms,
            CachePolicy::WriteBack,
            u64::MAX,
        )?;
    }
    let stack_bottom = BOOT_STACK_TOP
        .checked_sub(stack.bytes())
        .ok_or(Failure::Address("the boot stack"))?;
    map_virtual(
        &mut mapper,
        stack_bottom,
        stack,
        Permissions::READ_WRITE,
        "the boot stack",
    )?;
    map_virtual(
        &mut mapper,
        BOOT_INFO_VADDR,
        info_page,
        Permissions::READ_ONLY,
        "the boot information page",
    )?;
    PhysFrameRange::new(root, 1).map_err(|_| Failure::Address("the page table root"))
}

/// Maps `pages` frames from `start` at `base + start`.
fn map_physical(
    mapper: &mut Mapper<'_, X86Entry, IdentityAccess, NoTlb, PoolFrames>,
    start: u64,
    pages: u64,
    base: u64,
    perms: Permissions,
) -> Result<(), Failure> {
    let frame = PhysAddr::new(start)
        .and_then(PhysFrame::from_start)
        .map_err(|_| Failure::Address("a memory map entry"))?;
    let virtual_start = base
        .checked_add(start)
        .ok_or(Failure::Address("a memory map entry"))?;
    let page = VirtAddr::new(virtual_start)
        .and_then(Page::from_start)
        .map_err(|_| Failure::Address("a memory map entry"))?;
    let range = PageRange::new(page, pages).map_err(|_| Failure::Address("a memory map entry"))?;
    mapper.map_range(range, frame, perms, CachePolicy::WriteBack, u64::MAX)?;
    Ok(())
}

/// Maps `frames` at the virtual address `start`.
fn map_virtual(
    mapper: &mut Mapper<'_, X86Entry, IdentityAccess, NoTlb, PoolFrames>,
    start: u64,
    frames: PhysFrameRange,
    perms: Permissions,
    what: &'static str,
) -> Result<(), Failure> {
    let page = VirtAddr::new(start)
        .and_then(Page::from_start)
        .map_err(|_| Failure::Address(what))?;
    let range = PageRange::new(page, frames.count()).map_err(|_| Failure::Address(what))?;
    mapper.map_range(
        range,
        frames.start(),
        perms,
        CachePolicy::WriteBack,
        u64::MAX,
    )?;
    Ok(())
}

/// Reads the memory map one last time and leaves the boot services with
/// the key that map carries.
fn leave_boot_services(
    firmware: &Firmware<'_>,
    map_frames: PhysFrameRange,
) -> Result<MemoryMapInfo, Failure> {
    let mut attempt = 0u32;
    loop {
        let map = read_map(firmware, map_frames, map_frames.bytes())
            .map_err(|status| Failure::Firmware("the memory map", status))?;
        let status = firmware.exit_boot_services(map.key);
        if status.is_success() {
            return Ok(map);
        }
        attempt = attempt.saturating_add(1);
        if attempt >= EXIT_ATTEMPTS {
            return Err(Failure::Firmware("leaving the boot services", status));
        }
    }
}

/// Fills the boot information page. The boot services are gone, so a
/// failure can only end the machine.
fn write_boot_info(
    info_page: PhysFrameRange,
    map_frames: PhysFrameRange,
    map: MemoryMapInfo,
    ranges: &Ranges,
    framebuffer: Option<Framebuffer>,
) {
    // SAFETY: the page was allocated for the boot information, nothing
    // else borrows it, and the identity mapping is active.
    let Some(bytes) = (unsafe { memory::bytes_mut(info_page, PAGE_SIZE) }) else {
        exit::die();
    };
    let Ok(page) = <&mut [u8; BOOT_INFO_PAGE_LEN]>::try_from(bytes) else {
        exit::die();
    };
    let Some(len) = u64::try_from(map.size).ok() else {
        exit::die();
    };
    // SAFETY: the frames hold the memory map the firmware wrote last,
    // nothing else borrows them, and the identity mapping is active.
    let Some(buffer) = (unsafe { memory::bytes_mut(map_frames, len) }) else {
        exit::die();
    };
    if bootinfo::write(page, buffer, map.descriptor_size, ranges, framebuffer).is_err() {
        exit::die();
    }
}
