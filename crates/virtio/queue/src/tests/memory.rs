// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The region arithmetic and the bounds-checked accessors.

use crate::error::{Area, QueueError};
use crate::memory::{
    DESCRIPTOR_BYTES, EVENT_BYTES, MAX_QUEUE_SIZE, RING_HEADER_BYTES, USED_ELEMENT_BYTES,
    available_ring_bytes, descriptor_table_bytes, read_u16, read_u32, used_ring_bytes, write_u16,
    write_u32, write_u64,
};

#[test]
fn a_region_is_as_wide_as_the_structure_in_it() {
    assert_eq!(descriptor_table_bytes(8), 8 * DESCRIPTOR_BYTES);
    assert_eq!(
        available_ring_bytes(8),
        RING_HEADER_BYTES + 8 * 2 + EVENT_BYTES
    );
    assert_eq!(
        used_ring_bytes(8),
        RING_HEADER_BYTES + 8 * USED_ELEMENT_BYTES + EVENT_BYTES
    );
}

#[test]
fn the_largest_queue_has_a_region_that_still_fits_a_usize() {
    assert_eq!(
        descriptor_table_bytes(MAX_QUEUE_SIZE),
        32768 * DESCRIPTOR_BYTES
    );
    assert_eq!(used_ring_bytes(MAX_QUEUE_SIZE), 4 + 32768 * 8 + 2);
}

#[test]
fn a_value_reads_back_as_it_was_written() {
    let mut region = [0u8; 16];
    write_u16(&mut region, Area::UsedRing, 0, 0xBEEF).expect("in range");
    write_u32(&mut region, Area::UsedRing, 2, 0xDEAD_BEEF).expect("in range");
    write_u64(&mut region, Area::UsedRing, 6, 0x0123_4567_89AB_CDEF).expect("in range");
    assert_eq!(read_u16(&region, Area::UsedRing, 0), Ok(0xBEEF));
    assert_eq!(read_u32(&region, Area::UsedRing, 2), Ok(0xDEAD_BEEF));
    assert_eq!(region.get(6), Some(&0xEF));
    assert_eq!(region.get(13), Some(&0x01));
}

#[test]
fn an_access_past_the_region_is_refused_and_says_what_it_needed() {
    let mut region = [0u8; 4];
    assert_eq!(
        read_u16(&region, Area::UsedRing, 3),
        Err(QueueError::Region {
            area: Area::UsedRing,
            needed: 5,
            given: 4,
        })
    );
    assert_eq!(
        read_u32(&region, Area::UsedRing, 1),
        Err(QueueError::Region {
            area: Area::UsedRing,
            needed: 5,
            given: 4,
        })
    );
    assert_eq!(
        write_u16(&mut region, Area::AvailableRing, 3, 0),
        Err(QueueError::Region {
            area: Area::AvailableRing,
            needed: 5,
            given: 4,
        })
    );
    assert_eq!(
        write_u32(&mut region, Area::AvailableRing, 1, 0),
        Err(QueueError::Region {
            area: Area::AvailableRing,
            needed: 5,
            given: 4,
        })
    );
    assert_eq!(
        write_u64(&mut region, Area::DescriptorTable, 0, 0),
        Err(QueueError::Region {
            area: Area::DescriptorTable,
            needed: 8,
            given: 4,
        })
    );
}

#[test]
fn an_offset_past_the_end_of_the_address_space_saturates_instead_of_wrapping() {
    let region = [0u8; 4];
    assert_eq!(
        read_u16(&region, Area::UsedRing, usize::MAX),
        Err(QueueError::Region {
            area: Area::UsedRing,
            needed: usize::MAX,
            given: 4,
        })
    );
}

#[test]
fn every_area_and_error_says_what_it_is() {
    assert_eq!(Area::DescriptorTable.to_string(), "descriptor table");
    assert_eq!(Area::AvailableRing.to_string(), "available ring");
    assert_eq!(Area::UsedRing.to_string(), "used ring");
    assert_eq!(
        QueueError::Region {
            area: Area::UsedRing,
            needed: 8,
            given: 4,
        }
        .to_string(),
        "the used ring needs 8 bytes and has 4"
    );
    assert_eq!(
        QueueError::Size(3).to_string(),
        "3 is not a usable queue size"
    );
    assert_eq!(
        QueueError::EmptyChain.to_string(),
        "a chain needs at least one buffer"
    );
    assert_eq!(
        QueueError::BufferOrder.to_string(),
        "a buffer the device writes stands before one it reads"
    );
    assert_eq!(
        QueueError::ChainTooLong {
            buffers: 9,
            size: 8
        }
        .to_string(),
        "9 buffers do not fit a queue of 8 descriptors"
    );
    assert_eq!(
        QueueError::QueueFull { free: 0, needed: 2 }.to_string(),
        "2 descriptors are needed and 0 are free"
    );
    assert_eq!(QueueError::NotLive.to_string(), "the device is not live");
    assert_eq!(
        QueueError::NotConfigured.to_string(),
        "the device has not settled its feature set"
    );
    assert_eq!(
        QueueError::UnknownDescriptor(9).to_string(),
        "the used ring names the unknown descriptor 9"
    );
    assert_eq!(
        QueueError::UsedIndex {
            last: 5,
            now: 4,
            in_flight: 1,
        }
        .to_string(),
        "the used index went from 5 to 4 with 1 chains outstanding"
    );
    assert_eq!(
        QueueError::UsedLength {
            reported: 66,
            writable: 65,
        }
        .to_string(),
        "the device reports 66 bytes written where 65 were writable"
    );
    assert_eq!(
        QueueError::CorruptChain(2).to_string(),
        "the chain at descriptor 2 does not end"
    );
}
