// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Processor identity and private descriptor and APIC state.
use crate::apic::LocalApic;
use audhsos_sync::{ExclusiveToken, Preset, UncontendedToken};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use kernel_types::VirtAddr;
use kernel_x86_tables::{gdt::GDT_ENTRIES, tss::TSS_LEN};

/// Fixed capacity shared with the firmware parser.
pub const CPUS: usize = 16;
const _: () = assert!(CPUS == kernel_acpi::madt::MAX_PROCESSORS);
/// Mapped local APIC window; zero before interrupt bring-up.
pub static APIC_WINDOW: AtomicU64 = AtomicU64::new(0);
/// Firmware identifiers, with the boot processor first.
pub static IDENTIFIERS: [AtomicU32; CPUS] = [const { AtomicU32::new(u32::MAX) }; CPUS];
/// Successfully initialized processors.
pub static ONLINE: [AtomicU32; CPUS] = [const { AtomicU32::new(0) }; CPUS];
/// Per-processor timer observations.
pub static TICKS: [AtomicU64; CPUS] = [const { AtomicU64::new(0) }; CPUS];

/// Base of each processor's private GDT; zero before `install_private`.
pub(crate) static GDT_BASES: [AtomicU64; CPUS] = [const { AtomicU64::new(0) }; CPUS];

/// The calling processor, without borrowing a cell.
///
/// Reads the GDT base with `sgdt`: a local APIC read is MMIO, which QEMU
/// TCG serializes on its global lock, and every lock-wait spin calls this.
/// A processor without its private GDT loaded reads the APIC ID.
#[must_use]
pub fn processor() -> Option<u8> {
    let gdt = crate::instructions::global_descriptor_table_base();
    if gdt != 0
        && let Some(index) = GDT_BASES
            .iter()
            .position(|entry| entry.load(Ordering::Relaxed) == gdt)
    {
        return u8::try_from(index).ok();
    }
    let base = APIC_WINDOW.load(Ordering::Acquire);
    // Bring-up publishes the complete identifier list before the window.
    // With no AP candidate, every caller is the boot processor.
    if base == 0 || IDENTIFIERS[1].load(Ordering::Relaxed) == u32::MAX {
        return Some(0);
    }
    let address = usize::try_from(base.checked_add(0x20)?).ok()?;
    // SAFETY: bring-up publishes a permanent, uncached APIC mapping.
    let id = unsafe { core::ptr::without_provenance::<u32>(address).read_volatile() } >> 24;
    IDENTIFIERS
        .iter()
        .position(|entry| entry.load(Ordering::Relaxed) == id)
        .and_then(|index| u8::try_from(index).ok())
}

/// Token used with interrupts disabled on the current processor.
#[derive(Clone, Copy, Debug)]
pub struct KernelToken;
impl ExclusiveToken for KernelToken {
    fn owner(&self) -> u32 {
        processor().map_or(u32::MAX, u32::from)
    }
    fn wait(&self) {
        crate::remote::poll();
        core::hint::spin_loop();
    }
}

/// Descriptor images and APIC handle owned by one processor.
#[derive(Debug)]
pub(crate) struct Processor {
    pub(crate) stack: [u8; crate::descriptors::DOUBLE_FAULT_STACK_LEN],
    pub(crate) tss: [u8; TSS_LEN],
    pub(crate) gdt: [u64; GDT_ENTRIES],
    pub(crate) local: Option<LocalApic>,
}
impl Processor {
    const fn new() -> Self {
        Self {
            stack: [0; crate::descriptors::DOUBLE_FAULT_STACK_LEN],
            tss: [0; TSS_LEN],
            gdt: [0; GDT_ENTRIES],
            local: None,
        }
    }
}
pub(crate) static PROCESSORS: [Preset<Processor>; CPUS] =
    [const { Preset::new(Processor::new()) }; CPUS];

/// Accesses only the calling processor's APIC with interrupts disabled.
pub fn with_local<R>(body: impl FnOnce(&mut LocalApic) -> R) -> Option<R> {
    let _guard = crate::instructions::InterruptGuard::new();
    let mut data = PROCESSORS
        .get(usize::from(processor()?))?
        .borrow(&UncontendedToken)
        .ok()?;
    Some(body(data.local.as_mut()?))
}

/// Installs the private APIC handle.
///
/// # Safety
/// The APIC window is mapped, the IDT is loaded, and interrupts are off.
pub unsafe fn install_local() {
    let Ok(base) = VirtAddr::new(APIC_WINDOW.load(Ordering::Acquire)) else {
        crate::entry::fail(b"invalid APIC window\n");
    };
    // SAFETY: each processor accesses its own hardware through this mapping.
    let mut local = unsafe { LocalApic::new(base) };
    // SAFETY: the caller installed the interrupt gates.
    unsafe {
        local.enable(crate::vectors::SPURIOUS);
    }
    if let Some(cpu) = processor()
        && let Some(slot) = PROCESSORS.get(usize::from(cpu))
        && let Ok(mut data) = slot.borrow(&UncontendedToken)
    {
        data.local = Some(local);
    }
}

/// A checked processor-table entry; an invalid processor is a kernel fault.
pub fn slot<T>(table: &[T], cpu: usize) -> &T {
    match table.get(cpu) {
        Some(entry) => entry,
        None => crate::entry::fail(b"invalid processor number\n"),
    }
}
