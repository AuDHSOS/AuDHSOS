// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::pool`, covering the pool items of the catalog 6.6.6.

#![allow(clippy::arithmetic_side_effects)]

use std::collections::HashMap;

use crate::pool::{ObjectId, Pool, PoolError};
use crate::strategies::{PoolOp, any_pool_op};
use test_support::generators::BoxGen;
use test_support::model::{ModelTest, run_model_test};

/// Capacity of the pool the model test runs against.
const CAPACITY: usize = 8;

#[test]
fn a_new_pool_is_empty_and_reports_its_capacity() {
    let pool: Pool<u32, 4> = Pool::new();
    assert_eq!(pool.capacity(), 4);
    assert_eq!(pool.live(), 0);
    assert!(pool.is_empty());
    let default: Pool<u32, 4> = Pool::default();
    assert_eq!(default.live(), 0);
}

#[test]
fn a_pool_without_slots_is_exhausted_from_the_start() {
    let mut pool: Pool<u32, 0> = Pool::new();
    assert_eq!(pool.capacity(), 0);
    assert_eq!(pool.allocate(1), Err(PoolError::Exhausted));
}

#[test]
fn allocating_from_a_full_pool_is_exhausted() {
    let mut pool: Pool<u32, 3> = Pool::new();
    let ids: Vec<ObjectId<u32>> = (0..3).map(|value| pool.allocate(value).unwrap()).collect();
    assert_eq!(pool.live(), 3);
    assert_eq!(pool.allocate(9), Err(PoolError::Exhausted));
    assert_eq!(pool.live(), 3);
    assert_eq!(pool.release(ids[0]), Ok(true));
    assert!(pool.allocate(9).is_ok());
}

#[test]
fn allocation_stores_the_value_and_hands_out_one_reference() {
    let mut pool: Pool<u32, 2> = Pool::new();
    let id = pool.allocate(42).unwrap();
    assert_eq!(pool.get(id), Ok(&42));
    assert_eq!(pool.references(id), Ok(1));
    *pool.get_mut(id).unwrap() = 43;
    assert_eq!(pool.get(id), Ok(&43));
    assert_eq!(id.index(), 0);
    assert_eq!(id.generation(), 1);
}

#[test]
fn an_id_with_a_stale_generation_is_rejected_after_reuse() {
    let mut pool: Pool<u32, 1> = Pool::new();
    let first = pool.allocate(7).unwrap();
    assert_eq!(pool.release(first), Ok(true));
    let second = pool.allocate(8).unwrap();
    assert_eq!(second.index(), first.index());
    assert_eq!(second.generation(), first.generation() + 1);
    assert_eq!(pool.get(first), Err(PoolError::StaleId));
    assert_eq!(pool.get_mut(first).err(), Some(PoolError::StaleId));
    assert_eq!(pool.retain(first), Err(PoolError::StaleId));
    assert_eq!(pool.release(first), Err(PoolError::StaleId));
    assert_eq!(pool.references(first), Err(PoolError::StaleId));
    assert_eq!(pool.get(second), Ok(&8));
}

#[test]
fn an_id_of_a_free_slot_or_an_index_out_of_range_is_rejected() {
    let mut pool: Pool<u32, 2> = Pool::new();
    let never_used: ObjectId<u32> = ObjectId::new(1, 1);
    assert_eq!(pool.get(never_used), Err(PoolError::StaleId));
    let out_of_range: ObjectId<u32> = ObjectId::new(9, 1);
    assert_eq!(pool.get(out_of_range), Err(PoolError::StaleId));
    assert_eq!(pool.retain(out_of_range), Err(PoolError::StaleId));
    assert_eq!(pool.release(out_of_range), Err(PoolError::StaleId));
    let beyond_usize: ObjectId<u32> = ObjectId::new(u32::MAX, 1);
    assert_eq!(pool.get(beyond_usize), Err(PoolError::StaleId));
    assert!(pool.allocate(1).is_ok());
}

#[test]
fn the_generation_wraps_and_never_becomes_zero() {
    let mut pool: Pool<u32, 1> = Pool::new();
    assert!(pool.set_generation(0, u32::MAX - 1));
    assert!(!pool.set_generation(9, 5), "an unknown slot is not changed");
    let last = pool.allocate(1).unwrap();
    assert_eq!(last.generation(), u32::MAX);
    assert_eq!(pool.release(last), Ok(true));
    assert_eq!(
        pool.generation_of(0),
        Some(u32::MAX),
        "a released slot keeps its generation until it is handed out again"
    );
    let wrapped = pool.allocate(2).unwrap();
    assert_eq!(wrapped.generation(), 1, "zero is skipped");
    assert_eq!(pool.get(wrapped), Ok(&2));
    assert_eq!(
        pool.get(last),
        Err(PoolError::StaleId),
        "the id from before the wrap stays stale"
    );
}

#[test]
fn an_empty_pool_is_a_constant() {
    // The pools of the kernel are `static` cells of this type, and they
    // reach the `.bss` only because the empty pool is a constant (D-66).
    const POOL: Pool<u32, 4> = Pool::new();
    let pool = POOL;
    assert_eq!(pool.live(), 0);
    assert!(pool.is_empty());
    assert_eq!(pool.capacity(), 4);
}

#[test]
fn a_fresh_pool_hands_out_its_slots_in_index_order() {
    let mut pool: Pool<u32, 3> = Pool::new();
    let first = pool.allocate(1).unwrap();
    let second = pool.allocate(2).unwrap();
    let third = pool.allocate(3).unwrap();
    assert_eq!(
        [first.index(), second.index(), third.index()],
        [0, 1, 2],
        "the high-water mark walks the pool once"
    );
    assert_eq!(pool.allocate(4), Err(PoolError::Exhausted));
}

#[test]
fn the_first_generation_of_every_slot_is_one() {
    let mut pool: Pool<u32, 3> = Pool::new();
    for value in 0..3 {
        let id = pool.allocate(value).unwrap();
        assert_eq!(id.generation(), 1, "slot {}", id.index());
    }
}

#[test]
fn a_released_slot_is_handed_out_before_one_that_was_never_used() {
    let mut pool: Pool<u32, 3> = Pool::new();
    let first = pool.allocate(1).unwrap();
    assert_eq!(pool.release(first), Ok(true));
    let next = pool.allocate(2).unwrap();
    assert_eq!(next.index(), first.index(), "the released slot comes first");
    assert_eq!(next.generation(), 2, "and with the next generation");
    let fresh = pool.allocate(3).unwrap();
    assert_eq!(fresh.index(), 1, "then the high-water mark moves on");
}

#[test]
fn an_id_of_a_slot_that_was_never_used_names_nothing() {
    let pool: Pool<u32, 4> = Pool::new();
    assert_eq!(pool.get(ObjectId::new(0, 0)), Err(PoolError::StaleId));
    assert_eq!(pool.get(ObjectId::new(0, 1)), Err(PoolError::StaleId));
    assert_eq!(pool.get(ObjectId::new(3, 1)), Err(PoolError::StaleId));
}

#[test]
fn only_the_last_release_destroys_the_value() {
    let mut pool: Pool<u32, 2> = Pool::new();
    let id = pool.allocate(5).unwrap();
    assert_eq!(pool.retain(id), Ok(()));
    assert_eq!(pool.retain(id), Ok(()));
    assert_eq!(pool.references(id), Ok(3));
    assert_eq!(pool.release(id), Ok(false));
    assert_eq!(pool.release(id), Ok(false));
    assert_eq!(pool.references(id), Ok(1));
    assert_eq!(pool.live(), 1);
    assert_eq!(pool.release(id), Ok(true));
    assert_eq!(pool.live(), 0);
    assert_eq!(
        pool.release(id),
        Err(PoolError::StaleId),
        "releasing twice is impossible through the API"
    );
}

#[test]
fn retaining_at_the_maximum_reference_count_overflows() {
    let mut pool: Pool<u32, 1> = Pool::new();
    let id = pool.allocate(1).unwrap();
    assert!(
        !pool.set_generation(0, 5),
        "an occupied slot keeps its generation"
    );
    assert!(pool.set_references(0, u32::MAX));
    assert_eq!(pool.retain(id), Err(PoolError::RefOverflow));
    assert_eq!(pool.references(id), Ok(u32::MAX));
    assert!(!pool.set_references(9, 1), "an unknown slot is not changed");
}

#[test]
fn linking_behind_a_slot_outside_the_pool_leaves_the_free_list_alone() {
    let mut pool: Pool<u32, 2> = Pool::new();
    let first = pool.allocate(1).unwrap();
    let second = pool.allocate(2).unwrap();
    pool.link_free(9, 0);
    assert_eq!(pool.release(first), Ok(true));
    assert_eq!(pool.release(second), Ok(true));
    assert_eq!(pool.allocate(3).unwrap().index(), 0);
    assert_eq!(pool.allocate(4).unwrap().index(), 1);
}

#[test]
fn freed_slots_are_reused_in_the_order_they_were_freed() {
    let mut pool: Pool<u32, 4> = Pool::new();
    let ids: Vec<ObjectId<u32>> = (0..4).map(|value| pool.allocate(value).unwrap()).collect();
    assert_eq!(
        ids.iter().map(|id| id.index()).collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(pool.release(ids[2]), Ok(true));
    assert_eq!(pool.release(ids[0]), Ok(true));
    assert_eq!(pool.release(ids[3]), Ok(true));
    assert_eq!(pool.allocate(10).unwrap().index(), 2);
    assert_eq!(pool.allocate(11).unwrap().index(), 0);
    assert_eq!(pool.allocate(12).unwrap().index(), 3);
    assert_eq!(pool.allocate(13), Err(PoolError::Exhausted));
}

#[test]
fn emptying_a_pool_completely_refills_the_free_list() {
    let mut pool: Pool<u32, 2> = Pool::new();
    let first = pool.allocate(1).unwrap();
    let second = pool.allocate(2).unwrap();
    assert_eq!(pool.release(first), Ok(true));
    assert_eq!(pool.release(second), Ok(true));
    assert!(pool.is_empty());
    assert_eq!(pool.allocate(3).unwrap().index(), 0);
    assert_eq!(pool.allocate(4).unwrap().index(), 1);
    assert_eq!(pool.allocate(5), Err(PoolError::Exhausted));
}

#[test]
fn ids_compare_hash_and_render_by_index_and_generation() {
    let a: ObjectId<u32> = ObjectId::new(1, 2);
    let b: ObjectId<u32> = ObjectId::new(1, 2);
    let c: ObjectId<u32> = ObjectId::new(1, 3);
    let d: ObjectId<u32> = ObjectId::new(2, 2);
    assert_eq!(a, b);
    assert_ne!(a, c, "the generation differs");
    assert_ne!(a, d, "the index differs");
    assert_eq!(a, a.clone());
    let mut seen = HashMap::new();
    seen.insert(a, 1);
    assert_eq!(seen.get(&b), Some(&1));
    assert_eq!(seen.get(&c), None);
    assert!(format!("{a:?}").contains("gen 2"));
}

#[test]
fn errors_render_a_message_and_map_to_the_abi() {
    let cases = [
        (PoolError::Exhausted, audhsos_abi::Error::PoolExhausted),
        (PoolError::StaleId, audhsos_abi::Error::InvalidHandle),
        (PoolError::RefOverflow, audhsos_abi::Error::QuotaExceeded),
    ];
    for (error, expected) in cases {
        assert!(!format!("{error}").is_empty());
        assert_eq!(audhsos_abi::Error::from(error), expected);
    }
}

/// What the model knows: the live objects by id, and every id ever handed
/// out, in order.
#[derive(Debug, Default)]
struct State {
    live: HashMap<(u32, u32), (u32, u32)>,
    ids: Vec<(u32, u32)>,
}

impl State {
    fn pick(&self, position: usize) -> Option<(u32, u32)> {
        if self.ids.is_empty() {
            return None;
        }
        self.ids.get(position % self.ids.len()).copied()
    }
}

struct PoolModel;

impl ModelTest for PoolModel {
    type Op = PoolOp;
    type Sut = Pool<u32, CAPACITY>;
    type Model = State;

    fn generator(&self) -> BoxGen<PoolOp> {
        any_pool_op()
    }

    fn new_sut(&self) -> Self::Sut {
        Pool::new()
    }

    fn new_model(&self) -> Self::Model {
        State::default()
    }

    fn step(&self, sut: &mut Self::Sut, model: &mut State, op: &PoolOp) -> Result<(), String> {
        match *op {
            PoolOp::Allocate(payload) => match sut.allocate(payload) {
                Ok(id) => {
                    if model.live.len() >= CAPACITY {
                        return Err("allocated from a full pool".to_owned());
                    }
                    let key = (id.index(), id.generation());
                    if model.live.contains_key(&key) {
                        return Err(format!("id {key:?} handed out twice"));
                    }
                    model.live.insert(key, (payload, 1));
                    model.ids.push(key);
                }
                Err(PoolError::Exhausted) => {
                    if model.live.len() != CAPACITY {
                        return Err("exhausted although a slot was free".to_owned());
                    }
                }
                Err(other) => return Err(format!("unexpected error {other:?}")),
            },
            PoolOp::Retain(position) => {
                if let Some(key) = model.pick(position) {
                    let id = ObjectId::new(key.0, key.1);
                    let expected = model.live.get_mut(&key);
                    match (sut.retain(id), expected) {
                        (Ok(()), Some(entry)) => entry.1 += 1,
                        (Err(PoolError::StaleId), None) => {}
                        (result, expected) => {
                            return Err(format!("retain: {result:?} against {expected:?}"));
                        }
                    }
                }
            }
            PoolOp::Release(position) => {
                if let Some(key) = model.pick(position) {
                    let id = ObjectId::new(key.0, key.1);
                    let entry = model.live.get(&key).copied();
                    match (sut.release(id), entry) {
                        (Ok(false), Some((payload, refs))) if refs > 1 => {
                            model.live.insert(key, (payload, refs - 1));
                        }
                        (Ok(true), Some((_, 1))) => {
                            model.live.remove(&key);
                        }
                        (Err(PoolError::StaleId), None) => {}
                        (result, expected) => {
                            return Err(format!("release: {result:?} against {expected:?}"));
                        }
                    }
                }
            }
            PoolOp::Get(position) => {
                if let Some(key) = model.pick(position) {
                    let id = ObjectId::new(key.0, key.1);
                    let expected = model.live.get(&key).map(|entry| entry.0);
                    match (sut.get(id), expected) {
                        (Ok(value), Some(payload)) if *value == payload => {}
                        (Err(PoolError::StaleId), None) => {}
                        (result, expected) => {
                            return Err(format!("get: {result:?} against {expected:?}"));
                        }
                    }
                }
            }
            PoolOp::GetStale(index, generation) => {
                let id: ObjectId<u32> = ObjectId::new(index, generation);
                let expected = model.live.get(&(index, generation)).map(|entry| entry.0);
                match (sut.get(id), expected) {
                    (Ok(value), Some(payload)) if *value == payload => {}
                    (Err(PoolError::StaleId), None) => {}
                    (result, expected) => {
                        return Err(format!("stale get: {result:?} against {expected:?}"));
                    }
                }
            }
        }
        if u32::try_from(model.live.len()).unwrap_or(u32::MAX) != sut.live() {
            return Err(format!(
                "live count {} against {}",
                sut.live(),
                model.live.len()
            ));
        }
        Ok(())
    }
}

#[test]
fn model_allocate_retain_release_and_get_agree_with_a_map() {
    run_model_test("pool_model", &PoolModel, 64);
}
