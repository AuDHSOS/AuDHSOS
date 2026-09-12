// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Generators of operation sequences for the model tests. Available behind
//! the feature `test-strategies` and in this crate's own tests.
//!
//! Every operation carries a position rather than a value the container
//! holds, because a generator that guessed keys and indices would spend
//! its cases on misses. The test maps a position onto what is actually in
//! the container.

use test_support::generators::{BoxGen, Generator, pair, range};

/// One operation on a vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecOp {
    /// Append a value.
    Push(u32),
    /// Remove the last value.
    Pop,
    /// Insert a value at a position.
    Insert(usize, u32),
    /// Remove the value at a position.
    Remove(usize),
    /// Read the value at a position.
    Get(usize),
    /// Remove everything.
    Clear,
}

/// One operation on a ring buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingOp {
    /// Append a value.
    Push(u32),
    /// Remove the oldest value.
    Pop,
    /// Read the oldest value.
    Peek,
    /// Remove everything.
    Clear,
}

/// One operation on a bit set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitOp {
    /// Set a bit.
    Set(usize),
    /// Clear a bit.
    Clear(usize),
    /// Read a bit.
    Test(usize),
    /// Ask for the lowest set bit.
    FirstSet,
    /// Ask for the lowest clear bit.
    FirstClear,
}

/// One operation on a list over caller-owned links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListOp {
    /// Put a node at the front.
    PushFront(u32),
    /// Put a node at the back.
    PushBack(u32),
    /// Take the first node.
    PopFront,
    /// Take the last node.
    PopBack,
    /// Take a named node out.
    Unlink(u32),
    /// Put a node behind another, or at the front when there is none.
    InsertAfter(u32, Option<u32>),
}

/// One operation on a map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapOp {
    /// Store a value under a key.
    Insert(u32, u32),
    /// Read the value under a key.
    Get(u32),
    /// Remove a key.
    Remove(u32),
    /// Remove everything.
    Clear,
}

/// An operation on a vector, weighted towards filling it.
#[must_use]
pub fn any_vec_op() -> BoxGen<VecOp> {
    pair(range(0u8..=9), pair(range(0usize..=9), range(0u32..=99)))
        .map(|(choice, (position, value))| match choice {
            0 | 1 => VecOp::Pop,
            2 | 3 => VecOp::Insert(position, value),
            4 => VecOp::Remove(position),
            5 => VecOp::Get(position),
            6 => VecOp::Clear,
            _ => VecOp::Push(value),
        })
        .boxed()
}

/// An operation on a ring buffer.
#[must_use]
pub fn any_ring_op() -> BoxGen<RingOp> {
    pair(range(0u8..=7), range(0u32..=99))
        .map(|(choice, value)| match choice {
            0..=2 => RingOp::Pop,
            3 => RingOp::Peek,
            4 => RingOp::Clear,
            _ => RingOp::Push(value),
        })
        .boxed()
}

/// An operation on a bit set of `bits` bits, with a tenth of the indices
/// beyond the end so that the error path is exercised.
#[must_use]
pub fn any_bit_op(bits: usize) -> BoxGen<BitOp> {
    let highest = bits.saturating_add(bits.wrapping_div(8)).saturating_add(2);
    pair(range(0u8..=6), range(0usize..=highest))
        .map(|(choice, index)| match choice {
            0 | 1 => BitOp::Clear(index),
            2 => BitOp::Test(index),
            3 => BitOp::FirstSet,
            4 => BitOp::FirstClear,
            _ => BitOp::Set(index),
        })
        .boxed()
}

/// An operation on a list over `nodes` links, with some node numbers
/// beyond the slice.
#[must_use]
pub fn any_list_op(nodes: u32) -> BoxGen<ListOp> {
    let highest = nodes.saturating_add(2);
    pair(
        range(0u8..=9),
        pair(
            range(0u32..=highest),
            range(0u32..=highest.saturating_add(1)),
        ),
    )
    .map(move |(choice, (node, after))| match choice {
        0 | 1 => ListOp::PushBack(node),
        2 => ListOp::PopFront,
        3 => ListOp::PopBack,
        4 | 5 => ListOp::Unlink(node),
        6 | 7 => ListOp::InsertAfter(node, (after <= highest).then_some(after)),
        _ => ListOp::PushFront(node),
    })
    .boxed()
}

/// An operation on a map over a small range of keys, so that duplicates
/// and hits are common.
#[must_use]
pub fn any_map_op() -> BoxGen<MapOp> {
    pair(range(0u8..=7), pair(range(0u32..=11), range(0u32..=99)))
        .map(|(choice, (key, value))| match choice {
            0 | 1 => MapOp::Get(key),
            2 | 3 => MapOp::Remove(key),
            4 => MapOp::Clear,
            _ => MapOp::Insert(key, value),
        })
        .boxed()
}
