// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![doc = include_str!("../README.md")]

mod task;

use core::panic::PanicInfo;

use audhsos_abi::layout::{BOOT_STACK_TOP, TICKS_PER_SECOND};
use kernel_core::memory::MemoryError;
use kernel_core::state::KernelState;
use kernel_core::trap::{Exception, Response};
use kernel_core::{boot, memory, println, tick, trap};
use kernel_hal_api::exit::{ExitStatus, TestExit};
use kernel_hal_x86_64::bootinfo::X86Platform;
use kernel_hal_x86_64::exit::QemuExit;
use kernel_hal_x86_64::interrupts::{self, ApicError};
use kernel_hal_x86_64::paging::{LocalTlb, X86Entry, active_root};
use kernel_hal_x86_64::traps::{self, TrapReport};
use kernel_hal_x86_64::window::PhysicalWindow;
use kernel_hal_x86_64::{entry, instructions, vectors};
use kernel_types::{PhysFrameRange, VirtAddr};

/// The entry the loader jumps to, with the address of the boot information
/// page in the first argument.
///
/// # Safety
///
/// The loader calls this once, with interrupts off, on the stack it set
/// up, and never returns to it.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_entry(boot_info: u64) -> ! {
    // SAFETY: the loader passes the address of the page it wrote and
    // mapped readable, sets up the boot stack, and calls this once on the
    // boot processor with interrupts off.
    unsafe { entry::start(boot_info, BOOT_STACK_TOP, on_trap, run) }
}

/// What the kernel does once the machine is ready.
fn run(platform: &X86Platform) {
    let outcome = entry::with_console(|console| {
        boot::run(platform, console)?;
        take_memory(platform)?;
        memory::with_memory(|memory| memory::report(memory, console));
        Ok(())
    });
    match outcome {
        Some(Ok(())) => {}
        Some(Err(error)) => {
            entry::with_console(|console| boot::abort(error, console, &mut QemuExit::new()));
            return;
        }
        None => entry::fail(b"[boot] the console is not reachable\n"),
    }
    if let Err(error) = take_interrupts(platform) {
        entry::with_console(|console| {
            println!(console, "[boot] {error}");
            QemuExit::new().exit(ExitStatus::Failure);
        });
        entry::fail(b"[boot] the interrupt hardware did not come up\n");
    }

    traps::set_syscall_handler(on_syscall);
    if !task::start(platform) {
        entry::with_console(|console| {
            println!(console, "[boot] there is no root task to start");
            QemuExit::new().exit(ExitStatus::Failure);
        });
        entry::fail(b"[boot] there is no root task to start\n");
    }
    idle();
}

/// What the kernel is once the root task runs: the thread that takes the
/// processor when nothing else can, and gives it up again the moment
/// something can.
fn idle() -> ! {
    loop {
        task::run(None);
        // A switch carries no interrupt flag: the six words it saves are
        // the callee-saved registers and nothing else. So a thread that
        // gives the processor up inside a trap — every system call is one,
        // and an interrupt gate clears the flag — hands the processor on
        // with interrupts off, and every thread but this one turns them
        // back on by returning to user mode through `iretq`. This one
        // returns nowhere; it halts. Halting with interrupts off stops the
        // machine for good, so they go back on here first.
        //
        // SAFETY: nothing of the machine is borrowed here — `task::run`
        // gave every borrow back — and this is the boot processor, whose
        // descriptor tables carry a handler for every vector.
        unsafe {
            instructions::enable_interrupts();
        }
        instructions::halt();
    }
}

/// What the kernel does with a system call: read the buffer of the thread
/// that made it, answer, clear away what ended, and switch if the answer
/// asks for it.
fn on_syscall() {
    let Some(Some(caller)) = kernel_core::with_machine(|machine| machine.scheduler.current())
    else {
        return;
    };
    let frame = kernel_core::with_machine(|machine| {
        machine
            .objects
            .threads
            .get(caller)
            .ok()
            .map(|thread| thread.ipc_buffer)
    })
    .flatten();
    let Some(frame) = frame else {
        return;
    };
    // SAFETY: the kernel tables are active, so the window maps every
    // physical frame read and write, and this is the only writer of the
    // buffer while the call runs.
    let mut buffer_window = unsafe { PhysicalWindow::kernel() };
    let Some(bytes) = PhysicalWindow::frame_bytes_mut(&mut buffer_window, frame) else {
        return;
    };
    // The interrupt controller and the ports, which the calls that touch a
    // line or a port need. Holding it turns interrupts off for the length
    // of the call.
    let reschedule = interrupts::with_controller(|apics| {
        let mut devices = kernel_hal_x86_64::ports::DeviceAccess::new(apics);
        task::answer(caller, bytes, Some(&mut devices))
    })
    .unwrap_or(false);
    if reschedule {
        task::run(Some(caller));
    }
}

/// Takes the interrupt hardware over: the tables of the machine, the two
/// APIC register windows, the legacy controllers, and the timer. Returns
/// the number of ticks that had arrived by the time the kernel had nothing
/// left to do.
fn take_interrupts(platform: &X86Platform) -> Result<u64, ApicError> {
    traps::set_interrupt_handler(on_interrupt);
    // SAFETY: the tables the loader built are active, the descriptor
    // tables carry a handler for every vector of the plan, interrupts are
    // off, and this runs once on the boot processor.
    unsafe { interrupts::bring_up(platform, map_device) }?;
    // SAFETY: the interval timer belongs to the kernel, interrupts are
    // still off, and this is the processor the controller belongs to.
    unsafe { interrupts::start_timer(TICKS_PER_SECOND) }?;
    // SAFETY: the descriptor table is loaded and every vector the hardware
    // can raise has a handler.
    unsafe {
        instructions::enable_interrupts();
    }
    Ok(wait_for_ticks(TICKS_BEFORE_HANDOVER))
}

/// How many ticks the kernel waits for before it reports that the timer
/// runs. Phase 5 replaces the wait with a scheduler.
const TICKS_BEFORE_HANDOVER: u64 = 3;

/// Waits until `wanted` ticks have arrived, halting between them.
fn wait_for_ticks(wanted: u64) -> u64 {
    let mut seen = interrupts::ticks();
    while seen < wanted {
        instructions::halt();
        seen = interrupts::ticks();
    }
    seen
}

/// Maps the frames of a device register window into the physical window,
/// uncached, out of the address space the memory bring-up left.
fn map_device(frames: PhysFrameRange) -> Option<VirtAddr> {
    // SAFETY: the kernel tables are active, so the window maps every
    // physical frame read and write, and the kernel is the only writer.
    let mut window = unsafe { PhysicalWindow::kernel() };
    let mut tlb = LocalTlb::new();
    memory::with_memory(|kernel| {
        kernel
            .map_device::<X86Entry, _, _>(&mut window, &mut tlb, frames)
            .ok()
    })
    .flatten()
}

/// Counts a device interrupt in the kernel state, forwards it to whoever
/// holds the interrupt object of its vector, and acknowledges it at the
/// hardware.
fn on_interrupt(vector: u8) {
    let counted = kernel_core::with_state(|state| {
        state.record_interrupt();
        match vector {
            vectors::TIMER => {
                tick::on_tick(state);
            }
            vectors::SPURIOUS => state.record_spurious(),
            _ => {}
        }
    });
    let _ = counted;
    if vector != vectors::TIMER && vector != vectors::SPURIOUS {
        // The forwarding acknowledges at the hardware itself: the order of
        // the three steps is part of what 2.7 asks for.
        forward(vector);
        return;
    }
    interrupts::acknowledge(vector);
}

/// Hands `vector` to the driver that holds its interrupt object, in the
/// order [2.7](../../../docs/02-architecture.md) gives: mask the line at
/// the I/O APIC, end the interrupt at the local APIC, then signal the
/// notification. A vector no interrupt object names is acknowledged and
/// nothing else.
///
/// Interrupts arrive through interrupt gates, so no device interrupt reaches
/// a kernel that holds the machine borrow; one that finds it busy all the
/// same leaves the bits for the next one.
///
/// The switch a woken driver of higher priority asks for is the last of the
/// three steps, and it is here now that the image starts a root task: a
/// driver that the interrupt made runnable takes the processor from
/// whatever was on it, which is the whole point of waking it.
fn forward(vector: u8) {
    let line = kernel_core::with_machine(|machine| {
        kernel_ipc::interrupt_for(&machine.objects, vector).and_then(|id| {
            machine
                .objects
                .interrupts
                .get(id)
                .ok()
                .map(|held| held.line)
        })
    })
    .flatten();
    if let Some(line) = line {
        interrupts::with_controller(|apics| {
            use kernel_hal_api::interrupt::{InterruptController, InterruptLine};
            apics.mask(InterruptLine::new(line));
        });
    }
    interrupts::acknowledge(vector);
    let outcome = kernel_core::with_machine(|machine| {
        kernel_ipc::deliver(&mut machine.objects, &mut machine.scheduler, vector)
    })
    .flatten();
    let Some(outcome) = outcome else {
        return;
    };
    if let Some(wakeup) = outcome.wakeup {
        announce(wakeup);
    }
    if outcome.reschedule {
        task::run(None);
    }
}

/// Writes the status word and the return words a thread that a device
/// interrupt woke finds in its buffer.
fn announce(wakeup: kernel_ipc::Wakeup) {
    let frame = kernel_core::with_machine(|machine| {
        machine
            .objects
            .threads
            .get(wakeup.thread)
            .ok()
            .map(|thread| thread.ipc_buffer)
    })
    .flatten();
    let Some(frame) = frame else {
        return;
    };
    // SAFETY: the kernel tables are active, so the window maps every
    // physical frame read and write, and the kernel is the only writer.
    let mut window = unsafe { PhysicalWindow::kernel() };
    let Some(bytes) = PhysicalWindow::frame_bytes_mut(&mut window, frame) else {
        return;
    };
    let mut writer = audhsos_abi::ipc_buffer::BufferMut::new(bytes);
    writer.clear_result();
    writer.set_status(wakeup.status);
    for (index, value) in wakeup.values.iter().enumerate() {
        writer.set_return_word(index, *value);
    }
}

/// Takes the memory of the machine over: the reserve, the regions of the
/// kernel address space, and the end of the loader's identity mapping.
fn take_memory(platform: &X86Platform) -> Result<(), boot::BootError> {
    let root = active_root().map_err(|_| MemoryError::Address)?;
    // SAFETY: the tables the loader built are active, so the window maps
    // every physical frame read and write; the kernel is the only writer,
    // because nothing else runs yet and interrupts are off.
    let mut window = unsafe { PhysicalWindow::kernel() };
    let mut tlb = LocalTlb::new();
    let override_bytes = requested_reserve(platform);
    memory::initialize::<X86Entry, _, _, _>(platform, root, &mut window, &mut tlb, override_bytes)?;
    Ok(())
}

/// The reserve size the header of the boot image asks for, or zero for the
/// default. A boot image the kernel cannot read asks for nothing.
fn requested_reserve(platform: &X86Platform) -> u64 {
    let Some((start, _)) = memory::boot_image(platform) else {
        return 0;
    };
    // SAFETY: the tables the loader built are active, so the window maps
    // every physical frame read and write.
    let Some(bytes) = (unsafe { kernel_hal_x86_64::memory::read_frame(start) }) else {
        return 0;
    };
    memory::requested_reserve(platform, &bytes)
}

/// What the kernel does with a processor exception.
///
/// An exception of the kernel's own ends the machine: there is nothing left
/// that could be trusted to report it. One from user mode is a fault of the
/// thread that took it, and becomes a message to whoever handles that
/// process — which is what [2.6](../../../docs/02-architecture.md) asks for
/// and what a system with a root task can now do.
fn on_trap(report: TrapReport) {
    let exception = Exception {
        vector: report.vector,
        error_code: report.error_code,
        ip: report.ip,
        sp: report.sp,
        cr2: report.fault_address,
        user: report.from_user(),
    };
    if exception.response() == Response::StopMachine {
        let mut state = KernelState::new();
        let reported = entry::with_console(|console| {
            trap::on_exception(exception, &mut state, console, &mut QemuExit::new());
        });
        if reported.is_none() {
            entry::fail(b"[trap] a trap arrived while one was being reported\n");
        }
        return;
    }

    let mut state = KernelState::new();
    let reported = entry::with_console(|console| {
        kernel_core::trap::on_user_fault(exception, &mut state, console);
    });
    if reported.is_none() {
        entry::fail(b"[trap] the console is not reachable\n");
    }
    let Some(Some(faulted)) = kernel_core::with_machine(|machine| machine.scheduler.current())
    else {
        entry::fail(b"[trap] a user thread faulted and the kernel holds none\n");
    };
    let fault = exception.fault().unwrap_or(audhsos_abi::Fault {
        kind: audhsos_abi::FaultKind::GeneralProtection,
        address: exception.cr2,
        instruction_pointer: exception.ip,
        error_code: exception.error_code,
    });
    if task::deliver_fault(faulted, fault) {
        task::run(Some(faulted));
    }
}

/// Reports a panic through the console and ends the machine.
#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    // As `entry::fail`: the machine is ending, so the line is the
    // kernel's again.
    kernel_hal_x86_64::console::reclaim();
    entry::with_console(|console| {
        use kernel_core::println;
        println!(console, "[panic] {}", info.message());
    });
    entry::fail(b"")
}
