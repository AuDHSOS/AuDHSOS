// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `RingBuffer` across its wrap boundary, and against a `VecDeque`.

use std::collections::VecDeque;

use test_support::generators::BoxGen;
use test_support::model::{ModelTest, run_model_test};

use crate::error::CollectionError;
use crate::ring::RingBuffer;
use crate::strategies::{RingOp, any_ring_op};

/// Capacity of the buffer the model test runs against.
const CAPACITY: usize = 5;

/// The values of a buffer, oldest first.
fn values<const N: usize>(ring: &RingBuffer<u32, N>) -> Vec<u32> {
    ring.iter().copied().collect()
}

#[test]
fn a_new_buffer_is_empty_and_reports_its_capacity() {
    let ring: RingBuffer<u32, 4> = RingBuffer::new();
    assert_eq!(ring.capacity(), 4);
    assert_eq!(RingBuffer::<u32, 4>::CAPACITY, 4);
    assert_eq!(ring.len(), 0);
    assert_eq!(ring.free(), 4);
    assert!(ring.is_empty());
    assert!(!ring.is_full());
    assert_eq!(ring.peek(), None);
    let default: RingBuffer<u32, 4> = RingBuffer::default();
    assert!(default.is_empty());
    assert!(!format!("{default:?}").is_empty());
}

#[test]
fn values_are_written_and_read_across_the_wrap_boundary() {
    let mut ring: RingBuffer<u32, 3> = RingBuffer::new();
    for value in [1, 2, 3] {
        ring.push(value).expect("room");
    }
    assert_eq!(ring.pop(), Some(1));
    assert_eq!(ring.pop(), Some(2));
    // The next two writes land where the first two stood.
    ring.push(4).expect("room");
    ring.push(5).expect("room");
    assert_eq!(values(&ring), vec![3, 4, 5]);
    assert_eq!(ring.pop(), Some(3));
    assert_eq!(ring.pop(), Some(4));
    assert_eq!(ring.pop(), Some(5));
    assert_eq!(ring.pop(), None);
}

#[test]
fn a_full_buffer_refuses_the_write_rather_than_overwriting() {
    let mut ring: RingBuffer<u32, 2> = RingBuffer::new();
    ring.push(1).expect("room");
    ring.push(2).expect("room");
    assert!(ring.is_full());
    assert_eq!(ring.push(3), Err(CollectionError::Full));
    assert_eq!(values(&ring), vec![1, 2]);
    assert_eq!(ring.len(), 2);
}

#[test]
fn the_free_space_is_the_number_of_writes_that_then_succeed() {
    let mut ring: RingBuffer<u32, 5> = RingBuffer::new();
    for value in [1, 2, 3] {
        ring.push(value).expect("room");
    }
    ring.pop().expect("a value");
    let free = ring.free();
    assert_eq!(free, 3);
    for value in 0..free {
        assert_eq!(
            ring.push(u32::try_from(value).unwrap_or(0)),
            Ok(()),
            "{value}"
        );
    }
    assert_eq!(ring.free(), 0);
    assert_eq!(ring.push(9), Err(CollectionError::Full));
}

#[test]
fn a_buffer_without_slots_is_full_from_the_start() {
    let mut ring: RingBuffer<u32, 0> = RingBuffer::new();
    assert!(ring.is_full());
    assert!(ring.is_empty());
    assert_eq!(ring.free(), 0);
    assert_eq!(ring.push(1), Err(CollectionError::Full));
    assert_eq!(ring.pop(), None);
    assert_eq!(ring.get(0), None);
}

#[test]
fn a_position_beyond_the_length_reads_nothing() {
    let mut ring: RingBuffer<u32, 4> = RingBuffer::new();
    ring.push(10).expect("room");
    ring.push(20).expect("room");
    assert_eq!(ring.get(0), Some(&10));
    assert_eq!(ring.get(1), Some(&20));
    assert_eq!(ring.get(2), None);
    assert_eq!(ring.peek(), Some(&10));
}

#[test]
fn clearing_leaves_the_buffer_empty_and_writable() {
    let mut ring: RingBuffer<u32, 3> = RingBuffer::new();
    for value in [1, 2, 3] {
        ring.push(value).expect("room");
    }
    ring.pop().expect("a value");
    ring.clear();
    assert!(ring.is_empty());
    assert_eq!(ring.free(), 3);
    ring.push(9).expect("room");
    assert_eq!(values(&ring), vec![9]);
}

#[test]
fn a_value_that_is_not_copy_moves_in_and_out() {
    let mut ring: RingBuffer<String, 2> = RingBuffer::new();
    ring.push("first".to_owned()).expect("room");
    ring.push("second".to_owned()).expect("room");
    assert_eq!(ring.pop().as_deref(), Some("first"));
    assert_eq!(ring.pop().as_deref(), Some("second"));
    assert!(ring.is_empty());
}

/// The buffer against a `VecDeque` bounded at the same capacity.
struct RingModel;

impl ModelTest for RingModel {
    type Op = RingOp;
    type Sut = RingBuffer<u32, CAPACITY>;
    type Model = VecDeque<u32>;

    fn generator(&self) -> BoxGen<RingOp> {
        any_ring_op()
    }

    fn new_sut(&self) -> RingBuffer<u32, CAPACITY> {
        RingBuffer::new()
    }

    fn new_model(&self) -> VecDeque<u32> {
        VecDeque::new()
    }

    fn step(
        &self,
        sut: &mut RingBuffer<u32, CAPACITY>,
        model: &mut VecDeque<u32>,
        op: &RingOp,
    ) -> Result<(), String> {
        match *op {
            RingOp::Push(value) => match sut.push(value) {
                Ok(()) => {
                    if model.len() >= CAPACITY {
                        return Err("pushed into a full buffer".to_owned());
                    }
                    model.push_back(value);
                }
                Err(CollectionError::Full) => {
                    if model.len() != CAPACITY {
                        return Err("full although a slot was free".to_owned());
                    }
                }
                Err(other) => return Err(format!("unexpected error {other:?}")),
            },
            RingOp::Pop => {
                let expected = model.pop_front();
                if sut.pop() != expected {
                    return Err(format!("pop against {expected:?}"));
                }
            }
            RingOp::Peek => {
                if sut.peek() != model.front() {
                    return Err(format!("peek against {:?}", model.front()));
                }
            }
            RingOp::Clear => {
                sut.clear();
                model.clear();
            }
        }
        if sut.len() != model.len() {
            return Err(format!("length {} against {}", sut.len(), model.len()));
        }
        if sut.free() != CAPACITY.wrapping_sub(model.len()) {
            return Err(format!("free {} against {}", sut.free(), model.len()));
        }
        let expected: Vec<u32> = model.iter().copied().collect();
        if values(sut) != expected {
            return Err(format!("{:?} against {expected:?}", values(sut)));
        }
        Ok(())
    }
}

#[test]
fn model_a_buffer_agrees_with_a_deque() {
    run_model_test("ring_model", &RingModel, 64);
}
