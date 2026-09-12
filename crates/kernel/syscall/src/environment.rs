// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the system calls need from outside this crate.
//!
//! Invariants: an implementation hands out no frame twice before it is
//! released; a mapping it makes is reachable from the address space it
//! names and from no other, apart from the kernel half that every address
//! space shares; a kernel stack slot it hands out is mapped and guarded.

use audhsos_abi::ipc_buffer::SIZE;
use audhsos_abi::{Ecam, Error, Framebuffer, WallClockSource};
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

    /// Writes every byte of `bytes` to `port`, one after another.
    ///
    /// The range check is the caller's, as for [`Environment::read_port`].
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] on a machine without port access.
    fn write_port_string(&mut self, port: u16, bytes: &[u8]) -> Result<(), Error>;

    /// The microseconds since the kernel started, at the resolution of the
    /// timer tick. `clock_now` answers this word, and every deadline of the
    /// interface is in the same scale.
    fn now_micros(&self) -> u64;

    /// The moment the firmware clock stood at when the loader read it, in
    /// seconds from the Unix epoch, and how far it can be trusted; `None`
    /// on a machine that reported no clock.
    ///
    /// It is the moment of the boot and not the moment of the call.
    /// `clock_wall` adds [`Environment::now_micros`] to it, which is why
    /// the drift of that count is the drift of the wall clock too.
    fn boot_wall(&self) -> Option<(i64, WallClockSource)>;

    /// Four words drawn from the entropy source of the machine, which is
    /// the thirty-two bytes a stream cipher takes as a seed.
    ///
    /// # Errors
    ///
    /// [`Error::Unavailable`] when a word could not be drawn inside the
    /// retry bound of the adapter, or when the machine has no source. No
    /// partial result reaches the caller.
    fn random_seed(&mut self) -> Result<[u64; 4], Error>;

    /// Takes one vector out of the message interrupt space and answers with
    /// the vector, the address a device writes to, and the value it writes.
    ///
    /// Nothing is routed and nothing is masked: a message interrupt reaches
    /// the processor because the driver programmed its device to write
    /// there.
    ///
    /// # Errors
    ///
    /// [`Error::NoVector`] when the space has nothing left;
    /// [`Error::Unsupported`] on a machine whose controller is not up.
    fn allocate_message_vector(&mut self) -> Result<(u8, u64, u32), Error>;

    /// Gives a message interrupt vector back to the space it came from.
    fn release_message_vector(&mut self, vector: u8);

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

    /// `true` when `frames` lies wholly inside one aperture the machine
    /// reported as device memory. `memory_create_device` makes an object
    /// only over such a range: everything else is either memory of the
    /// machine, which belongs to the memory server, or nothing at all.
    fn is_device_memory(&self, frames: PhysFrameRange) -> bool;

    /// The framebuffer the loader described, if the machine has one.
    fn framebuffer(&self) -> Option<Framebuffer>;

    /// The physical address of the root system description pointer, or zero
    /// when the platform named none. It is the only thing of the firmware
    /// the kernel keeps, and `system_info` is what reports it.
    fn acpi_pointer(&self) -> u64;

    /// The configuration window of the bus, if the firmware published one.
    /// `system_info` reports it, and the root task makes the device memory
    /// object of the program that enumerates out of it.
    fn ecam(&self) -> Option<Ecam>;
}
