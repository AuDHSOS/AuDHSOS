// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The three kinds of body this crate reads, and the many it does not.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a record at known offsets"
)]

use net_wire::{Ipv4Addr, Ipv6Addr, WireError, Writer};

use crate::error::DnsError;
use crate::name::Name;
use crate::record::{Class, Record, RecordData, RecordType};
use crate::tests::harness::{a, aaaa, cname, name};

/// The address of the example host, over both families.
const V4: Ipv4Addr = Ipv4Addr::new(93, 184, 216, 34);

/// The same host over IPv6.
const V6: Ipv6Addr = Ipv6Addr::from_octets([
    0x26, 0x06, 0x28, 0x00, 0x02, 0x20, 0, 0x01, 0x02, 0x48, 0x18, 0x93, 0x25, 0xc8, 0x19, 0x46,
]);

/// The bytes of `record`, written on its own.
fn written(record: &Record<'_>) -> Vec<u8> {
    let mut buffer = [0u8; 512];
    let mut writer = Writer::new(&mut buffer);
    record.write(&mut writer).expect("room");
    writer.finish().to_vec()
}

#[test]
fn an_address_record_reads_back_as_the_address() {
    for record in [a("example.com", V4), aaaa("example.com", V6)] {
        let bytes = written(&record);
        let (again, after) = Record::read(&bytes, 0).expect("a record");
        assert_eq!(again, record);
        assert_eq!(after, bytes.len());
    }
    assert_eq!(a("example.com", V4).data.wire_len(), 4);
    assert_eq!(aaaa("example.com", V6).data.wire_len(), 16);
}

#[test]
fn an_alias_reads_back_as_the_name_it_points_at() {
    let record = cname("www.example.com", "example.com");
    let bytes = written(&record);
    let (again, after) = Record::read(&bytes, 0).expect("a record");
    assert_eq!(again.record_type, RecordType::CNAME);
    assert_eq!(again.data, RecordData::Cname(name("example.com")));
    assert_eq!(after, bytes.len());
}

#[test]
fn an_alias_may_point_into_the_message_it_stands_in() {
    // `www.example.com.` at the front, and behind it a `CNAME` whose body
    // is nothing but a pointer to the `example.com.` inside it.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x03www\x07example\x03com\x00");
    let at = bytes.len();
    bytes.extend_from_slice(b"\xc0\x00");
    bytes.extend_from_slice(&RecordType::CNAME.get().to_be_bytes());
    bytes.extend_from_slice(&Class::IN.get().to_be_bytes());
    bytes.extend_from_slice(&300u32.to_be_bytes());
    bytes.extend_from_slice(&2u16.to_be_bytes());
    bytes.extend_from_slice(b"\xc0\x04");
    let (record, after) = Record::read(&bytes, at).expect("a record");
    assert_eq!(record.name, name("www.example.com"));
    assert_eq!(record.data, RecordData::Cname(name("example.com")));
    assert_eq!(after, bytes.len());
}

#[test]
fn a_type_this_crate_has_no_use_for_is_carried_whole() {
    let record = Record {
        name: name("example.com"),
        record_type: RecordType::TXT,
        class: Class::IN,
        ttl: 60,
        data: RecordData::Other(b"\x05hello"),
    };
    let bytes = written(&record);
    let (again, _) = Record::read(&bytes, 0).expect("a record");
    assert_eq!(again.data, RecordData::Other(b"\x05hello"));
    assert_eq!(again.record_type, RecordType::TXT);
    assert_eq!(RecordType::NS.get(), 2);
    assert_eq!(RecordType::SOA.get(), 6);
    assert_eq!(RecordType::new(99).get(), 99);
}

#[test]
fn a_class_this_crate_has_no_use_for_is_carried_whole_whatever_the_type() {
    let record = Record {
        name: name("example.com"),
        record_type: RecordType::A,
        class: Class::new(3),
        ttl: 60,
        data: RecordData::Other(&[1, 2, 3, 4]),
    };
    let bytes = written(&record);
    let (again, _) = Record::read(&bytes, 0).expect("a record");
    assert_eq!(again.class, Class::new(3));
    assert_eq!(again.data, RecordData::Other(&[1, 2, 3, 4]));
}

#[test]
fn a_body_that_is_not_the_length_its_type_has_is_refused() {
    for (record_type, body) in [
        (RecordType::A, vec![1, 2, 3]),
        (RecordType::A, vec![1, 2, 3, 4, 5]),
        (RecordType::AAAA, vec![0u8; 15]),
        (RecordType::AAAA, vec![0u8; 17]),
    ] {
        let record = Record {
            name: name("example.com"),
            record_type,
            class: Class::IN,
            ttl: 0,
            data: RecordData::Other(&body),
        };
        let bytes = written(&record);
        assert_eq!(
            Record::read(&bytes, 0).err(),
            Some(DnsError::Rdata {
                record_type,
                len: body.len()
            })
        );
    }
}

#[test]
fn an_alias_that_does_not_end_where_its_body_does_is_refused() {
    // The name inside is `example.com.`, thirteen bytes, and the length
    // field claims fourteen.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x03www\x00");
    bytes.extend_from_slice(&RecordType::CNAME.get().to_be_bytes());
    bytes.extend_from_slice(&Class::IN.get().to_be_bytes());
    bytes.extend_from_slice(&0u32.to_be_bytes());
    bytes.extend_from_slice(&14u16.to_be_bytes());
    bytes.extend_from_slice(b"\x07example\x03com\x00\x00");
    assert_eq!(
        Record::read(&bytes, 0).err(),
        Some(DnsError::Rdata {
            record_type: RecordType::CNAME,
            len: 14
        })
    );
}

#[test]
fn a_record_that_reaches_past_the_message_is_refused() {
    let bytes = written(&a("example.com", V4));
    for cut in 1..bytes.len() {
        assert!(
            matches!(
                Record::read(&bytes[..cut], 0),
                Err(DnsError::Wire(WireError::OutOfBounds { .. }) | DnsError::Rdata { .. })
            ),
            "a record read out of {cut} bytes"
        );
    }
}

#[test]
fn a_body_longer_than_the_length_field_is_refused_on_write() {
    let long = vec![0u8; 70_000];
    let record = Record {
        name: Name::ROOT,
        record_type: RecordType::TXT,
        class: Class::IN,
        ttl: 0,
        data: RecordData::Other(&long),
    };
    let mut buffer = vec![0u8; 80_000];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        record.write(&mut writer),
        Err(DnsError::Rdata {
            record_type: RecordType::TXT,
            len: 70_000
        })
    );
}

#[test]
fn a_record_that_does_not_fit_its_buffer_says_so() {
    let record = a("example.com", V4);
    for room in 0..written(&record).len() {
        let mut buffer = vec![0u8; room];
        let mut writer = Writer::new(&mut buffer);
        assert!(matches!(
            record.write(&mut writer),
            Err(DnsError::Wire(WireError::OutOfBounds { .. }))
        ));
    }
}
