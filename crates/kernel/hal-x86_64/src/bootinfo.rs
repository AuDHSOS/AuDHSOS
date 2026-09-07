// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The boot information the loader left behind, as the kernel's
//! [`Platform`].
//!
//! Invariant: the page the loader wrote is mapped read-only at the address
//! it passes to the kernel, and it stays mapped for the whole run; the
//! parsing itself is `audhsos-abi`, which is safe and fuzzable.

use audhsos_abi::boot_info::{
    BOOT_INFO_PAGE_LEN, BootInfoError, BootInfoView, BootRegionKind, Framebuffer,
};
use audhsos_abi::layout::MAX_BOOT_REGIONS;
use kernel_hal_api::platform::{MemoryRegion, MemoryRegionKind, Platform};
use kernel_types::{PhysAddr, VirtAddr};

/// The four fixed ranges plus the boot information page.
const EXTRA_REGIONS: usize = 5;

/// Number of regions the platform can report.
pub const PLATFORM_REGIONS: usize = MAX_BOOT_REGIONS + EXTRA_REGIONS;

/// The machine as the loader described it.
#[derive(Clone, Copy, Debug)]
pub struct X86Platform {
    regions: [MemoryRegion; PLATFORM_REGIONS],
    count: usize,
    window: u64,
    rsdp: Option<u64>,
    framebuffer: Option<Framebuffer>,
}

impl X86Platform {
    /// Reads the boot information at `address`.
    ///
    /// # Errors
    ///
    /// The errors of [`BootInfoView::parse`].
    ///
    /// # Safety
    ///
    /// `address` must name a page the loader wrote the boot information
    /// into and mapped readable for the whole run.
    pub unsafe fn from_address(address: u64) -> Result<Self, BootInfoError> {
        let pointer = core::ptr::without_provenance::<[u8; BOOT_INFO_PAGE_LEN]>(
            usize::try_from(address).map_err(|_| BootInfoError::TooShort)?,
        );
        // SAFETY: the caller promises that the page is mapped readable for
        // the whole run and holds the boot information the loader wrote.
        let page = unsafe { &*pointer };
        Self::from_page(page)
    }

    /// Reads the boot information out of a page the caller already holds.
    ///
    /// # Errors
    ///
    /// The errors of [`BootInfoView::parse`].
    pub fn from_page(page: &[u8; BOOT_INFO_PAGE_LEN]) -> Result<Self, BootInfoError> {
        let view = BootInfoView::parse(page)?;
        let header = view.header();
        let mut regions = [EMPTY_REGION; PLATFORM_REGIONS];
        let mut count = 0usize;
        for region in view.regions() {
            push(
                &mut regions,
                &mut count,
                region.start,
                region.len,
                kind_of(region.kind),
            );
        }
        let fixed = [
            (
                header.kernel_phys_start,
                header.kernel_phys_len,
                MemoryRegionKind::Kernel,
            ),
            (
                header.boot_image_phys_start,
                header.boot_image_phys_len,
                MemoryRegionKind::BootImage,
            ),
            (
                header.page_tables_phys_start,
                header.page_tables_phys_len,
                MemoryRegionKind::PageTables,
            ),
            (
                header.boot_stack_phys_start,
                header.boot_stack_phys_len,
                MemoryRegionKind::BootStack,
            ),
        ];
        for (start, len, kind) in fixed {
            push(&mut regions, &mut count, start, len, kind);
        }
        Ok(X86Platform {
            regions,
            count,
            window: view.phys_window_base(),
            rsdp: view.acpi_rsdp(),
            framebuffer: view.framebuffer(),
        })
    }

    /// Appends the boot information page as a region of its own.
    ///
    /// The boot information names its own physical address in no field,
    /// which is why the page cannot report itself: the loader mapped it at
    /// [`audhsos_abi::layout::BOOT_INFO_VADDR`], and only a walk of the
    /// loader's page tables says which frame that is. The caller does that
    /// walk and passes the frame here.
    ///
    /// Returns `false` if the region array is full.
    pub fn push_boot_info(&mut self, start: PhysAddr) -> bool {
        let before = self.count;
        push(
            &mut self.regions,
            &mut self.count,
            start.as_u64(),
            u64::try_from(BOOT_INFO_PAGE_LEN).unwrap_or(0),
            MemoryRegionKind::BootInfo,
        );
        self.count != before
    }
}

/// The region a fresh array is filled with.
const EMPTY_REGION: MemoryRegion = MemoryRegion {
    start: PhysAddr::ZERO,
    len: 0,
    kind: MemoryRegionKind::Reserved,
};

/// The kernel's kind for a boot region kind code.
const fn kind_of(code: u32) -> MemoryRegionKind {
    match BootRegionKind::from_code(code) {
        Some(BootRegionKind::Usable) => MemoryRegionKind::Usable,
        Some(BootRegionKind::AcpiReclaimable) => MemoryRegionKind::AcpiReclaimable,
        Some(BootRegionKind::AcpiNvs) => MemoryRegionKind::AcpiNvs,
        Some(BootRegionKind::MmioReserved) => MemoryRegionKind::MmioReserved,
        _ => MemoryRegionKind::Reserved,
    }
}

/// Appends a region unless the array is full or the region is empty.
fn push(
    regions: &mut [MemoryRegion; PLATFORM_REGIONS],
    count: &mut usize,
    start: u64,
    len: u64,
    kind: MemoryRegionKind,
) {
    if len == 0 {
        return;
    }
    let Ok(start) = PhysAddr::new(start) else {
        return;
    };
    if let Some(slot) = regions.get_mut(*count) {
        *slot = MemoryRegion { start, len, kind };
        *count = count.saturating_add(1);
    }
}

impl Platform for X86Platform {
    fn memory_regions(&self) -> &[MemoryRegion] {
        self.regions.get(..self.count).unwrap_or(&[])
    }

    fn physical_window_base(&self) -> VirtAddr {
        VirtAddr::new(self.window).unwrap_or(VirtAddr::ZERO)
    }

    fn framebuffer(&self) -> Option<Framebuffer> {
        self.framebuffer
    }

    fn acpi_rsdp(&self) -> Option<PhysAddr> {
        self.rsdp.and_then(|address| PhysAddr::new(address).ok())
    }
}
