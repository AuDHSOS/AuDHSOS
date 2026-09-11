// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two hardware seams a kernel with userland drivers needs at once.
//!
//! Invariant: an implementation reaches the interrupt controller and the I/O
//! ports of one machine, so a line it masks and a port it writes belong to
//! the same processor.
//!
//! It is one bound and not three because the system call layer has one
//! place to hold it: `interrupt_create`, `interrupt_ack`, `ioport_read`,
//! `ioport_write`, and `random_bytes` are five calls of one table, and the
//! kernel environment carries what they need in one field.

use crate::interrupt::InterruptController;
use crate::port::PortAccess;
use crate::random::Random;

/// The machine as the system calls reach it: the interrupt controller, the
/// I/O ports, and the entropy source.
pub trait Devices: InterruptController + PortAccess + Random {}

impl<T: InterruptController + PortAccess + Random> Devices for T {}
