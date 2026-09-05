// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the system calls need from outside this crate.
//!
//! Invariants: an implementation hands out no frame twice before it is
//! released; a mapping it makes is reachable from the address space it
//! names and from no other, apart from the kernel half that every address
//! space shares; a kernel stack slot it hands out is mapped and guarded.

use audhsos_abi::Error;
use kernel_mm::page_table::Permissions;
use kernel_types::{CachePolicy, Page, PhysFrame, VirtAddr};

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
}
