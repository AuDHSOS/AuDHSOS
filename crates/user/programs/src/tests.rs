// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use std::vec::Vec;

use virtio_queue::error::{Area, QueueError};
use virtio_queue::memory::{QueueMemory, descriptor_table_bytes};

use crate::dma::{Dma, NO_STATUS, QUEUE_SIZE, REGION_BYTES};

/// Storage aligned for the descriptor table, one region long.
fn storage() -> Vec<u128> {
    std::vec![0u128; REGION_BYTES.div_ceil(16)]
}

/// The region over the `len` words at `storage`, at physical address
/// 0x1000.
fn region<'a>(storage: *mut u128, len: usize) -> Dma<'a> {
    // SAFETY: the storage outlives the value; the tests write device
    // bytes through `device_writes` only.
    unsafe { Dma::new(storage.cast(), len * 16, 0x1000) }.expect("DMA region")
}

/// Acts as the device: writes `value` at `at` through a raw pointer.
fn device_writes(storage: *mut u128, at: usize, value: u8) {
    // SAFETY: callers pass an offset inside the region.
    unsafe { storage.cast::<u8>().add(at).write_volatile(value) };
}

/// The offset of a physical address in the region.
fn offset(address: u64) -> usize {
    usize::try_from(address - 0x1000).expect("offset")
}

#[test]
fn device_written_bytes_are_read_after_a_raw_write() {
    let mut storage = storage();
    let pointer = storage.as_mut_ptr();
    let mut dma = region(pointer, storage.len());
    dma.clear();
    let index = offset(dma.rings().used_ring) + 2;
    device_writes(pointer, index, 7);
    assert_eq!(dma.read_used_u16(2), Ok(7));
    device_writes(pointer, index, 9);
    assert_eq!(
        dma.read_used_u16(2),
        Ok(9),
        "the second read sees the second write"
    );

    dma.clear_status(0);
    assert_eq!(dma.status(0), NO_STATUS);
    device_writes(pointer, offset(dma.chain(0, true).status), 0);
    assert_eq!(dma.status(0), 0);
    assert_eq!(
        dma.descriptor_table().len(),
        descriptor_table_bytes(QUEUE_SIZE)
    );
}

#[test]
fn a_used_ring_read_past_its_end_is_refused() {
    let mut storage = storage();
    let dma = region(storage.as_mut_ptr(), storage.len());
    let mut out = [0; 4];
    assert!(matches!(
        dma.read_used(usize::MAX - 1, &mut out),
        Err(QueueError::Region {
            area: Area::UsedRing,
            ..
        })
    ));
}

#[test]
fn a_region_that_is_short_null_or_unaligned_is_refused() {
    let mut storage = storage();
    let pointer = storage.as_mut_ptr().cast::<u8>();
    // SAFETY: each call refuses before it keeps the pointer.
    unsafe {
        assert!(Dma::new(pointer, REGION_BYTES - 1, 0x1000).is_none());
        assert!(Dma::new(core::ptr::null_mut(), REGION_BYTES, 0x1000).is_none());
        assert!(Dma::new(pointer, REGION_BYTES, 0x1008).is_none());
    }
}
