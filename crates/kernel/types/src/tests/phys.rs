// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::phys`.

#![allow(clippy::arithmetic_side_effects)]

use crate::phys::{MAX_FRAME_NUMBER, MAX_PHYS_ADDR, PhysAddr, PhysFrame, PhysFrameRange};
use crate::strategies::{any_phys_addr, any_phys_frame, any_phys_frame_range};
use crate::{Alignment, Error};
use test_support::generators::{pair, range};
use test_support::property::check;

#[test]
fn zero_and_maximum_are_valid_one_above_is_not() {
    assert_eq!(PhysAddr::new(0), Ok(PhysAddr::ZERO));
    assert_eq!(PhysAddr::new(MAX_PHYS_ADDR), Ok(PhysAddr::MAX));
    assert_eq!(
        PhysAddr::new(MAX_PHYS_ADDR + 1),
        Err(Error::ExceedsPhysicalLimit(MAX_PHYS_ADDR + 1))
    );
    assert_eq!(
        PhysAddr::new(u64::MAX),
        Err(Error::ExceedsPhysicalLimit(u64::MAX))
    );
    assert_eq!(MAX_PHYS_ADDR, (1 << 52) - 1);
}

#[test]
fn alignment_boundaries() {
    let page = Alignment::PAGE;
    let two_mib = Alignment::new(2 << 20).unwrap();
    let one_gib = Alignment::new(1 << 30).unwrap();
    for (align, boundary) in [(page, 4096u64), (two_mib, 2 << 20), (one_gib, 1 << 30)] {
        let below = PhysAddr::new(boundary - 1).unwrap();
        let at = PhysAddr::new(boundary).unwrap();
        let above = PhysAddr::new(boundary + 1).unwrap();
        assert!(!below.is_aligned(align) && at.is_aligned(align) && !above.is_aligned(align));
        assert_eq!(below.align_down(align), PhysAddr::ZERO);
        assert_eq!(above.align_down(align), at);
        assert_eq!(below.align_up(align), Ok(at));
        assert_eq!(above.align_up(align).unwrap().as_u64(), 2 * boundary);
    }
}

#[test]
fn align_up_beyond_the_physical_limit_fails() {
    assert_eq!(
        PhysAddr::MAX.align_up(Alignment::PAGE),
        Err(Error::Overflow)
    );
    let last_frame_start = PhysAddr::new(MAX_PHYS_ADDR - 4095).unwrap();
    assert_eq!(
        last_frame_start.align_up(Alignment::PAGE),
        Ok(last_frame_start)
    );
    assert_eq!(PhysAddr::MAX.align_up(Alignment::BYTE), Ok(PhysAddr::MAX));
}

#[test]
fn checked_arithmetic_at_the_limits() {
    assert_eq!(PhysAddr::MAX.checked_add(1), None);
    assert_eq!(PhysAddr::MAX.checked_add(0), Some(PhysAddr::MAX));
    assert_eq!(
        PhysAddr::ZERO.checked_add(MAX_PHYS_ADDR),
        Some(PhysAddr::MAX)
    );
    assert_eq!(PhysAddr::ZERO.checked_add(u64::MAX), None);
    assert_eq!(PhysAddr::ZERO.checked_sub(1), None);
    assert_eq!(
        PhysAddr::MAX.checked_sub(MAX_PHYS_ADDR),
        Some(PhysAddr::ZERO)
    );
}

#[test]
fn frames_round_trip_and_reject_unaligned_starts() {
    let addr = PhysAddr::new(0x1234_5678).unwrap();
    assert_eq!(addr.offset_in_frame(), 0x678);
    assert_eq!(addr.frame().start().as_u64(), 0x1234_5000);
    assert_eq!(
        PhysFrame::from_start(addr),
        Err(Error::Unaligned {
            address: 0x1234_5678,
            alignment: 4096
        })
    );
    let frame = PhysFrame::from_start(PhysAddr::new(0x1234_5000).unwrap()).unwrap();
    assert_eq!(frame.number(), 0x1_2345);
    assert_eq!(PhysFrame::from_number(0x1_2345), Ok(frame));
    assert_eq!(frame.last_address().as_u64(), 0x1234_5FFF);
    assert_eq!(PhysFrame::containing(addr), frame);
}

#[test]
fn highest_frame_round_trips_and_one_above_is_rejected() {
    let highest = PhysFrame::from_number(MAX_FRAME_NUMBER).unwrap();
    assert_eq!(highest.start().as_u64(), MAX_PHYS_ADDR - 4095);
    assert_eq!(highest.last_address(), PhysAddr::MAX);
    assert_eq!(
        PhysFrame::from_number(MAX_FRAME_NUMBER + 1),
        Err(Error::Overflow)
    );
    assert_eq!(highest.checked_add(1), None);
    assert_eq!(highest.checked_add(0), Some(highest));
    assert_eq!(PhysFrame::from_number(0).unwrap().checked_sub(1), None);
    assert_eq!(
        highest.checked_sub(MAX_FRAME_NUMBER),
        PhysFrame::from_number(0).ok()
    );
}

#[test]
fn ranges_empty_single_and_at_the_top() {
    let first = PhysFrame::from_number(0).unwrap();
    let empty = PhysFrameRange::new(first, 0).unwrap();
    assert!(empty.is_empty());
    assert_eq!(empty.last(), None);
    assert_eq!(empty.iter().count(), 0);
    let single = PhysFrameRange::new(first, 1).unwrap();
    assert_eq!(single.last(), Some(first));
    assert_eq!(single.bytes(), 4096);
    assert!(single.contains(first));
    assert!(!single.contains(PhysFrame::from_number(1).unwrap()));
    let all = PhysFrameRange::new(first, MAX_FRAME_NUMBER + 1).unwrap();
    assert_eq!(all.last(), PhysFrame::from_number(MAX_FRAME_NUMBER).ok());
    assert_eq!(
        PhysFrameRange::new(first, MAX_FRAME_NUMBER + 2),
        Err(Error::Overflow)
    );
    let top = PhysFrame::from_number(MAX_FRAME_NUMBER).unwrap();
    assert_eq!(PhysFrameRange::new(top, 2), Err(Error::Overflow));
    assert!(PhysFrameRange::new(top, 1).is_ok());
}

#[test]
fn from_numbers_validates_order_and_limits() {
    let range = PhysFrameRange::from_numbers(2, 5).unwrap();
    assert_eq!(range.start().number(), 2);
    assert_eq!(range.count(), 3);
    assert_eq!(range.end_number(), 5);
    assert_eq!(PhysFrameRange::from_numbers(5, 2), Err(Error::Overflow));
    assert_eq!(
        PhysFrameRange::from_numbers(MAX_FRAME_NUMBER + 1, MAX_FRAME_NUMBER + 1),
        Err(Error::Overflow)
    );
    assert_eq!(PhysFrameRange::from_numbers(3, 3).unwrap().count(), 0);
}

#[test]
fn overlap_and_intersection_at_touching_and_nested_ranges() {
    let a = PhysFrameRange::from_numbers(10, 20).unwrap();
    let touching = PhysFrameRange::from_numbers(20, 30).unwrap();
    let nested = PhysFrameRange::from_numbers(12, 15).unwrap();
    let partial = PhysFrameRange::from_numbers(15, 25).unwrap();
    assert!(!a.overlaps(touching) && a.intersection(touching).is_none());
    assert_eq!(a.intersection(nested), Some(nested));
    assert_eq!(nested.intersection(a), Some(nested));
    assert_eq!(
        a.intersection(partial),
        PhysFrameRange::from_numbers(15, 20).ok()
    );
    let empty = PhysFrameRange::from_numbers(12, 12).unwrap();
    assert!(!a.overlaps(empty));
    assert!(!empty.overlaps(a));
    assert!(!a.contains(PhysFrame::from_number(9).unwrap()));
    assert!(!a.contains(PhysFrame::from_number(20).unwrap()));
    let above = PhysFrameRange::from_numbers(30, 40).unwrap();
    assert!(!above.overlaps(a) && !a.overlaps(above));
}

#[test]
fn iteration_yields_every_frame_in_order() {
    let range = PhysFrameRange::from_numbers(7, 10).unwrap();
    let numbers: Vec<u64> = range.into_iter().map(PhysFrame::number).collect();
    assert_eq!(numbers, vec![7, 8, 9]);
    assert_eq!(range.iter().size_hint(), (3, Some(3)));
}

#[test]
fn debug_and_display_are_hexadecimal() {
    let addr = PhysAddr::new(0xABC).unwrap();
    assert_eq!(format!("{addr:?}"), "PhysAddr(0xabc)");
    assert_eq!(format!("{addr}"), "0xabc");
    assert_eq!(format!("{:?}", addr.frame()), "PhysFrame(0x0)");
}

#[test]
fn property_frame_number_and_start_round_trip() {
    check("frame_round_trip", &any_phys_frame(), |&frame| {
        if PhysFrame::from_number(frame.number()) != Ok(frame) {
            return Err("number does not round-trip".into());
        }
        if PhysFrame::from_start(frame.start()) != Ok(frame) {
            return Err("start does not round-trip".into());
        }
        if frame.start().as_u64() != frame.number() * 4096 {
            return Err("start is not number times page size".into());
        }
        Ok(())
    });
}

#[test]
fn property_checked_add_agrees_with_wide_arithmetic() {
    let generator = pair(any_phys_addr(), range(0u64..=u64::MAX));
    check("phys_checked_add", &generator, |&(addr, bytes)| {
        let wide = u128::from(addr.as_u64()) + u128::from(bytes);
        let expected = if wide <= u128::from(MAX_PHYS_ADDR) {
            Some(wide)
        } else {
            None
        };
        if addr.checked_add(bytes).map(|a| u128::from(a.as_u64())) != expected {
            return Err(format!("checked_add({addr}, {bytes:#x}) disagrees"));
        }
        Ok(())
    });
}

#[test]
fn property_intersection_is_symmetric_and_inside_both() {
    let generator = pair(any_phys_frame_range(), any_phys_frame_range());
    check("frame_range_intersection", &generator, |&(a, b)| {
        let ab = a.intersection(b);
        if ab != b.intersection(a) {
            return Err("intersection is not symmetric".into());
        }
        if ab.is_some() != a.overlaps(b) {
            return Err("intersection and overlaps disagree".into());
        }
        if let Some(i) = ab {
            for frame in i.iter().take(4) {
                if !a.contains(frame) || !b.contains(frame) {
                    return Err("intersection frame outside an input".into());
                }
            }
        }
        Ok(())
    });
}
