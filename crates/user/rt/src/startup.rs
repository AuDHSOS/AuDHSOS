// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The startup message as a program reads it: named fields instead of
//! pairs.
//!
//! `audhsos_abi::startup` says what stands in the buffer; this says what a
//! program does with it. Every role but one is a field that a process
//! either was given or was not, so a program that needs the name server
//! finds `None` when it was started without one rather than a handle that
//! names nothing.
//!
//! The exception is [`Role::Ram`](audhsos_abi::Role::Ram), which the root
//! task receives once per free region of memory. It is a list, and its
//! capacity is the number of regions the boot information can carry, so a
//! machine whose memory is in as many pieces as the loader can report still
//! fits.
//!
//! Invariants: a role that appears twice is an error and not a field that
//! silently keeps the last of them; the message is read once, before the
//! first system call, because a call overwrites the buffer it stands in.

use audhsos_abi::layout::MAX_BOOT_REGIONS;
use audhsos_abi::startup::{Role, StartupError};
use audhsos_abi::{Buffer, Handle};
use audhsos_collections::ArrayVec;

use crate::handle::{
    EndpointHandle, InterruptHandle, IoPortHandle, MemoryHandle, ProcessHandle,
    SystemControlHandle, Typed,
};

/// How many memory objects the root task can be given. One per usable
/// region the boot information holds, which is the most the kernel can
/// have found.
pub const MAX_RAM_OBJECTS: usize = MAX_BOOT_REGIONS;

/// Why a startup message could not be turned into fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReadError {
    /// The message is none, or a pair of it is unreadable.
    Message(StartupError),
    /// A role that takes one handle appeared twice.
    Duplicate(Role),
    /// More memory objects than [`MAX_RAM_OBJECTS`].
    TooManyRam,
}

impl From<StartupError> for ReadError {
    fn from(error: StartupError) -> Self {
        ReadError::Message(error)
    }
}

impl core::fmt::Display for ReadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReadError::Message(error) => write!(f, "{error}"),
            ReadError::Duplicate(role) => write!(f, "the role {} appears twice", role.name()),
            ReadError::TooManyRam => write!(f, "more than {MAX_RAM_OBJECTS} memory objects"),
        }
    }
}

/// What a process was given at its start.
#[derive(Debug)]
pub struct Startup {
    /// The process itself, which it needs to map memory into its own
    /// address space.
    pub own_process: Option<ProcessHandle>,
    /// The endpoint the process receives requests on.
    pub own_endpoint: Option<EndpointHandle>,
    /// The right to create interrupts, port ranges, and device memory.
    pub system_control: Option<SystemControlHandle>,
    /// The boot image, which carries the archive.
    pub boot_image: Option<MemoryHandle>,
    /// One memory object per free region of memory.
    pub ram: ArrayVec<MemoryHandle, MAX_RAM_OBJECTS>,
    /// The endpoint of the name server.
    pub name_server: Option<EndpointHandle>,
    /// The endpoint of the memory server.
    pub memory_server: Option<EndpointHandle>,
    /// The endpoint diagnostics go to.
    pub log: Option<EndpointHandle>,
    /// The ports the process may reach.
    pub io_ports: Option<IoPortHandle>,
    /// The interrupt the process serves.
    pub interrupt: Option<InterruptHandle>,
}

impl Default for Startup {
    fn default() -> Self {
        Self::new()
    }
}

impl Startup {
    /// A process that was given nothing.
    #[must_use]
    pub const fn new() -> Self {
        Startup {
            own_process: None,
            own_endpoint: None,
            system_control: None,
            boot_image: None,
            ram: ArrayVec::new(),
            name_server: None,
            memory_server: None,
            log: None,
            io_ports: None,
            interrupt: None,
        }
    }

    /// Reads the startup message that stands in `buffer`.
    ///
    /// # Errors
    ///
    /// [`ReadError`] for a message that is none, that carries a pair which
    /// is unreadable, that gives one role twice, or that carries more
    /// memory objects than the list holds.
    pub fn read(buffer: Buffer<'_>) -> Result<Self, ReadError> {
        let mut startup = Startup::new();
        for given in audhsos_abi::startup::read(buffer)? {
            startup.take(given.role, given.handle)?;
        }
        Ok(startup)
    }

    /// Puts one handle into the field its role names.
    fn take(&mut self, role: Role, handle: Handle) -> Result<(), ReadError> {
        match role {
            Role::OwnProcess => once(&mut self.own_process, role, handle),
            Role::OwnEndpoint => once(&mut self.own_endpoint, role, handle),
            Role::SystemControl => once(&mut self.system_control, role, handle),
            Role::BootImage => once(&mut self.boot_image, role, handle),
            Role::NameServer => once(&mut self.name_server, role, handle),
            Role::MemoryServer => once(&mut self.memory_server, role, handle),
            Role::Log => once(&mut self.log, role, handle),
            Role::IoPorts => once(&mut self.io_ports, role, handle),
            Role::Interrupt => once(&mut self.interrupt, role, handle),
            Role::Ram => self
                .ram
                .push(MemoryHandle::from_handle(handle))
                .map_err(|_| ReadError::TooManyRam),
        }
    }
}

/// Fills a field that may be given once.
fn once<T: Typed>(field: &mut Option<T>, role: Role, handle: Handle) -> Result<(), ReadError> {
    if field.is_some() {
        return Err(ReadError::Duplicate(role));
    }
    *field = Some(T::from_handle(handle));
    Ok(())
}
