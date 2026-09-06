// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What has to be mapped where, for a program that is about to be started.
//!
//! The ELF itself is read by `audhsos-elf`, which checks the format and the
//! bounds; what this module adds is the part that is about user space. A
//! segment has to lie inside the user half and above the page that is never
//! mapped, its pages have to be whole pages, and the addresses of the
//! program may not reach into the range the IPC buffers of its threads
//! occupy.
//!
//! The result is a plan and not an action: a list of regions with the
//! permissions they take, the entry point, and where the stack goes. The
//! program that holds the capabilities carries it out — allocate, map into
//! itself, copy, unmap, map into the child — and that part is the wiring of
//! the root task, not a parser.
//!
//! Invariants: every region of a plan is page-aligned at both ends and lies
//! inside the user half; no two regions of one plan overlap; a plan that
//! could not be made is an error and never a plan with a region missing.

use audhsos_abi::layout::{PAGE_SIZE, THREADS_PER_PROCESS, USER_SPACE_START, ipc_buffer_address};
use audhsos_elf::{Constraints, ElfError, Image, MAX_SEGMENTS, Segment};

/// How many pages the stack of a thread gets.
pub const STACK_PAGES: u64 = 16;

/// The first address the buffers of a process occupy. Nothing a program is
/// linked at may reach it.
const BUFFERS_START: u64 = match ipc_buffer_address(THREADS_PER_PROCESS - 1) {
    Some(lowest) => lowest,
    None => 0,
};

/// The constraints a user program is read under: from the lowest mappable
/// address up to the page below the buffers of the process.
pub const USER_CONSTRAINTS: Constraints = Constraints {
    lowest_vaddr: USER_SPACE_START,
    highest_vaddr: BUFFERS_START.saturating_sub(1),
};

/// Why a program could not be turned into a plan.
///
/// The list is short because `audhsos-elf` has already refused most of what
/// could be wrong: the magic, the class, the machine, the type, the program
/// header table, the bounds of every segment against the constraints of
/// [`USER_CONSTRAINTS`], a segment that is writable and executable at once,
/// an alignment that is no power of two, and an entry point that lies in no
/// executable segment. What is left is what only a loader can see.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProgramError {
    /// The ELF itself was refused.
    Elf(ElfError),
    /// Two segments share a page once their ranges are rounded to whole
    /// pages. The ELF reader allows it — nothing of the format forbids two
    /// segments in one page — and a loader that maps pages cannot.
    Overlap {
        /// The region that starts inside the one below it.
        vaddr: u64,
    },
    /// The program is linked so low that no stack fits below it.
    NoRoomForStack,
}

impl From<ElfError> for ProgramError {
    fn from(error: ElfError) -> Self {
        ProgramError::Elf(error)
    }
}

impl core::fmt::Display for ProgramError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ProgramError::Elf(error) => write!(f, "{error}"),
            ProgramError::Overlap { vaddr } => {
                write!(f, "the region at {vaddr:#x} overlaps the one below it")
            }
            ProgramError::NoRoomForStack => f.write_str("no room below the program for a stack"),
        }
    }
}

/// One region of the plan: whole pages, with what may be done to them and
/// what has to be copied into them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Region {
    /// The first address, page-aligned.
    pub vaddr: u64,
    /// How many bytes the region covers, a multiple of the page size.
    pub len: u64,
    /// Where in the file the bytes of this region begin.
    pub file_offset: u64,
    /// How many bytes come from the file. The rest is what `.bss` is: zeros
    /// the allocator has already put there.
    pub file_size: u64,
    /// The offset of the file bytes inside the region, which is what the
    /// rounding down to a page boundary left in front of them.
    pub leading: u64,
    /// Writing is allowed.
    pub write: bool,
    /// Executing is allowed.
    pub execute: bool,
}

impl Region {
    /// The first address above the region.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.vaddr.saturating_add(self.len)
    }

    /// How many pages the region covers.
    #[must_use]
    pub const fn pages(&self) -> u64 {
        self.len.wrapping_div(PAGE_SIZE)
    }
}

/// Everything that has to exist before a program can be started.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    regions: [Option<Region>; MAX_SEGMENTS],
    count: usize,
    /// Where the first thread begins.
    pub entry: u64,
    /// The address one past the top of the stack, which the thread starts
    /// with in its stack pointer.
    pub stack_top: u64,
    /// The first address of the stack.
    pub stack_base: u64,
}

impl Plan {
    /// The regions, lowest address first.
    pub fn regions(&self) -> impl Iterator<Item = Region> + '_ {
        self.regions.iter().take(self.count).flatten().copied()
    }

    /// How many regions there are.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// `true` for a plan with no region, which no program has.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// How many bytes of memory the program needs, the stack included.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        let regions = self
            .regions()
            .fold(0u64, |total, region| total.saturating_add(region.len));
        regions.saturating_add(STACK_PAGES.saturating_mul(PAGE_SIZE))
    }
}

/// Reads `bytes` as a user program and works out what has to be mapped.
///
/// # Errors
///
/// [`ProgramError::Elf`] for everything the ELF reader refuses;
/// [`ProgramError::Overlap`] for two segments whose pages overlap once
/// rounded; [`ProgramError::NoRoomForStack`] when nothing is left below the
/// program for a stack.
pub fn plan(bytes: &[u8]) -> Result<Plan, ProgramError> {
    let image: Image<'_> = audhsos_elf::parse(bytes, USER_CONSTRAINTS)?;
    let mut regions: [Option<Region>; MAX_SEGMENTS] = [None; MAX_SEGMENTS];
    let mut count = 0usize;
    let mut previous_end = 0u64;
    // The array and the segments are zipped rather than indexed, because
    // the ELF reader hands out at most `MAX_SEGMENTS` of them and this is
    // the way to say so without a branch that can never be taken.
    for (slot, segment) in regions.iter_mut().zip(image.segments()) {
        let region = region_of(&segment);
        if region.vaddr < previous_end {
            return Err(ProgramError::Overlap {
                vaddr: region.vaddr,
            });
        }
        previous_end = region.end();
        *slot = Some(region);
        count = count.wrapping_add(1);
    }

    // There is at least one segment: the ELF reader refuses an entry point
    // that lies in no executable one, and an image without segments has
    // none.
    let lowest = image
        .segments()
        .map(|segment| segment.vaddr)
        .min()
        .unwrap_or(u64::MAX);
    let base = lowest.wrapping_sub(lowest.wrapping_rem(PAGE_SIZE));
    let stack_bytes = STACK_PAGES.saturating_mul(PAGE_SIZE);
    // One unmapped page between the stack and the program, so that a stack
    // that runs over the end faults instead of writing into the program's
    // own bytes.
    let stack_top = base
        .checked_sub(PAGE_SIZE)
        .ok_or(ProgramError::NoRoomForStack)?;
    let stack_base = stack_top
        .checked_sub(stack_bytes)
        .ok_or(ProgramError::NoRoomForStack)?;
    if stack_base < USER_SPACE_START {
        return Err(ProgramError::NoRoomForStack);
    }

    Ok(Plan {
        regions,
        count,
        entry: image.entry,
        stack_top,
        stack_base,
    })
}

/// The whole pages a segment occupies.
///
/// Everything the arithmetic could overflow on is already bounded by
/// [`USER_CONSTRAINTS`], which the ELF reader checked every segment
/// against, so the saturating forms here cannot saturate.
const fn region_of(segment: &Segment) -> Region {
    let start = segment
        .vaddr
        .wrapping_sub(segment.vaddr.wrapping_rem(PAGE_SIZE));
    let end = segment
        .vaddr
        .saturating_add(segment.mem_size)
        .next_multiple_of(PAGE_SIZE);
    Region {
        vaddr: start,
        len: end.saturating_sub(start),
        file_offset: segment.file_offset,
        file_size: segment.file_size,
        leading: segment.vaddr.wrapping_sub(start),
        write: segment.write,
        execute: segment.execute,
    }
}
