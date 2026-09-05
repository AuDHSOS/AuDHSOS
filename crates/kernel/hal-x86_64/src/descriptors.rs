// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The descriptor tables the processor needs before it can take a trap.
//!
//! Invariants: the tables live for the whole run, because they are
//! `static`; the task state segment descriptor names the byte image that
//! lives next to it; the tables are loaded once, before interrupts are
//! turned on.

use audhsos_sync::{Global, UncontendedToken};

use kernel_x86_tables::gdt::{
    GDT_ENTRIES, KERNEL_CODE_SELECTOR, KERNEL_DATA_SELECTOR, TSS_SELECTOR, build_gdt,
};
use kernel_x86_tables::idt::{DOUBLE_FAULT_IST, IDT_ENTRIES, MISSING};
use kernel_x86_tables::tss::{TSS_LEN, TaskStateSegment};

use crate::instructions::{
    DescriptorTablePointer, load_global_descriptor_table, load_interrupt_descriptor_table,
    load_task_register, reload_segments,
};

/// Size of the stack the double fault handler runs on.
pub const DOUBLE_FAULT_STACK_LEN: usize = 16 * 1024;

/// The stack the double fault handler runs on, so that a kernel stack
/// overflow still has room to report itself.
static DOUBLE_FAULT_STACK: Global<[u8; DOUBLE_FAULT_STACK_LEN]> = Global::new();

/// The byte image of the task state segment.
static TSS_IMAGE: Global<[u8; TSS_LEN]> = Global::new();

/// The global descriptor table.
static GDT: Global<[u64; GDT_ENTRIES]> = Global::new();

/// The interrupt descriptor table.
static IDT: Global<[[u64; 2]; IDT_ENTRIES]> = Global::new();

/// Why the tables could not be installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// `install` ran twice, or something else holds the tables.
    AlreadyInstalled,
}

/// The address a static array of `len` bytes at `base` ends at, which is
/// where a stack starts growing down from.
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
    let token = UncontendedToken;
    DOUBLE_FAULT_STACK
        .init([0; DOUBLE_FAULT_STACK_LEN])
        .map_err(|_| InstallError::AlreadyInstalled)?;
    let stack_top = {
        let stack = DOUBLE_FAULT_STACK
            .borrow(&token)
            .map_err(|_| InstallError::AlreadyInstalled)?;
        address_of(&*stack).saturating_add(u64::try_from(DOUBLE_FAULT_STACK_LEN).unwrap_or(0))
    };

    let segment = TaskStateSegment::new()
        .with_kernel_stack(kernel_stack_top)
        .with_interrupt_stack(usize::from(DOUBLE_FAULT_IST), stack_top);
    TSS_IMAGE
        .init(segment.to_bytes())
        .map_err(|_| InstallError::AlreadyInstalled)?;
    let tss_base = {
        let image = TSS_IMAGE
            .borrow(&token)
            .map_err(|_| InstallError::AlreadyInstalled)?;
        address_of(&*image)
    };

    GDT.init(build_gdt(tss_base))
        .map_err(|_| InstallError::AlreadyInstalled)?;
    let gdt_pointer = {
        let gdt = GDT
            .borrow(&token)
            .map_err(|_| InstallError::AlreadyInstalled)?;
        table_pointer(address_of(&*gdt), size_of_val(&*gdt))
    };
    // SAFETY: the table lives in a `static` cell, so it outlives the load,
    // and its entries come from `build_gdt`, which the host tests check.
    unsafe {
        load_global_descriptor_table(&gdt_pointer);
    }
    // SAFETY: the table just loaded holds a code and a data segment at
    // these selectors.
    unsafe {
        reload_segments(KERNEL_CODE_SELECTOR.as_u16(), KERNEL_DATA_SELECTOR.as_u16());
    }
    // SAFETY: the table just loaded holds the task state segment
    // descriptor at this selector, and it is present.
    unsafe {
        load_task_register(TSS_SELECTOR.as_u16());
    }

    let mut table = [MISSING; IDT_ENTRIES];
    crate::traps::fill(&mut table);
    IDT.init(table)
        .map_err(|_| InstallError::AlreadyInstalled)?;
    let idt_pointer = {
        let idt = IDT
            .borrow(&token)
            .map_err(|_| InstallError::AlreadyInstalled)?;
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

/// The operand of `lgdt` and `lidt` for a table of `len` bytes at `base`.
fn table_pointer(base: u64, len: usize) -> DescriptorTablePointer {
    DescriptorTablePointer {
        limit: u16::try_from(len.saturating_sub(1)).unwrap_or(u16::MAX),
        base,
    }
}
