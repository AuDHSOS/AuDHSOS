// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A used element and a receive buffer of arbitrary bytes: the driver
//! yields a frame or refuses, it never reads outside the buffer it was
//! given, and the buffer it took is back in the available ring either way.
//!
//! The first four bytes are the length the device reports and the rest are
//! what it wrote into the buffer.

use driver_virtio_net::doubles::{RamDevice, RamFrames};
use driver_virtio_net::queues::{Queues, Rings, Vectors};
use driver_virtio_net::{Frames, Net};
use virtio_queue::doubles::RamQueue;
use virtio_queue::{F_VERSION_1, Queue};

/// Descriptors of the queue, and buffers of the area.
const QUEUE_SIZE: u16 = 4;

/// Bytes of one buffer.
const STRIDE: u32 = 1526;

/// `VIRTIO_NET_F_MAC`, which the driver asks for.
const F_MAC: u64 = 1 << 5;

/// Where the buffers begin.
const BASE: u64 = 0x10_0000;

/// Where the rings of the two queues lie.
const QUEUES: Queues = Queues {
    receive: Rings {
        descriptor_table: 0x1000,
        available_ring: 0x2000,
        used_ring: 0x3000,
        size: QUEUE_SIZE,
    },
    transmit: Rings {
        descriptor_table: 0x4000,
        available_ring: 0x5000,
        used_ring: 0x6000,
        size: QUEUE_SIZE,
    },
};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let mut reported = [0u8; 4];
    let taken = bytes.get(..4).unwrap_or(&[]);
    reported
        .get_mut(..taken.len())
        .unwrap_or(&mut [])
        .copy_from_slice(taken);
    let reported = u32::from_le_bytes(reported);
    let written = bytes.get(4..).unwrap_or(&[]);

    let mut registers = RamDevice::new(F_VERSION_1 | F_MAC, QUEUE_SIZE, [2, 0, 0, 0, 0, 1]);
    let mut net = Net::<4>::new(1);
    net.reset(&mut registers);
    net.initialize(&mut registers, &QUEUES, &Vectors::none())
        .expect("a device that behaves comes up");
    let mut memory = RamQueue::new(QUEUE_SIZE);
    let mut queue = Queue::<1>::new(&mut memory, QUEUE_SIZE).expect("a queue");
    let mut area = RamFrames::new(QUEUE_SIZE, STRIDE, BASE);
    net.fill(&mut queue, &mut memory, &area)
        .expect("the buffers go in");

    let head = memory.available_entry(0);
    let _delivered = area.deliver(0, &[], written);
    memory.complete(u32::from(head), reported);

    let mut into = [0u8; 64];
    let capacity = into.len();
    match net.receive(&mut queue, &mut memory, &area, &mut into) {
        Ok(Some(frame)) => {
            assert!(frame.len() <= capacity, "the frame left the buffer");
            let header = 12usize;
            let len = usize::try_from(reported).unwrap_or(0) - header;
            assert_eq!(frame.len(), len, "the frame is not what was reported");
            let bytes = area.bytes(0).expect("the buffer is there");
            assert_eq!(
                frame,
                bytes.get(header..header + len).expect("inside the buffer"),
                "the frame is not the bytes of the buffer"
            );
        }
        Ok(None) => panic!("the used element was not seen"),
        // A refusal of the driver's own puts the buffer back; a refusal of
        // the queue leaves the queue to be rebuilt, and what is checked
        // then is that no more than the one element's descriptor left the
        // ring.
        Err(refused) => assert!(
            queue.free_count() <= 1,
            "{refused} lost more than the buffer it refused"
        ),
    }
});
