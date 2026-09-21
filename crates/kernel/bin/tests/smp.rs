// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Actual AP startup, remote preemption, concurrent users and invalidation.
#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use kernel_hal_api::paging::TlbControl;
use kernel_hal_x86_64::{instructions, processor, remote};
use kernel_types::{Page, VirtAddr};

use support::runtime;
#[path = "../src/smp.rs"]
mod smp;
mod support;
mod task {
    pub(crate) fn idle_thread(slot: u32) {
        crate::support::idle_on(slot);
    }
}

kernel_hal_x86_64::test_kernel!();

static CONTEND: AtomicBool = AtomicBool::new(false);
static GO: AtomicBool = AtomicBool::new(false);
static WAITING: AtomicBool = AtomicBool::new(false);
static COUNTERS: AtomicU64 = AtomicU64::new(0);
static WORKERS: AtomicU64 = AtomicU64::new(0);
static UNMAPPED: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicU64 = AtomicU64::new(0);
static HIGH: AtomicU64 = AtomicU64::new(0);
static HIGH_STARTED: AtomicBool = AtomicBool::new(false);
static HIGH_RAN: AtomicBool = AtomicBool::new(false);
static UNMAP_TICKS: AtomicU64 = AtomicU64::new(0);
static RETIRING: AtomicBool = AtomicBool::new(false);
static ROOT: AtomicU64 = AtomicU64::new(0);

fn idle() -> ! {
    loop {
        if CONTEND.load(Ordering::Acquire) && processor::processor() == Some(1) {
            let _guard = instructions::InterruptGuard::new();
            WAITING.store(true, Ordering::Release);
            while !GO.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
            runtime::with_machine(|_| ());
        }
        support::run_until_idle();
        instructions::halt();
    }
}

fn online() -> usize {
    processor::ONLINE
        .iter()
        .filter(|cpu| cpu.load(Ordering::Acquire) != 0)
        .count()
}

fn tick(now: u64) -> bool {
    if processor::processor() != Some(0)
        || WORKERS.load(Ordering::Acquire) == 0
        || UNMAPPED.load(Ordering::Acquire)
    {
        return false;
    }
    assert!(
        now.saturating_sub(STARTED.load(Ordering::Relaxed)) < 5_000,
        "workers stalled"
    );
    let count = usize::try_from(WORKERS.load(Ordering::Acquire)).unwrap_or(0);
    let address = usize::try_from(COUNTERS.load(Ordering::Relaxed)).unwrap_or(0);
    // SAFETY: the test retains this mapped frame until every worker faults.
    let counters = unsafe { &*core::ptr::without_provenance::<[AtomicU64; 16]>(address) };
    if counters
        .iter()
        .take(count)
        .any(|counter| counter.load(Ordering::Acquire) < 100)
    {
        return false;
    }
    if !HIGH_STARTED.swap(true, Ordering::AcqRel) {
        return support::start(support::unpack(HIGH.load(Ordering::Acquire)).unwrap());
    }
    if !HIGH_RAN.load(Ordering::Acquire) {
        return false;
    }
    let root = kernel_types::PhysAddr::new(ROOT.load(Ordering::Relaxed))
        .and_then(kernel_types::PhysFrame::from_start)
        .unwrap();
    let page = Page::from_start(VirtAddr::new(support::SHARED_BASE).unwrap()).unwrap();
    let started = instructions::read_tsc();
    runtime::with_memory(|memory| {
        let mut window = support::window();
        let mut tlb = remote::SharedTlb::new();
        if RETIRING.load(Ordering::Relaxed) {
            tlb.retire(root, memory.root());
            for active in &remote::ROOTS {
                assert_ne!(active.load(Ordering::SeqCst), root.start().as_u64());
            }
            kernel_mm::kernel_half::free_user_half::<kernel_hal_x86_64::paging::X86Entry, _, _>(
                &mut window,
                memory.frames_mut(),
                root,
            );
            return;
        }
        let mut mapper =
            kernel_mm::mapper::Mapper::<kernel_hal_x86_64::paging::X86Entry, _, _, _>::new(
                root,
                &mut window,
                &mut tlb,
                memory.frames_mut(),
            );
        mapper.unmap(page).unwrap();
    });
    UNMAP_TICKS.store(
        instructions::read_tsc().wrapping_sub(started),
        Ordering::Relaxed,
    );
    UNMAPPED.store(true, Ordering::Release);
    false
}

/// Online processors advance counters and fault after a synchronous unmap.
#[test_case]
fn processors_run_and_invalidate() {
    support::bring_up();
    support::idle_thread();
    support::start_timer(tick);
    smp::start();
    support::say!("startup returned");
    let wanted = processor::IDENTIFIERS
        .iter()
        .filter(|id| id.load(Ordering::Relaxed) != u32::MAX)
        .count();
    assert_eq!(online(), wanted);
    let until = support::ticks().saturating_add(10);
    while support::ticks() < until {
        instructions::halt();
    }
    for ticks in processor::TICKS.iter().take(wanted) {
        assert!(ticks.load(Ordering::Relaxed) > 0);
    }
    support::say!("all local timers tick");
    if wanted > 1 {
        kernel_hal_x86_64::interrupts::with_controller(|_| {
            let before = processor::TICKS[1].load(Ordering::Relaxed);
            let start = instructions::read_tsc();
            while processor::TICKS[1].load(Ordering::Relaxed) < before.saturating_add(3) {
                assert!(
                    instructions::read_tsc().wrapping_sub(start) < 10_000_000_000,
                    "AP timer depends on the controller lock"
                );
                core::hint::spin_loop();
            }
        });
        CONTEND.store(true, Ordering::Release);
        while !WAITING.load(Ordering::Acquire) {
            core::hint::spin_loop();
        }
        runtime::with_machine(|_| {
            support::say!("AP ready to contend");
            GO.store(true, Ordering::Release);
            let before = remote::ACKNOWLEDGED[1].load(Ordering::Acquire);
            // The memory lock precedes the machine lock in production. This test
            // owns only the machine lock and deliberately requests an invalidation.
            remote::SharedTlb::new().flush_all();
            assert_eq!(
                remote::ACKNOWLEDGED[1].load(Ordering::Acquire),
                before.wrapping_add(1)
            );
            CONTEND.store(false, Ordering::Release);
        });
    }
    support::say!("invalidation while contended completed");
    run_workers(wanted, false);
    run_workers(wanted, true);
}

fn run_workers(wanted: usize, retiring: bool) {
    WORKERS.store(0, Ordering::Release);
    UNMAPPED.store(false, Ordering::Relaxed);
    HIGH_STARTED.store(false, Ordering::Relaxed);
    HIGH_RAN.store(false, Ordering::Relaxed);
    RETIRING.store(retiring, Ordering::Relaxed);
    let before = support::faults();
    let mut process = support::create_process(include_bytes!(concat!(
        env!("AUDHSOS_USER_TESTS_DIR"),
        "/smp_worker.bin"
    )));
    let frame = support::share_page(&process);
    let address = audhsos_abi::layout::PHYS_WINDOW_BASE.saturating_add(frame.start().as_u64());
    COUNTERS.store(address, Ordering::Relaxed);
    ROOT.store(process.root.start().as_u64(), Ordering::Relaxed);
    let mut threads = [None; processor::CPUS];
    for (cpu, slot) in threads.iter_mut().enumerate().take(wanted) {
        let thread = support::add_thread(&mut process, support::DEFAULT_PRIORITY);
        runtime::with_machine(|machine| {
            machine.objects.threads.get_mut(thread.thread).unwrap().cpu = u8::try_from(cpu).unwrap()
        });
        support::set_buffer_word(thread.buffer, 0, support::SHARED_BASE);
        support::set_buffer_word(thread.buffer, 1, u64::try_from(cpu).unwrap());
        *slot = Some(thread.thread);
    }
    let mut high_process = support::create_process(include_bytes!(concat!(
        env!("AUDHSOS_USER_TESTS_DIR"),
        "/count_and_exit.bin"
    )));
    let high = support::add_thread(&mut high_process, support::MAX_PRIORITY);
    runtime::with_machine(|machine| {
        machine.objects.threads.get_mut(high.thread).unwrap().cpu = if wanted > 1 { 1 } else { 0 }
    });
    HIGH.store(support::pack(high.thread), Ordering::Release);
    support::set_call_hook(|_| {
        let running = runtime::with_machine(|machine| machine.scheduler.current()).flatten();
        if running == support::unpack(HIGH.load(Ordering::Acquire)) {
            HIGH_RAN.store(true, Ordering::Release);
        }
    });
    STARTED.store(support::ticks(), Ordering::Relaxed);
    WORKERS.store(u64::try_from(wanted).unwrap(), Ordering::Release);
    support::without_ticks(|| {
        for thread in threads.iter().flatten() {
            support::start(*thread);
        }
    });
    support::idle_until(|| {
        support::faults() == before.saturating_add(u32::try_from(wanted).unwrap())
    });
    assert!(UNMAPPED.load(Ordering::Acquire));
    assert!(HIGH_RAN.load(Ordering::Acquire));
    kernel_hal_x86_64::testing::measure(
        if retiring {
            "smp::retire_root"
        } else {
            "smp::shared_page_unmap"
        },
        UNMAP_TICKS.load(Ordering::Relaxed),
        1,
    );
    for thread in threads.iter().flatten() {
        assert_eq!(
            support::state_of(*thread),
            Some(audhsos_abi::ThreadState::Faulted)
        );
    }
    support::say!("{wanted} processors counted; all faulted after retirement={retiring}");
}
