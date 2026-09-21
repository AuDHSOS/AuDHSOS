// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Serialized remote invalidations; lock waiters service requests with interrupts off.
use crate::paging::LocalTlb;
use crate::processor::{CPUS, IDENTIFIERS, ONLINE, processor, with_local};
use core::sync::atomic::{AtomicU64, Ordering};
use kernel_hal_api::paging::TlbControl;
use kernel_types::{Page, PhysFrame};
use kernel_x86_tables::{lapic::Command, vectors};

/// Active roots, published before loading CR3.
pub static ROOTS: [AtomicU64; CPUS] = [const { AtomicU64::new(0) }; CPUS];
static REQUEST: [AtomicU64; CPUS] = [const { AtomicU64::new(0) }; CPUS];
static GENERATION: [AtomicU64; CPUS] = [const { AtomicU64::new(0) }; CPUS];
/// Completed request generations.
pub static ACKNOWLEDGED: [AtomicU64; CPUS] = [const { AtomicU64::new(0) }; CPUS];
const ALL: u64 = u64::MAX;
const RETIRE: u64 = 1;

/// Services a request without borrowing any kernel cell.
pub fn poll() {
    let Some(cpu) = processor().map(usize::from) else {
        return;
    };
    let generation = crate::processor::slot(&GENERATION, cpu).load(Ordering::Acquire);
    if generation == crate::processor::slot(&ACKNOWLEDGED, cpu).load(Ordering::Relaxed) {
        return;
    }
    let address = crate::processor::slot(&REQUEST, cpu).load(Ordering::Relaxed);
    if address == ALL {
        LocalTlb.flush_all();
    } else if address & RETIRE != 0 {
        let replacement = kernel_types::PhysAddr::new(address & !RETIRE)
            .and_then(PhysFrame::from_start)
            .unwrap_or_else(|_| crate::entry::fail(b"invalid replacement root\n"));
        crate::paging::replace_retired_root(replacement);
    } else if let Ok(page) = kernel_types::VirtAddr::new(address).and_then(Page::from_start) {
        LocalTlb.flush_page(page);
    }
    crate::processor::slot(&ACKNOWLEDGED, cpu).store(generation, Ordering::Release);
}

/// TLB control used while the memory cell serializes page-table edits.
#[derive(Clone, Copy, Debug, Default)]
pub struct SharedTlb {
    root: u64,
}
impl SharedTlb {
    /// The root is selected by `Mapper::new` before any edit.
    #[must_use]
    pub const fn new() -> Self {
        Self { root: 0 }
    }
    fn remote(self, address: u64, shared: bool) {
        let Some(caller) = processor().map(usize::from) else {
            return;
        };
        for cpu in 0..CPUS {
            if cpu == caller || crate::processor::slot(&ONLINE, cpu).load(Ordering::Acquire) == 0 {
                continue;
            }
            if !shared && crate::processor::slot(&ROOTS, cpu).load(Ordering::SeqCst) != self.root {
                continue;
            }
            let generation = crate::processor::slot(&GENERATION, cpu)
                .load(Ordering::Relaxed)
                .wrapping_add(1);
            crate::processor::slot(&REQUEST, cpu).store(address, Ordering::Relaxed);
            crate::processor::slot(&GENERATION, cpu).store(generation, Ordering::Release);
            let destination =
                u8::try_from(crate::processor::slot(&IDENTIFIERS, cpu).load(Ordering::Relaxed))
                    .unwrap_or_else(|_| crate::entry::fail(b"invalid online APIC\n"));
            assert_eq!(
                with_local(|local| local.send(Command::fixed(destination, vectors::INVALIDATE))),
                Some(true),
                "invalidation IPI failed"
            );
            while crate::processor::slot(&ACKNOWLEDGED, cpu).load(Ordering::Acquire) != generation {
                poll();
                core::hint::spin_loop();
            }
        }
    }
}
impl TlbControl for SharedTlb {
    fn target(&mut self, root: PhysFrame) {
        self.root = root.start().as_u64();
    }
    fn retire(&mut self, root: PhysFrame, replacement: PhysFrame) {
        self.target(root);
        self.remote(replacement.start().as_u64() | RETIRE, false);
        LocalTlb.retire(root, replacement);
    }
    fn flush_page(&mut self, page: Page) {
        LocalTlb.flush_page(page);
        self.remote(page.start().as_u64(), !page.is_user());
    }
    fn flush_all(&mut self) {
        LocalTlb.flush_all();
        self.remote(ALL, true);
    }
}
