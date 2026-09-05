// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the kernel does with a processor exception.
//!
//! Invariant: an exception the kernel cannot handle is reported in full
//! and ends the machine with a failure, never silently.

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
    exit.exit(ExitStatus::Failure);
}
