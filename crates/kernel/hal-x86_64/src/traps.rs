// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The trap handlers and the table that names them.
//!
//! Invariants: every processor vector has a handler; a handler reads only
//! the frame the processor pushed and hands it to the function the kernel
//! registered; the double fault handler runs on its own stack.

use audhsos_sync::{Global, UncontendedToken};

use kernel_x86_tables::gdt::KERNEL_CODE_SELECTOR;
use kernel_x86_tables::idt::{
    DOUBLE_FAULT_IST, GATE_INTERRUPT_DPL0, GATE_INTERRUPT_DPL3, IDT_ENTRIES, MISSING,
    SYSCALL_VECTOR, gate,
};

use crate::instructions::read_fault_address;

/// What the processor pushes before a handler runs.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct InterruptFrame {
    /// The instruction that was about to run.
    pub ip: u64,
    /// The code segment it ran in.
    pub code_segment: u64,
    /// The flags register at the trap.
    pub flags: u64,
    /// The stack pointer at the trap.
    pub sp: u64,
    /// The stack segment at the trap.
    pub stack_segment: u64,
}

/// What a handler reports to the kernel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrapReport {
    /// The vector the processor entered.
    pub vector: u8,
    /// The error code, or zero.
    pub error_code: u64,
    /// The instruction pointer at the trap.
    pub ip: u64,
    /// The stack pointer at the trap.
    pub sp: u64,
    /// The address a page fault named.
    pub fault_address: u64,
}

/// What the kernel registers to receive a trap.
pub type TrapHandler = fn(TrapReport);

/// What the kernel registers to receive a device interrupt. A device
/// interrupt carries nothing but its vector: what happened is the device's
/// business, not the processor's.
pub type InterruptHandler = fn(u8);

/// The registered handler; empty before the kernel registers one.
static HANDLER: Global<TrapHandler> = Global::new();

/// The registered device interrupt handler; empty before the kernel
/// registers one.
static DEVICE_HANDLER: Global<InterruptHandler> = Global::new();

/// Registers the function every handler calls. A second call is ignored,
/// because the kernel registers once during boot.
pub fn set_handler(handler: TrapHandler) {
    let _ = HANDLER.init(handler);
}

/// Registers the function every device vector calls. A second call is
/// ignored, because the kernel registers once during boot.
pub fn set_interrupt_handler(handler: InterruptHandler) {
    let _ = DEVICE_HANDLER.init(handler);
}

/// What a system call runs: the kernel reads the IPC buffer of the thread
/// that made it, answers, and switches if the answer asks for it.
pub type SyscallHandler = fn();

/// The function vector `0x80` calls. The kernel registers one during boot.
static SYSCALL_HANDLER: Global<SyscallHandler> = Global::new();

/// Registers the function vector `0x80` calls. A second call is ignored,
/// because the kernel registers once during boot.
pub fn set_syscall_handler(handler: SyscallHandler) {
    let _ = SYSCALL_HANDLER.init(handler);
}

/// Hands a system call to the kernel. The borrow ends before the handler
/// runs, so that the kernel may take an interrupt of its own without
/// finding the cell busy.
///
/// A system call made before the kernel has registered a handler returns
/// without an answer, which is what a thread that runs before the kernel
/// is ready would see; no such thread exists.
fn call_kernel() {
    let handler = SYSCALL_HANDLER
        .borrow(&UncontendedToken)
        .ok()
        .map(|handler| *handler);
    if let Some(handler) = handler {
        handler();
    }
}

/// Vector `0x80`, the one gate of this table the processor may enter from
/// ring three.
extern "x86-interrupt" fn syscall_entry(_frame: InterruptFrame) {
    call_kernel();
}

/// Hands `report` to the registered handler. A trap the kernel cannot
/// report, because none is registered or one is already being reported,
/// is not survivable: the machine halts.
fn dispatch(report: TrapReport) {
    let Ok(handler) = HANDLER.borrow(&UncontendedToken) else {
        crate::instructions::halt_forever();
    };
    (*handler)(report);
}

/// Hands `vector` to the registered device interrupt handler. The borrow
/// ends before the handler runs, so that the handler may take an interrupt
/// of its own without finding the cell busy.
///
/// An interrupt that arrives before the kernel has registered a handler is
/// dropped without an end-of-interrupt: the only interrupt that can arrive
/// then is one the kernel has not turned on yet, and the machine goes on.
fn deliver(vector: u8) {
    let handler = DEVICE_HANDLER
        .borrow(&UncontendedToken)
        .ok()
        .map(|handler| *handler);
    if let Some(handler) = handler {
        handler(vector);
    }
}

/// Declares one handler per vector and the table that names them.
macro_rules! handlers {
    ($($name:ident = $vector:literal),+ $(,)?) => {
        $(
            extern "x86-interrupt" fn $name(frame: InterruptFrame) {
                dispatch(TrapReport {
                    vector: $vector,
                    error_code: 0,
                    ip: frame.ip,
                    sp: frame.sp,
                    fault_address: 0,
                });
            }
        )+
    };
}

/// The same for the vectors the processor pushes an error code for.
macro_rules! handlers_with_code {
    ($($name:ident = $vector:literal),+ $(,)?) => {
        $(
            extern "x86-interrupt" fn $name(frame: InterruptFrame, error_code: u64) {
                dispatch(TrapReport {
                    vector: $vector,
                    error_code,
                    ip: frame.ip,
                    sp: frame.sp,
                    fault_address: if $vector == 14u8 { read_fault_address() } else { 0 },
                });
            }
        )+
    };
}

handlers! {
    divide_error = 0,
    debug = 1,
    non_maskable = 2,
    breakpoint = 3,
    overflow = 4,
    bound_range = 5,
    invalid_opcode = 6,
    device_not_available = 7,
    coprocessor_segment = 9,
    floating_point = 16,
    machine_check = 18,
    simd = 19,
    virtualization = 20,
}

/// Declares one handler per device vector. A device interrupt carries no
/// error code and nothing of the frame the processor pushed, so every
/// handler is the same but for the vector it names.
macro_rules! device_handlers {
    ($($name:ident = $vector:literal),+ $(,)?) => {
        $(
            extern "x86-interrupt" fn $name(_frame: InterruptFrame) {
                deliver($vector);
            }
        )+

        /// Every device vector that has a handler, with its handler.
        const DEVICE_VECTORS: &[(u8, extern "x86-interrupt" fn(InterruptFrame))] =
            &[$(($vector, $name)),+];
    };
}

// The vectors of the plan in `kernel_x86_tables::vectors` that a device can
// raise: the range the two legacy controllers were moved to, the timer, one
// per global system interrupt, and the spurious vector of the local APIC.
// The system call vector is not among them: it has its own handler and the
// one gate of this table the processor may enter from ring three.
device_handlers! {
    device_20 = 0x20, device_21 = 0x21, device_22 = 0x22, device_23 = 0x23, device_24 = 0x24, device_25 = 0x25,
    device_26 = 0x26, device_27 = 0x27, device_28 = 0x28, device_29 = 0x29, device_2a = 0x2A, device_2b = 0x2B,
    device_2c = 0x2C, device_2d = 0x2D, device_2e = 0x2E, device_2f = 0x2F, device_30 = 0x30, device_40 = 0x40,
    device_41 = 0x41, device_42 = 0x42, device_43 = 0x43, device_44 = 0x44, device_45 = 0x45, device_46 = 0x46,
    device_47 = 0x47, device_48 = 0x48, device_49 = 0x49, device_4a = 0x4A, device_4b = 0x4B, device_4c = 0x4C,
    device_4d = 0x4D, device_4e = 0x4E, device_4f = 0x4F, device_50 = 0x50, device_51 = 0x51, device_52 = 0x52,
    device_53 = 0x53, device_54 = 0x54, device_55 = 0x55, device_56 = 0x56, device_57 = 0x57, device_ff = 0xFF,
}

handlers_with_code! {
    double_fault = 8,
    invalid_tss = 10,
    segment_not_present = 11,
    stack_segment = 12,
    general_protection = 13,
    page_fault = 14,
    alignment_check = 17,
    control_protection = 21,
    hypervisor_injection = 28,
    vmm_communication = 29,
    security = 30,
}

/// Fills `table` with a gate for every vector the processor defines and
/// for every device vector of the plan.
#[expect(
    clippy::as_conversions,
    clippy::fn_to_numeric_cast_any,
    reason = "a gate carries the address of its handler, and `as` is the only way to read a function pointer as a number"
)]
pub fn fill(table: &mut [[u64; 2]; IDT_ENTRIES]) {
    let selector = KERNEL_CODE_SELECTOR.as_u16();
    let simple: [(u8, extern "x86-interrupt" fn(InterruptFrame)); 13] = [
        (0, divide_error),
        (1, debug),
        (2, non_maskable),
        (3, breakpoint),
        (4, overflow),
        (5, bound_range),
        (6, invalid_opcode),
        (7, device_not_available),
        (9, coprocessor_segment),
        (16, floating_point),
        (18, machine_check),
        (19, simd),
        (20, virtualization),
    ];
    let with_code: [(u8, extern "x86-interrupt" fn(InterruptFrame, u64)); 11] = [
        (8, double_fault),
        (10, invalid_tss),
        (11, segment_not_present),
        (12, stack_segment),
        (13, general_protection),
        (14, page_fault),
        (17, alignment_check),
        (21, control_protection),
        (28, hypervisor_injection),
        (29, vmm_communication),
        (30, security),
    ];
    for entry in table.iter_mut() {
        *entry = MISSING;
    }
    for (vector, handler) in simple {
        let address = handler_address(handler as usize);
        put(
            table,
            vector,
            gate(address, selector, 0, GATE_INTERRUPT_DPL0),
        );
    }
    for (vector, handler) in with_code {
        let address = handler_address(handler as usize);
        let stack = if vector == 8 { DOUBLE_FAULT_IST } else { 0 };
        put(
            table,
            vector,
            gate(address, selector, stack, GATE_INTERRUPT_DPL0),
        );
    }
    for (vector, handler) in DEVICE_VECTORS {
        let address = handler_address(*handler as usize);
        put(
            table,
            *vector,
            gate(address, selector, 0, GATE_INTERRUPT_DPL0),
        );
    }
    // The one gate a user thread may enter through. Everything else in
    // this table is ring zero, so a user thread that raises any other
    // vector takes a general protection fault instead.
    let address = handler_address((syscall_entry as *const ()) as usize);
    put(
        table,
        SYSCALL_VECTOR,
        gate(address, selector, 0, GATE_INTERRUPT_DPL3),
    );
}

/// The address of a handler, as a gate carries it.
fn handler_address(address: usize) -> u64 {
    u64::try_from(address).unwrap_or(0)
}

fn put(table: &mut [[u64; 2]; IDT_ENTRIES], vector: u8, entry: [u64; 2]) {
    if let Some(slot) = table.get_mut(usize::from(vector)) {
        *slot = entry;
    }
}
