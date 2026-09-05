// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Page-table entries behind an architecture-neutral trait, and the
//! `x86_64` four-level format that implements it.
//!
//! Invariants: an entry produced by [`EntryFormat::table`] or
//! [`EntryFormat::leaf`] is present and carries a frame-aligned address; an
//! entry that carries bits this release does not define is reported by
//! [`EntryFormat::validate`] instead of being interpreted.

use core::fmt;

use audhsos_abi::layout::PAGE_SHIFT;
use kernel_types::{Page, PhysAddr, PhysFrame};

/// What a mapping allows. A mapping always allows reading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Permissions {
    /// Writing is allowed.
    pub write: bool,
    /// Executing is allowed.
    pub execute: bool,
    /// User mode may use the mapping.
    pub user: bool,
}

impl Permissions {
    /// Read-only for the kernel.
    pub const READ_ONLY: Permissions = Permissions {
        write: false,
        execute: false,
        user: false,
    };

    /// Read and write for the kernel.
    pub const READ_WRITE: Permissions = Permissions {
        write: true,
        execute: false,
        user: false,
    };

    /// Read and execute for the kernel.
    pub const READ_EXECUTE: Permissions = Permissions {
        write: false,
        execute: true,
        user: false,
    };

    /// `true` if the mapping is both writable and executable.
    #[must_use]
    pub const fn is_write_execute(self) -> bool {
        self.write && self.execute
    }

    /// The same permissions, reachable from user mode.
    #[must_use]
    pub const fn for_user(self) -> Permissions {
        Permissions {
            write: self.write,
            execute: self.execute,
            user: true,
        }
    }
}

// The policy belongs to the memory and not to the table that maps it, so
// it lives in `kernel-types`, where a memory object reaches it too. The
// re-export keeps the path this module has always had.
pub use kernel_types::CachePolicy;

/// Why an entry read from memory was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryError {
    /// A bit this release does not define is set.
    ReservedBits(u64),
    /// The entry maps a large page, which this release does not support.
    HugePage,
}

impl fmt::Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntryError::ReservedBits(raw) => {
                write!(f, "the entry {raw:#x} sets reserved bits")
            }
            EntryError::HugePage => f.write_str("large pages are not supported"),
        }
    }
}

/// The format of one page-table entry.
pub trait EntryFormat: Copy + Default + 'static {
    /// Number of translation levels.
    const LEVELS: usize;
    /// Number of index bits per level.
    const INDEX_BITS: u32;
    /// Number of entries in one table.
    const ENTRIES: usize;
    /// An entry that maps nothing.
    const EMPTY: Self;

    /// The index `page` selects at `level`; level zero is the table that
    /// holds the leaves.
    fn index(level: usize, page: Page) -> usize;

    /// `true` if the entry maps something.
    fn is_present(self) -> bool;

    /// The frame the entry points at, if it is present.
    fn frame(self) -> Option<PhysFrame>;

    /// An entry pointing at the next-level table in `frame`.
    fn table(frame: PhysFrame) -> Self;

    /// An entry mapping `frame` with the given permissions.
    fn leaf(frame: PhysFrame, perms: Permissions, cache: CachePolicy, global: bool) -> Self;

    /// The permissions the entry grants.
    fn permissions(self) -> Permissions;

    /// The same entry with different permissions.
    #[must_use]
    fn with_permissions(self, perms: Permissions) -> Self;

    /// Rejects entries this release cannot interpret.
    ///
    /// # Errors
    ///
    /// [`EntryError`] for reserved bits and for large pages.
    fn validate(self) -> Result<(), EntryError>;
}

/// One page table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C, align(4096))]
pub struct PageTable<F: EntryFormat> {
    /// The entries, indexed by [`EntryFormat::index`].
    pub entries: [F; 512],
}

impl<F: EntryFormat> Default for PageTable<F> {
    fn default() -> Self {
        PageTable {
            entries: [F::EMPTY; 512],
        }
    }
}

impl<F: EntryFormat> PageTable<F> {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The entry at `index`. An index at or above 512 is out of range and
    /// reports the empty entry; [`EntryFormat::index`] never produces one.
    #[must_use]
    pub fn entry(&self, index: usize) -> F {
        self.entries.get(index).copied().unwrap_or(F::EMPTY)
    }

    /// Writes `entry` at `index` and reports whether the index was in
    /// range; an index out of range changes nothing.
    pub fn set_entry(&mut self, index: usize, entry: F) -> bool {
        match self.entries.get_mut(index) {
            Some(slot) => {
                *slot = entry;
                true
            }
            None => false,
        }
    }

    /// `true` if no entry of the table is present.
    #[must_use]
    pub fn is_unused(&self) -> bool {
        self.entries.iter().all(|entry| !entry.is_present())
    }
}

/// Bit 0: the entry maps something.
pub const X86_PRESENT: u64 = 1 << 0;
/// Bit 1: writing is allowed.
pub const X86_WRITABLE: u64 = 1 << 1;
/// Bit 2: user mode may use the mapping.
pub const X86_USER: u64 = 1 << 2;
/// Bit 3: write-through caching.
pub const X86_WRITE_THROUGH: u64 = 1 << 3;
/// Bit 4: caching is disabled.
pub const X86_NO_CACHE: u64 = 1 << 4;
/// Bit 5: the entry was read.
pub const X86_ACCESSED: u64 = 1 << 5;
/// Bit 6: the page was written.
pub const X86_DIRTY: u64 = 1 << 6;
/// Bit 7: the entry maps a large page.
pub const X86_HUGE: u64 = 1 << 7;
/// Bit 8: the translation survives an address space switch.
pub const X86_GLOBAL: u64 = 1 << 8;
/// Bit 63: executing is forbidden.
pub const X86_NO_EXECUTE: u64 = 1 << 63;

/// The address bits of an entry.
pub const X86_ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// The bits that must be zero in a present entry.
pub const X86_RESERVED_MASK: u64 = 0x7FF0_0000_0000_0000;

/// One entry of an `x86_64` page table.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct X86Entry(u64);

impl X86Entry {
    /// The entry with the given raw bits.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        X86Entry(raw)
    }

    /// The raw bits.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// `true` if the given flag bit is set.
    #[must_use]
    pub const fn has(self, flag: u64) -> bool {
        self.0 & flag != 0
    }

    const fn with(self, flag: u64, set: bool) -> Self {
        if set {
            X86Entry(self.0 | flag)
        } else {
            X86Entry(self.0 & !flag)
        }
    }
}

impl fmt::Debug for X86Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "X86Entry({:#x})", self.0)
    }
}

/// Number of index bits per level on `x86_64`.
const X86_INDEX_BITS: u32 = 9;

/// Number of entries in one `x86_64` table.
const X86_ENTRIES: usize = 512;

/// The bits of one table index.
const X86_INDEX_MASK: u64 = 0x1FF;

impl EntryFormat for X86Entry {
    const LEVELS: usize = 4;
    const INDEX_BITS: u32 = X86_INDEX_BITS;
    const ENTRIES: usize = X86_ENTRIES;
    const EMPTY: Self = X86Entry(0);

    fn index(level: usize, page: Page) -> usize {
        let shift = X86_INDEX_BITS.saturating_mul(u32::try_from(level).unwrap_or(u32::MAX));
        let index = page.number().checked_shr(shift).unwrap_or(0) & X86_INDEX_MASK;
        usize::try_from(index).unwrap_or(0)
    }

    fn is_present(self) -> bool {
        self.has(X86_PRESENT)
    }

    fn frame(self) -> Option<PhysFrame> {
        if !self.is_present() {
            return None;
        }
        PhysAddr::new(self.0 & X86_ADDRESS_MASK)
            .and_then(PhysFrame::from_start)
            .ok()
    }

    fn table(frame: PhysFrame) -> Self {
        X86Entry(frame.start().as_u64() | X86_PRESENT | X86_WRITABLE | X86_USER)
    }

    fn leaf(frame: PhysFrame, perms: Permissions, cache: CachePolicy, global: bool) -> Self {
        X86Entry(frame.start().as_u64() | X86_PRESENT)
            .with(X86_WRITABLE, perms.write)
            .with(X86_USER, perms.user)
            .with(X86_NO_EXECUTE, !perms.execute)
            .with(X86_GLOBAL, global)
            .with(X86_NO_CACHE, matches!(cache, CachePolicy::Uncached))
            .with(X86_WRITE_THROUGH, matches!(cache, CachePolicy::Uncached))
    }

    fn permissions(self) -> Permissions {
        Permissions {
            write: self.has(X86_WRITABLE),
            execute: !self.has(X86_NO_EXECUTE),
            user: self.has(X86_USER),
        }
    }

    fn with_permissions(self, perms: Permissions) -> Self {
        self.with(X86_WRITABLE, perms.write)
            .with(X86_USER, perms.user)
            .with(X86_NO_EXECUTE, !perms.execute)
    }

    fn validate(self) -> Result<(), EntryError> {
        if !self.is_present() {
            return Ok(());
        }
        if self.0 & X86_RESERVED_MASK != 0 {
            return Err(EntryError::ReservedBits(self.0));
        }
        if self.has(X86_HUGE) {
            return Err(EntryError::HugePage);
        }
        Ok(())
    }
}

const _: () = assert!(PAGE_SHIFT == 12);
const _: () = assert!(X86_ADDRESS_MASK & X86_RESERVED_MASK == 0);
