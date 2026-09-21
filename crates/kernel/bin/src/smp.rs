// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Serial AP startup and entry into each processor's idle thread.
use crate::runtime::{with_machine, with_memory};
use audhsos_abi::layout::TICKS_PER_SECOND;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use kernel_core::println;
use kernel_hal_x86_64::paging::{LocalTlb, X86Entry};
use kernel_hal_x86_64::window::PhysicalWindow;
use kernel_hal_x86_64::{
    apic::Command, descriptors, entry, instructions, interrupts, processor, startup, timer,
};

static RELEASE: [AtomicBool; processor::CPUS] = [const { AtomicBool::new(false) }; processor::CPUS];
static SLOTS: [AtomicU32; processor::CPUS] = [const { AtomicU32::new(0) }; processor::CPUS];
static TOPS: [AtomicU64; processor::CPUS] = [const { AtomicU64::new(0) }; processor::CPUS];
static ROOT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn start() {
    const _: () = assert!(processor::CPUS == kernel_objects::config::CPUS);
    processor::ONLINE[0].store(1, Ordering::Release);
    interrupts::with_madt(|madt| {
        if madt.omitted_processors != 0 {
            report(
                1,
                "firmware processor list exceeds capacity; extra processors remain offline",
            );
        }
    });
    let Some(root) = with_memory(|memory| memory.root()) else {
        return;
    };
    ROOT.store(root.start().as_u64(), Ordering::Relaxed);
    kernel_hal_x86_64::remote::ROOTS[0].store(root.start().as_u64(), Ordering::SeqCst);
    if processor::IDENTIFIERS[1].load(Ordering::Relaxed) == u32::MAX {
        return;
    }
    if root.start().as_u64() >= 1u64 << 32 {
        report(1, "page-table root exceeds 4 GiB");
        return;
    }
    // SAFETY: the memory cell serializes all references made through this window.
    let mut window = unsafe { PhysicalWindow::kernel() };
    let Some(Ok(Some(frame))) =
        with_memory(|memory| memory.startup_page::<X86Entry, _, _>(&mut window, &mut LocalTlb))
    else {
        report(1, "no usable startup frame");
        return;
    };
    let mut count: usize = 1;
    let mut complete = true;
    for cpu in 1..processor::CPUS {
        let Ok(destination) =
            u8::try_from(processor::slot(&processor::IDENTIFIERS, cpu).load(Ordering::Relaxed))
        else {
            continue;
        };
        let Some(Ok(stack)) = with_memory(|memory| {
            memory.allocate_stack::<X86Entry, _, _>(
                &mut window,
                &mut kernel_hal_x86_64::remote::SharedTlb::new(),
            )
        }) else {
            break;
        };
        let Some(top) = stack.top() else {
            break;
        };
        processor::slot(&SLOTS, cpu).store(stack.index(), Ordering::Relaxed);
        processor::slot(&TOPS, cpu).store(top.as_u64(), Ordering::Relaxed);
        let number = u8::try_from(cpu).unwrap_or_else(|_| entry::fail(b"processor capacity\n"));
        // SAFETY: startup is serial; earlier APs have left this page.
        if unsafe { startup::prepare(frame, root, application_entry, top.as_u64(), number) }
            .is_none()
        {
            break;
        }
        send_startup(destination, frame);
        processor::slot(&RELEASE, cpu).store(true, Ordering::Release);
        let deadline = interrupts::ticks().saturating_add(100);
        while processor::slot(&processor::ONLINE, cpu).load(Ordering::Acquire) == 0
            && interrupts::ticks() < deadline
        {
            core::hint::spin_loop();
        }
        if processor::slot(&processor::ONLINE, cpu).load(Ordering::Acquire) == 0 {
            // A late AP may still read the startup page. Retain its mapping and parameters.
            complete = false;
            break;
        }
        count = count.saturating_add(1);
    }
    if complete {
        let mut tlb = kernel_hal_x86_64::remote::SharedTlb::new();
        if !matches!(
            with_memory(|memory| memory.finish_startup::<X86Entry, _, _>(
                &mut window,
                &mut tlb,
                frame
            )),
            Some(Ok(()))
        ) {
            entry::fail(b"startup identity mapping remained present\n");
        }
    }
    report(
        count,
        if complete {
            "ready"
        } else {
            "startup timeout; page retained"
        },
    );
}

/// Sends the serial INIT–SIPI–SIPI sequence before releasing this AP.
fn send_startup(destination: u8, frame: kernel_types::PhysFrame) {
    let _guard = instructions::InterruptGuard::new();
    assert_eq!(
        processor::with_local(|local| local.send(Command::init(destination))),
        Some(true)
    );
    // SAFETY: no AP calibrates until RELEASE is published below.
    unsafe {
        timer::wait_micros(10_000).unwrap_or_else(|_| entry::fail(b"INIT delay\n"));
    }
    let page =
        u8::try_from(frame.number()).unwrap_or_else(|_| entry::fail(b"startup below 1 MiB\n"));
    assert_eq!(
        processor::with_local(|local| local.send(Command::startup(destination, page))),
        Some(true)
    );
    // SAFETY: BSP still owns channel two.
    unsafe {
        timer::wait_micros(200).unwrap_or_else(|_| entry::fail(b"SIPI delay\n"));
    }
    assert_eq!(
        processor::with_local(|local| local.send(Command::startup(destination, page))),
        Some(true)
    );
}

extern "C" fn application_entry(number: u64) -> ! {
    let cpu = usize::try_from(number).unwrap_or_else(|_| entry::fail(b"processor number\n"));
    while !processor::slot(&RELEASE, cpu).load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
    processor::slot(&kernel_hal_x86_64::remote::ROOTS, cpu)
        .store(ROOT.load(Ordering::Relaxed), Ordering::SeqCst);
    // SAFETY: this AP enters once on its allocated stack with interrupts off.
    unsafe {
        descriptors::install_application(processor::slot(&TOPS, cpu).load(Ordering::Relaxed))
            .unwrap_or_else(|_| entry::fail(b"AP descriptors\n"));
    }
    // SAFETY: this AP owns its local APIC handle.
    unsafe {
        processor::install_local();
    }
    processor::with_local(|local| {
        // SAFETY: the BSP waits for this processor before using PIT channel two again.
        let rate = unsafe { timer::calibrate(local) }
            .unwrap_or_else(|_| entry::fail(b"AP timer calibration\n"));
        let count = timer::initial_count(rate, TICKS_PER_SECOND)
            .unwrap_or_else(|_| entry::fail(b"AP timer rate\n"));
        local.set_timer(kernel_hal_x86_64::vectors::TIMER, true, false);
        local.set_timer_count(count);
    });
    crate::task::idle_thread(processor::slot(&SLOTS, cpu).load(Ordering::Relaxed));
    with_machine(|machine| {
        machine
            .scheduler
            .online(u8::try_from(cpu).unwrap_or_else(|_| entry::fail(b"processor capacity\n")));
    });
    processor::slot(&processor::ONLINE, cpu).store(1, Ordering::Release);
    crate::idle();
}

fn report(count: usize, reason: &str) {
    entry::with_console(|console| println!(console, "[smp] {count} processors: {reason}"));
}
