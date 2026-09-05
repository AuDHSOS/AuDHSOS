// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The last four instructions of the loader.
//!
//! Invariant: nothing after the jump belongs to the loader; the kernel
//! finds the boot information address in its first argument and the stack
//! the loader mapped in `RSP`.

use core::arch::naked_asm;

/// Turns interrupts off, activates `cr3`, moves to `stack_top`, and jumps
/// to `entry` with `boot_info` in the first argument of the `sysv64` call
/// the kernel entry point expects.
///
/// The function itself takes its arguments in the `sysv64` registers
/// `RDI`, `RSI`, `RDX`, and `RCX`, which is why the body can move them
/// into place without touching memory: the loader's own stack is gone
/// after the second instruction.
///
/// # Safety
///
/// `cr3` must be the root of tables that map the loader's own code
/// identically, the kernel at `entry`, the stack below `stack_top`, and
/// the boot information page at `boot_info`.
#[unsafe(naked)]
pub(crate) unsafe extern "sysv64" fn enter_kernel(
    cr3: u64,
    stack_top: u64,
    boot_info: u64,
    entry: u64,
) -> ! {
    naked_asm!(
        "cli",
        "mov cr3, rdi",
        "mov rsp, rsi",
        "mov rdi, rdx",
        "jmp rcx"
    )
}
