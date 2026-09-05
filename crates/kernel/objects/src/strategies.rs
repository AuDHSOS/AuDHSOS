// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Generators of pool and quota operations for property tests. Available
//! behind the feature `test-strategies` and in this crate's own tests.

use test_support::generators::{BoxGen, Generator, pair, range};

/// One operation of a pool model test. The positions name entries of the
/// list of ids the test has handed out so far, modulo its length; an empty
/// list turns every position into a no-operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolOp {
    /// Store the given payload in a free slot.
    Allocate(u32),
    /// Add a reference to the id at this position.
    Retain(usize),
    /// Drop a reference to the id at this position.
    Release(usize),
    /// Read the id at this position.
    Get(usize),
    /// Read an id that names no live object.
    GetStale(u32, u32),
}

/// Any pool operation, shrinking toward allocating the payload zero.
#[must_use]
pub fn any_pool_op() -> BoxGen<PoolOp> {
    pair(range(0u32..=4), pair(range(0u32..=64), range(0usize..=16)))
        .map(|(choice, (payload, position))| match choice {
            1 => PoolOp::Retain(position),
            2 => PoolOp::Release(position),
            3 => PoolOp::Get(position),
            4 => PoolOp::GetStale(payload, payload),
            _ => PoolOp::Allocate(payload),
        })
        .boxed()
}

/// Any charge or refund of a quota, as an amount and a flag that selects
/// the direction.
#[must_use]
pub fn any_quota_op() -> BoxGen<(bool, u32)> {
    pair(range(0u32..=1), range(0u32..=8))
        .map(|(direction, amount)| (direction == 0, amount))
        .boxed()
}
