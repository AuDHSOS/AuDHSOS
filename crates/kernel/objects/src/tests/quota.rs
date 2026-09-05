// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::quota`, covering the quota items of the catalog 6.6.6.

#![allow(clippy::arithmetic_side_effects)]

use crate::quota::{Quota, QuotaExceeded};
use crate::strategies::any_quota_op;
use test_support::generators::vec;
use test_support::property::check;

#[test]
fn a_charge_exactly_at_the_limit_succeeds_and_one_above_fails() {
    let mut quota = Quota::new(4);
    assert_eq!(quota.remaining(), 4);
    assert!(quota.is_empty());
    assert_eq!(quota.charge(4), Ok(()));
    assert_eq!(quota.used(), 4);
    assert_eq!(quota.remaining(), 0);
    assert_eq!(
        quota.charge(1),
        Err(QuotaExceeded {
            limit: 4,
            used: 4,
            requested: 1
        })
    );
    assert_eq!(quota.used(), 4, "the rejected charge changed nothing");
}

#[test]
fn a_charge_one_above_the_limit_fails_from_the_start() {
    let mut quota = Quota::new(4);
    assert_eq!(
        quota.charge(5),
        Err(QuotaExceeded {
            limit: 4,
            used: 0,
            requested: 5
        })
    );
    assert_eq!(quota.used(), 0);
    assert_eq!(quota.limit(), 4);
}

#[test]
fn a_charge_that_would_overflow_is_rejected() {
    let mut quota = Quota::new(u32::MAX);
    assert_eq!(quota.charge(u32::MAX), Ok(()));
    assert_eq!(quota.charge(1).map_err(|error| error.requested), Err(1));
    assert_eq!(quota.used(), u32::MAX);
}

#[test]
fn a_refund_restores_the_previous_value() {
    let mut quota = Quota::new(8);
    assert_eq!(quota.charge(3), Ok(()));
    assert_eq!(quota.charge(2), Ok(()));
    quota.refund(2);
    assert_eq!(quota.used(), 3);
    quota.refund(3);
    assert!(quota.is_empty());
    quota.refund(7);
    assert_eq!(quota.used(), 0, "refunding more than is used stays at zero");
}

#[test]
fn a_zero_charge_is_always_accepted() {
    let mut quota = Quota::new(0);
    assert_eq!(quota.charge(0), Ok(()));
    assert_eq!(
        quota.charge(1),
        Err(QuotaExceeded {
            limit: 0,
            used: 0,
            requested: 1
        })
    );
    assert_eq!(Quota::default(), Quota::new(0));
}

#[test]
fn property_used_never_exceeds_the_limit() {
    check("quota_invariant", &vec(any_quota_op(), 0..=32), |ops| {
        let mut quota = Quota::new(16);
        for (charge, amount) in ops {
            if *charge {
                let _ = quota.charge(*amount);
            } else {
                quota.refund(*amount);
            }
            if quota.used() > quota.limit() {
                return Err(format!("used {} above the limit", quota.used()));
            }
            if quota.remaining() != quota.limit() - quota.used() {
                return Err("remaining does not match".to_owned());
            }
        }
        Ok(())
    });
}

#[test]
fn the_error_renders_a_message_and_maps_to_the_abi() {
    let error = QuotaExceeded {
        limit: 1,
        used: 1,
        requested: 1,
    };
    assert!(!format!("{error}").is_empty());
    assert_eq!(
        audhsos_abi::Error::from(error),
        audhsos_abi::Error::QuotaExceeded
    );
}
