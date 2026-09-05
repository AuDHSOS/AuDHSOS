// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The cursor: what it reads, what it writes, and where it stops.

use test_support::generators::bytes;
use test_support::property::check;

use crate::addr::{Ipv4Addr, MacAddr, Port};
use crate::cursor::{Reader, Writer};
use crate::error::WireError;

#[test]
fn integers_come_out_most_significant_byte_first() {
    let frame = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
    let mut reader = Reader::new(&frame);
    assert_eq!(reader.read_u8(), Ok(0x01));
    assert_eq!(reader.read_u16(), Ok(0x0203));
    assert_eq!(reader.read_u32(), Ok(0x0405_0607));
    assert_eq!(reader.position(), 7);
    assert!(reader.is_empty());
    assert_eq!(reader.remaining(), 0);
    assert_eq!(reader.rest(), &[]);
}

#[test]
fn a_read_past_the_end_leaves_the_position_where_it_was() {
    let frame = [0xAA, 0xBB, 0xCC];
    let mut reader = Reader::new(&frame);
    assert_eq!(reader.read_u16(), Ok(0xAABB));
    assert_eq!(
        reader.read_u16(),
        Err(WireError::OutOfBounds {
            needed: 2,
            available: 1
        })
    );
    assert_eq!(reader.position(), 2);
    assert_eq!(reader.remaining(), 1);
    assert_eq!(reader.read_u8(), Ok(0xCC));
    assert_eq!(
        reader.read_u8(),
        Err(WireError::OutOfBounds {
            needed: 1,
            available: 0
        })
    );
    assert_eq!(
        reader.read_u32(),
        Err(WireError::OutOfBounds {
            needed: 4,
            available: 0
        })
    );
}

#[test]
fn a_length_that_overflows_the_position_is_out_of_bounds_and_not_a_wrap() {
    let frame = [0x01, 0x02];
    let mut reader = Reader::new(&frame);
    assert_eq!(reader.read_u8(), Ok(0x01));
    assert_eq!(
        reader.read_bytes(usize::MAX),
        Err(WireError::OutOfBounds {
            needed: usize::MAX,
            available: 1
        })
    );
    assert_eq!(reader.position(), 1);
}

#[test]
fn the_rest_is_the_payload_of_the_layer_above() {
    let frame = [0x08, 0x00, 0x45, 0x00];
    let mut reader = Reader::new(&frame);
    assert_eq!(reader.read_u16(), Ok(0x0800));
    assert_eq!(reader.rest(), &[0x45, 0x00]);
    assert_eq!(reader.remaining(), 2);
    // `rest` does not consume.
    assert_eq!(reader.rest(), &[0x45, 0x00]);
    assert_eq!(reader.read_bytes(2), Ok(&frame[2..]));
}

#[test]
fn skipping_steps_over_bytes_and_stops_at_the_end() {
    let frame = [0u8; 8];
    let mut reader = Reader::new(&frame);
    assert_eq!(reader.skip(4), Ok(()));
    assert_eq!(reader.position(), 4);
    assert_eq!(
        reader.skip(5),
        Err(WireError::OutOfBounds {
            needed: 5,
            available: 4
        })
    );
    assert_eq!(reader.position(), 4);
    assert_eq!(reader.skip(4), Ok(()));
    assert!(reader.is_empty());
}

#[test]
fn an_empty_reader_reads_nothing_and_says_so() {
    let mut reader = Reader::new(&[]);
    assert!(reader.is_empty());
    assert_eq!(reader.position(), 0);
    assert_eq!(reader.rest(), &[]);
    assert_eq!(reader.read_bytes(0), Ok(&[][..]));
    assert_eq!(
        reader.read_u8(),
        Err(WireError::OutOfBounds {
            needed: 1,
            available: 0
        })
    );
}

#[test]
fn the_addresses_read_and_write_in_wire_order() {
    let frame = [
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 192, 168, 1, 1, 0x01, 0xBB,
    ];
    let mut reader = Reader::new(&frame);
    assert_eq!(reader.read_mac(), Ok(MacAddr::BROADCAST));
    assert_eq!(reader.read_ipv4(), Ok(Ipv4Addr::new(192, 168, 1, 1)));
    assert_eq!(reader.read_port(), Ok(Port::new(443)));
    assert!(reader.is_empty());

    let mut buffer = [0u8; 12];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(writer.write_mac(MacAddr::BROADCAST), Ok(()));
    assert_eq!(writer.write_ipv4(Ipv4Addr::new(192, 168, 1, 1)), Ok(()));
    assert_eq!(writer.write_port(Port::new(443)), Ok(()));
    assert_eq!(writer.finish(), &frame);
}

#[test]
fn a_write_that_does_not_fit_writes_nothing_at_all() {
    let mut buffer = [0u8; 3];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(writer.write_u8(0xAA), Ok(()));
    assert_eq!(writer.remaining(), 2);
    assert_eq!(
        writer.write_u32(0xDEAD_BEEF),
        Err(WireError::OutOfBounds {
            needed: 4,
            available: 2
        })
    );
    assert_eq!(writer.position(), 1);
    assert_eq!(writer.written(), &[0xAA]);
    assert_eq!(writer.write_u16(0xBBCC), Ok(()));
    assert_eq!(
        writer.write_u8(0xDD),
        Err(WireError::OutOfBounds {
            needed: 1,
            available: 0
        })
    );
    assert_eq!(writer.finish(), &[0xAA, 0xBB, 0xCC]);
    assert_eq!(buffer, [0xAA, 0xBB, 0xCC]);
}

#[test]
fn a_write_whose_length_overflows_the_position_is_out_of_bounds() {
    let mut buffer = [0u8; 4];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(writer.write_u8(0x11), Ok(()));
    assert_eq!(
        writer.write_zeros(usize::MAX),
        Err(WireError::OutOfBounds {
            needed: usize::MAX,
            available: 3
        })
    );
    assert_eq!(writer.position(), 1);
}

#[test]
fn zeros_leave_room_for_a_field_that_is_filled_in_later() {
    let mut buffer = [0xFFu8; 8];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(writer.write_u16(0x4500), Ok(()));
    assert_eq!(writer.write_zeros(2), Ok(()));
    assert_eq!(writer.write_u32(0x0A00_0001), Ok(()));
    assert_eq!(writer.written(), &[0x45, 0x00, 0x00, 0x00, 10, 0, 0, 1]);
    assert_eq!(writer.patch_u16(2, 0xB861), Ok(()));
    assert_eq!(writer.finish(), &[0x45, 0x00, 0xB8, 0x61, 10, 0, 0, 1]);
}

#[test]
fn a_patch_reaches_only_what_has_been_written() {
    let mut buffer = [0u8; 8];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(writer.write_u32(0), Ok(()));
    assert_eq!(writer.patch_u16(2, 0xABCD), Ok(()));
    assert_eq!(
        writer.patch_u16(3, 0xABCD),
        Err(WireError::OutOfBounds {
            needed: 2,
            available: 1
        })
    );
    assert_eq!(
        writer.patch_u16(4, 0xABCD),
        Err(WireError::OutOfBounds {
            needed: 2,
            available: 0
        })
    );
    assert_eq!(
        writer.patch_u16(usize::MAX, 0xABCD),
        Err(WireError::OutOfBounds {
            needed: 2,
            available: 0
        })
    );
    assert_eq!(writer.written(), &[0x00, 0x00, 0xAB, 0xCD]);
}

#[test]
fn an_empty_writer_has_written_nothing() {
    let mut buffer = [0u8; 0];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(writer.position(), 0);
    assert_eq!(writer.remaining(), 0);
    assert_eq!(writer.written(), &[]);
    assert_eq!(writer.write_bytes(&[]), Ok(()));
    assert_eq!(
        writer.write_u8(1),
        Err(WireError::OutOfBounds {
            needed: 1,
            available: 0
        })
    );
    assert_eq!(format!("{writer:?}"), "Writer { capacity: 0, position: 0 }");
    assert_eq!(writer.finish(), &[]);
}

#[test]
fn a_reader_can_be_cloned_and_read_twice() {
    let frame = [0x01, 0x02, 0x03, 0x04];
    let mut reader = Reader::new(&frame);
    assert_eq!(reader.read_u16(), Ok(0x0102));
    let mut branch = reader.clone();
    assert_eq!(branch.read_u16(), Ok(0x0304));
    assert_eq!(reader.read_u16(), Ok(0x0304));
    assert!(format!("{reader:?}").contains("position"));
}

#[test]
fn what_the_writer_wrote_is_what_the_reader_reads_back() {
    check("cursor round trip", &bytes(0..=64), |data| {
        let mut buffer = vec![0u8; data.len().saturating_add(7)];
        let written = {
            let mut writer = Writer::new(&mut buffer);
            writer.write_u8(0xA5).map_err(|error| error.to_string())?;
            writer
                .write_u16(0x1234)
                .map_err(|error| error.to_string())?;
            writer
                .write_u32(0x89AB_CDEF)
                .map_err(|error| error.to_string())?;
            writer
                .write_bytes(data)
                .map_err(|error| error.to_string())?;
            writer.position()
        };
        if written != data.len().saturating_add(7) {
            return Err(format!("{written} bytes written for {}", data.len()));
        }
        let mut reader = Reader::new(&buffer);
        let head = (
            reader.read_u8().map_err(|error| error.to_string())?,
            reader.read_u16().map_err(|error| error.to_string())?,
            reader.read_u32().map_err(|error| error.to_string())?,
        );
        if head != (0xA5, 0x1234, 0x89AB_CDEF) {
            return Err(format!("the header came back as {head:?}"));
        }
        if reader.rest() == data.as_slice() {
            Ok(())
        } else {
            Err(format!("{} bytes came back changed", data.len()))
        }
    });
}

#[test]
fn no_sequence_of_reads_takes_the_cursor_out_of_its_buffer() {
    check("reader stays inside", &bytes(0..=64), |data| {
        let mut reader = Reader::new(data);
        for step in 0..data.len().saturating_mul(2).saturating_add(4) {
            let before = reader.position();
            let taken = match step % 4 {
                0 => reader.read_u8().map(|_| 1),
                1 => reader.read_u16().map(|_| 2),
                2 => reader.read_u32().map(|_| 4),
                _ => reader.skip(3).map(|()| 3),
            };
            let expected = match taken {
                Ok(count) => before.saturating_add(count),
                Err(_) => before,
            };
            if reader.position() != expected {
                return Err(format!(
                    "step {step} left the cursor at {}",
                    reader.position()
                ));
            }
        }
        if reader.position() <= data.len() {
            Ok(())
        } else {
            Err(format!("the position left {} bytes", data.len()))
        }
    });
}
