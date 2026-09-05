// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The internet checksum against RFC 1071 and against sums worked out by
//! hand.

#![allow(
    clippy::indexing_slicing,
    reason = "a test writes a checksum into a field at a known offset"
)]

use test_support::generators::bytes;
use test_support::property::check;

use crate::addr::Ipv4Addr;
use crate::checksum::{Checksum, checksum, is_valid, transport_v4};
use crate::error::WireError;
use crate::protocol::Protocol;

/// The eight bytes of the worked example in RFC 1071, section 3.
const RFC_1071_EXAMPLE: [u8; 8] = [0x00, 0x01, 0xF2, 0x03, 0xF4, 0xF5, 0xF6, 0xF7];

#[test]
fn the_rfc_1071_example_sums_to_the_value_the_memo_prints() {
    // RFC 1071, section 3: every column of that table ends in `ddf2`, the
    // one's complement sum; the checksum is its complement.
    let mut sum = Checksum::new();
    sum.add_bytes(&RFC_1071_EXAMPLE);
    assert_eq!(sum.sum(), 0xDDF2);
    assert_eq!(sum.finish(), 0x220D);
    assert_eq!(checksum(&RFC_1071_EXAMPLE), 0x220D);
}

#[test]
fn the_rfc_1071_example_split_across_an_odd_boundary_sums_the_same() {
    // RFC 1071, section 3, last table: the same eight bytes as a group of
    // three and a group of five, which the memo carries through `f201`,
    // `f0eb`, and the byte swap to `ddf2`. Here the pending byte does that
    // work, and the answer is the same sum.
    let (head, tail) = RFC_1071_EXAMPLE.split_at(3);
    let mut sum = Checksum::new();
    sum.add_bytes(head);
    sum.add_bytes(tail);
    assert_eq!(sum.sum(), 0xDDF2);
    assert_eq!(sum.finish(), 0x220D);

    // And one byte at a time, which is the same split taken to its end.
    let mut byte_by_byte = Checksum::new();
    for byte in RFC_1071_EXAMPLE {
        byte_by_byte.add_bytes(&[byte]);
    }
    assert_eq!(byte_by_byte.finish(), 0x220D);
}

#[test]
fn an_odd_number_of_bytes_pairs_the_last_one_with_a_zero() {
    // RFC 1071, section 1, case [2]: the last byte becomes `[Z,0]`.
    // `ddf2 +' ff00 = dcf3`, complemented `230c`.
    let odd = [0x00, 0x01, 0xF2, 0x03, 0xF4, 0xF5, 0xF6, 0xF7, 0xFF];
    assert_eq!(checksum(&odd), 0x230C);

    // The zero is the low half of that last word, so writing it out
    // changes nothing.
    let with_zero = [0x00, 0x01, 0xF2, 0x03, 0xF4, 0xF5, 0xF6, 0xF7, 0xFF, 0x00];
    assert_eq!(checksum(&with_zero), 0x230C);
}

#[test]
fn an_empty_slice_sums_to_zero_and_checksums_to_all_ones() {
    let mut sum = Checksum::new();
    sum.add_bytes(&[]);
    assert_eq!(sum.sum(), 0);
    assert_eq!(sum.finish(), 0xFFFF);
    assert_eq!(checksum(&[]), 0xFFFF);
    assert_eq!(Checksum::new(), Checksum::default());
}

#[test]
fn a_sum_that_carries_out_of_every_word_still_ends_inside_sixteen_bits() {
    // `ffff +' ffff = ffff`, four times over.
    assert_eq!(checksum(&[0xFF; 8]), 0x0000);
    // `ffff +' 0001 = 0001`: the carry comes back into the low bit.
    assert_eq!(checksum(&[0xFF, 0xFF, 0x00, 0x01]), 0xFFFE);
    // Sixty-four kilobytes of ones, which is more words than an IPv4
    // datagram has and every one of them carrying.
    let all_ones = vec![0xFFu8; 65536];
    assert_eq!(checksum(&all_ones), 0x0000);
}

#[test]
fn an_empty_call_after_an_odd_one_keeps_the_byte_pending() {
    let mut sum = Checksum::new();
    sum.add_bytes(&[0x12]);
    sum.add_bytes(&[]);
    sum.add_bytes(&[0x34]);
    assert_eq!(sum.finish(), checksum(&[0x12, 0x34]));
}

#[test]
fn a_block_that_carries_its_own_checksum_sums_to_zero() {
    // An IPv4 header this project writes: version 4, header length 5, total
    // length 40, identification 0x1c46, don't fragment, TTL 64, TCP, from
    // 10.0.0.2 to 10.0.0.1, with the checksum field zero.
    let mut header = [
        0x45, 0x00, 0x00, 0x28, 0x1C, 0x46, 0x40, 0x00, 0x40, 0x06, 0x00, 0x00, 10, 0, 0, 2, 10, 0,
        0, 1,
    ];
    let computed = checksum(&header);
    assert_eq!(computed, 0x0A88);
    assert!(!is_valid(&header));

    header[10..12].copy_from_slice(&computed.to_be_bytes());
    assert!(is_valid(&header));
    assert_eq!(checksum(&header), 0x0000);
}

#[test]
fn the_udp_pseudo_header_reaches_the_value_worked_out_by_hand() {
    // From 10.0.0.1:53 to 10.0.0.2:49152, ten bytes of datagram carrying
    // `hi`, checksum field zero. The words are
    //   0a00 0001 0a00 0002 0011 000a  (the pseudo-header)
    //   0035 c000 000a 0000 6869        (the datagram)
    // whose one's complement sum is 3cc7 and whose complement is c338.
    let source = Ipv4Addr::new(10, 0, 0, 1);
    let destination = Ipv4Addr::new(10, 0, 0, 2);
    let mut datagram = [0x00, 0x35, 0xC0, 0x00, 0x00, 0x0A, 0x00, 0x00, b'h', b'i'];
    let computed = transport_v4(source, destination, Protocol::UDP, &datagram)
        .expect("ten bytes fit in a length");
    assert_eq!(computed, 0xC338);

    datagram[6..8].copy_from_slice(&computed.to_be_bytes());
    assert_eq!(
        transport_v4(source, destination, Protocol::UDP, &datagram),
        Ok(0x0000)
    );
}

#[test]
fn a_datagram_with_no_payload_is_summed_over_its_header_alone() {
    let source = Ipv4Addr::new(10, 0, 0, 1);
    let destination = Ipv4Addr::new(10, 0, 0, 2);
    let header = [0x00, 0x35, 0xC0, 0x00, 0x00, 0x08, 0x00, 0x00];
    assert_eq!(
        transport_v4(source, destination, Protocol::UDP, &header),
        Ok(0x2BA6)
    );
}

#[test]
fn the_tcp_pseudo_header_names_its_own_protocol_and_length() {
    // From 10.0.0.2:49152 to 10.0.0.1:80, a bare SYN of twenty bytes with
    // sequence number 1 and a window of 65535. The sum of
    //   0a00 0002 0a00 0001 0006 0014
    //   c000 0050 0000 0001 0000 0000 5002 ffff 0000 0000
    // is 2471, and its complement db8e.
    let segment = [
        0xC0, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x50, 0x02, 0xFF,
        0xFF, 0x00, 0x00, 0x00, 0x00,
    ];
    assert_eq!(
        transport_v4(
            Ipv4Addr::new(10, 0, 0, 2),
            Ipv4Addr::new(10, 0, 0, 1),
            Protocol::TCP,
            &segment,
        ),
        Ok(0xDB8E)
    );
}

#[test]
fn a_segment_longer_than_a_sixteen_bit_length_is_refused() {
    let segment = vec![0u8; 65536];
    assert_eq!(
        transport_v4(
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::BROADCAST,
            Protocol::UDP,
            &segment,
        ),
        Err(WireError::Length(65536))
    );
}

#[test]
fn the_pseudo_header_can_be_added_on_its_own() {
    let mut piecewise = Checksum::new();
    piecewise.add_pseudo_header_v4(
        Ipv4Addr::new(10, 0, 0, 1),
        Ipv4Addr::new(10, 0, 0, 2),
        Protocol::UDP,
        10,
    );
    piecewise.add_bytes(&[0x00, 0x35, 0xC0, 0x00, 0x00, 0x0A, 0x00, 0x00]);
    piecewise.add_bytes(b"hi");
    assert_eq!(piecewise.finish(), 0xC338);
}

#[test]
fn a_block_that_carries_its_checksum_sums_to_zero_whatever_it_holds() {
    check("checksum verifies", &bytes(0..=512), |data| {
        let mut block = data.clone();
        // The field sits at an even offset, as it does in every header
        // that has one. A zero pad to get there costs nothing: it is the
        // low half of the last word, which RFC 1071 case [2] already
        // supplies.
        if block.len() % 2 == 1 {
            block.push(0);
        }
        let computed = checksum(&block);
        block.extend_from_slice(&computed.to_be_bytes());
        if is_valid(&block) {
            Ok(())
        } else {
            Err(format!(
                "{:04x} over {} bytes does not verify",
                computed,
                data.len()
            ))
        }
    });
}

#[test]
fn splitting_the_data_anywhere_gives_the_same_sum() {
    check("checksum is associative", &bytes(0..=256), |data| {
        let whole = checksum(data);
        for at in 0..=data.len() {
            let (head, tail) = data.split_at(at);
            let mut sum = Checksum::new();
            sum.add_bytes(head);
            sum.add_bytes(tail);
            if sum.finish() != whole {
                return Err(format!("a split at {at} of {} bytes differs", data.len()));
            }
        }
        Ok(())
    });
}
