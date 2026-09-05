// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::frame_allocator`, covering the catalog items 6.6.3.

#![allow(clippy::arithmetic_side_effects)]

use std::collections::BTreeSet;

use kernel_hal_api::paging::FrameSource;
use kernel_types::{Alignment, PhysFrame, PhysFrameRange};
use test_support::generators::{range, vec};
use test_support::property::check;

use crate::frame_allocator::{
    BitmapFrameAllocator, FrameError, MAX_MANAGED_FRAMES, NoFrames, RESERVE_WORDS,
};

fn frame(number: u64) -> PhysFrame {
    PhysFrame::from_number(number).unwrap()
}

fn bitmap(start: u64, count: u64) -> BitmapFrameAllocator {
    BitmapFrameAllocator::new(PhysFrameRange::new(frame(start), count).unwrap()).unwrap()
}

#[test]
fn a_new_allocator_reports_its_range_and_free_count() {
    let allocator = bitmap(16, 8);
    assert_eq!(allocator.capacity(), 8);
    assert_eq!(allocator.free_count(), 8);
    assert_eq!(allocator.base(), frame(16));
    assert_eq!(allocator.range().count(), 8);
    assert!(allocator.manages(frame(16)));
    assert!(allocator.manages(frame(23)));
    assert!(!allocator.manages(frame(24)));
    assert!(!allocator.manages(frame(15)));
    assert_eq!(
        RESERVE_WORDS * 64,
        usize::try_from(MAX_MANAGED_FRAMES).unwrap()
    );
}

#[test]
fn a_range_larger_than_the_bitmap_is_rejected() {
    let too_large = PhysFrameRange::new(frame(0), MAX_MANAGED_FRAMES + 1).unwrap();
    assert_eq!(
        BitmapFrameAllocator::new(too_large).err(),
        Some(FrameError::InvalidCount)
    );
    let exact = PhysFrameRange::new(frame(0), MAX_MANAGED_FRAMES).unwrap();
    assert_eq!(
        BitmapFrameAllocator::new(exact).map(|a| a.capacity()),
        Ok(MAX_MANAGED_FRAMES)
    );
}

#[test]
fn allocating_until_exhaustion_reports_out_of_frames() {
    let mut allocator = bitmap(4, 3);
    let frames: Vec<PhysFrame> = (0..3).map(|_| allocator.allocate().unwrap()).collect();
    assert_eq!(frames, vec![frame(4), frame(5), frame(6)]);
    assert_eq!(allocator.free_count(), 0);
    assert_eq!(allocator.allocate().err(), Some(FrameError::OutOfFrames));
    assert_eq!(allocator.allocate().err(), Some(FrameError::OutOfFrames));
}

#[test]
fn an_allocator_without_frames_is_exhausted_from_the_start() {
    let mut allocator = bitmap(4, 0);
    assert_eq!(allocator.capacity(), 0);
    assert_eq!(allocator.allocate().err(), Some(FrameError::OutOfFrames));
    assert_eq!(
        allocator.allocate_contiguous(1, Alignment::PAGE).err(),
        Some(FrameError::InvalidCount)
    );
}

#[test]
fn a_freed_frame_is_handed_out_again() {
    let mut allocator = bitmap(4, 2);
    let first = allocator.allocate().unwrap();
    let second = allocator.allocate().unwrap();
    assert_eq!(allocator.free(first), Ok(()));
    assert_eq!(allocator.free_count(), 1);
    assert_eq!(allocator.allocate(), Ok(first));
    assert_eq!(allocator.allocate().err(), Some(FrameError::OutOfFrames));
    assert_eq!(allocator.free(second), Ok(()));
}

#[test]
fn a_double_free_and_a_free_of_an_unmanaged_frame_are_reported() {
    let mut allocator = bitmap(4, 2);
    let first = allocator.allocate().unwrap();
    assert_eq!(allocator.free(first), Ok(()));
    assert_eq!(allocator.free(first).err(), Some(FrameError::NotAllocated));
    assert_eq!(
        allocator.free(frame(5)).err(),
        Some(FrameError::NotAllocated),
        "a frame that was never allocated is detected"
    );
    assert_eq!(allocator.free(frame(3)).err(), Some(FrameError::NotManaged));
    assert_eq!(allocator.free(frame(6)).err(), Some(FrameError::NotManaged));
    assert_eq!(allocator.free_count(), 2);
}

#[test]
fn a_frame_can_be_claimed_by_address_and_only_once() {
    let mut allocator = bitmap(4, 4);
    assert_eq!(allocator.allocate_at(frame(6)), Ok(()));
    assert_eq!(allocator.free_count(), 3);
    assert_eq!(
        allocator.allocate_at(frame(6)).err(),
        Some(FrameError::AlreadyAllocated)
    );
    assert_eq!(
        allocator.allocate_at(frame(99)).err(),
        Some(FrameError::NotManaged)
    );
    assert_eq!(allocator.allocate(), Ok(frame(4)));
    assert_eq!(allocator.allocate(), Ok(frame(5)));
    assert_eq!(allocator.allocate(), Ok(frame(7)));
}

#[test]
fn contiguous_allocation_checks_its_count() {
    let mut allocator = bitmap(4, 8);
    assert_eq!(
        allocator.allocate_contiguous(0, Alignment::PAGE).err(),
        Some(FrameError::InvalidCount)
    );
    assert_eq!(
        allocator.allocate_contiguous(9, Alignment::PAGE).err(),
        Some(FrameError::InvalidCount)
    );
    let single = allocator.allocate_contiguous(1, Alignment::PAGE).unwrap();
    assert_eq!(single.start(), frame(4));
    assert_eq!(single.count(), 1);
    let run = allocator.allocate_contiguous(7, Alignment::PAGE).unwrap();
    assert_eq!(run.start(), frame(5));
    assert_eq!(allocator.free_count(), 0);
}

#[test]
fn contiguous_allocation_skips_runs_that_are_too_short() {
    let mut allocator = bitmap(0, 16);
    assert_eq!(allocator.allocate_at(frame(2)), Ok(()));
    assert_eq!(allocator.allocate_at(frame(9)), Ok(()));
    let run = allocator.allocate_contiguous(4, Alignment::PAGE).unwrap();
    assert_eq!(run.start(), frame(3), "the run before frame 2 is too short");
    assert_eq!(run.count(), 4);
    let next = allocator.allocate_contiguous(6, Alignment::PAGE).unwrap();
    assert_eq!(next.start(), frame(10));
}

#[test]
fn contiguous_allocation_that_would_leave_the_range_fails() {
    let mut allocator = bitmap(0, 8);
    assert_eq!(allocator.allocate_at(frame(0)), Ok(()));
    assert_eq!(allocator.allocate_at(frame(1)), Ok(()));
    assert_eq!(
        allocator.allocate_contiguous(7, Alignment::PAGE).err(),
        Some(FrameError::OutOfFrames),
        "seven frames no longer fit behind the two taken ones"
    );
    assert_eq!(allocator.free_count(), 6);
}

#[test]
fn an_alignment_larger_than_the_range_leaves_no_candidate() {
    let mut allocator = bitmap(1, 3);
    let two_mib = Alignment::new(2 << 20).unwrap();
    assert_eq!(
        allocator.allocate_contiguous(1, two_mib).err(),
        Some(FrameError::OutOfFrames)
    );
    assert_eq!(allocator.free_count(), 3);
    let mut aligned = bitmap(512, 1024);
    let range = aligned.allocate_contiguous(2, two_mib).unwrap();
    assert_eq!(range.start(), frame(512), "frame 512 starts at 2 MiB");
}

#[test]
fn allocation_straddles_word_boundaries_at_every_relevant_size() {
    for count in [63u64, 64, 65, 127, 128, 129] {
        let mut allocator = bitmap(0, count);
        let mut seen = BTreeSet::new();
        for _ in 0..count {
            assert!(seen.insert(allocator.allocate().unwrap().number()));
        }
        assert_eq!(allocator.allocate().err(), Some(FrameError::OutOfFrames));
        assert_eq!(seen.len(), usize::try_from(count).unwrap());
        assert_eq!(allocator.free_count(), 0);

        let mut fresh = bitmap(0, count);
        let run = fresh.allocate_contiguous(count, Alignment::PAGE).unwrap();
        assert_eq!(run.count(), count);
        assert_eq!(fresh.free_count(), 0);
        assert_eq!(fresh.free_contiguous(run), Ok(()));
        assert_eq!(fresh.free_count(), count);
    }
}

#[test]
fn a_run_across_a_word_boundary_is_found() {
    let mut allocator = bitmap(0, 128);
    for number in 0..60 {
        assert_eq!(allocator.allocate_at(frame(number)), Ok(()));
    }
    let run = allocator.allocate_contiguous(8, Alignment::PAGE).unwrap();
    assert_eq!(run.start(), frame(60));
    assert_eq!(run.count(), 8);
    assert!(run.contains(frame(63)) && run.contains(frame(64)));
}

#[test]
fn freeing_a_range_is_all_or_nothing() {
    let mut allocator = bitmap(4, 8);
    let run = allocator.allocate_contiguous(4, Alignment::PAGE).unwrap();
    assert_eq!(allocator.free(frame(5)), Ok(()));
    assert_eq!(
        allocator.free_contiguous(run).err(),
        Some(FrameError::NotAllocated)
    );
    assert_eq!(
        allocator.free_count(),
        5,
        "the rejected range free changed nothing"
    );
    let partly_outside = PhysFrameRange::new(frame(10), 4).unwrap();
    assert_eq!(
        allocator.free_contiguous(partly_outside).err(),
        Some(FrameError::NotAllocated),
        "the first frame of the range is managed but free"
    );
    let outside = PhysFrameRange::new(frame(20), 4).unwrap();
    assert_eq!(
        allocator.free_contiguous(outside).err(),
        Some(FrameError::NotManaged)
    );
    assert_eq!(allocator.allocate_at(frame(5)), Ok(()));
    assert_eq!(allocator.free_contiguous(run), Ok(()));
    assert_eq!(allocator.free_count(), 8);
    assert_eq!(
        allocator.free_contiguous(PhysFrameRange::EMPTY),
        Ok(()),
        "an empty range frees nothing and fails on nothing"
    );
    assert_eq!(allocator.free_count(), 8);
}

#[test]
fn the_frame_source_hands_out_and_takes_back_frames() {
    let mut allocator = bitmap(4, 2);
    let first = allocator.allocate_frame().unwrap();
    let second = allocator.allocate_frame().unwrap();
    assert_eq!(allocator.allocate_frame(), None);
    allocator.release_frame(first);
    allocator.release_frame(first);
    assert_eq!(allocator.free_count(), 1, "a second release is ignored");
    allocator.release_frame(second);
    assert_eq!(allocator.free_count(), 2);
    allocator.release_frame(frame(99));
    assert_eq!(allocator.free_count(), 2, "an unmanaged frame is ignored");
}

#[test]
fn errors_render_a_message_and_map_to_the_abi() {
    let cases = [
        (
            FrameError::OutOfFrames,
            audhsos_abi::Error::OutOfKernelMemory,
        ),
        (FrameError::NotManaged, audhsos_abi::Error::InvalidArgument),
        (
            FrameError::NotAllocated,
            audhsos_abi::Error::InvalidArgument,
        ),
        (
            FrameError::InvalidCount,
            audhsos_abi::Error::InvalidArgument,
        ),
        (
            FrameError::AlreadyAllocated,
            audhsos_abi::Error::AlreadyExists,
        ),
    ];
    for (error, expected) in cases {
        assert!(!format!("{error}").is_empty());
        assert_eq!(audhsos_abi::Error::from(error), expected);
    }
}

#[test]
fn property_allocated_frames_are_disjoint_and_inside_the_range() {
    let operations = vec(range(0u32..=2), 0..=64);
    check("frame_allocator_invariants", &operations, |ops| {
        let mut allocator = bitmap(8, 24);
        let mut held: Vec<PhysFrame> = Vec::new();
        for op in ops {
            match op {
                0 => {
                    if let Ok(frame) = allocator.allocate() {
                        if held.contains(&frame) {
                            return Err(format!("{frame:?} handed out twice"));
                        }
                        if !allocator.manages(frame) {
                            return Err(format!("{frame:?} is outside the range"));
                        }
                        held.push(frame);
                    }
                }
                1 => {
                    if let Some(frame) = held.pop() {
                        allocator.free(frame).map_err(|error| error.to_string())?;
                    }
                }
                _ => {
                    if let Ok(run) = allocator.allocate_contiguous(3, Alignment::PAGE) {
                        for frame in run {
                            if held.contains(&frame) {
                                return Err(format!("{frame:?} handed out twice"));
                            }
                            held.push(frame);
                        }
                    }
                }
            }
            let held_count = u64::try_from(held.len()).unwrap_or(0);
            if held_count + allocator.free_count() != allocator.capacity() {
                return Err(format!(
                    "held {held_count} plus free {} is not {}",
                    allocator.free_count(),
                    allocator.capacity()
                ));
            }
        }
        Ok(())
    });
}

#[test]
fn the_bitmap_helpers_ignore_numbers_outside_the_bitmap() {
    let mut allocator = bitmap(4, 8);
    let beyond = MAX_MANAGED_FRAMES + 64;
    assert!(!allocator.is_set(beyond));
    allocator.set(beyond, true);
    assert!(
        !allocator.is_set(beyond),
        "a number outside the bitmap changes nothing"
    );
    assert_eq!(allocator.free_count(), 8);
    assert!(!allocator.is_set(0));
    allocator.set(0, true);
    assert!(allocator.is_set(0));
    allocator.set(0, false);
    assert!(!allocator.is_set(0));
}

#[test]
fn the_empty_frame_source_hands_out_nothing_and_takes_anything_back() {
    let mut frames = NoFrames::new();
    assert!(frames.allocate_frame().is_none());
    frames.release_frame(PhysFrame::from_number(1).unwrap());
    assert!(frames.allocate_frame().is_none());
    assert_eq!(format!("{NoFrames:?}"), "NoFrames");
}
