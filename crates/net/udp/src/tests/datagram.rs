// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The datagram of RFC 768: its length field, and the checksum rule of
//! each family.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a datagram at known offsets"
)]

use net_wire::{IpAddr, Ipv4Addr, Ipv6Addr, Port, Protocol, WireError, Writer, transport};
use test_support::generators::{bytes, pair, range};
use test_support::property::check;

use crate::datagram::{ChecksumPolicy, Datagram, HEADER_LEN, MAX_PAYLOAD_LEN};
use crate::error::UdpError;

/// This host, over IPv4.
const V4_SOURCE: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));

/// The other host, over IPv4.
const V4_DESTINATION: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));

/// This host, over IPv6.
const V6_SOURCE: IpAddr = IpAddr::V6(Ipv6Addr::from_octets([
    0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01,
]));

/// The other host, over IPv6.
const V6_DESTINATION: IpAddr = IpAddr::V6(Ipv6Addr::from_octets([
    0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x02,
]));

/// Writes a datagram and returns it.
fn write(
    source: IpAddr,
    destination: IpAddr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
    checksum: ChecksumPolicy,
    buffer: &mut [u8],
) -> usize {
    let datagram = Datagram {
        source_port: Port::new(source_port),
        destination_port: Port::new(destination_port),
        payload,
    };
    let mut writer = Writer::new(buffer);
    datagram
        .write(&mut writer, source, destination, checksum)
        .expect("room for the datagram");
    writer.position()
}

#[test]
fn a_datagram_without_a_payload_is_a_header_and_reads_back() {
    let mut buffer = [0u8; 64];
    let len = write(
        V4_SOURCE,
        V4_DESTINATION,
        1024,
        53,
        &[],
        ChecksumPolicy::Computed,
        &mut buffer,
    );
    assert_eq!(len, HEADER_LEN);
    assert_eq!(u16::from_be_bytes([buffer[4], buffer[5]]), 8);
    let datagram = Datagram::parse(&buffer[..len], V4_SOURCE, V4_DESTINATION).expect("a datagram");
    assert_eq!(datagram.source_port, Port::new(1024));
    assert_eq!(datagram.destination_port, Port::new(53));
    assert!(datagram.payload.is_empty());
}

#[test]
fn the_longest_payload_the_length_field_can_express_is_carried() {
    let payload = vec![0x5Au8; MAX_PAYLOAD_LEN];
    let mut buffer = vec![0u8; MAX_PAYLOAD_LEN + HEADER_LEN];
    let len = write(
        V4_SOURCE,
        V4_DESTINATION,
        1024,
        7,
        &payload,
        ChecksumPolicy::Computed,
        &mut buffer,
    );
    assert_eq!(len, usize::from(u16::MAX));
    let datagram = Datagram::parse(&buffer[..len], V4_SOURCE, V4_DESTINATION).expect("a datagram");
    assert_eq!(datagram.payload.len(), MAX_PAYLOAD_LEN);
}

#[test]
fn one_byte_more_than_the_length_field_holds_is_refused() {
    let payload = vec![0u8; MAX_PAYLOAD_LEN + 1];
    let mut buffer = vec![0u8; MAX_PAYLOAD_LEN + HEADER_LEN + 8];
    let datagram = Datagram {
        source_port: Port::new(1024),
        destination_port: Port::new(7),
        payload: &payload,
    };
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        datagram.write(
            &mut writer,
            V4_SOURCE,
            V4_DESTINATION,
            ChecksumPolicy::Computed
        ),
        Err(UdpError::TooLarge(MAX_PAYLOAD_LEN + 1))
    );
    assert_eq!(writer.position(), 0);
}

#[test]
fn a_payload_beyond_the_length_field_entirely_is_refused() {
    let payload = vec![0u8; usize::from(u16::MAX) + 1];
    let mut buffer = vec![0u8; 16];
    let datagram = Datagram {
        source_port: Port::new(1024),
        destination_port: Port::new(7),
        payload: &payload,
    };
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        datagram.write(
            &mut writer,
            V4_SOURCE,
            V4_DESTINATION,
            ChecksumPolicy::Computed
        ),
        Err(UdpError::TooLarge(usize::from(u16::MAX) + 1))
    );
}

/// A datagram from port 1024 to port 53 carrying `four`, between
/// `192.168.1.10` and `192.168.1.20`, with the checksum summed by hand:
/// the pseudo-header words `c0a8 010a c0a8 0114 0011 000c`, the header
/// words `0400 0035 000c 0000`, and the payload words `666f 7572` add up
/// with the end-around carry to `63af`, whose complement is `9c50`.
const VECTOR: [u8; 12] = [
    0x04, 0x00, 0x00, 0x35, 0x00, 0x0C, 0x9C, 0x50, 0x66, 0x6F, 0x75, 0x72,
];

#[test]
fn the_fields_sit_where_rfc_768_puts_them() {
    let datagram = Datagram::parse(&VECTOR, V4_SOURCE, V4_DESTINATION).expect("a datagram");
    assert_eq!(datagram.source_port, Port::new(1024));
    assert_eq!(datagram.destination_port, Port::new(53));
    assert_eq!(datagram.payload, b"four");

    let mut buffer = [0u8; 12];
    let len = write(
        V4_SOURCE,
        V4_DESTINATION,
        1024,
        53,
        b"four",
        ChecksumPolicy::Computed,
        &mut buffer,
    );
    assert_eq!(&buffer[..len], &VECTOR);
}

#[test]
fn a_length_field_below_the_header_is_refused() {
    let mut buffer = [0u8; 32];
    let len = write(
        V4_SOURCE,
        V4_DESTINATION,
        1024,
        53,
        b"four",
        ChecksumPolicy::Computed,
        &mut buffer,
    );
    buffer[4] = 0;
    buffer[5] = 7;
    assert_eq!(
        Datagram::parse(&buffer[..len], V4_SOURCE, V4_DESTINATION),
        Err(UdpError::Length(7))
    );
}

#[test]
fn a_length_field_beyond_the_bytes_that_arrived_is_refused() {
    let mut buffer = [0u8; 32];
    let len = write(
        V4_SOURCE,
        V4_DESTINATION,
        1024,
        53,
        b"four",
        ChecksumPolicy::Computed,
        &mut buffer,
    );
    buffer[4] = 0;
    buffer[5] = 32;
    assert_eq!(
        Datagram::parse(&buffer[..len], V4_SOURCE, V4_DESTINATION),
        Err(UdpError::Length(32))
    );
}

#[test]
fn a_length_field_below_the_bytes_that_arrived_cuts_the_padding_away() {
    let mut buffer = [0u8; 64];
    let len = write(
        V4_SOURCE,
        V4_DESTINATION,
        1024,
        53,
        b"four",
        ChecksumPolicy::Computed,
        &mut buffer,
    );
    // What a link layer that pads to its own minimum leaves behind.
    let padded = &buffer[..len + 18];
    let datagram = Datagram::parse(padded, V4_SOURCE, V4_DESTINATION).expect("a datagram");
    assert_eq!(datagram.payload, b"four");
}

#[test]
fn fewer_bytes_than_a_header_are_not_a_datagram() {
    let short = [0u8; 7];
    assert_eq!(
        Datagram::parse(&short, V4_SOURCE, V4_DESTINATION),
        Err(UdpError::Wire(WireError::OutOfBounds {
            needed: HEADER_LEN,
            available: 7,
        }))
    );
}

#[test]
fn a_checksum_of_zero_is_accepted_over_ipv4_and_refused_over_ipv6() {
    let mut buffer = [0u8; 32];
    let len = write(
        V4_SOURCE,
        V4_DESTINATION,
        1024,
        53,
        b"four",
        ChecksumPolicy::Omitted,
        &mut buffer,
    );
    assert_eq!(u16::from_be_bytes([buffer[6], buffer[7]]), 0);
    assert!(Datagram::parse(&buffer[..len], V4_SOURCE, V4_DESTINATION).is_ok());

    let mut over_v6 = [0u8; 32];
    let len = write(
        V6_SOURCE,
        V6_DESTINATION,
        1024,
        53,
        b"four",
        ChecksumPolicy::Computed,
        &mut over_v6,
    );
    over_v6[6] = 0;
    over_v6[7] = 0;
    assert_eq!(
        Datagram::parse(&over_v6[..len], V6_SOURCE, V6_DESTINATION),
        Err(UdpError::ChecksumRequired)
    );
}

#[test]
fn a_checksum_may_not_be_omitted_over_ipv6() {
    let mut buffer = [0u8; 32];
    let datagram = Datagram {
        source_port: Port::new(1024),
        destination_port: Port::new(53),
        payload: b"four",
    };
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        datagram.write(
            &mut writer,
            V6_SOURCE,
            V6_DESTINATION,
            ChecksumPolicy::Omitted
        ),
        Err(UdpError::ChecksumRequired)
    );
    assert_eq!(writer.position(), 0);
}

#[test]
fn a_wrong_checksum_is_refused_in_either_family() {
    for (source, destination) in [(V4_SOURCE, V4_DESTINATION), (V6_SOURCE, V6_DESTINATION)] {
        let mut buffer = [0u8; 32];
        let len = write(
            source,
            destination,
            1024,
            53,
            b"four",
            ChecksumPolicy::Computed,
            &mut buffer,
        );
        let carried = u16::from_be_bytes([buffer[6], buffer[7]]);
        buffer[7] ^= 0x01;
        let spoiled = u16::from_be_bytes([buffer[6], buffer[7]]);
        assert_ne!(carried, spoiled);
        assert_eq!(
            Datagram::parse(&buffer[..len], source, destination),
            Err(UdpError::Checksum(spoiled))
        );
    }
}

#[test]
fn the_checksum_written_on_send_verifies_in_either_family() {
    for (source, destination) in [(V4_SOURCE, V4_DESTINATION), (V6_SOURCE, V6_DESTINATION)] {
        let mut buffer = [0u8; 64];
        let len = write(
            source,
            destination,
            49_152,
            53,
            b"a payload of some length",
            ChecksumPolicy::Computed,
            &mut buffer,
        );
        assert_ne!(u16::from_be_bytes([buffer[6], buffer[7]]), 0);
        assert_eq!(
            transport(source, destination, Protocol::UDP, &buffer[..len]),
            Ok(0)
        );
        assert!(Datagram::parse(&buffer[..len], source, destination).is_ok());
    }
}

#[test]
fn a_sum_of_zero_goes_on_the_wire_as_all_ones() {
    // The two payload bytes that make this datagram sum to zero; there is
    // one such pair for every set of the other fields.
    let mut found = None;
    for value in 0..=u16::MAX {
        let payload = value.to_be_bytes();
        let mut buffer = [0u8; 16];
        let len = write(
            V4_SOURCE,
            V4_DESTINATION,
            1024,
            53,
            &payload,
            ChecksumPolicy::Omitted,
            &mut buffer,
        );
        if transport(V4_SOURCE, V4_DESTINATION, Protocol::UDP, &buffer[..len]) == Ok(0) {
            found = Some(payload);
            break;
        }
    }
    let payload = found.expect("a payload whose sum is zero");
    let mut buffer = [0u8; 16];
    let len = write(
        V4_SOURCE,
        V4_DESTINATION,
        1024,
        53,
        &payload,
        ChecksumPolicy::Computed,
        &mut buffer,
    );
    assert_eq!(u16::from_be_bytes([buffer[6], buffer[7]]), 0xFFFF);
    let datagram = Datagram::parse(&buffer[..len], V4_SOURCE, V4_DESTINATION).expect("a datagram");
    assert_eq!(datagram.payload, payload);
}

#[test]
fn two_addresses_of_different_families_have_no_pseudo_header() {
    let mut buffer = [0u8; 32];
    let len = write(
        V4_SOURCE,
        V4_DESTINATION,
        1024,
        53,
        b"four",
        ChecksumPolicy::Computed,
        &mut buffer,
    );
    assert_eq!(
        Datagram::parse(&buffer[..len], V4_SOURCE, V6_DESTINATION),
        Err(UdpError::MixedFamilies)
    );
    let datagram = Datagram {
        source_port: Port::new(1024),
        destination_port: Port::new(53),
        payload: b"four",
    };
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        datagram.write(
            &mut writer,
            V6_SOURCE,
            V4_DESTINATION,
            ChecksumPolicy::Computed
        ),
        Err(UdpError::MixedFamilies)
    );
}

#[test]
fn a_buffer_without_room_takes_nothing_at_all() {
    let mut buffer = [0u8; 11];
    let datagram = Datagram {
        source_port: Port::new(1024),
        destination_port: Port::new(53),
        payload: b"four",
    };
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        datagram.write(
            &mut writer,
            V4_SOURCE,
            V4_DESTINATION,
            ChecksumPolicy::Computed
        ),
        Err(UdpError::Wire(WireError::OutOfBounds {
            needed: 12,
            available: 11,
        }))
    );
    assert_eq!(writer.position(), 0);
    assert_eq!(buffer, [0u8; 11]);
}

#[test]
fn no_byte_stream_makes_the_parser_panic() {
    check("udp parse", &bytes(0..=64), |stream| {
        for (source, destination) in [(V4_SOURCE, V4_DESTINATION), (V6_SOURCE, V6_DESTINATION)] {
            if let Ok(datagram) = Datagram::parse(stream, source, destination) {
                let claimed = HEADER_LEN + datagram.payload.len();
                if claimed > stream.len() {
                    return Err(format!("{claimed} bytes read out of {}", stream.len()));
                }
            }
        }
        Ok(())
    });
}

#[test]
fn every_datagram_that_was_written_reads_back_and_writes_the_same_bytes() {
    let generator = pair(bytes(0..=48), range(0u16..=u16::MAX));
    check("udp round trip", &generator, |(payload, ports)| {
        for (source, destination) in [(V4_SOURCE, V4_DESTINATION), (V6_SOURCE, V6_DESTINATION)] {
            let mut first = [0u8; 64];
            let len = write(
                source,
                destination,
                *ports,
                ports.wrapping_add(1),
                payload,
                ChecksumPolicy::Computed,
                &mut first,
            );
            let datagram = match Datagram::parse(&first[..len], source, destination) {
                Ok(datagram) => datagram,
                Err(error) => return Err(format!("{error} for {len} bytes")),
            };
            if datagram.payload != payload.as_slice() {
                return Err(format!("{:?} came back as {:?}", payload, datagram.payload));
            }
            let mut second = [0u8; 64];
            let again = write(
                source,
                destination,
                datagram.source_port.get(),
                datagram.destination_port.get(),
                datagram.payload,
                ChecksumPolicy::Computed,
                &mut second,
            );
            if first[..len] != second[..again] {
                return Err(format!(
                    "{:?} rewrote as {:?}",
                    &first[..len],
                    &second[..again]
                ));
            }
        }
        Ok(())
    });
}
