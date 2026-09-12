// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The interrupt controller.
//!
//! Invariants: a [`Vector`] is at least [`Vector::FIRST_DEVICE`], so that
//! device interrupts never alias CPU exceptions.

use core::fmt;

/// A hardware interrupt line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InterruptLine(u8);

impl InterruptLine {
    /// The line with the given number.
    #[must_use]
    pub const fn new(line: u8) -> Self {
        InterruptLine(line)
    }

    /// The line number.
    #[must_use]
    pub const fn number(self) -> u8 {
        self.0
    }
}

/// An interrupt vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Vector(u8);

impl Vector {
    /// The first vector available for devices; lower vectors are CPU
    /// exceptions.
    pub const FIRST_DEVICE: u8 = 32;

    /// A device vector.
    ///
    /// # Errors
    ///
    /// `ReservedVector` below [`Vector::FIRST_DEVICE`].
    pub const fn new(vector: u8) -> Result<Self, InterruptError> {
        if vector >= Self::FIRST_DEVICE {
            Ok(Vector(vector))
        } else {
            Err(InterruptError::ReservedVector(vector))
        }
    }

    /// The vector number.
    #[must_use]
    pub const fn number(self) -> u8 {
        self.0
    }
}

/// A message interrupt: the vector it was allocated out of, and the write
/// that raises it. A driver puts the address and the data into its device's
/// MSI-X table, which is in the device's own window and therefore in the
/// driver's address space and not the kernel's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageInterrupt {
    /// The vector the write delivers on.
    pub vector: Vector,
    /// The address the device writes to.
    pub address: u64,
    /// The value it writes there.
    pub data: u32,
}

/// Errors of the interrupt controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterruptError {
    /// The vector is reserved for CPU exceptions.
    ReservedVector(u8),
    /// The controller has no such line.
    UnknownLine(u8),
    /// The line is already routed.
    AlreadyRouted(u8),
    /// The vector space has nothing left.
    NoVector,
}

impl fmt::Display for InterruptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InterruptError::ReservedVector(v) => write!(f, "vector {v} is reserved for exceptions"),
            InterruptError::UnknownLine(l) => write!(f, "interrupt line {l} does not exist"),
            InterruptError::AlreadyRouted(l) => write!(f, "interrupt line {l} is already routed"),
            InterruptError::NoVector => f.write_str("the vector space has nothing left"),
        }
    }
}

/// Routes, masks, and acknowledges interrupt lines.
pub trait InterruptController {
    /// The vector the plan of this machine routes `line` to, or `None` for a
    /// line it reserves no vector for.
    ///
    /// The plan is the architecture's, which is why this is a question of
    /// the controller: the system call layer derives the vector of an
    /// interrupt object from it and refuses a line that has none.
    fn vector_of(&self, line: InterruptLine) -> Option<Vector>;

    /// Routes `line` to `vector`, initially masked.
    ///
    /// # Errors
    ///
    /// Fails for unknown or already routed lines.
    fn route(&mut self, line: InterruptLine, vector: Vector) -> Result<(), InterruptError>;

    /// Stops delivery of `line`.
    fn mask(&mut self, line: InterruptLine);

    /// Resumes delivery of `line`.
    fn unmask(&mut self, line: InterruptLine);

    /// Signals that the handler of `vector` has finished.
    fn end_of_interrupt(&mut self, vector: Vector);

    /// Takes one vector out of the space the lines are allocated from and
    /// answers with the write that raises it.
    ///
    /// Nothing is routed: a message interrupt reaches the processor because
    /// the device writes the address, and the driver is what programs the
    /// device.
    ///
    /// # Errors
    ///
    /// [`InterruptError::NoVector`] when the space has nothing left.
    fn allocate_msi(&mut self) -> Result<MessageInterrupt, InterruptError>;

    /// Gives `vector` back to the space it came from. A vector that was
    /// never handed out is left alone.
    fn release_msi(&mut self, vector: Vector);
}
