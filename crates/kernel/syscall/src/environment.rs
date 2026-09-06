// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the system calls need from outside this crate.
//!
//! Invariants: an implementation hands out no frame twice before it is
//! released; a mapping it makes is reachable from the address space it
//! names and from no other, apart from the kernel half that every address
//! space shares; a kernel stack slot it hands out is mapped and guarded.

use audhsos_abi::Error;
use audhsos_abi::ipc_buffer::SIZE;
use kernel_mm::page_table::Permissions;
use kernel_types::{CachePolicy, Page, PhysFrame, PhysFrameRange, VirtAddr};

/// A kernel stack the kernel handed out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KernelStack {
    /// The slot of the kernel stack area.
    pub slot: u32,
    /// The address one past the top of the stack, which is what the task
    /// state segment and the first switch need.
    pub top: VirtAddr,
}

/// Everything the calls do that is not a change to an object.
///
/// The kernel implements this over its memory bring-up and its console; a
/// test implements it with a recording double, which is what makes every
/// error path of every call reachable on the host.
pub trait Environment {
    /// A fresh address space: a page-table root that carries the kernel
    /// half and maps nothing of the user half.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfKernelMemory`] when the reserve has no frame left.
    fn create_address_space(&mut self) -> Result<PhysFrame, Error>;

    /// Releases an address space and every page table below it. The
    /// mappings of the user half are gone with it; the kernel half is
    /// shared and stays.
    fn destroy_address_space(&mut self, root: PhysFrame);

    /// Maps `page` of the address space at `root` to `frame`.
    ///
    /// # Errors
    ///
    /// [`Error::AlreadyMapped`] when the page is mapped;
    /// [`Error::OutOfKernelMemory`] when a page table is missing and the
    /// reserve has no frame left; [`Error::InvalidArgument`] for a page
    /// outside the user half.
    fn map(
        &mut self,
        root: PhysFrame,
        page: Page,
        frame: PhysFrame,
        perms: Permissions,
        cache: CachePolicy,
    ) -> Result<(), Error>;

    /// Unmaps `page` of the address space at `root`.
    ///
    /// # Errors
    ///
    /// [`Error::NotMapped`] when the page carries no mapping.
    fn unmap(&mut self, root: PhysFrame, page: Page) -> Result<(), Error>;

    /// Changes what `page` of the address space at `root` allows.
    ///
    /// # Errors
    ///
    /// [`Error::NotMapped`] when the page carries no mapping.
    fn protect(&mut self, root: PhysFrame, page: Page, perms: Permissions) -> Result<(), Error>;

    /// A kernel stack for a new thread.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfKernelMemory`] when no slot is free or the reserve has
    /// no frame left.
    fn allocate_kernel_stack(&mut self) -> Result<KernelStack, Error>;

    /// Writes the frame a new thread returns through into the top of its
    /// kernel stack, and answers with the word the switch loads as its
    /// stack pointer.
    ///
    /// This is the one step of starting a thread that knows what a
    /// processor register is, and it is why the trait has it: a thread the
    /// kernel creates but never prepares is one the first switch jumps into
    /// with a stack pointer of zero.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfKernelMemory`] when the top of the stack is not
    /// reachable, which no stack the kernel has just allocated is.
    fn prepare_thread(
        &mut self,
        stack_top: VirtAddr,
        entry: VirtAddr,
        user_stack: VirtAddr,
        ipc_buffer: VirtAddr,
    ) -> Result<VirtAddr, Error>;

    /// Returns the kernel stack in `slot`.
    fn release_kernel_stack(&mut self, slot: u32);

    /// A zeroed frame of the reserve, for the IPC buffer of a thread.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfKernelMemory`] when the reserve has no frame left.
    fn allocate_frame(&mut self) -> Result<PhysFrame, Error>;

    /// Returns a frame obtained from [`Environment::allocate_frame`].
    fn release_frame(&mut self, frame: PhysFrame);

    /// Writes the bytes of a `debug_log` call. A build without the debug
    /// console drops them.
    fn log(&mut self, bytes: &[u8]);

    /// Runs `body` on the IPC buffer in `frame`.
    ///
    /// This is the seam to a second buffer. The dispatcher holds the buffer
    /// of the calling thread and nothing else, and every transfer has the
    /// caller on one side: a send copies out of the caller's buffer into the
    /// one this reaches, a receive copies the other way.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] when the frame is not reachable, which no
    /// IPC buffer of a live thread is.
    fn with_buffer<R>(
        &mut self,
        frame: PhysFrame,
        body: impl FnOnce(&mut [u8; SIZE]) -> R,
    ) -> Result<R, Error>;

    /// Reads `width` bytes from `port`.
    ///
    /// The range check is the caller's: this is reached only after an
    /// `IoPortRange` capability has allowed the access.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] for a width that is not 1, 2, or 4.
    fn read_port(&mut self, port: u16, width: u8) -> Result<u64, Error>;

    /// Writes the low `width` bytes of `value` to `port`.
    ///
    /// # Errors
    ///
    /// As [`Environment::read_port`].
    fn write_port(&mut self, port: u16, width: u8, value: u64) -> Result<(), Error>;

    /// The vector the plan of this machine routes `line` to, or `None` for a
    /// line it reserves no vector for.
    ///
    /// The plan is the architecture's, which is why this is a question and
    /// not a table of this crate: nothing here depends on
    /// `kernel-x86-tables` and nothing here will start to.
    fn interrupt_vector(&self, line: u8) -> Option<u8>;

    /// Routes `line` to `vector` at the interrupt controller, masked.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] for a line the controller does not have;
    /// [`Error::AlreadyExists`] for one it has already routed.
    fn route_interrupt(&mut self, line: u8, vector: u8) -> Result<(), Error>;

    /// Stops delivery of `line`.
    fn mask_interrupt(&mut self, line: u8);

    /// Resumes delivery of `line`.
    fn unmask_interrupt(&mut self, line: u8);

    /// `true` when any frame of `frames` is memory the machine reported as
    /// usable. A device object is an aperture, and the frames of the memory
    /// map belong to the memory server.
    fn meets_ram(&self, frames: PhysFrameRange) -> bool;

    /// The physical address of the root system description pointer, or zero
    /// when the platform named none. It is the only thing of the firmware
    /// the kernel keeps, and `system_info` is what reports it.
    fn acpi_pointer(&self) -> u64;
}
