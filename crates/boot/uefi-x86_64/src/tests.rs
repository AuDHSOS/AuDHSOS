// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use audhsos_elf::image::Segment;
use audhsos_uefi::status::Status;

use crate::{exit_retry, loader_math};

fn segment(vaddr: u64, mem_size: u64) -> Segment {
    Segment {
        file_offset: 0,
        file_size: 0,
        vaddr,
        mem_size,
        align: 4096,
        write: false,
        execute: false,
    }
}

#[test]
fn memory_map_keeps_room_for_later_descriptors() {
    assert_eq!(loader_math::map_buffer_pages(16, 60 * 1024), Some(16));
    assert_eq!(loader_math::map_buffer_pages(16, 1360 * 48), Some(32));
    assert_eq!(loader_math::map_buffer_pages(256, 256 * 4096), None);
}

#[test]
fn page_table_pool_covers_large_and_unaligned_images() {
    let start = 0xFFFF_FFFF_8000_0000;
    let small = loader_math::pool_frames(1 << 30, start, 2 << 20);
    let unaligned = loader_math::pool_frames(1 << 30, start + 4096, 2 << 20);
    let large = loader_math::pool_frames(1 << 30, start, (40 << 20) + 4096);
    assert_eq!(unaligned, small + 1);
    assert_eq!(large, 1056);
}

#[test]
fn empty_segment_does_not_expand_image_span() {
    let start = 0xFFFF_FFFF_8000_0000;
    let segments = [segment(0xFFFF_8000_0000_0000, 0), segment(start, 4096)];
    assert_eq!(
        loader_math::image_span(segments.into_iter()),
        Ok((start, 4096))
    );
    assert_eq!(
        loader_math::image_span([segment(start, 0)].into_iter()),
        Err(loader_math::SpanError::NoSegments)
    );
}

#[test]
fn failed_exit_never_returns_a_console_reportable_error() {
    let mut reads = 0usize;
    let mut exits = 0usize;
    let result = exit_retry::leave(
        || {
            reads += 1;
            Ok((reads, reads))
        },
        |_| {
            exits += 1;
            Status::INVALID_PARAMETER
        },
    );
    assert_eq!(result, Err(exit_retry::ExitError::AfterExit));
    assert_eq!((reads, exits), (3, 3));

    let mut reads = 0usize;
    let result = exit_retry::leave(
        || {
            reads += 1;
            if reads == 1 {
                Ok(((), 1))
            } else {
                Err(Status::BUFFER_TOO_SMALL)
            }
        },
        |_| Status::INVALID_PARAMETER,
    );
    assert_eq!(result, Err(exit_retry::ExitError::AfterExit));
}
