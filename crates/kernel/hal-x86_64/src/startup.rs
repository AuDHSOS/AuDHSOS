// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! INIT-SIPI startup (Intel SDM Vol. 3A, 11.4.4.1).
use crate::window::PhysicalWindow;
use core::arch::naked_asm;
use kernel_types::PhysFrame;

/// Reset-mode code copied below 1 MiB.
///
/// # Safety
/// Entered only by SIPI with the parameter block initialized.
#[unsafe(naked)]
#[unsafe(link_section = ".apstart")]
pub unsafe extern "C" fn trampoline() -> ! {
    naked_asm!(
        ".code16",
        "cli",
        "cld",
        "mov ax, cs",
        "mov ds, ax",
        "movzx ebx, ax",
        "shl ebx, 4",
        "lgdt [0xf20]",
        "mov eax, cr4",
        "or eax, 0x620",
        "mov cr4, eax",
        "mov eax, dword ptr [0xf00]",
        "mov cr3, eax",
        "mov ecx, 0xc0000080",
        "rdmsr",
        "or eax, 0x900",
        "wrmsr",
        "mov eax, cr0",
        "and eax, 0x9ffffffb",
        "or eax, 0x80010023",
        "mov cr0, eax",
        "jmp fword ptr [0xf48]",
        ".code64",
        "2:",
        ".global __apstart_long",
        ".set __apstart_long, 2b",
        "xor eax, eax",
        "mov ds, ax",
        "mov es, ax",
        "mov ss, ax",
        "mov rsp, [rbx + 0xf10]",
        "sub rsp, 8",
        "mov rdi, [rbx + 0xf18]",
        "jmp qword ptr [rbx + 0xf08]",
    )
}

unsafe extern "C" {
    static __apstart_start: u8;
    static __apstart_end: u8;
    static __apstart_long: u8;
}

/// Writes code and parameters through the physical window.
///
/// # Safety
/// The frame is reserved, identity-mapped, and no AP still reads its parameters.
pub unsafe fn prepare(
    frame: PhysFrame,
    root: PhysFrame,
    entry: extern "C" fn(u64) -> !,
    stack: u64,
    cpu: u8,
) -> Option<()> {
    let root = u32::try_from(root.start().as_u64()).ok()?;
    let base = u32::try_from(frame.start().as_u64()).ok()?;
    let start = core::ptr::addr_of!(__apstart_start);
    let end = core::ptr::addr_of!(__apstart_end);
    let length = end.addr().checked_sub(start.addr())?;
    if length >= 0xf00 {
        return None;
    }
    // SAFETY: linker symbols bound the retained startup section.
    let code = unsafe { core::slice::from_raw_parts(start, length) };
    // SAFETY: the kernel owns the reserved frame exclusively until SIPI.
    let mut window = unsafe { PhysicalWindow::kernel() };
    let bytes = window.frame_bytes_mut(frame)?;
    bytes.fill(0);
    bytes.get_mut(..length)?.copy_from_slice(code);
    put(bytes, 0xf00, &root.to_le_bytes())?;
    #[expect(
        clippy::as_conversions,
        reason = "a function address stored in the startup ABI"
    )]
    let entry_address = (entry as *const ()).addr();
    put(bytes, 0xf08, &entry_address.to_le_bytes())?;
    put(bytes, 0xf10, &stack.to_le_bytes())?;
    put(bytes, 0xf18, &u64::from(cpu).to_le_bytes())?;
    put(bytes, 0xf20, &23u16.to_le_bytes())?;
    put(bytes, 0xf22, &(base.checked_add(0xf30)?).to_le_bytes())?;
    put(bytes, 0xf38, &0x00cf_9a00_0000_ffffu64.to_le_bytes())?;
    put(bytes, 0xf40, &0x00af_9a00_0000_ffffu64.to_le_bytes())?;
    let suffix = core::ptr::addr_of!(__apstart_long)
        .addr()
        .checked_sub(start.addr())?;
    put(
        bytes,
        0xf48,
        &(base.checked_add(u32::try_from(suffix).ok()?)?).to_le_bytes(),
    )?;
    put(bytes, 0xf4c, &16u16.to_le_bytes())?;
    Some(())
}

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) -> Option<()> {
    bytes
        .get_mut(offset..offset.checked_add(value.len())?)?
        .copy_from_slice(value);
    Some(())
}
