// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The descriptor tables the processor needs before it can take a trap.
//!
//! Invariants: the tables live for the whole run, because they are
//! `static`; the task state segment descriptor names the byte image that
//! lives next to it; the tables are loaded once, before interrupts are
//! turned on.

use core::sync::atomic::{AtomicBool, Ordering};

use audhsos_sync::{Preset, UncontendedToken};

use kernel_x86_tables::gdt::{KERNEL_CODE_SELECTOR, KERNEL_DATA_SELECTOR, TSS_SELECTOR, build_gdt};
use kernel_x86_tables::idt::{DOUBLE_FAULT_IST, IDT_ENTRIES, MISSING};
use kernel_x86_tables::tss::{RSP0_OFFSET, TaskStateSegment, write_at};

use crate::instructions::{
    DescriptorTablePointer, load_global_descriptor_table, load_interrupt_descriptor_table,
    load_task_register, reload_segments,
};

/// Size of the stack the double fault handler runs on.
pub const DOUBLE_FAULT_STACK_LEN: usize = 16 * 1024;

/// The interrupt descriptor table, in `.bss` and filled in place (D-66).
static IDT: Preset<[[u64; 2]; IDT_ENTRIES]> = Preset::new([MISSING; IDT_ENTRIES]);

/// Set once [`install`] has filled [`IDT`].
static IDT_FILLED: AtomicBool = AtomicBool::new(false);

/// Why the tables could not be installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// `install` ran twice, or something else holds the tables.
    AlreadyInstalled,
}

/// The address of `value`.
fn address_of<T>(value: &T) -> u64 {
    u64::try_from(core::ptr::from_ref(value).addr()).unwrap_or(0)
}

/// Fills and loads the global descriptor table, the task state segment,
/// and the interrupt descriptor table, then reloads the segment registers.
///
/// # Errors
///
/// [`InstallError::AlreadyInstalled`] if the tables are already in place.
///
/// # Safety
///
/// This must run exactly once, on the boot processor, before interrupts
/// are turned on. No other code may hold a reference to the tables.
pub unsafe fn install(kernel_stack_top: u64) -> Result<(), InstallError> {
    if IDT_FILLED.load(Ordering::Acquire) {
        return Err(InstallError::AlreadyInstalled);
    }
    // SAFETY: the caller owns this processor with interrupts disabled.
    unsafe {
        install_private(kernel_stack_top)?;
    }
    let idt_pointer = {
        let mut idt = IDT
            .borrow(&UncontendedToken)
            .map_err(|_| InstallError::AlreadyInstalled)?;
        crate::traps::fill(&mut idt);
        IDT_FILLED.store(true, Ordering::Release);
        table_pointer(address_of(&*idt), size_of_val(&*idt))
    };
    // SAFETY: the table lives in a `static` cell, so it outlives the load,
    // and every gate names a handler with the interrupt calling
    // convention.
    unsafe {
        load_interrupt_descriptor_table(&idt_pointer);
    }
    Ok(())
}

/// Writes the stack pointer the processor takes on a trap from user mode
/// into the task state segment.
///
/// The kernel calls this on every switch to a thread, because that stack
/// is the one of the thread that is about to run.
///
/// # Errors
///
/// [`InstallError::AlreadyInstalled`] while something else holds the
/// segment, and before [`install`] has put one in place.
pub fn set_kernel_stack(top: u64) -> Result<(), InstallError> {
    let mut data = crate::processor::slot(
        &crate::processor::PROCESSORS,
        usize::from(crate::processor::processor().unwrap_or(0)),
    )
    .borrow(&UncontendedToken)
    .map_err(|_| InstallError::AlreadyInstalled)?;
    write_at(&mut data.tss[..], RSP0_OFFSET, &top.to_le_bytes());
    Ok(())
}

/// The stack pointer the task state segment names, for the test that reads
/// it back.
///
/// # Errors
///
/// [`InstallError::AlreadyInstalled`] while something else holds the
/// segment.
pub fn kernel_stack() -> Result<u64, InstallError> {
    let data = crate::processor::slot(
        &crate::processor::PROCESSORS,
        usize::from(crate::processor::processor().unwrap_or(0)),
    )
    .borrow(&UncontendedToken)
    .map_err(|_| InstallError::AlreadyInstalled)?;
    let bytes = data
        .tss
        .get(RSP0_OFFSET..RSP0_OFFSET.saturating_add(8))
        .and_then(|slice| <[u8; 8]>::try_from(slice).ok())
        .unwrap_or([0; 8]);
    Ok(u64::from_le_bytes(bytes))
}

/// The operand of `lgdt` and `lidt` for a table of `len` bytes at `base`.
fn table_pointer(base: u64, len: usize) -> DescriptorTablePointer {
    DescriptorTablePointer {
        limit: u16::try_from(len.saturating_sub(1)).unwrap_or(u16::MAX),
        base,
    }
}

/// Installs the calling processor's GDT, TSS, and double-fault stack.
///
/// # Safety
/// Called once on this processor with interrupts disabled.
unsafe fn install_private(kernel_stack_top: u64) -> Result<(), InstallError> {
    let cpu = usize::from(crate::processor::processor().unwrap_or(0));
    let mut data = crate::processor::slot(&crate::processor::PROCESSORS, cpu)
        .borrow(&UncontendedToken)
        .map_err(|_| InstallError::AlreadyInstalled)?;
    let stack_top =
        address_of(&data.stack).saturating_add(u64::try_from(DOUBLE_FAULT_STACK_LEN).unwrap_or(0));
    data.tss = TaskStateSegment::new()
        .with_kernel_stack(kernel_stack_top)
        .with_interrupt_stack(usize::from(DOUBLE_FAULT_IST), stack_top)
        .to_bytes();
    data.gdt = build_gdt(address_of(&data.tss));
    let pointer = table_pointer(address_of(&data.gdt), size_of_val(&data.gdt));
    // SAFETY: the private static images remain mapped for this processor's lifetime.
    unsafe {
        load_global_descriptor_table(&pointer);
    }
    // SAFETY: the loaded GDT carries these selectors.
    unsafe {
        reload_segments(KERNEL_CODE_SELECTOR.as_u16(), KERNEL_DATA_SELECTOR.as_u16());
    }
    // SAFETY: this processor owns the TSS descriptor in the loaded GDT.
    unsafe {
        load_task_register(TSS_SELECTOR.as_u16());
    }
    Ok(())
}

/// Loads the shared IDT and private descriptors on an application processor.
///
/// # Errors
/// The descriptor images are already borrowed or the IDT is absent.
///
/// # Safety
/// The BSP initialized the IDT; this processor runs here once with interrupts off.
pub unsafe fn install_application(kernel_stack_top: u64) -> Result<(), InstallError> {
    // SAFETY: this processor owns its descriptor images.
    unsafe {
        install_private(kernel_stack_top)?;
    }
    if !IDT_FILLED.load(Ordering::Acquire) {
        return Err(InstallError::AlreadyInstalled);
    }
    let pointer = {
        let idt = IDT
            .borrow(&crate::processor::KernelToken)
            .map_err(|_| InstallError::AlreadyInstalled)?;
        table_pointer(address_of(&*idt), size_of_val(&*idt))
    };
    // SAFETY: the BSP published these permanent interrupt gates before startup.
    unsafe {
        load_interrupt_descriptor_table(&pointer);
    }
    Ok(())
}
