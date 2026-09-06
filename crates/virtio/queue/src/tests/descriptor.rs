// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One descriptor: its fields in memory, and the link to the next.

use crate::descriptor::{
    Buffer, DESC_F_INDIRECT, DESC_F_NEXT, DESC_F_WRITE, Descriptor, Direction,
};
use crate::error::{Area, QueueError};
use crate::memory::DESCRIPTOR_BYTES;

#[test]
fn a_descriptor_lies_in_sixteen_little_endian_bytes() {
    let mut table = [0u8; 2 * DESCRIPTOR_BYTES];
    let descriptor = Descriptor {
        address: 0x0123_4567_89AB_CDEF,
        length: 0x1122_3344,
        flags: DESC_F_NEXT | DESC_F_WRITE,
        next: 0x0007,
    };
    descriptor.write(&mut table, 1).expect("in range");

    assert_eq!(
        table.get(16..24),
        Some(&[0xEF, 0xCD, 0xAB, 0x89, 0x67, 0x45, 0x23, 0x01][..])
    );
    assert_eq!(table.get(24..28), Some(&[0x44, 0x33, 0x22, 0x11][..]));
    assert_eq!(table.get(28..30), Some(&[0x03, 0x00][..]));
    assert_eq!(table.get(30..32), Some(&[0x07, 0x00][..]));
    assert_eq!(Descriptor::read(&table, 1), Ok(descriptor));
    assert_eq!(Descriptor::offset(1), DESCRIPTOR_BYTES);
}

#[test]
fn a_descriptor_says_whether_the_chain_goes_on() {
    let ends = Descriptor {
        address: 0,
        length: 0,
        flags: DESC_F_WRITE,
        next: 4,
    };
    assert!(!ends.has_next());
    let goes_on = Descriptor {
        flags: DESC_F_NEXT,
        ..ends
    };
    assert!(goes_on.has_next());
}

#[test]
fn a_direction_is_the_flag_the_device_reads_it_by() {
    assert_eq!(Direction::DriverToDevice.flag(), 0);
    assert_eq!(Direction::DeviceToDriver.flag(), DESC_F_WRITE);
    assert_eq!(
        Buffer::to_device(0x40, 8),
        Buffer {
            address: 0x40,
            length: 8,
            direction: Direction::DriverToDevice,
        }
    );
    assert_eq!(
        Buffer::from_device(0x40, 8),
        Buffer {
            address: 0x40,
            length: 8,
            direction: Direction::DeviceToDriver,
        }
    );
}

#[test]
fn the_indirect_flag_exists_and_is_never_set_by_this_crate() {
    assert_eq!(DESC_F_INDIRECT, 4);
    let mut table = [0u8; DESCRIPTOR_BYTES];
    Descriptor {
        address: 1,
        length: 1,
        flags: Direction::DeviceToDriver.flag(),
        next: 0,
    }
    .write(&mut table, 0)
    .expect("in range");
    let written = Descriptor::read(&table, 0).expect("in range");
    assert_eq!(written.flags & DESC_F_INDIRECT, 0);
}

#[test]
fn linking_sets_the_next_flag_and_leaves_the_buffer_alone() {
    let mut table = [0u8; 2 * DESCRIPTOR_BYTES];
    let descriptor = Descriptor {
        address: 0xAABB,
        length: 7,
        flags: DESC_F_WRITE,
        next: 0,
    };
    descriptor.write(&mut table, 0).expect("in range");
    Descriptor::link_to(&mut table, 0, 1).expect("in range");

    assert_eq!(
        Descriptor::read(&table, 0),
        Ok(Descriptor {
            flags: DESC_F_WRITE | DESC_F_NEXT,
            next: 1,
            ..descriptor
        })
    );
    assert_eq!(
        Descriptor::link_of(&table, 0),
        Ok((DESC_F_WRITE | DESC_F_NEXT, 1))
    );
}

#[test]
fn a_descriptor_past_the_table_is_refused_in_every_direction() {
    const fn short(needed: usize) -> QueueError {
        QueueError::Region {
            area: Area::DescriptorTable,
            needed,
            given: DESCRIPTOR_BYTES,
        }
    }
    let mut table = [0u8; DESCRIPTOR_BYTES];
    let descriptor = Descriptor {
        address: 0,
        length: 0,
        flags: 0,
        next: 0,
    };
    assert_eq!(descriptor.write(&mut table, 1), Err(short(24)));
    assert_eq!(Descriptor::read(&table, 1), Err(short(24)));
    assert_eq!(Descriptor::link_of(&table, 1), Err(short(30)));
    assert_eq!(Descriptor::link_to(&mut table, 1, 0), Err(short(30)));
}

#[test]
fn a_short_table_refuses_each_field_of_a_descriptor_in_turn() {
    let mut table = [0u8; 4];
    let descriptor = Descriptor {
        address: 0,
        length: 0,
        flags: 0,
        next: 0,
    };
    assert!(matches!(
        descriptor.write(&mut table, 0),
        Err(QueueError::Region { needed: 8, .. })
    ));
    assert!(matches!(
        Descriptor::read(&table, 0),
        Err(QueueError::Region { needed: 8, .. })
    ));

    let mut table = [0u8; 13];
    assert!(matches!(
        descriptor.write(&mut table, 0),
        Err(QueueError::Region { needed: 14, .. })
    ));
    assert!(matches!(
        Descriptor::read(&table, 0),
        Err(QueueError::Region { needed: 14, .. })
    ));
}

#[test]
fn a_table_that_holds_the_length_but_not_the_flags_refuses_the_length_write() {
    let mut table = [0u8; 10];
    let descriptor = Descriptor {
        address: 0,
        length: 0,
        flags: 0,
        next: 0,
    };
    assert!(matches!(
        descriptor.write(&mut table, 0),
        Err(QueueError::Region { needed: 12, .. })
    ));
    assert!(matches!(
        Descriptor::read(&table, 0),
        Err(QueueError::Region { needed: 12, .. })
    ));
}
