// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::handle`.

use crate::handle::Handle;
use test_support::generators::{pair, range};
use test_support::property::check;

#[test]
fn zero_is_never_a_handle() {
    assert_eq!(Handle::from_raw(0), None);
}

#[test]
fn generation_zero_is_rejected() {
    assert_eq!(Handle::new(0, 0), None);
    assert_eq!(Handle::new(u32::MAX, 0), None);
    assert_eq!(Handle::from_raw(1), None);
    assert_eq!(Handle::from_raw(u64::from(u32::MAX)), None);
}

#[test]
fn boundaries_round_trip() {
    for (index, generation) in [(0, 1), (u32::MAX, 1), (0, u32::MAX), (u32::MAX, u32::MAX)] {
        let handle = Handle::new(index, generation).unwrap();
        assert_eq!(handle.index(), index);
        assert_eq!(handle.generation(), generation);
        assert_eq!(Handle::from_raw(handle.raw()), Some(handle));
    }
}

#[test]
fn index_and_generation_never_overlap() {
    let low = Handle::new(u32::MAX, 1).unwrap();
    let high = Handle::new(0, u32::MAX).unwrap();
    assert_eq!(low.raw(), 0x1_FFFF_FFFF);
    assert_eq!(high.raw(), 0xFFFF_FFFF_0000_0000);
    assert_eq!(low.raw() & 0xFFFF_FFFF, u64::from(u32::MAX));
    assert_eq!(high.raw() & 0xFFFF_FFFF, 0);
    assert_eq!(low.raw() & high.raw(), 1 << 32);
}

#[test]
fn property_every_valid_pair_round_trips() {
    let generator = pair(range(0u32..=u32::MAX), range(1u32..=u32::MAX));
    check("handle_round_trip", &generator, |&(index, generation)| {
        let handle = Handle::new(index, generation).ok_or("rejected a valid pair")?;
        if handle.index() != index || handle.generation() != generation {
            return Err("fields changed".into());
        }
        if Handle::from_raw(handle.raw()) != Some(handle) {
            return Err("raw value does not round-trip".into());
        }
        Ok(())
    });
}
