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
//! The exception is [`Role::Ram`], which the root
//! task receives once per free region of memory. It is a list, and its
//! capacity is the number of regions the boot information can carry, so a
//! machine whose memory is in as many pieces as the loader can report still
//! fits.
//!
//! Invariants: a role that appears twice is an error and not a field that
//! silently keeps the last of them; the message is read once, before the
//! first system call, because a call overwrites the buffer it stands in.

use audhsos_abi::layout::MAX_BOOT_REGIONS;
use audhsos_abi::startup::{BusRange, Location, Payload, Role, Screen, StartupError};
use audhsos_abi::{Buffer, Handle};
use audhsos_collections::ArrayVec;

use crate::handle::{
    EndpointHandle, InterruptHandle, IoPortHandle, MemoryHandle, NotificationHandle, ProcessHandle,
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
    /// More block devices than [`MAX_BLOCK_DEVICES`].
    TooManyBlocks,
    /// A role that describes a block device stood before the
    /// [`Role::BlockRegisters`] that opens one.
    BlockWithoutRegisters(Role),
    /// A role that describes a device stood before the register window
    /// that opens one.
    DeviceWithoutRegisters(Role),
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
            ReadError::TooManyBlocks => {
                write!(f, "more than {MAX_BLOCK_DEVICES} block devices")
            }
            ReadError::BlockWithoutRegisters(role) | ReadError::DeviceWithoutRegisters(role) => {
                write!(f, "the role {} names no device yet", role.name())
            }
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
    /// The interrupt of the second line of the controller the process
    /// serves, which the driver of the PS/2 controller is given for the
    /// mouse (D-109).
    pub aux_interrupt: Option<InterruptHandle>,
    /// The endpoint of the process that started this one, badged with what
    /// that process knows this one by.
    pub parent: Option<EndpointHandle>,
    /// The endpoint of the display server.
    pub display_server: Option<EndpointHandle>,
    /// The endpoint of the input server.
    pub input_server: Option<EndpointHandle>,
    /// The device memory over the framebuffer of the machine.
    pub framebuffer: Option<MemoryHandle>,
    /// The width and the height of that framebuffer, packed as
    /// [`Role::FramebufferGeometry`] carries them.
    pub framebuffer_geometry: Option<u64>,
    /// The stride and the format of that framebuffer, packed as
    /// [`Role::FramebufferLine`] carries them.
    pub framebuffer_line: Option<u64>,
    /// The device memory over the configuration window of the PCI bus.
    pub ecam: Option<MemoryHandle>,
    /// The segment group and the bus range of that window, packed as
    /// [`Role::EcamBuses`] carries them.
    pub ecam_buses: Option<u64>,
    /// One entry per virtio block device the process was given.
    pub blocks: ArrayVec<Device, MAX_BLOCK_DEVICES>,
    /// The virtio network device, where the process was given one.
    pub net: Option<Device>,
    /// The endpoint of the network server.
    pub net_server: Option<EndpointHandle>,
    /// The endpoint of the compositor, badged with what it knows this
    /// process by. A program that opens a window is given one.
    pub desk_server: Option<EndpointHandle>,
    /// Whether the compositor is the only program that draws, as
    /// [`Role::DeskAlone`] carries it. Only the compositor is given one.
    pub desk_alone: Option<u64>,
}

/// How many virtio block devices a process can be given. The reference
/// machine carries two: the disk the firmware read and the disk the system
/// writes (3.1.1).
pub const MAX_BLOCK_DEVICES: usize = 2;

/// One virtio device, as the nine roles that open with a register window
/// describe one.
///
/// The roles of one device arrive together, the register window first: it
/// is what opens a device, and the eight after it belong to the device it
/// opened. A machine with two disks sends the nine of the block device
/// twice, and the network device sends nine of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device {
    /// The device memory over the registers of the device.
    pub registers: MemoryHandle,
    /// Where the common configuration structure lies in that window,
    /// packed as [`Role::BlockCommon`] carries it.
    pub common: Option<u64>,
    /// The same for the notification structure.
    pub notify: Option<u64>,
    /// The same for the interrupt status structure.
    pub isr: Option<u64>,
    /// The same for the device configuration structure.
    pub config: Option<u64>,
    /// The multiplier a queue index is scaled by inside the notification
    /// structure.
    pub notify_multiplier: Option<u64>,
    /// The message interrupt of the device.
    pub interrupt: Option<InterruptHandle>,
    /// The notification that interrupt is bound to.
    pub notification: Option<NotificationHandle>,
    /// The bit of that notification the interrupt sets.
    pub vector_bit: Option<u64>,
}

impl Device {
    /// A device nothing but its register window is known of yet.
    const fn new(registers: MemoryHandle) -> Self {
        Device {
            registers,
            common: None,
            notify: None,
            isr: None,
            config: None,
            notify_multiplier: None,
            interrupt: None,
            notification: None,
            vector_bit: None,
        }
    }

    /// Where the four structures lie in the window, or `None` when the
    /// description is incomplete.
    #[must_use]
    pub const fn structures(&self) -> Option<[Location; 4]> {
        let (Some(common), Some(notify), Some(isr), Some(config)) =
            (self.common, self.notify, self.isr, self.config)
        else {
            return None;
        };
        Some([
            Location::from_word(common),
            Location::from_word(notify),
            Location::from_word(isr),
            Location::from_word(config),
        ])
    }
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
            aux_interrupt: None,
            parent: None,
            display_server: None,
            input_server: None,
            framebuffer: None,
            framebuffer_geometry: None,
            framebuffer_line: None,
            ecam: None,
            ecam_buses: None,
            blocks: ArrayVec::new(),
            net: None,
            net_server: None,
            desk_server: None,
            desk_alone: None,
        }
    }

    /// The device the block roles that arrive now belong to, which is the
    /// last one `BlockRegisters` opened.
    fn block(&mut self) -> Option<&mut Device> {
        let last = self.blocks.len().checked_sub(1)?;
        self.blocks.get_mut(last)
    }

    /// The buses of the configuration window, or `None` when the process
    /// was given no window.
    #[must_use]
    pub const fn bus_range(&self) -> Option<BusRange> {
        match self.ecam_buses {
            Some(word) => Some(BusRange::from_word(word)),
            None => None,
        }
    }

    /// The mode of the framebuffer, or `None` when the process was given no
    /// framebuffer or an incomplete description of one.
    #[must_use]
    pub fn screen(&self) -> Option<Screen> {
        Screen::from_words(self.framebuffer_geometry?, self.framebuffer_line?)
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
            match given.payload {
                Payload::Handle(handle) => startup.take(given.role, handle)?,
                Payload::Value(value) => startup.learn(given.role, value)?,
            }
        }
        Ok(startup)
    }

    /// Puts one number into the field of the device it belongs to.
    fn learn_of_block(&mut self, role: Role, value: u64) -> Result<(), ReadError> {
        let device = self.block().ok_or(ReadError::BlockWithoutRegisters(role))?;
        let field = match role {
            Role::BlockCommon => &mut device.common,
            Role::BlockNotify => &mut device.notify,
            Role::BlockIsr => &mut device.isr,
            Role::BlockConfig => &mut device.config,
            Role::BlockNotifyMultiplier => &mut device.notify_multiplier,
            Role::BlockVectorBit => &mut device.vector_bit,
            _ => return Ok(()),
        };
        if field.is_some() {
            return Err(ReadError::Duplicate(role));
        }
        *field = Some(value);
        Ok(())
    }

    /// Puts one number into the field of the network device.
    fn learn_of_net(&mut self, role: Role, value: u64) -> Result<(), ReadError> {
        let device = self
            .net
            .as_mut()
            .ok_or(ReadError::DeviceWithoutRegisters(role))?;
        let field = match role {
            Role::NetCommon => &mut device.common,
            Role::NetNotify => &mut device.notify,
            Role::NetIsr => &mut device.isr,
            Role::NetConfig => &mut device.config,
            Role::NetNotifyMultiplier => &mut device.notify_multiplier,
            Role::NetVectorBit => &mut device.vector_bit,
            _ => return Ok(()),
        };
        if field.is_some() {
            return Err(ReadError::Duplicate(role));
        }
        *field = Some(value);
        Ok(())
    }

    /// Puts one number into the field its role names.
    fn learn(&mut self, role: Role, value: u64) -> Result<(), ReadError> {
        let field = match role {
            Role::FramebufferGeometry => &mut self.framebuffer_geometry,
            Role::FramebufferLine => &mut self.framebuffer_line,
            Role::DeskAlone => &mut self.desk_alone,
            Role::EcamBuses => &mut self.ecam_buses,
            Role::BlockCommon
            | Role::BlockNotify
            | Role::BlockIsr
            | Role::BlockConfig
            | Role::BlockNotifyMultiplier
            | Role::BlockVectorBit => return self.learn_of_block(role, value),
            Role::NetCommon
            | Role::NetNotify
            | Role::NetIsr
            | Role::NetConfig
            | Role::NetNotifyMultiplier
            | Role::NetVectorBit => return self.learn_of_net(role, value),
            _ => return Ok(()),
        };
        if field.is_some() {
            return Err(ReadError::Duplicate(role));
        }
        *field = Some(value);
        Ok(())
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
            Role::AuxInterrupt => once(&mut self.aux_interrupt, role, handle),
            Role::Parent => once(&mut self.parent, role, handle),
            Role::DisplayServer => once(&mut self.display_server, role, handle),
            Role::InputServer => once(&mut self.input_server, role, handle),
            Role::Framebuffer => once(&mut self.framebuffer, role, handle),
            Role::Ecam => once(&mut self.ecam, role, handle),
            Role::BlockRegisters => self
                .blocks
                .push(Device::new(MemoryHandle::from_handle(handle)))
                .map_err(|_| ReadError::TooManyBlocks),
            Role::BlockInterrupt => {
                let device = self.block().ok_or(ReadError::BlockWithoutRegisters(role))?;
                once(&mut device.interrupt, role, handle)
            }
            Role::BlockNotification => {
                let device = self.block().ok_or(ReadError::BlockWithoutRegisters(role))?;
                once(&mut device.notification, role, handle)
            }
            Role::NetRegisters => {
                if self.net.is_some() {
                    return Err(ReadError::Duplicate(role));
                }
                self.net = Some(Device::new(MemoryHandle::from_handle(handle)));
                Ok(())
            }
            Role::NetInterrupt => {
                let device = self
                    .net
                    .as_mut()
                    .ok_or(ReadError::DeviceWithoutRegisters(role))?;
                once(&mut device.interrupt, role, handle)
            }
            Role::NetNotification => {
                let device = self
                    .net
                    .as_mut()
                    .ok_or(ReadError::DeviceWithoutRegisters(role))?;
                once(&mut device.notification, role, handle)
            }
            Role::NetServer => once(&mut self.net_server, role, handle),
            Role::DeskServer => once(&mut self.desk_server, role, handle),
            Role::DeskAlone
            | Role::FramebufferGeometry
            | Role::FramebufferLine
            | Role::EcamBuses
            | Role::BlockCommon
            | Role::BlockNotify
            | Role::BlockIsr
            | Role::BlockConfig
            | Role::BlockNotifyMultiplier
            | Role::BlockVectorBit
            | Role::NetCommon
            | Role::NetNotify
            | Role::NetIsr
            | Role::NetConfig
            | Role::NetNotifyMultiplier
            | Role::NetVectorBit => Ok(()),
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
