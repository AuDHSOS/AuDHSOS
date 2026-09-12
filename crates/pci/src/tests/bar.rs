// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::bar`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use crate::address::Address;
use crate::bar::{Bar, MAX_BARS, Space, Width, probe};
use crate::doubles::{RECORDED_LEN, RecordedConfigSpace};
use crate::error::PciError;
use crate::header::read_command;
use crate::space::ConfigSpace;
use crate::tests::build::{Builder, one};

/// The command register the builder leaves: memory and I/O decoding.
const DECODING: u16 = 0x0003;

/// The function every test of this file probes.
fn address(space: &RecordedConfigSpace) -> Address {
    space.address(0, 0).expect("the device and function exist")
}

#[test]
fn a_thirty_two_bit_memory_register_decodes_its_range() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.bar(0, 0x8004_0000, 0xFFFF_F000);
    let mut space = one(&builder);
    let at = address(&space);
    let bars = probe(&mut space, at).expect("the probe answers");
    assert_eq!(
        bars[0],
        Some(Bar {
            index: 0,
            space: Space::Memory {
                width: Width::Bits32,
                prefetchable: false,
            },
            base: 0x8004_0000,
            len: 0x1000,
        })
    );
}

#[test]
fn a_sixty_four_bit_register_consumes_the_next_index() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.bar(4, 0x0000_000C, 0xFFFF_C00C);
    builder.bar(5, 0x0000_00C0, 0xFFFF_FFFF);
    let mut space = one(&builder);
    let at = address(&space);
    let bars = probe(&mut space, at).unwrap();
    assert_eq!(
        bars[4],
        Some(Bar {
            index: 4,
            space: Space::Memory {
                width: Width::Bits64,
                prefetchable: true,
            },
            base: 0x0000_00C0_0000_0000,
            len: 0x4000,
        })
    );
    assert_eq!(bars[5], None, "the index the register above it consumed");
    assert_eq!(bars[4].unwrap().registers(), 2);
}

#[test]
fn an_io_register_decodes_its_ports() {
    let mut builder = Builder::new(0x8086, 0x2922);
    builder.bar(4, 0x0000_6041, 0xFFFF_FFE1);
    let mut space = one(&builder);
    let at = address(&space);
    let bars = probe(&mut space, at).unwrap();
    assert_eq!(
        bars[4],
        Some(Bar {
            index: 4,
            space: Space::Io,
            base: 0x6040,
            len: 0x20,
        })
    );
    assert!(!bars[4].unwrap().is_memory());
    assert_eq!(bars[4].unwrap().registers(), 1);
}

#[test]
fn a_register_that_reads_zero_decodes_nothing() {
    let builder = Builder::new(0x8086, 0x1234);
    let mut space = one(&builder);
    let at = address(&space);
    let bars = probe(&mut space, at).unwrap();
    assert_eq!(bars, [None; MAX_BARS]);
}

#[test]
fn a_memory_register_of_a_reserved_type_decodes_nothing() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.bar(0, 0x0010_0002, 0xFFFF_F002);
    let mut space = one(&builder);
    let at = address(&space);
    let bars = probe(&mut space, at).unwrap();
    assert_eq!(bars[0], None);
}

#[test]
fn probing_restores_the_command_register_and_every_register_it_wrote() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.bar(0, 0x8004_0000, 0xFFFF_F000);
    let mut space = one(&builder);
    let address = address(&space);
    probe(&mut space, address).unwrap();
    assert_eq!(read_command(&space, address), Ok(DECODING));
    assert_eq!(space.peek(0, 0, 0x10), Some(0x8004_0000));
}

#[test]
fn probing_restores_the_command_register_when_it_found_nothing() {
    let builder = Builder::new(0x8086, 0x1234);
    let mut space = one(&builder);
    let address = address(&space);
    probe(&mut space, address).unwrap();
    assert_eq!(read_command(&space, address), Ok(DECODING));
}

#[test]
fn the_decode_bits_are_clear_while_a_register_holds_all_ones() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.bar(0, 0x8004_0000, 0xFFFF_F000);
    let mut space = Watching {
        inner: one(&builder),
        command_while_probing: None,
    };
    let address = space.inner.address(0, 0).unwrap();
    probe(&mut space, address).unwrap();
    assert_eq!(
        space.command_while_probing,
        Some(0),
        "the function decoded while a register held all ones"
    );
}

#[test]
fn a_header_that_is_no_type_zero_one_is_refused_before_anything_is_written() {
    let mut builder = Builder::new(0x8086, 0x2918);
    builder.header_type(0x01);
    builder.word(0x18, 0x0002_0100);
    builder.word(0x20, 0x8110_8100);
    let mut space = one(&builder);
    let at = address(&space);
    assert_eq!(probe(&mut space, at), Err(PciError::HeaderType(0x01)));
    assert_eq!(
        space.peek(0, 0, 0x18),
        Some(0x0002_0100),
        "the bus numbers of a bridge are no base address register"
    );
    assert_eq!(space.peek(0, 0, 0x20), Some(0x8110_8100));
    assert_eq!(read_command(&space, at), Ok(DECODING));
}

#[test]
fn a_header_type_the_crate_does_not_lay_out_is_refused_as_well() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.header_type(0x82);
    let mut space = one(&builder);
    let at = address(&space);
    assert_eq!(probe(&mut space, at), Err(PciError::HeaderType(0x82)));
}

#[test]
fn probing_leaves_the_sticky_bits_of_the_status_register_standing() {
    let mut builder = Builder::new(0x8086, 0x1234);
    // Received master abort and detected parity error, the status bits 13
    // and 15, which a device sets and only a write of a one clears. The
    // status register is the upper half of the word the command register
    // is the lower half of.
    builder.word(0x04, 0xA000_0000 | u32::from(DECODING));
    builder.bar(0, 0x8004_0000, 0xFFFF_F000);
    let mut space = one(&builder);
    let at = address(&space);
    probe(&mut space, at).unwrap();
    assert_eq!(
        space.peek(0, 0, 0x04).unwrap() >> 16,
        0xA000,
        "the probe wrote the status half back and cleared what the device had set"
    );
    assert_eq!(read_command(&space, at), Ok(DECODING));
}

#[test]
fn a_sixty_four_bit_register_in_the_last_index_is_refused() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.bar(5, 0x0000_0004, 0xFFFF_C004);
    let mut space = one(&builder);
    let address = address(&space);
    assert_eq!(probe(&mut space, address), Err(PciError::BarTruncated(5)));
    assert_eq!(
        read_command(&space, address),
        Ok(DECODING),
        "the command register is restored on the way out"
    );
}

#[test]
fn a_space_that_answers_nothing_leaves_the_command_register_as_it_was() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.bar(0, 0x8004_0000, 0xFFFF_F000);
    let mut space = Deaf {
        inner: one(&builder),
        silent_from: 0x10,
    };
    let address = space.inner.address(0, 0).unwrap();
    assert_eq!(probe(&mut space, address), Err(PciError::Unreadable(0x10)));
    assert_eq!(read_command(&space.inner, address), Ok(DECODING));
}

/// A space that records the command register while a probe value stands in
/// a base address register.
struct Watching {
    inner: RecordedConfigSpace,
    command_while_probing: Option<u16>,
}

impl ConfigSpace for Watching {
    fn read_u32(&self, address: Address, offset: u16) -> Option<u32> {
        self.inner.read_u32(address, offset)
    }

    fn write_u32(&mut self, address: Address, offset: u16, value: u32) {
        self.inner.write_u32(address, offset, value);
        if (0x10..0x28).contains(&offset) && value == 0xFFFF_FFFF {
            let command = self.inner.peek(0, 0, 0x04).unwrap_or(0) & 0xFFFF;
            self.command_while_probing = Some(command as u16);
        }
    }
}

/// A space that answers nothing at and above one offset.
struct Deaf {
    inner: RecordedConfigSpace,
    silent_from: u16,
}

impl ConfigSpace for Deaf {
    fn read_u32(&self, address: Address, offset: u16) -> Option<u32> {
        if offset >= self.silent_from {
            return None;
        }
        self.inner.read_u32(address, offset)
    }

    fn write_u32(&mut self, address: Address, offset: u16, value: u32) {
        self.inner.write_u32(address, offset, value);
    }
}

#[test]
fn a_function_of_the_double_holds_the_bytes_it_was_given() {
    let builder = Builder::new(0x8086, 0x1234);
    let (bytes, masks) = builder.build();
    let mut space = RecordedConfigSpace::blank();
    assert!(space.install(1, 0, bytes, masks));
    assert_eq!(bytes.len(), RECORDED_LEN);
    assert_eq!(space.peek(1, 0, 0x00), Some(0x1234_8086));
    assert_eq!(space.peek(2, 0, 0x00), None, "no function was put there");
}
