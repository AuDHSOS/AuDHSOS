// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The worker pool.
//!
//! Every document is independent of every other: it is read, parsed, laid
//! out, and written without looking at anything else. So the pool is the
//! simple one — a shared cursor into the list of jobs, and as many threads
//! as the machine has, each taking the next job whenever it has finished
//! one. There is no work stealing to do and no order to preserve while the
//! work runs, only at the end, where the results are returned in the order
//! the jobs were given.
//!
//! The threads are scoped, so they borrow the jobs rather than owning a
//! copy of them, and the scope cannot end while one is still running.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// How many workers to use when nothing was asked for. A machine that
/// cannot say gets one.
#[must_use]
pub(crate) fn parallelism() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

/// Runs `work` over every job, several at a time, and returns what it
/// produced in the order the jobs were given.
///
/// The jobs are taken in order, so putting the long ones first is what
/// keeps the last worker from finishing long after the others.
pub(crate) fn map<J, R>(jobs: &[J], workers: usize, work: impl Fn(&J) -> R + Sync) -> Vec<R>
where
    J: Sync,
    R: Send,
{
    let mut results: Vec<Option<R>> = Vec::new();
    results.resize_with(jobs.len(), || None);
    if jobs.is_empty() {
        return Vec::new();
    }
    let workers = workers.clamp(1, jobs.len());
    let next = AtomicUsize::new(0);
    let slots = Mutex::new(results);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(index) else {
                        break;
                    };
                    let result = work(job);
                    // The lock is taken only to hand the result over, never
                    // while a document is being built.
                    if let Ok(mut slots) = slots.lock()
                        && let Some(slot) = slots.get_mut(index)
                    {
                        *slot = Some(result);
                    }
                }
            });
        }
    });
    slots
        .into_inner()
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect()
}
