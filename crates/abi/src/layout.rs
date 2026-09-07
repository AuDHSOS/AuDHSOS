// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Layout constants shared by the loader, the kernel, and the userland.
//!
//! Invariants: every constant here is part of the ABI; a change is an ABI
//! version change.

#![expect(
    clippy::as_conversions,
    reason = "the one conversion here widens a slot index that the bound above it keeps small, in a const fn"
)]

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

/// Number of pages the boot stack occupies, guard page excluded. The
/// unoptimized build of a kernel test image needs well over 64 KiB before
/// it reaches the harness, so the stack is a quarter of a mebibyte.
pub const BOOT_STACK_PAGES: u64 = 64;

/// Virtual address the loader maps the boot information page at.
pub const BOOT_INFO_VADDR: u64 = KERNEL_BASE - 0x0200_0000;

/// Virtual base of the kernel stack area. Each stack occupies a slot of
/// [`KERNEL_STACK_SLOT_PAGES`] pages: one guard page that stays unmapped,
/// then [`KERNEL_STACK_PAGES`] mapped pages the stack grows down through.
pub const KERNEL_STACKS_BASE: u64 = KERNEL_BASE - 0x4000_0000;

/// Number of mapped pages of one kernel stack.
///
/// Eight, not the four of D-57, and for the reason `BOOT_STACK_PAGES` is
/// sixty-four: the unoptimized build the tests run needs the room. The
/// deepest path from the system call gate is `process_create`, which
/// carries a `Process` — four kilobytes of region table and thread slots —
/// through three frames of the object pool; it measures twenty-three
/// kilobytes, and sixteen is not enough for it (D-73).
pub const KERNEL_STACK_PAGES: u64 = 8;

/// Number of pages one kernel stack slot occupies, guard page included.
pub const KERNEL_STACK_SLOT_PAGES: u64 = KERNEL_STACK_PAGES + 1;

/// Number of kernel stack slots the area holds. One slot per thread the
/// machine can hold, which is what `kernel-core::config::THREADS` says
/// (D-57).
pub const KERNEL_STACK_SLOTS: u64 = 256;

/// Largest amount of physical memory the window can map before it would
/// reach the kernel stack area.
pub const MAX_PHYS_WINDOW_BYTES: u64 = KERNEL_STACKS_BASE - PHYS_WINDOW_BASE;

/// Maximum number of regions in the boot information structure.
pub const MAX_BOOT_REGIONS: usize = 128;

/// Size of the IPC buffer of a thread in bytes.
pub const IPC_BUFFER_SIZE: u64 = PAGE_SIZE;

/// The page the IPC buffer of the first thread of a process is mapped at.
/// The buffer of the thread in slot `n` lies `n` pages below it, so the
/// buffers of one process are the top of its address space and nothing
/// else is ever mapped there.
pub const IPC_BUFFER_TOP: u64 = USER_SPACE_END - PAGE_SIZE;

/// The address the IPC buffer of the thread in slot `index` is mapped at,
/// or `None` for a slot the address space has no room for.
#[must_use]
pub const fn ipc_buffer_address(index: usize) -> Option<u64> {
    if index >= THREADS_PER_PROCESS {
        return None;
    }
    let offset = (index as u64).wrapping_mul(PAGE_SIZE);
    IPC_BUFFER_TOP.checked_sub(offset)
}

/// Maximum number of argument words a system call reads from the IPC
/// buffer.
pub const MAX_SYSCALL_ARGUMENTS: usize = 6;

/// Maximum number of return words a system call writes into the IPC
/// buffer, the status word excluded.
pub const MAX_SYSCALL_RETURN_WORDS: usize = 2;

/// Maximum number of payload words in a message.
pub const MAX_MESSAGE_WORDS: usize = 480;

/// Maximum number of handles in a message.
pub const MAX_MESSAGE_HANDLES: usize = 4;

/// Maximum number of words a system call result writes into the message
/// area of the caller's own buffer when it does not fit into
/// [`MAX_SYSCALL_RETURN_WORDS`] return words. `system_info` writes
/// twenty-six, which is the widest result of this interface: twenty about
/// the kernel and six about the framebuffer.
pub const MAX_RESULT_WORDS: usize = 26;

/// Number of threads one process may hold.
pub const THREADS_PER_PROCESS: usize = 64;

/// Number of regions one process may map.
pub const REGIONS_PER_PROCESS: usize = 64;

/// How many watchers may wait for the end of one process.
///
/// A watcher is a server that holds something of a program and wants it
/// back when the program is gone: the display server holds its surface, and
/// in a later phase the input server holds its subscription. Four is more
/// than this system has servers of that kind; a fifth is refused rather
/// than dropped, so nobody believes it is being watched when it is not.
pub const WATCHERS_PER_PROCESS: usize = 4;

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
const _: () = assert!(MAX_RESULT_WORDS <= MAX_MESSAGE_WORDS);
const _: () = assert!(BOOT_STACK_TOP.is_multiple_of(PAGE_SIZE));
const _: () = assert!(BOOT_INFO_VADDR.is_multiple_of(PAGE_SIZE));
const _: () = assert!(BOOT_STACK_TOP < KERNEL_BASE);
// The boot stack and its guard page stay clear of the boot information.
const _: () =
    assert!(BOOT_INFO_VADDR + PAGE_SIZE <= BOOT_STACK_TOP - (BOOT_STACK_PAGES + 1) * PAGE_SIZE);
const _: () = assert!(KERNEL_STACKS_BASE.is_multiple_of(PAGE_SIZE));
const _: () = assert!(KERNEL_STACKS_BASE > PHYS_WINDOW_BASE);
// The stack area ends below the boot information page.
const _: () = assert!(
    KERNEL_STACKS_BASE + KERNEL_STACK_SLOTS * KERNEL_STACK_SLOT_PAGES * PAGE_SIZE
        <= BOOT_INFO_VADDR
);

// The buffers of a process lie inside its own address space, below the
// end of the user half and above everything a program is linked at.
const _: () = assert!(IPC_BUFFER_TOP + PAGE_SIZE == USER_SPACE_END);
const _: () = {
    match ipc_buffer_address(THREADS_PER_PROCESS - 1) {
        Some(lowest) => assert!(lowest > ROOT_TASK_BASE),
        None => panic!("every slot of a process has a buffer"),
    }
};
const _: () = assert!(ipc_buffer_address(THREADS_PER_PROCESS).is_none());
