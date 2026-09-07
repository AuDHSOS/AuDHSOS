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

/// Turns interrupts off for as long as it lives, and back on when it goes
/// if they were on when it was made.
///
/// This is the guard the safety policy names: kernel state that an
/// interrupt handler also reaches is borrowed behind one of these, so that
/// no handler ever finds the cell busy.
#[derive(Debug)]
pub struct InterruptGuard {
    restore: bool,
}

impl InterruptGuard {
    /// Turns interrupts off and remembers whether they were on.
    #[must_use]
    pub fn new() -> Self {
        let restore = interrupts_enabled();
        if restore {
            // SAFETY: the guard turns them on again when it goes out of
            // scope, which is what `disable_interrupts` asks of a caller.
            unsafe {
                disable_interrupts();
            }
        }
        InterruptGuard { restore }
    }
}

impl Default for InterruptGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        if self.restore {
            // SAFETY: interrupts were on when the guard was made, so the
            // descriptor table is loaded and every vector the hardware can
            // raise has a handler; this only restores what was found.
            unsafe {
                enable_interrupts();
            }
        }
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

/// The time-stamp counter, which counts on since the processor was reset.
///
/// The instruction is not serializing: the processor may read the counter
/// before an earlier instruction has retired and after a later one has
/// begun, so a single difference of two reads is not a measurement. What
/// this is for is the median of many, where that noise cancels and the
/// figure that remains is the one
/// [08-roadmap.md 8.10](../../../../docs/08-roadmap.md) records.
#[must_use]
pub fn read_tsc() -> u64 {
    let low: u32;
    let high: u32;
    // SAFETY: reading the counter changes no register the caller holds and
    // no memory; it is readable at every privilege level this kernel runs
    // at, because nothing of it sets the time-stamp disable bit of `CR4`.
    unsafe {
        asm!("rdtsc", out("eax") low, out("edx") high, options(nomem, nostack, preserves_flags));
    }
    u64::from(low) | (u64::from(high) << 32)
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

/// Reads a model-specific register.
///
/// # Safety
///
/// The processor must implement `msr`; a register it does not implement
/// raises a general protection fault.
#[must_use]
pub unsafe fn read_msr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    // SAFETY: the caller promises that the processor implements the
    // register; the instruction reads two registers and changes no memory.
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    u64::from(low) | (u64::from(high) << 32)
}

/// Writes a model-specific register.
///
/// # Safety
///
/// The processor must implement `msr`, and `value` must be one it accepts:
/// a reserved bit or an unimplemented register raises a general protection
/// fault, and the register decides how the processor behaves afterwards.
pub unsafe fn write_msr(msr: u32, value: u64) {
    let low = u32::try_from(value & 0xFFFF_FFFF).unwrap_or(0);
    let high = u32::try_from(value >> 32).unwrap_or(0);
    // SAFETY: the caller promises that the register exists and that the
    // value is one it accepts.
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") low,
            in("edx") high,
            options(nomem, nostack, preserves_flags),
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

/// Reads a word from a port.
///
/// # Safety
///
/// The port must belong to a device the kernel owns; a read can change the
/// state of the device.
#[must_use]
pub unsafe fn read_port_u16(port: u16) -> u16 {
    let value: u16;
    // SAFETY: the caller promises that the port belongs to the kernel.
    unsafe {
        asm!("in ax, dx", out("ax") value, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    value
}

/// Writes a word to a port.
///
/// # Safety
///
/// The port must belong to a device the kernel owns.
pub unsafe fn write_port_u16(port: u16, value: u16) {
    // SAFETY: the caller promises that the port belongs to the kernel.
    unsafe {
        asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
    }
}

/// Reads a double word from a port.
///
/// # Safety
///
/// The port must belong to a device the kernel owns; a read can change the
/// state of the device.
#[must_use]
pub unsafe fn read_port_u32(port: u16) -> u32 {
    let value: u32;
    // SAFETY: the caller promises that the port belongs to the kernel.
    unsafe {
        asm!("in eax, dx", out("eax") value, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    value
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
