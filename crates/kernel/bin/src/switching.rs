// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Fixed-home context slots outside the machine borrow.
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use kernel_types::VirtAddr;

/// Context storage stays outside the machine cell across the assembly switch.
static SAVED_CONTEXTS: [AtomicU64; kernel_objects::config::THREADS] =
    [const { AtomicU64::new(0) }; kernel_objects::config::THREADS];
static CONTEXT_GENERATIONS: [AtomicU32; kernel_objects::config::THREADS] =
    [const { AtomicU32::new(0) }; kernel_objects::config::THREADS];
static EXECUTING: [AtomicU64; kernel_objects::config::CPUS] =
    [const { AtomicU64::new(0) }; kernel_objects::config::CPUS];

pub(crate) fn executing() -> Option<kernel_objects::ThreadId> {
    let cpu = usize::from(kernel_hal_x86_64::processor::processor()?);
    let word = EXECUTING.get(cpu)?.load(Ordering::Relaxed);
    if word == 0 {
        return None;
    }
    Some(kernel_objects::ThreadId::new(
        u32::try_from(word & 0xffff_ffff).ok()?.checked_sub(1)?,
        u32::try_from(word >> 32).ok()?,
    ))
}

pub(crate) fn record_executing(id: kernel_objects::ThreadId) {
    let cpu = usize::from(kernel_hal_x86_64::processor::processor().unwrap_or(0));
    if let Some(slot) = EXECUTING.get(cpu) {
        slot.store(
            (u64::from(id.generation()) << 32) | (u64::from(id.index()).saturating_add(1)),
            Ordering::Relaxed,
        );
    }
}

pub(crate) fn saved_context(
    id: kernel_objects::ThreadId,
    initial: VirtAddr,
) -> Option<&'static AtomicU64> {
    let index = usize::try_from(id.index()).ok()?;
    let generation = CONTEXT_GENERATIONS.get(index)?;
    let context = SAVED_CONTEXTS.get(index)?;
    if generation.load(Ordering::Relaxed) != id.generation() {
        context.store(initial.as_u64(), Ordering::Relaxed);
        generation.store(id.generation(), Ordering::Relaxed);
    }
    Some(context)
}
