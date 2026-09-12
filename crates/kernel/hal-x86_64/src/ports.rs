// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The I/O ports of the machine, as the system call layer reaches them.
//!
//! Invariant: a value of this type is the kernel's permission to touch any
//! port at all, and the system call layer hands it only what an
//! `IoPortRange` capability allows — the range check is there and not here,
//! because a range is an object of the kernel and a port is a number.

use kernel_hal_api::port::PortAccess;

use crate::instructions;

/// The ports of this processor.
///
/// The type holds nothing: a port access needs no state, and one value of
/// this is what says that the caller has the right to make one.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ports;

impl Ports {
    /// The ports of this processor.
    #[must_use]
    pub const fn new() -> Self {
        Ports
    }
}

impl PortAccess for Ports {
    fn read_u8(&mut self, port: u16) -> u8 {
        // SAFETY: the system call layer has checked the port against an
        // `IoPortRange` capability of the caller, which the root task
        // created from `SystemControl` and handed to the driver that owns
        // the device.
        unsafe { instructions::read_port_u8(port) }
    }

    fn write_u8(&mut self, port: u16, value: u8) {
        // SAFETY: as `read_u8`.
        unsafe {
            instructions::write_port_u8(port, value);
        }
    }

    fn read_u16(&mut self, port: u16) -> u16 {
        // SAFETY: as `read_u8`.
        unsafe { instructions::read_port_u16(port) }
    }

    fn write_u16(&mut self, port: u16, value: u16) {
        // SAFETY: as `read_u8`.
        unsafe {
            instructions::write_port_u16(port, value);
        }
    }

    fn read_u32(&mut self, port: u16) -> u32 {
        // SAFETY: as `read_u8`.
        unsafe { instructions::read_port_u32(port) }
    }

    fn write_u32(&mut self, port: u16, value: u32) {
        // SAFETY: as `read_u8`.
        unsafe {
            instructions::write_port_u32(port, value);
        }
    }
}

/// The interrupt controller, the ports, and the entropy source of this
/// machine in one value, which is what `kernel_core::KernelEnvironment`
/// holds.
///
/// The controller is borrowed, because it lives in the one cell every
/// interrupt handler reaches; the ports and the entropy source are values,
/// because they hold nothing.
#[derive(Debug)]
pub struct DeviceAccess<'a> {
    apics: &'a mut crate::apic::Apics,
    ports: Ports,
    random: Option<crate::random::HardwareRandom>,
}

impl<'a> DeviceAccess<'a> {
    /// The devices over `apics` and the ports of this processor.
    #[must_use]
    pub const fn new(apics: &'a mut crate::apic::Apics) -> Self {
        DeviceAccess {
            apics,
            ports: Ports::new(),
            random: None,
        }
    }

    /// The same devices with the entropy source of this processor, on a
    /// processor that has `RDSEED`. Without it `random_bytes` answers
    /// `Unavailable`, which is what the reference machine looked like
    /// before its CPU model gained the two feature flags.
    #[must_use]
    pub fn with_entropy(self) -> Self {
        DeviceAccess {
            random: crate::random::HardwareRandom::of_this_processor(),
            ..self
        }
    }
}

impl kernel_hal_api::random::Random for DeviceAccess<'_> {
    fn seed(
        &mut self,
    ) -> Result<[u64; kernel_hal_api::random::SEED_WORDS], kernel_hal_api::random::RandomError>
    {
        self.random
            .as_mut()
            .ok_or(kernel_hal_api::random::RandomError::Unavailable)?
            .seed()
    }
}

impl kernel_hal_api::interrupt::InterruptController for DeviceAccess<'_> {
    fn vector_of(
        &self,
        line: kernel_hal_api::interrupt::InterruptLine,
    ) -> Option<kernel_hal_api::interrupt::Vector> {
        self.apics.vector_of(line)
    }

    fn route(
        &mut self,
        line: kernel_hal_api::interrupt::InterruptLine,
        vector: kernel_hal_api::interrupt::Vector,
    ) -> Result<(), kernel_hal_api::interrupt::InterruptError> {
        self.apics.route(line, vector)
    }

    fn mask(&mut self, line: kernel_hal_api::interrupt::InterruptLine) {
        self.apics.mask(line);
    }

    fn unmask(&mut self, line: kernel_hal_api::interrupt::InterruptLine) {
        self.apics.unmask(line);
    }

    fn end_of_interrupt(&mut self, vector: kernel_hal_api::interrupt::Vector) {
        self.apics.end_of_interrupt(vector);
    }

    fn allocate_msi(
        &mut self,
    ) -> Result<
        kernel_hal_api::interrupt::MessageInterrupt,
        kernel_hal_api::interrupt::InterruptError,
    > {
        self.apics.allocate_msi()
    }

    fn release_msi(&mut self, vector: kernel_hal_api::interrupt::Vector) {
        self.apics.release_msi(vector);
    }
}

/// The ports of the first serial controller, which the kernel writes its
/// own diagnostics on until somebody else asks for them.
const COM1: core::ops::Range<u16> = 0x3F8..0x400;

/// Notes that userland has reached `port`, and gives the serial controller
/// up when that is what the port belongs to.
///
/// The handover is here and not at `ioport_create`, because this is where
/// it is true: a capability that has been created and not used yet has
/// taken nothing over.
fn note(port: u16) {
    if COM1.contains(&port) {
        crate::console::give_up();
    }
}

impl PortAccess for DeviceAccess<'_> {
    fn read_u8(&mut self, port: u16) -> u8 {
        note(port);
        self.ports.read_u8(port)
    }

    fn write_u8(&mut self, port: u16, value: u8) {
        note(port);
        self.ports.write_u8(port, value);
    }

    fn read_u16(&mut self, port: u16) -> u16 {
        note(port);
        self.ports.read_u16(port)
    }

    fn write_u16(&mut self, port: u16, value: u16) {
        note(port);
        self.ports.write_u16(port, value);
    }

    fn read_u32(&mut self, port: u16) -> u32 {
        note(port);
        self.ports.read_u32(port)
    }

    fn write_u32(&mut self, port: u16, value: u32) {
        note(port);
        self.ports.write_u32(port, value);
    }
}
