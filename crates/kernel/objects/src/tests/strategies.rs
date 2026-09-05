// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::strategies`.

use crate::strategies::{PoolOp, any_pool_op, any_quota_op};
use test_support::generators::vec;
use test_support::property::check;

#[test]
fn generated_operations_stay_in_their_ranges() {
    check("pool_ops", &vec(any_pool_op(), 0..=8), |ops| {
        for op in ops {
            let ok = match op {
                PoolOp::Allocate(payload) | PoolOp::GetStale(payload, _) => *payload <= 64,
                PoolOp::Retain(position) | PoolOp::Release(position) | PoolOp::Get(position) => {
                    *position <= 16
                }
            };
            if !ok {
                return Err(format!("{op:?} out of range"));
            }
        }
        Ok(())
    });
    check("quota_ops", &any_quota_op(), |(_charge, amount)| {
        if *amount <= 8 {
            Ok(())
        } else {
            Err(format!("amount {amount} out of range"))
        }
    });
}
