// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The kernel reserve: the one contiguous range of physical frames the
//! kernel ever allocates from.
//!
//! Invariant: the reserve lies inside one usable range of the normalized
//! map and is removed from the map that comes back with it.

use audhsos_abi::layout::{PAGE_SHIFT, PAGE_SIZE};
use kernel_types::{Alignment, PhysFrameRange};

use crate::memory_map::{MapError, NormalizedMap};

/// Smallest default reserve.
pub const MIN_RESERVE_BYTES: u64 = 4 * 1024 * 1024;

/// Largest default reserve, and the size the bitmap allocator is built for.
pub const MAX_RESERVE_BYTES: u64 = 64 * 1024 * 1024;

/// Fraction of the usable memory the default reserve asks for.
pub const RESERVE_DIVISOR: u64 = 16;

/// The default reserve size for `total_bytes` of usable memory: a sixteenth
/// of it, clamped to [`MIN_RESERVE_BYTES`] and [`MAX_RESERVE_BYTES`] and
/// rounded up to a frame.
#[must_use]
pub const fn default_reserve_bytes(total_bytes: u64) -> u64 {
    let share = total_bytes.wrapping_div(RESERVE_DIVISOR);
    let clamped = if share < MIN_RESERVE_BYTES {
        MIN_RESERVE_BYTES
    } else if share > MAX_RESERVE_BYTES {
        MAX_RESERVE_BYTES
    } else {
        share
    };
    match Alignment::PAGE.align_up(clamped) {
        Some(bytes) => bytes,
        None => MAX_RESERVE_BYTES,
    }
}

/// Carves the kernel reserve out of the first usable range large enough for
/// it and returns the reserve together with the map without those frames.
///
/// `override_bytes` of zero selects [`default_reserve_bytes`]; any other
/// value must be frame-aligned and below the usable memory.
///
/// # Errors
///
/// [`MapError::Address`] if `override_bytes` is unaligned or not below the
/// usable memory; [`MapError::ReserveDoesNotFit`] if no usable range holds
/// the reserve.
pub fn select_reserve(
    map: &NormalizedMap,
    override_bytes: u64,
) -> Result<(PhysFrameRange, NormalizedMap), MapError> {
    let total = map.total_bytes();
    let bytes = if override_bytes == 0 {
        default_reserve_bytes(total)
    } else {
        if !override_bytes.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Address(kernel_types::Error::Unaligned {
                address: override_bytes,
                alignment: PAGE_SIZE,
            }));
        }
        if override_bytes >= total {
            return Err(MapError::Address(kernel_types::Error::Overflow));
        }
        override_bytes
    };
    let frames = bytes >> PAGE_SHIFT;
    if frames == 0 {
        return Err(MapError::ReserveDoesNotFit);
    }
    let range = map
        .iter()
        .find(|candidate| candidate.count() >= frames)
        .ok_or(MapError::ReserveDoesNotFit)?;
    let reserve = PhysFrameRange::new(range.start(), frames).map_err(MapError::Address)?;
    let remaining = map.without(reserve)?;
    Ok((reserve, remaining))
}
