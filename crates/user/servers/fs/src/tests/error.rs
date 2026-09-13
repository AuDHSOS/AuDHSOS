// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::error`.

use audhsos_abi::Error;
use fs_fat::Error as FatError;

use crate::error::refusal;

/// Every variant `fs-fat` has, so that one added there is one this test
/// does not name and the match in `refusal` refuses to compile.
const EVERY: [FatError; 20] = [
    FatError::Device(3),
    FatError::Signature,
    FatError::SectorSize(1024),
    FatError::ClusterSize(3),
    FatError::Layout,
    FatError::NotFat32(100),
    FatError::TooSmall(8),
    FatError::Cluster(2),
    FatError::FreeInChain(5),
    FatError::BadCluster(6),
    FatError::ChainLoop(7),
    FatError::Full,
    FatError::Name,
    FatError::EntryName,
    FatError::Exists,
    FatError::NotFound,
    FatError::Kind,
    FatError::Offset(9),
    FatError::TooLarge,
    FatError::Time,
];

#[test]
fn what_the_caller_asked_wrongly_comes_back_as_what_it_may_change() {
    assert_eq!(refusal(FatError::NotFound), Error::NotFound);
    assert_eq!(refusal(FatError::Exists), Error::AlreadyExists);
    assert_eq!(refusal(FatError::Kind), Error::WrongObjectType);
    assert_eq!(refusal(FatError::Name), Error::InvalidArgument);
    assert_eq!(refusal(FatError::Offset(4)), Error::InvalidArgument);
    assert_eq!(refusal(FatError::TooLarge), Error::InvalidArgument);
}

#[test]
fn a_volume_with_nothing_left_and_a_device_that_would_not_move_differ() {
    assert_eq!(refusal(FatError::Full), Error::OutOfMemory);
    assert_eq!(refusal(FatError::Device(11)), Error::Unavailable);
}

#[test]
fn every_disagreement_between_the_bytes_and_the_format_is_one_refusal() {
    let volume = [
        FatError::Signature,
        FatError::SectorSize(1024),
        FatError::ClusterSize(3),
        FatError::Layout,
        FatError::NotFat32(100),
        FatError::TooSmall(8),
        FatError::Cluster(2),
        FatError::FreeInChain(5),
        FatError::BadCluster(6),
        FatError::ChainLoop(7),
        FatError::EntryName,
        FatError::Time,
    ];
    for error in volume {
        assert_eq!(refusal(error), Error::InvalidState, "{error:?}");
    }
}

#[test]
fn every_variant_answers_something_and_no_two_groups_overlap() {
    for error in EVERY {
        let answer = refusal(error);
        assert_ne!(answer, Error::Unsupported, "{error:?} fell through");
    }
}
