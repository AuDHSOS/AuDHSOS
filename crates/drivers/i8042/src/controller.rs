// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The controller itself: its four registers, the sequence that brings it
//! up, and the read side the driver uses afterwards.
//!
//! Two ports carry everything. `0x60` is the data register, read and
//! written; `0x64` is the status register when read and the command
//! register when written. A byte in the data register belongs to the
//! keyboard or to the mouse, and the `AUX` bit of the status register says
//! which — which is why the byte and the bit are read together and never
//! one without the other.
//!
//! Invariants: no wait loops without a bound, and every one of them yields
//! [`Error::Timeout`] instead of spinning; the interrupts of both lines are
//! turned on last, when both devices have answered, so no byte arrives
//! before there is a decoder for it.

use core::fmt;

/// The data register: the byte a device sent, and the byte sent to one.
pub const DATA_PORT: u16 = 0x60;

/// The status register when read, the command register when written.
pub const COMMAND_PORT: u16 = 0x64;

/// Status bit: a byte is waiting in the data register.
pub const OUTPUT_FULL: u8 = 0x01;

/// Status bit: the controller has not taken the last byte written to it.
pub const INPUT_FULL: u8 = 0x02;

/// Status bit: the byte waiting in the data register came from the mouse.
pub const AUX: u8 = 0x20;

/// Command: hand the configuration byte over.
pub const READ_CONFIG: u8 = 0x20;

/// Command: take a new configuration byte.
pub const WRITE_CONFIG: u8 = 0x60;

/// Command: turn the mouse port off.
pub const DISABLE_AUX: u8 = 0xA7;

/// Command: turn the mouse port on.
pub const ENABLE_AUX: u8 = 0xA8;

/// Command: test the mouse port.
pub const TEST_AUX: u8 = 0xA9;

/// Command: test the controller itself.
pub const SELF_TEST: u8 = 0xAA;

/// Command: test the keyboard port.
pub const TEST_KBD: u8 = 0xAB;

/// Command: turn the keyboard port off.
pub const DISABLE_KBD: u8 = 0xAD;

/// Command: turn the keyboard port on.
pub const ENABLE_KBD: u8 = 0xAE;

/// Command: the next byte written to the data register goes to the mouse.
pub const WRITE_AUX: u8 = 0xD4;

/// What a controller that works answers to [`SELF_TEST`].
pub const SELF_TEST_PASSED: u8 = 0x55;

/// What a port that works answers to [`TEST_KBD`] and [`TEST_AUX`].
pub const PORT_TEST_PASSED: u8 = 0x00;

/// Configuration bit: a byte from the keyboard raises its interrupt.
pub const CONFIG_KBD_INTERRUPT: u8 = 0x01;

/// Configuration bit: a byte from the mouse raises its interrupt.
pub const CONFIG_AUX_INTERRUPT: u8 = 0x02;

/// Configuration bit: the keyboard port is off.
pub const CONFIG_KBD_DISABLED: u8 = 0x10;

/// Configuration bit: the mouse port is off.
pub const CONFIG_AUX_DISABLED: u8 = 0x20;

/// Configuration bit: the controller translates set 2 into set 1.
///
/// It is cleared, because this driver decodes set 2 and a controller that
/// translates would hand it something else.
pub const CONFIG_TRANSLATE: u8 = 0x40;

/// How often a wait asks before it gives up.
pub const MAX_POLLS: u32 = 100_000;

/// Access to the two ports of one controller.
pub trait Ports {
    /// Reads the data register.
    fn read_data(&mut self) -> u8;

    /// Reads the status register.
    fn read_status(&mut self) -> u8;

    /// Writes the data register.
    fn write_data(&mut self, value: u8);

    /// Writes the command register.
    fn write_command(&mut self, value: u8);
}

/// Why an operation on the controller failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// A buffer did not become ready within [`MAX_POLLS`] polls.
    Timeout,
    /// The self-test answered something other than [`SELF_TEST_PASSED`].
    SelfTest(u8),
    /// A device answered a command with something that is no
    /// acknowledgement.
    NotAcknowledged(u8),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Timeout => f.write_str("the controller did not become ready"),
            Error::SelfTest(answer) => {
                write!(f, "the self-test answered {answer:#04x} and not 0x55")
            }
            Error::NotAcknowledged(answer) => {
                write!(
                    f,
                    "a device answered {answer:#04x} and not an acknowledgement"
                )
            }
        }
    }
}

/// Which devices the controller found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Devices {
    /// Whether the keyboard port passed its test and the keyboard
    /// answered.
    pub keyboard: bool,
    /// Whether the mouse port passed its test and the mouse answered.
    pub mouse: bool,
    /// What the mouse said its identifier is. Three means the wheel, and
    /// with it the four-byte packet.
    pub mouse_id: u8,
}

/// One i8042 controller.
#[derive(Clone, Copy, Debug)]
pub struct Controller<P: Ports> {
    ports: P,
}

impl<P: Ports> Controller<P> {
    /// Takes the ports over without touching them.
    pub const fn new(ports: P) -> Self {
        Controller { ports }
    }

    /// Gives the ports back.
    pub fn into_ports(self) -> P {
        self.ports
    }

    /// The ports, for a caller that has to reach them directly.
    pub const fn ports(&mut self) -> &mut P {
        &mut self.ports
    }

    /// Brings the controller and both devices up.
    ///
    /// The order is the one a controller of this age wants: both ports off
    /// so that nothing arrives while the configuration is read, the buffer
    /// emptied of whatever the firmware left in it, translation and both
    /// interrupts off, the self-test, the port tests, the ports on, the
    /// devices reset, and the interrupts on last of all.
    ///
    /// A port that fails its test, or a device that does not answer, is
    /// reported as absent in [`Devices`]; the other device still works.
    ///
    /// # Errors
    ///
    /// [`Error::Timeout`] when a buffer never becomes ready, and
    /// [`Error::SelfTest`] when the controller itself is broken. Those two
    /// are the whole controller, so neither leaves anything to run.
    pub fn init(&mut self) -> Result<Devices, Error> {
        self.command(DISABLE_KBD)?;
        self.command(DISABLE_AUX)?;
        self.flush();

        let config = self.read_config()?;
        let quiet = config & !(CONFIG_KBD_INTERRUPT | CONFIG_AUX_INTERRUPT | CONFIG_TRANSLATE)
            | CONFIG_KBD_DISABLED
            | CONFIG_AUX_DISABLED;
        self.write_config(quiet)?;

        let answer = self.ask(SELF_TEST)?;
        if answer != SELF_TEST_PASSED {
            return Err(Error::SelfTest(answer));
        }
        // The self-test resets the controller on some machines, which puts
        // the configuration byte back the way it was. Writing it again
        // costs two port accesses and makes the state after the test the
        // state before it.
        self.write_config(quiet)?;

        let keyboard_port = self.ask(TEST_KBD)? == PORT_TEST_PASSED;
        let mouse_port = self.ask(TEST_AUX)? == PORT_TEST_PASSED;
        let mut devices = Devices {
            keyboard: keyboard_port,
            mouse: mouse_port,
            mouse_id: 0,
        };
        if devices.keyboard {
            self.command(ENABLE_KBD)?;
            devices.keyboard = crate::device::start_keyboard(self).is_ok();
        }
        if devices.mouse {
            self.command(ENABLE_AUX)?;
            match crate::device::start_mouse(self) {
                Ok(id) => devices.mouse_id = id,
                Err(_) => devices.mouse = false,
            }
        }

        let mut running = quiet;
        if devices.keyboard {
            running = running & !CONFIG_KBD_DISABLED | CONFIG_KBD_INTERRUPT;
        }
        if devices.mouse {
            running = running & !CONFIG_AUX_DISABLED | CONFIG_AUX_INTERRUPT;
        }
        self.write_config(running)?;
        Ok(devices)
    }

    /// The byte waiting in the data register and whether it came from the
    /// mouse, or `None` when the buffer is empty.
    ///
    /// The status is read once and the byte is taken out of that same
    /// reading, so the [`AUX`] bit belongs to the byte and not to whatever
    /// arrived while this was running.
    pub fn take(&mut self) -> Option<(u8, bool)> {
        let status = self.ports.read_status();
        if status & OUTPUT_FULL == 0 {
            return None;
        }
        Some((self.ports.read_data(), status & AUX != 0))
    }

    /// `true` when a byte stands in the output buffer.
    ///
    /// This is [`take`](Self::take) without taking: a driver that has just
    /// let its lines go asks it, because a byte that arrived while the mask
    /// was still on raised a line that will not raise it again.
    pub fn pending(&mut self) -> bool {
        self.ports.read_status() & OUTPUT_FULL != 0
    }

    /// Empties the output buffer of whatever stands in it.
    ///
    /// Bounded like every other loop here: a controller whose output buffer
    /// never empties is a controller this driver gives up on rather than
    /// one it spins for.
    pub fn flush(&mut self) {
        let mut polls = 0u32;
        while polls < MAX_POLLS && self.take().is_some() {
            polls = polls.saturating_add(1);
        }
    }

    /// Sends one command byte.
    ///
    /// # Errors
    ///
    /// [`Error::Timeout`] when the controller does not take it.
    pub fn command(&mut self, command: u8) -> Result<(), Error> {
        self.wait_input()?;
        self.ports.write_command(command);
        Ok(())
    }

    /// Sends one byte to the keyboard.
    ///
    /// # Errors
    ///
    /// [`Error::Timeout`] when the controller does not take it.
    pub fn write(&mut self, value: u8) -> Result<(), Error> {
        self.wait_input()?;
        self.ports.write_data(value);
        Ok(())
    }

    /// Sends one byte to the mouse, which is the same register behind the
    /// command that redirects it.
    ///
    /// # Errors
    ///
    /// As [`write`](Self::write).
    pub fn write_aux(&mut self, value: u8) -> Result<(), Error> {
        self.command(WRITE_AUX)?;
        self.write(value)
    }

    /// Waits for a byte and reads it.
    ///
    /// # Errors
    ///
    /// [`Error::Timeout`] when no byte arrives.
    pub fn read(&mut self) -> Result<u8, Error> {
        self.wait_output()?;
        Ok(self.ports.read_data())
    }

    /// Sends a command and reads the byte it answers with.
    fn ask(&mut self, command: u8) -> Result<u8, Error> {
        self.command(command)?;
        self.read()
    }

    /// Reads the configuration byte.
    fn read_config(&mut self) -> Result<u8, Error> {
        self.ask(READ_CONFIG)
    }

    /// Writes the configuration byte.
    fn write_config(&mut self, config: u8) -> Result<(), Error> {
        self.command(WRITE_CONFIG)?;
        self.write(config)
    }

    /// Waits until the controller has taken the last byte written to it.
    fn wait_input(&mut self) -> Result<(), Error> {
        let mut polls = 0u32;
        while polls < MAX_POLLS {
            if self.ports.read_status() & INPUT_FULL == 0 {
                return Ok(());
            }
            polls = polls.saturating_add(1);
        }
        Err(Error::Timeout)
    }

    /// Waits until a byte stands in the data register.
    fn wait_output(&mut self) -> Result<(), Error> {
        let mut polls = 0u32;
        while polls < MAX_POLLS {
            if self.ports.read_status() & OUTPUT_FULL != 0 {
                return Ok(());
            }
            polls = polls.saturating_add(1);
        }
        Err(Error::Timeout)
    }
}
