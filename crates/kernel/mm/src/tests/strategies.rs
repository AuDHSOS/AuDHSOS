// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::strategies`.

#![allow(clippy::arithmetic_side_effects)]

use audhsos_abi::layout::{PAGE_SIZE, USER_SPACE_START};
use kernel_hal_api::platform::MemoryRegionKind;
use test_support::property::check;

use crate::strategies::{
    RegionOp, any_memory_region, any_memory_regions, any_permissions, any_populated_memory_regions,
    any_region_table_op, any_user_page_range,
};

#[test]
fn generated_regions_stay_below_seventeen_mebibytes() {
    check("regions_in_range", &any_memory_region(), |region| {
        let end = region.start.as_u64() + region.len;
        if end > 17 * 1024 * 1024 {
            return Err(format!("region reaches {end}"));
        }
        Ok(())
    });
    check("region_lists", &any_memory_regions(), |regions| {
        if regions.len() > 64 {
            return Err(format!("{} regions", regions.len()));
        }
        Ok(())
    });
}

#[test]
fn the_populated_generator_always_offers_usable_memory() {
    check("populated", &any_populated_memory_regions(), |regions| {
        if regions
            .iter()
            .any(|region| region.kind == MemoryRegionKind::Usable && region.len >= PAGE_SIZE)
        {
            Ok(())
        } else {
            Err("no usable memory".to_owned())
        }
    });
}

#[test]
fn generated_page_ranges_lie_in_the_mappable_user_half() {
    check("user_ranges", &any_user_page_range(), |pages| {
        if pages.is_empty() {
            return Err("empty range".to_owned());
        }
        if pages.start().start().as_u64() < USER_SPACE_START || !pages.is_user() {
            return Err(format!("{pages:?} is outside the user half"));
        }
        if pages.count() > 16 {
            return Err("range too long".to_owned());
        }
        Ok(())
    });
}

#[test]
fn every_permission_combination_can_be_generated() {
    check("permissions_total", &any_permissions(), |_| Ok(()));
}

#[test]
fn generated_region_operations_carry_valid_ranges() {
    check("region_ops", &any_region_table_op(), |op| {
        let pages = match *op {
            RegionOp::Insert(pages, _)
            | RegionOp::Remove(pages)
            | RegionOp::Protect(pages, _)
            | RegionOp::Find(pages) => pages,
        };
        if pages.is_empty() || !pages.is_user() {
            Err(format!("{op:?} carries an unusable range"))
        } else {
            Ok(())
        }
    });
}
