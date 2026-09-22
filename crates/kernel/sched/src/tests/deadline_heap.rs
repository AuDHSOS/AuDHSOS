// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Deadline ordering, cancellation, capacity, and comparison bounds.

use audhsos_abi::Error;
use kernel_objects::config::THREADS;
use kernel_objects::{Pool, ProcessId, Thread, ThreadId};
use kernel_types::{PhysAddr, PhysFrame};

use crate::deadline::Deadlines;

fn thread<const N: usize>(threads: &mut Pool<Thread, N>) -> ThreadId {
    let frame = PhysFrame::containing(PhysAddr::new(0x20_0000).unwrap());
    threads
        .allocate(Thread::new(ProcessId::new(0, 1), 1, 31, 0, frame).unwrap())
        .unwrap()
}

#[test]
fn decreasing_deadlines_take_logarithmic_comparisons_per_insert_and_pop() {
    let mut threads = Pool::<Thread, THREADS>::new();
    let mut heap = Deadlines::<THREADS>::new();
    let mut ids = Vec::new();
    for index in 0..THREADS {
        let id = thread(&mut threads);
        let before = heap.comparisons.get();
        heap.insert(&mut threads, id, u64::try_from(THREADS - index).unwrap())
            .unwrap();
        assert!(heap.comparisons.get() - before <= usize::try_from((index + 1).ilog2()).unwrap());
        ids.push(id);
    }
    for (index, id) in ids.into_iter().rev().enumerate() {
        let before = heap.comparisons.get();
        assert_eq!(heap.expired(&mut threads, u64::MAX), Some(id));
        assert!(
            heap.comparisons.get() - before
                <= 2 * usize::try_from((THREADS - index).ilog2()).unwrap()
        );
        assert_eq!(threads.get(id).unwrap().deadline_index, None);
    }
    assert_eq!(heap.len(), 0);
}

#[test]
fn cancelling_each_heap_position_preserves_order_and_indices() {
    // Removing 11 moves 4 below 10 and requires sifting upward.
    for remove in 0..7 {
        let mut threads = Pool::<Thread, 7>::new();
        let mut heap = Deadlines::<7>::new();
        let mut model = Vec::new();
        for deadline in [1, 10, 2, 11, 12, 3, 4] {
            let id = thread(&mut threads);
            heap.insert(&mut threads, id, deadline).unwrap();
            model.push((deadline, id));
        }
        let (_, id) = model.remove(remove);
        heap.remove(&mut threads, id);
        heap.remove(&mut threads, id);
        assert_eq!(threads.get(id).unwrap().deadline, None);
        assert_eq!(threads.get(id).unwrap().deadline_index, None);
        model.sort_by_key(|&(deadline, _)| deadline);
        for (_, id) in model {
            assert_eq!(heap.expired(&mut threads, u64::MAX), Some(id));
        }
        assert_eq!(heap.first(), None);
    }
}

#[test]
fn repeated_cancellation_and_reinsertion_agree_with_a_stable_sorted_model() {
    let mut threads = Pool::<Thread, 32>::new();
    let mut heap = Deadlines::<32>::new();
    let ids: Vec<_> = (0..32).map(|_| thread(&mut threads)).collect();
    let mut model: Vec<(u64, ThreadId)> = Vec::new();
    let mut seed = 55_u64;
    for step in 0..4_096 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let id = ids[usize::try_from(seed >> 32).unwrap() % ids.len()];
        if let Some(index) = model.iter().position(|&(_, held)| held == id) {
            model.remove(index);
            let before = heap.comparisons.get();
            heap.remove(&mut threads, id);
            assert!(
                heap.comparisons.get() - before <= 3 * usize::try_from(32_usize.ilog2()).unwrap()
            );
        } else {
            let deadline = seed % 8;
            heap.insert(&mut threads, id, deadline).unwrap();
            model.push((deadline, id));
            model.sort_by_key(|&(deadline, _)| deadline);
        }
        assert_eq!(heap.len(), model.len());
        assert_eq!(heap.first(), model.first().map(|&(_, id)| id));
        let indices: std::collections::HashSet<_> = model
            .iter()
            .map(|&(_, id)| threads.get(id).unwrap().deadline_index.unwrap())
            .collect();
        assert_eq!(indices.len(), model.len());
        assert!(indices.iter().all(|&index| index < heap.len()));
        if step % 17 == 0 {
            for (_, id) in model.drain(..) {
                assert_eq!(heap.expired(&mut threads, u64::MAX), Some(id));
            }
        }
    }
    for (_, id) in model {
        assert_eq!(heap.expired(&mut threads, u64::MAX), Some(id));
    }
}

#[test]
fn admission_errors_leave_the_heap_and_thread_unchanged() {
    let mut threads = Pool::<Thread, 3>::new();
    let mut heap = Deadlines::<1>::new();
    let first = thread(&mut threads);
    let second = thread(&mut threads);
    let stale = thread(&mut threads);
    threads.force_release(stale);
    assert_eq!(
        heap.insert(&mut threads, stale, 0),
        Err(Error::InvalidHandle)
    );
    heap.insert(&mut threads, first, 0).unwrap();
    assert_eq!(
        heap.insert(&mut threads, first, 1),
        Err(Error::InvalidState)
    );
    assert_eq!(
        heap.insert(&mut threads, second, 1),
        Err(Error::PoolExhausted)
    );
    assert_eq!(threads.get(first).unwrap().deadline, Some(0));
    assert_eq!(threads.get(second).unwrap().deadline, None);
    assert_eq!(threads.get(second).unwrap().deadline_index, None);
    heap.remove(&mut threads, stale);
    assert_eq!(heap.expired(&mut threads, 0), Some(first));
    heap.insert(&mut threads, second, u64::MAX).unwrap();
    assert_eq!(heap.expired(&mut threads, u64::MAX - 1), None);
    assert_eq!(heap.expired(&mut threads, u64::MAX), Some(second));
    let mut empty = Deadlines::<0>::new();
    assert_eq!(
        empty.insert(&mut threads, first, 0),
        Err(Error::PoolExhausted)
    );
    assert_eq!(empty.expired(&mut threads, 0), None);
}

#[test]
fn sequence_exhaustion_preserves_fifo_and_emptying_resets_the_sequence() {
    let mut threads = Pool::<Thread, 3>::new();
    let mut heap = Deadlines::<3>::new();
    let first = thread(&mut threads);
    let second = thread(&mut threads);
    let third = thread(&mut threads);
    heap.insert(&mut threads, first, 100).unwrap();
    heap.set_order(u64::MAX - 1);
    heap.insert(&mut threads, second, 100).unwrap();
    assert_eq!(
        heap.insert(&mut threads, third, 100),
        Err(Error::QuotaExceeded)
    );
    assert_eq!(heap.expired(&mut threads, 100), Some(first));
    assert_eq!(heap.expired(&mut threads, 100), Some(second));
    heap.insert(&mut threads, third, 100).unwrap();
    assert_eq!(heap.expired(&mut threads, 100), Some(third));
}
