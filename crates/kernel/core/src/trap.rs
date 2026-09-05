// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the kernel does with a processor exception.
//!
//! Invariants: an exception is reported in full, never silently; an
//! exception the kernel itself raised ends the machine, because nothing
//! below the kernel could carry on without it; an exception a user thread
//! raised ends that thread and nothing else.

use kernel_hal_api::console::DebugConsole;
use kernel_hal_api::exit::{ExitStatus, TestExit};

use crate::println;
use crate::state::KernelState;

/// Number of processor-defined exception vectors.
pub const EXCEPTION_VECTORS: u8 = 32;

/// What the processor reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Exception {
    /// The vector the processor entered.
    pub vector: u8,
    /// The error code, or zero for the vectors that push none.
    pub error_code: u64,
    /// The instruction pointer at the fault.
    pub ip: u64,
    /// The stack pointer at the fault.
    pub sp: u64,
    /// The address a page fault names; zero for every other vector.
    pub cr2: u64,
    /// Whether the processor was in user mode when the exception arrived.
    pub user: bool,
}

/// What an exception asks the kernel to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Response {
    /// The thread that raised it stops; the rest of the system runs on.
    StopThread,
    /// Nothing runs on: the kernel itself fell over, and there is no
    /// smaller thing to stop.
    StopMachine,
}

impl Exception {
    /// The name of the vector, or `exception` for a vector the processor
    /// does not define.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self.vector {
            0 => "divide error",
            1 => "debug",
            2 => "non-maskable interrupt",
            3 => "breakpoint",
            4 => "overflow",
            5 => "bound range exceeded",
            6 => "invalid opcode",
            7 => "device not available",
            8 => "double fault",
            10 => "invalid task state segment",
            11 => "segment not present",
            12 => "stack segment fault",
            13 => "general protection",
            14 => "page fault",
            16 => "floating point error",
            17 => "alignment check",
            18 => "machine check",
            19 => "simd floating point error",
            20 => "virtualization",
            21 => "control protection",
            _ => "exception",
        }
    }

    /// What the kernel does with the exception. Everything a user thread
    /// raises stops that thread; everything else stops the machine.
    #[must_use]
    pub const fn response(self) -> Response {
        if self.user {
            Response::StopThread
        } else {
            Response::StopMachine
        }
    }

    /// `true` for the vectors the processor pushes an error code for.
    #[must_use]
    pub const fn has_error_code(self) -> bool {
        matches!(self.vector, 8 | 10 | 11 | 12 | 13 | 14 | 17 | 21 | 29 | 30)
    }
}

/// Reports `exception`, counts it in `state`, and ends the machine with a
/// failure.
pub fn on_exception(
    exception: Exception,
    state: &mut KernelState,
    console: &mut impl DebugConsole,
    exit: &mut impl TestExit,
) {
    state.record_trap();
    describe(exception, console);
    exit.exit(ExitStatus::Failure);
}

/// Reports `exception` and counts it in `state`, without ending the
/// machine: a user thread raised it, and stopping that thread is the whole
/// of the answer.
pub fn on_user_fault(
    exception: Exception,
    state: &mut KernelState,
    console: &mut impl DebugConsole,
) {
    state.record_trap();
    println!(console, "[trap] a user thread faulted");
    describe(exception, console);
}

/// Writes what the processor reported, in the three lines the vector asks
/// for.
fn describe(exception: Exception, console: &mut impl DebugConsole) {
    println!(
        console,
        "[trap] {} (vector {}) at ip {:#x} sp {:#x}",
        exception.name(),
        exception.vector,
        exception.ip,
        exception.sp
    );
    if exception.has_error_code() {
        println!(console, "[trap] error code {:#x}", exception.error_code);
    }
    if exception.vector == 14 {
        println!(console, "[trap] faulting address {:#x}", exception.cr2);
    }
}
