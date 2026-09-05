// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ArrayVec` against its capacity, its order, and a `Vec`.

use test_support::generators::BoxGen;
use test_support::model::{ModelTest, run_model_test};

use crate::array_vec::ArrayVec;
use crate::error::CollectionError;
use crate::strategies::{VecOp, any_vec_op};

/// Capacity of the vector the model test runs against.
const CAPACITY: usize = 6;

/// The values of a vector, in order.
fn values<const N: usize>(vector: &ArrayVec<u32, N>) -> Vec<u32> {
    vector.iter().copied().collect()
}

#[test]
fn a_new_vector_is_empty_and_reports_its_capacity() {
    let vector: ArrayVec<u32, 4> = ArrayVec::new();
    assert_eq!(vector.capacity(), 4);
    assert_eq!(ArrayVec::<u32, 4>::CAPACITY, 4);
    assert_eq!(vector.len(), 0);
    assert!(vector.is_empty());
    assert!(!vector.is_full());
    assert_eq!(vector.first(), None);
    assert_eq!(vector.last(), None);
    let default: ArrayVec<u32, 4> = ArrayVec::default();
    assert!(default.is_empty());
    assert!(!format!("{default:?}").is_empty());
}

#[test]
fn pushing_to_capacity_succeeds_and_one_more_is_full() {
    let mut vector: ArrayVec<u32, 3> = ArrayVec::new();
    for value in 0..3 {
        assert_eq!(vector.push(value), Ok(()));
    }
    assert!(vector.is_full());
    assert_eq!(vector.push(3), Err(CollectionError::Full));
    assert_eq!(vector.len(), 3);
    assert_eq!(values(&vector), vec![0, 1, 2]);
}

#[test]
fn a_vector_without_slots_is_full_from_the_start() {
    let mut vector: ArrayVec<u32, 0> = ArrayVec::new();
    assert!(vector.is_full());
    assert!(vector.is_empty());
    assert_eq!(vector.push(1), Err(CollectionError::Full));
    assert_eq!(vector.pop(), None);
    assert_eq!(vector.insert(0, 1), Err(CollectionError::Full));
}

#[test]
fn popping_from_an_empty_vector_is_none() {
    let mut vector: ArrayVec<u32, 2> = ArrayVec::new();
    assert_eq!(vector.pop(), None);
    assert_eq!(vector.push(7), Ok(()));
    assert_eq!(vector.pop(), Some(7));
    assert_eq!(vector.pop(), None);
    assert!(vector.is_empty());
}

#[test]
fn inserting_at_either_end_and_in_the_middle_keeps_the_order() {
    let mut vector: ArrayVec<u32, 5> = ArrayVec::new();
    assert_eq!(vector.insert(0, 20), Ok(()));
    assert_eq!(vector.insert(0, 10), Ok(()));
    assert_eq!(vector.insert(2, 40), Ok(()));
    assert_eq!(vector.insert(2, 30), Ok(()));
    assert_eq!(values(&vector), vec![10, 20, 30, 40]);
    assert_eq!(vector.insert(5, 50), Err(CollectionError::Index(5)));
    assert_eq!(vector.insert(4, 50), Ok(()));
    assert_eq!(values(&vector), vec![10, 20, 30, 40, 50]);
    assert_eq!(vector.insert(0, 5), Err(CollectionError::Full));
}

#[test]
fn removing_at_either_end_and_in_the_middle_keeps_the_order() {
    let mut vector: ArrayVec<u32, 5> = ArrayVec::new();
    for value in [10, 20, 30, 40, 50] {
        vector.push(value).expect("room");
    }
    assert_eq!(vector.remove(2), Some(30));
    assert_eq!(values(&vector), vec![10, 20, 40, 50]);
    assert_eq!(vector.remove(0), Some(10));
    assert_eq!(values(&vector), vec![20, 40, 50]);
    assert_eq!(vector.remove(2), Some(50));
    assert_eq!(values(&vector), vec![20, 40]);
    assert_eq!(vector.remove(2), None);
}

#[test]
fn clearing_leaves_the_length_at_zero() {
    let mut vector: ArrayVec<u32, 4> = ArrayVec::new();
    for value in 0..4 {
        vector.push(value).expect("room");
    }
    vector.clear();
    assert_eq!(vector.len(), 0);
    assert!(vector.is_empty());
    assert_eq!(vector.get(0), None);
    assert_eq!(values(&vector), Vec::<u32>::new());
    assert_eq!(vector.push(9), Ok(()));
    assert_eq!(values(&vector), vec![9]);
}

#[test]
fn an_index_beyond_the_length_reads_nothing() {
    let mut vector: ArrayVec<u32, 4> = ArrayVec::new();
    vector.push(1).expect("room");
    vector.push(2).expect("room");
    assert_eq!(vector.get(0), Some(&1));
    assert_eq!(vector.get(1), Some(&2));
    assert_eq!(vector.get(2), None);
    assert_eq!(vector.get_mut(2), None);
    assert_eq!(vector.first(), Some(&1));
    assert_eq!(vector.last(), Some(&2));
    *vector.get_mut(0).expect("a value") = 5;
    assert_eq!(vector.get(0), Some(&5));
    for value in vector.iter_mut() {
        *value = value.wrapping_add(1);
    }
    assert_eq!(values(&vector), vec![6, 3]);
}

#[test]
fn a_vector_iterates_by_reference_and_reports_its_size() {
    let mut vector: ArrayVec<u32, 4> = ArrayVec::new();
    for value in [3, 1, 2] {
        vector.push(value).expect("room");
    }
    let mut seen = Vec::new();
    for value in &vector {
        seen.push(*value);
    }
    assert_eq!(seen, vec![3, 1, 2]);
    assert_eq!(vector.iter().size_hint(), (3, Some(3)));
    assert_eq!(vector.iter().count(), 3);
}

#[test]
fn a_value_that_is_not_copy_moves_in_and_out() {
    let mut vector: ArrayVec<String, 2> = ArrayVec::new();
    vector.push("first".to_owned()).expect("room");
    vector.push("second".to_owned()).expect("room");
    assert_eq!(vector.remove(0).as_deref(), Some("first"));
    assert_eq!(vector.pop().as_deref(), Some("second"));
    assert!(vector.is_empty());
}

/// The vector against a `Vec` of the same capacity.
struct VecModel;

impl ModelTest for VecModel {
    type Op = VecOp;
    type Sut = ArrayVec<u32, CAPACITY>;
    type Model = Vec<u32>;

    fn generator(&self) -> BoxGen<VecOp> {
        any_vec_op()
    }

    fn new_sut(&self) -> ArrayVec<u32, CAPACITY> {
        ArrayVec::new()
    }

    fn new_model(&self) -> Vec<u32> {
        Vec::new()
    }

    fn step(
        &self,
        sut: &mut ArrayVec<u32, CAPACITY>,
        model: &mut Vec<u32>,
        op: &VecOp,
    ) -> Result<(), String> {
        match *op {
            VecOp::Push(value) => match sut.push(value) {
                Ok(()) => {
                    if model.len() >= CAPACITY {
                        return Err("pushed into a full vector".to_owned());
                    }
                    model.push(value);
                }
                Err(CollectionError::Full) => {
                    if model.len() != CAPACITY {
                        return Err("full although a slot was free".to_owned());
                    }
                }
                Err(other) => return Err(format!("unexpected error {other:?}")),
            },
            VecOp::Pop => {
                let expected = model.pop();
                if sut.pop() != expected {
                    return Err(format!("pop against {expected:?}"));
                }
            }
            VecOp::Insert(position, value) => match sut.insert(position, value) {
                Ok(()) => {
                    if position > model.len() || model.len() >= CAPACITY {
                        return Err("inserted where it does not fit".to_owned());
                    }
                    model.insert(position, value);
                }
                Err(CollectionError::Index(at)) => {
                    if at != position || position <= model.len() {
                        return Err("refused a position that is inside".to_owned());
                    }
                }
                Err(CollectionError::Full) => {
                    if model.len() != CAPACITY {
                        return Err("full although a slot was free".to_owned());
                    }
                }
                Err(other) => return Err(format!("unexpected error {other:?}")),
            },
            VecOp::Remove(position) => {
                let expected = if position < model.len() {
                    Some(model.remove(position))
                } else {
                    None
                };
                if sut.remove(position) != expected {
                    return Err(format!("remove against {expected:?}"));
                }
            }
            VecOp::Get(position) => {
                if sut.get(position) != model.get(position) {
                    return Err(format!("get {position} against {:?}", model.get(position)));
                }
            }
            VecOp::Clear => {
                sut.clear();
                model.clear();
            }
        }
        if sut.len() != model.len() {
            return Err(format!("length {} against {}", sut.len(), model.len()));
        }
        if values(sut) != *model {
            return Err(format!("{:?} against {model:?}", values(sut)));
        }
        Ok(())
    }
}

#[test]
fn model_a_vector_agrees_with_a_vec() {
    run_model_test("array_vec_model", &VecModel, 64);
}
