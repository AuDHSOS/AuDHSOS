// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a refusal of `fs-fat` becomes in the file protocol.
//!
//! The protocol carries the errors of `audhsos_abi`, which say what the
//! caller may do about it; `fs-fat` says what it found on the volume. This
//! is the one place the two meet, and every variant is named, so a variant
//! added to `fs-fat` is a compile error here and not a refusal that
//! silently becomes something else.

use audhsos_abi::Error;
use fs_fat::Error as FatError;

/// The refusal the protocol carries for `error`.
///
/// Three groups: what the caller asked wrongly, what the volume will not
/// give, and what the volume itself is. The third is
/// [`Error::InvalidState`] throughout — the bytes on the disk disagree
/// with the format, which no argument of a caller changes.
#[must_use]
pub const fn refusal(error: FatError) -> Error {
    match error {
        // The caller asked for something that is not there, or is there
        // already, or is the other kind of thing.
        FatError::NotFound => Error::NotFound,
        FatError::Exists => Error::AlreadyExists,
        FatError::Kind => Error::WrongObjectType,
        FatError::Name | FatError::Offset(_) | FatError::TooLarge => Error::InvalidArgument,
        // The volume has nothing left to give.
        FatError::Full => Error::OutOfMemory,
        // The device would not move the sector.
        FatError::Device(_) => Error::Unavailable,
        // The volume is not what it says it is. Every one of these is the
        // bytes on the disk disagreeing with the format.
        FatError::Signature
        | FatError::SectorSize(_)
        | FatError::ClusterSize(_)
        | FatError::Layout
        | FatError::NotFat32(_)
        | FatError::TooSmall(_)
        | FatError::Cluster(_)
        | FatError::FreeInChain(_)
        | FatError::BadCluster(_)
        | FatError::ChainLoop(_)
        | FatError::EntryName
        | FatError::Time => Error::InvalidState,
    }
}
