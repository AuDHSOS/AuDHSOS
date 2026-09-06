// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The option walk: what it skips, what it stops at, and what it refuses.

use net_wire::{WireError, Writer};

use crate::error::DhcpError;
use crate::option::{OptionCode, Options, write_option};

/// Everything the walk yields, error included.
fn walk(bytes: &[u8]) -> Vec<Result<(OptionCode, Vec<u8>), DhcpError>> {
    Options::new(bytes)
        .map(|option| option.map(|(code, body)| (code, body.to_vec())))
        .collect()
}

#[test]
fn options_come_out_in_the_order_they_stand() {
    let bytes = [53u8, 1, 2, 1, 4, 255, 255, 255, 0, 255];
    let walked = walk(&bytes);
    assert_eq!(walked.len(), 2);
    assert_eq!(
        walked[0].as_ref().expect("an option"),
        &(OptionCode::MESSAGE_TYPE, vec![2])
    );
    assert_eq!(
        walked[1].as_ref().expect("an option"),
        &(OptionCode::SUBNET_MASK, vec![255, 255, 255, 0])
    );
}

#[test]
fn padding_is_skipped_and_the_end_marker_stops_the_walk() {
    let bytes = [0u8, 0, 53, 1, 5, 0, 255, 1, 4, 1, 2, 3, 4];
    let walked = walk(&bytes);
    assert_eq!(walked.len(), 1);
    assert_eq!(
        walked[0].as_ref().expect("an option"),
        &(OptionCode::MESSAGE_TYPE, vec![5])
    );
}

#[test]
fn an_option_this_client_has_no_use_for_is_stepped_over() {
    let bytes = [12u8, 3, b'a', b'b', b'c', 53, 1, 5, 255];
    let walked = walk(&bytes);
    assert_eq!(walked.len(), 2);
    assert_eq!(
        walked[0].as_ref().expect("an option"),
        &(OptionCode::new(12), vec![b'a', b'b', b'c'])
    );
}

#[test]
fn an_option_that_reaches_past_the_block_is_refused() {
    // A length that claims more than there is.
    let walked = walk(&[53u8, 8, 5, 255]);
    assert_eq!(
        walked,
        vec![Err(DhcpError::TruncatedOption(OptionCode::MESSAGE_TYPE))]
    );
    // And a code with no length behind it at all.
    let walked = walk(&[53u8]);
    assert_eq!(
        walked,
        vec![Err(DhcpError::TruncatedOption(OptionCode::MESSAGE_TYPE))]
    );
}

#[test]
fn a_block_that_runs_out_without_the_end_marker_is_refused() {
    assert_eq!(
        walk(&[53u8, 1, 5]),
        vec![
            Ok((OptionCode::MESSAGE_TYPE, vec![5])),
            Err(DhcpError::MissingEnd)
        ]
    );
    assert_eq!(walk(&[]), vec![Err(DhcpError::MissingEnd)]);
    assert_eq!(walk(&[0u8, 0]), vec![Err(DhcpError::MissingEnd)]);
}

#[test]
fn an_option_of_no_body_is_an_option() {
    let walked = walk(&[116u8, 0, 255]);
    assert_eq!(
        walked[0].as_ref().expect("an option"),
        &(OptionCode::new(116), Vec::new())
    );
}

#[test]
fn a_written_option_reads_back_as_itself() {
    let mut buffer = [0u8; 16];
    let mut writer = Writer::new(&mut buffer);
    write_option(&mut writer, OptionCode::LEASE_TIME, &3600u32.to_be_bytes()).expect("room");
    writer.write_u8(OptionCode::END.get()).expect("room");
    let bytes = writer.finish().to_vec();
    assert_eq!(
        walk(&bytes)[0].as_ref().expect("an option"),
        &(OptionCode::LEASE_TIME, 3600u32.to_be_bytes().to_vec())
    );
}

#[test]
fn a_body_longer_than_the_length_field_is_refused() {
    let long = vec![0u8; 256];
    let mut buffer = [0u8; 512];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        write_option(&mut writer, OptionCode::PARAMETER_LIST, &long),
        Err(DhcpError::TooLong(256))
    );
}

#[test]
fn an_option_that_does_not_fit_its_buffer_says_so() {
    for room in 0..6usize {
        let mut buffer = vec![0u8; room];
        let mut writer = Writer::new(&mut buffer);
        assert!(matches!(
            write_option(&mut writer, OptionCode::SUBNET_MASK, &[255, 255, 255, 0]),
            Err(DhcpError::Wire(WireError::OutOfBounds { .. }))
        ));
    }
}

#[test]
fn every_code_this_client_names_has_the_number_rfc_2132_gives_it() {
    let cases = [
        (OptionCode::PAD, 0),
        (OptionCode::SUBNET_MASK, 1),
        (OptionCode::ROUTER, 3),
        (OptionCode::DOMAIN_NAME_SERVER, 6),
        (OptionCode::REQUESTED_ADDRESS, 50),
        (OptionCode::LEASE_TIME, 51),
        (OptionCode::MESSAGE_TYPE, 53),
        (OptionCode::SERVER_IDENTIFIER, 54),
        (OptionCode::PARAMETER_LIST, 55),
        (OptionCode::MAX_MESSAGE_SIZE, 57),
        (OptionCode::RENEWAL_TIME, 58),
        (OptionCode::REBINDING_TIME, 59),
        (OptionCode::END, 255),
    ];
    for (code, number) in cases {
        assert_eq!(code.get(), number);
    }
}
