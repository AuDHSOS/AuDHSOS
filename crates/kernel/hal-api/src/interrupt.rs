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

/// Errors of the interrupt controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterruptError {
    /// The vector is reserved for CPU exceptions.
    ReservedVector(u8),
    /// The controller has no such line.
    UnknownLine(u8),
    /// The line is already routed.
    AlreadyRouted(u8),
}

impl fmt::Display for InterruptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InterruptError::ReservedVector(v) => write!(f, "vector {v} is reserved for exceptions"),
            InterruptError::UnknownLine(l) => write!(f, "interrupt line {l} does not exist"),
            InterruptError::AlreadyRouted(l) => write!(f, "interrupt line {l} is already routed"),
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
}
