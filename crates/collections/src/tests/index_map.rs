// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `IndexMap` in key order, and against a `BTreeMap`.

use std::collections::BTreeMap;

use test_support::generators::BoxGen;
use test_support::model::{ModelTest, run_model_test};

use crate::error::CollectionError;
use crate::index_map::IndexMap;
use crate::strategies::{MapOp, any_map_op};

/// Capacity of the map the model test runs against.
const CAPACITY: usize = 6;

/// The entries of a map, in key order.
fn entries<const N: usize>(map: &IndexMap<u32, u32, N>) -> Vec<(u32, u32)> {
    map.iter().map(|(key, value)| (*key, *value)).collect()
}

#[test]
fn a_new_map_is_empty_and_reports_its_capacity() {
    let map: IndexMap<u32, u32, 4> = IndexMap::new();
    assert_eq!(map.capacity(), 4);
    assert_eq!(IndexMap::<u32, u32, 4>::CAPACITY, 4);
    assert_eq!(map.len(), 0);
    assert!(map.is_empty());
    assert!(!map.is_full());
    assert_eq!(map.get(&1), None);
    assert!(!map.contains_key(&1));
    let default: IndexMap<u32, u32, 4> = IndexMap::default();
    assert!(default.is_empty());
    assert!(!format!("{default:?}").is_empty());
}

#[test]
fn insert_look_up_and_remove_work_in_any_order() {
    let mut map: IndexMap<u32, u32, 4> = IndexMap::new();
    assert_eq!(map.insert(30, 300), Ok(None));
    assert_eq!(map.insert(10, 100), Ok(None));
    assert_eq!(map.insert(20, 200), Ok(None));
    assert_eq!(map.len(), 3);
    assert_eq!(map.get(&10), Some(&100));
    assert_eq!(map.get(&20), Some(&200));
    assert_eq!(map.get(&30), Some(&300));
    assert_eq!(map.get(&40), None);
    assert_eq!(map.remove(&20), Some(200));
    assert_eq!(map.remove(&20), None);
    assert_eq!(entries(&map), vec![(10, 100), (30, 300)]);
}

#[test]
fn iteration_is_sorted_by_key_however_the_keys_arrived() {
    let mut map: IndexMap<u32, u32, 6> = IndexMap::new();
    for key in [5, 1, 4, 2, 3] {
        map.insert(key, key.wrapping_mul(10)).expect("room");
    }
    assert_eq!(
        entries(&map),
        vec![(1, 10), (2, 20), (3, 30), (4, 40), (5, 50)]
    );
    let keys: Vec<u32> = map.keys().copied().collect();
    assert_eq!(keys, vec![1, 2, 3, 4, 5]);
    let values: Vec<u32> = map.values().copied().collect();
    assert_eq!(values, vec![10, 20, 30, 40, 50]);
}

#[test]
fn a_duplicate_key_replaces_the_value_and_does_not_grow_the_map() {
    let mut map: IndexMap<u32, u32, 2> = IndexMap::new();
    assert_eq!(map.insert(1, 100), Ok(None));
    assert_eq!(map.len(), 1);
    assert_eq!(map.insert(1, 200), Ok(Some(100)));
    assert_eq!(map.len(), 1);
    assert_eq!(map.get(&1), Some(&200));
    assert_eq!(entries(&map), vec![(1, 200)]);
}

#[test]
fn a_full_map_takes_a_known_key_and_refuses_a_new_one() {
    let mut map: IndexMap<u32, u32, 2> = IndexMap::new();
    map.insert(1, 10).expect("room");
    map.insert(2, 20).expect("room");
    assert!(map.is_full());
    assert_eq!(map.insert(3, 30), Err(CollectionError::Full));
    // A key that is already there needs no room.
    assert_eq!(map.insert(2, 22), Ok(Some(20)));
    assert_eq!(entries(&map), vec![(1, 10), (2, 22)]);
}

#[test]
fn a_map_without_slots_refuses_everything() {
    let mut map: IndexMap<u32, u32, 0> = IndexMap::new();
    assert!(map.is_full());
    assert!(map.is_empty());
    assert_eq!(map.insert(1, 1), Err(CollectionError::Full));
    assert_eq!(map.get(&1), None);
    assert_eq!(map.remove(&1), None);
}

#[test]
fn a_value_can_be_changed_in_place() {
    let mut map: IndexMap<u32, u32, 4> = IndexMap::new();
    map.insert(1, 10).expect("room");
    map.insert(2, 20).expect("room");
    *map.get_mut(&2).expect("a value") = 22;
    assert_eq!(map.get(&2), Some(&22));
    assert_eq!(map.get_mut(&9), None);
}

#[test]
fn clearing_leaves_the_map_empty_and_usable() {
    let mut map: IndexMap<u32, u32, 4> = IndexMap::new();
    for key in 0..4 {
        map.insert(key, key).expect("room");
    }
    map.clear();
    assert!(map.is_empty());
    assert_eq!(entries(&map), Vec::<(u32, u32)>::new());
    map.insert(9, 90).expect("room");
    assert_eq!(entries(&map), vec![(9, 90)]);
}

#[test]
fn a_key_and_a_value_that_are_not_copy_move_in_and_out() {
    let mut map: IndexMap<String, String, 2> = IndexMap::new();
    map.insert("b".to_owned(), "second".to_owned())
        .expect("room");
    map.insert("a".to_owned(), "first".to_owned())
        .expect("room");
    let keys: Vec<&str> = map.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["a", "b"]);
    assert_eq!(map.remove(&"a".to_owned()).as_deref(), Some("first"));
    assert_eq!(map.len(), 1);
}

/// The map against a `BTreeMap` bounded at the same capacity.
struct MapModel;

impl ModelTest for MapModel {
    type Op = MapOp;
    type Sut = IndexMap<u32, u32, CAPACITY>;
    type Model = BTreeMap<u32, u32>;

    fn generator(&self) -> BoxGen<MapOp> {
        any_map_op()
    }

    fn new_sut(&self) -> IndexMap<u32, u32, CAPACITY> {
        IndexMap::new()
    }

    fn new_model(&self) -> BTreeMap<u32, u32> {
        BTreeMap::new()
    }

    fn step(
        &self,
        sut: &mut IndexMap<u32, u32, CAPACITY>,
        model: &mut BTreeMap<u32, u32>,
        op: &MapOp,
    ) -> Result<(), String> {
        match *op {
            MapOp::Insert(key, value) => {
                let known = model.contains_key(&key);
                let expected = if known || model.len() < CAPACITY {
                    Ok(model.insert(key, value))
                } else {
                    Err(CollectionError::Full)
                };
                let result = sut.insert(key, value);
                if result != expected {
                    return Err(format!("insert {key}: {result:?} against {expected:?}"));
                }
            }
            MapOp::Get(key) => {
                if sut.get(&key) != model.get(&key) {
                    return Err(format!("get {key} against {:?}", model.get(&key)));
                }
                if sut.contains_key(&key) != model.contains_key(&key) {
                    return Err(format!("contains_key {key} disagrees"));
                }
            }
            MapOp::Remove(key) => {
                let expected = model.remove(&key);
                if sut.remove(&key) != expected {
                    return Err(format!("remove {key} against {expected:?}"));
                }
            }
            MapOp::Clear => {
                sut.clear();
                model.clear();
            }
        }
        if sut.len() != model.len() {
            return Err(format!("length {} against {}", sut.len(), model.len()));
        }
        let expected: Vec<(u32, u32)> = model.iter().map(|(key, value)| (*key, *value)).collect();
        if entries(sut) != expected {
            return Err(format!("{:?} against {expected:?}", entries(sut)));
        }
        Ok(())
    }
}

#[test]
fn model_a_map_agrees_with_a_btree_map() {
    run_model_test("index_map_model", &MapModel, 64);
}
