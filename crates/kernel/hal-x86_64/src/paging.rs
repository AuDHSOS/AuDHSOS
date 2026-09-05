// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The translation lookaside buffer and the register that names the active
//! page tables.
//!
//! Invariant: a flush follows the change of a translation, never precedes
//! it; the tables a switch installs map the code, the stack, and the data
//! the caller uses next.

use kernel_hal_api::paging::TlbControl;
use kernel_types::{Error, Page, PhysAddr, PhysFrame};

/// The page-table entry format of this architecture, so that a caller of
/// the mapper need not name `kernel-mm` for it.
pub use kernel_mm::page_table::X86Entry;

/// The bits of `CR3` above the frame number, which do not belong to the
/// address of the root table.
const ROOT_ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// Invalidates translations on this processor.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalTlb;

impl LocalTlb {
    /// The lookaside buffer of this processor.
    #[must_use]
    pub const fn new() -> Self {
        LocalTlb
    }
}

impl TlbControl for LocalTlb {
    fn flush_page(&mut self, page: Page) {
        // SAFETY: the caller changed the tables before asking for the
        // flush, which is what `invalidate_page` requires.
        unsafe {
            crate::instructions::invalidate_page(page.start().as_u64());
        }
    }

    fn flush_all(&mut self) {
        let root = crate::instructions::read_page_table_root();
        // SAFETY: writing the root that is already active flushes every
        // translation that is not global and changes no mapping.
        unsafe {
            crate::instructions::write_page_table_root(root);
        }
    }
}

/// The frame the active page tables are rooted in.
///
/// # Errors
///
/// [`Error`] if `CR3` does not name an addressable frame, which the
/// processor does not allow but the type system does not know.
pub fn active_root() -> Result<PhysFrame, Error> {
    let raw = crate::instructions::read_page_table_root() & ROOT_ADDRESS_MASK;
    PhysAddr::new(raw).and_then(PhysFrame::from_start)
}

/// Switches to the page tables rooted in `root`.
///
/// # Safety
///
/// The tables must be complete: they must map the code, the stack, and the
/// data the caller uses after the switch.
pub unsafe fn activate(root: PhysFrame) {
    // SAFETY: the caller promises that the tables are complete.
    unsafe {
        crate::instructions::write_page_table_root(root.start().as_u64());
    }
}
