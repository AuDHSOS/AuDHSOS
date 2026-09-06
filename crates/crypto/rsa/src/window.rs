// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Sub-slices without an index.
//!
//! Every buffer in this crate is a fixed array of the widest key the
//! arithmetic allows, and every operation uses the front of it. The
//! workspace denies slicing by range, and `get` answers with an `Option`
//! whose empty half no test can reach. These three take the shorter of the
//! two lengths instead, so they are total and there is no arm to cover.

/// The first `count` bytes, or every byte there is when there are fewer.
pub(crate) fn head(bytes: &[u8], count: usize) -> &[u8] {
    bytes.split_at(count.min(bytes.len())).0
}

/// The first `count` bytes, or every byte there is when there are fewer.
pub(crate) fn head_mut(bytes: &mut [u8], count: usize) -> &mut [u8] {
    let count = count.min(bytes.len());
    bytes.split_at_mut(count).0
}

/// Everything from `from` on, or nothing when the slice is shorter.
pub(crate) fn tail_mut(bytes: &mut [u8], from: usize) -> &mut [u8] {
    let from = from.min(bytes.len());
    bytes.split_at_mut(from).1
}
