// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Layout constants shared by the loader, the kernel, and the userland.
//!
//! Invariants: every constant here is part of the ABI; a change is an ABI
//! version change.

/// Size of a page and of a frame in bytes.
pub const PAGE_SIZE: u64 = 4096;

/// `log2(PAGE_SIZE)`.
pub const PAGE_SHIFT: u32 = 12;

/// Number of address bits a virtual address carries before sign extension.
pub const VIRT_ADDR_BITS: u32 = 48;

/// Number of address bits a physical address may use.
pub const PHYS_ADDR_BITS: u32 = 52;

/// Lowest mappable user address; the range below it is never mapped.
pub const USER_SPACE_START: u64 = 0x1_0000;

/// First address above the user half (exclusive end of user space).
pub const USER_SPACE_END: u64 = 0x0000_8000_0000_0000;

/// First address of the kernel half.
pub const KERNEL_SPACE_START: u64 = 0xFFFF_8000_0000_0000;

/// Virtual base of the physical memory window.
pub const PHYS_WINDOW_BASE: u64 = 0xFFFF_8000_0000_0000;

/// Virtual base the kernel image is linked at.
pub const KERNEL_BASE: u64 = 0xFFFF_FFFF_8000_0000;

/// Virtual base the root task is linked at.
pub const ROOT_TASK_BASE: u64 = 0x0000_0000_1000_0000;

/// Address one past the boot stack; the stack grows down from here.
pub const BOOT_STACK_TOP: u64 = KERNEL_BASE - 0x0100_0000;

/// Number of pages the boot stack occupies, guard page excluded.
pub const BOOT_STACK_PAGES: u64 = 16;

/// Virtual address the loader maps the boot information page at.
pub const BOOT_INFO_VADDR: u64 = KERNEL_BASE - 0x0200_0000;

/// Maximum number of regions in the boot information structure.
pub const MAX_BOOT_REGIONS: usize = 128;

/// Size of the IPC buffer of a thread in bytes.
pub const IPC_BUFFER_SIZE: u64 = PAGE_SIZE;

/// Maximum number of payload words in a message.
pub const MAX_MESSAGE_WORDS: usize = 480;

/// Maximum number of handles in a message.
pub const MAX_MESSAGE_HANDLES: usize = 4;

/// Number of scheduling priorities; the highest priority is
/// `PRIORITY_COUNT - 1`.
pub const PRIORITY_COUNT: u8 = 32;

/// Timer ticks per second.
pub const TICKS_PER_SECOND: u32 = 1000;

/// Length of a scheduling time slice in ticks.
pub const DEFAULT_TIME_SLICE_TICKS: u32 = 10;

/// Maximum number of pages a range operation processes per system call
/// before returning `Partial`.
pub const MAX_PAGES_PER_CALL: u64 = 64;

/// Number of bits of the table index inside a handle.
pub const HANDLE_INDEX_BITS: u32 = 32;

/// Number of bits of the generation inside a handle.
pub const HANDLE_GENERATION_BITS: u32 = 32;

const _: () = assert!(PAGE_SIZE == 1 << PAGE_SHIFT);
const _: () = assert!(USER_SPACE_START.is_multiple_of(PAGE_SIZE));
const _: () = assert!(USER_SPACE_END == 1 << (VIRT_ADDR_BITS - 1));
const _: () = assert!(KERNEL_SPACE_START == u64::MAX - USER_SPACE_END + 1);
const _: () = assert!(PHYS_WINDOW_BASE >= KERNEL_SPACE_START);
const _: () = assert!(KERNEL_BASE > PHYS_WINDOW_BASE);
const _: () = assert!(ROOT_TASK_BASE >= USER_SPACE_START && ROOT_TASK_BASE < USER_SPACE_END);
const _: () = assert!(ROOT_TASK_BASE.is_multiple_of(PAGE_SIZE));
const _: () = assert!(MAX_MESSAGE_WORDS * 8 + MAX_MESSAGE_HANDLES * 8 + 3 * 8 + 10 * 8 <= 4096);
const _: () = assert!(BOOT_STACK_TOP.is_multiple_of(PAGE_SIZE));
const _: () = assert!(BOOT_INFO_VADDR.is_multiple_of(PAGE_SIZE));
const _: () = assert!(BOOT_STACK_TOP < KERNEL_BASE);
// The boot stack and its guard page stay clear of the boot information.
const _: () =
    assert!(BOOT_INFO_VADDR + PAGE_SIZE <= BOOT_STACK_TOP - (BOOT_STACK_PAGES + 1) * PAGE_SIZE);
