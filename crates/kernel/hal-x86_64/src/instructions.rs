// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One wrapper per privileged instruction, each with exactly one `asm!`
//! site.
//!
//! Invariant: a wrapper is a safe `fn` only when the instruction cannot
//! break a guarantee the rest of the kernel relies on; everything else is
//! an `unsafe fn` whose documentation names the precondition.

use core::arch::asm;

/// Stops the processor until the next interrupt.
pub fn halt() {
    // SAFETY: `hlt` only waits; it changes no register and no memory, and
    // the kernel is prepared for an interrupt to arrive at any point.
    unsafe {
        asm!("hlt", options(nomem, nostack, preserves_flags));
    }
}

/// Stops the processor for good.
pub fn halt_forever() -> ! {
    loop {
        disable_interrupts_and_halt();
    }
}

/// Turns interrupts off and waits, the pair a halted kernel needs.
fn disable_interrupts_and_halt() {
    // SAFETY: the caller never returns from `halt_forever`, so no code
    // that needs interrupts runs afterwards.
    unsafe {
        disable_interrupts();
    }
    halt();
}

/// Turns interrupts off.
///
/// # Safety
///
/// The caller must turn them on again, or never return; a kernel that
/// leaves them off forgets timer ticks.
pub unsafe fn disable_interrupts() {
    // SAFETY: the caller promises to restore the interrupt state.
    unsafe {
        asm!("cli", options(nomem, nostack));
    }
}

/// Turns interrupts on.
///
/// # Safety
///
/// The interrupt descriptor table must be loaded and every vector the
/// hardware can raise must have a handler.
pub unsafe fn enable_interrupts() {
    // SAFETY: the caller promises that the table is loaded.
    unsafe {
        asm!("sti", options(nomem, nostack));
    }
}

/// The flags register.
#[must_use]
pub fn read_flags() -> u64 {
    let flags: u64;
    // SAFETY: reading the flags changes nothing; the stack is used for the
    // push and the pop, which is why the block does not claim `nostack`.
    unsafe {
        asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags));
    }
    flags
}

/// `true` if interrupts are on.
#[must_use]
pub fn interrupts_enabled() -> bool {
    read_flags() & (1 << 9) != 0
}

/// The address the last page fault named.
#[must_use]
pub fn read_fault_address() -> u64 {
    let value: u64;
    // SAFETY: reading `CR2` changes nothing.
    unsafe {
        asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

/// The physical address of the active page table root.
#[must_use]
pub fn read_page_table_root() -> u64 {
    let value: u64;
    // SAFETY: reading `CR3` changes nothing.
    unsafe {
        asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

/// Switches to the page tables rooted at `root`.
///
/// # Safety
///
/// `root` must be the physical address of a valid top-level table that
/// maps the code, the stack, and the data the caller uses next.
pub unsafe fn write_page_table_root(root: u64) {
    // SAFETY: the caller promises that the tables are complete.
    unsafe {
        asm!("mov cr3, {}", in(reg) root, options(nostack, preserves_flags));
    }
}

/// Drops the translation of one page from the lookaside buffer.
///
/// # Safety
///
/// The page tables must already carry the translation the caller wants the
/// processor to see next.
pub unsafe fn invalidate_page(address: u64) {
    // SAFETY: the caller promises that the tables are up to date.
    unsafe {
        asm!("invlpg [{}]", in(reg) address, options(nostack, preserves_flags));
    }
}

/// The operand of `lgdt` and `lidt`: a limit and a base.
#[derive(Clone, Copy, Debug)]
#[repr(C, packed(2))]
pub struct DescriptorTablePointer {
    /// One less than the length of the table in bytes.
    pub limit: u16,
    /// Virtual address of the table.
    pub base: u64,
}

/// Loads the global descriptor table.
///
/// # Safety
///
/// `pointer` must name a table that lives for the rest of the run and
/// whose entries the processor accepts; the selectors in use must stay
/// valid across the load.
pub unsafe fn load_global_descriptor_table(pointer: &DescriptorTablePointer) {
    // SAFETY: the caller promises that the table outlives the load.
    unsafe {
        asm!("lgdt [{}]", in(reg) pointer, options(readonly, nostack, preserves_flags));
    }
}

/// Loads the interrupt descriptor table.
///
/// # Safety
///
/// `pointer` must name a table that lives for the rest of the run and
/// whose gates name handlers with the interrupt calling convention.
pub unsafe fn load_interrupt_descriptor_table(pointer: &DescriptorTablePointer) {
    // SAFETY: the caller promises that the table outlives the load.
    unsafe {
        asm!("lidt [{}]", in(reg) pointer, options(readonly, nostack, preserves_flags));
    }
}

/// Loads the task register.
///
/// # Safety
///
/// `selector` must name a present task state segment descriptor in the
/// loaded global descriptor table.
pub unsafe fn load_task_register(selector: u16) {
    // SAFETY: the caller promises that the descriptor is present.
    unsafe {
        asm!("ltr {0:x}", in(reg) selector, options(nomem, nostack, preserves_flags));
    }
}

/// Reloads the segment registers after a new global descriptor table.
///
/// # Safety
///
/// The table must be loaded and `code` and `data` must name a code and a
/// data segment in it.
pub unsafe fn reload_segments(code: u16, data: u16) {
    // SAFETY: a far return through the stack is the only way to load `CS`;
    // the caller promises that both selectors are valid.
    unsafe {
        asm!(
            "push {code}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            "mov ds, {data:x}",
            "mov es, {data:x}",
            "mov ss, {data:x}",
            code = in(reg) u64::from(code),
            data = in(reg) u64::from(data),
            tmp = lateout(reg) _,
        );
    }
}

/// Reads a byte from a port.
///
/// # Safety
///
/// The port must belong to a device the kernel owns; a read can change the
/// state of the device.
#[must_use]
pub unsafe fn read_port_u8(port: u16) -> u8 {
    let value: u8;
    // SAFETY: the caller promises that the port belongs to the kernel.
    unsafe {
        asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    value
}

/// Writes a byte to a port.
///
/// # Safety
///
/// The port must belong to a device the kernel owns.
pub unsafe fn write_port_u8(port: u16, value: u8) {
    // SAFETY: the caller promises that the port belongs to the kernel.
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
    }
}

/// Writes a double word to a port.
///
/// # Safety
///
/// The port must belong to a device the kernel owns.
pub unsafe fn write_port_u32(port: u16, value: u32) {
    // SAFETY: the caller promises that the port belongs to the kernel.
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
    }
}
