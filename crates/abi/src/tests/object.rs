// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::object`.

use crate::object::ObjectType;
use crate::{Error, Rights};
use std::collections::HashSet;

#[test]
fn every_type_round_trips_through_its_code() {
    for &ty in ObjectType::ALL {
        assert_eq!(ObjectType::from_code(ty.code()), Some(ty));
        assert_eq!(ObjectType::try_from(ty.code()), Ok(ty));
    }
}

#[test]
fn codes_are_unique_and_non_zero() {
    let codes: HashSet<u32> = ObjectType::ALL.iter().map(|t| t.code()).collect();
    assert_eq!(codes.len(), ObjectType::ALL.len());
    assert!(!codes.contains(&0));
    assert_eq!(ObjectType::from_code(0), None);
    assert_eq!(ObjectType::try_from(u32::MAX), Err(Error::InvalidArgument));
}

#[test]
fn every_mask_includes_the_generic_rights() {
    for &ty in ObjectType::ALL {
        let mask = ty.rights_mask();
        assert!(mask.contains(Rights::DUPLICATE), "{}", ty.name());
        assert!(mask.contains(Rights::TRANSFER), "{}", ty.name());
        assert!(mask.is_subset_of(Rights::ALL));
    }
}

#[test]
fn masks_match_the_object_table() {
    let expected = [
        (
            ObjectType::Process,
            Rights::MANAGE | Rights::MAP | Rights::INSTALL,
        ),
        (ObjectType::Thread, Rights::MANAGE),
        (
            ObjectType::MemoryObject,
            Rights::READ | Rights::WRITE | Rights::EXECUTE | Rights::MAP | Rights::INFO,
        ),
        (
            ObjectType::Endpoint,
            Rights::SEND | Rights::RECV | Rights::BADGE,
        ),
        (ObjectType::Reply, Rights::EMPTY),
        (
            ObjectType::Notification,
            Rights::SIGNAL | Rights::WAIT | Rights::BIND,
        ),
        (ObjectType::Interrupt, Rights::MANAGE),
        (ObjectType::IoPortRange, Rights::READ | Rights::WRITE),
        (ObjectType::SystemControl, Rights::MANAGE),
    ];
    assert_eq!(expected.len(), ObjectType::ALL.len());
    for (ty, specific) in expected {
        assert_eq!(
            ty.rights_mask(),
            specific | Rights::DUPLICATE | Rights::TRANSFER
        );
    }
}

#[test]
fn names_are_unique() {
    let names: HashSet<&str> = ObjectType::ALL.iter().map(|t| t.name()).collect();
    assert_eq!(names.len(), ObjectType::ALL.len());
}
