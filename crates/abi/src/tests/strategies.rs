// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::strategies`.

#![allow(clippy::arithmetic_side_effects)]

use crate::boot_image::BOOT_IMAGE_HEADER_LEN;
use crate::layout::PAGE_SIZE;
use crate::strategies::{
    any_boot_image_bytes, any_boot_image_header, any_boot_region, any_region_kind_code,
    image_len_for,
};
use test_support::property::check;

#[test]
fn generated_headers_are_aligned_and_do_not_overlap() {
    check("headers_valid", &any_boot_image_header(), |header| {
        if !header.root_task_offset.is_multiple_of(PAGE_SIZE)
            || !header.archive_offset.is_multiple_of(PAGE_SIZE)
        {
            return Err("unaligned offset".to_owned());
        }
        let root_end = header.root_task_offset + header.root_task_len;
        if header.archive_len != 0 && header.archive_offset < root_end {
            return Err("archive overlaps the root task".to_owned());
        }
        if image_len_for(header) < root_end {
            return Err("image length below the root task".to_owned());
        }
        Ok(())
    });
}

#[test]
fn generated_header_bytes_are_a_header_or_a_truncation() {
    check("header_bytes", &any_boot_image_bytes(), |bytes| {
        if bytes.len() == BOOT_IMAGE_HEADER_LEN
            || bytes.len() == BOOT_IMAGE_HEADER_LEN.saturating_sub(1)
        {
            Ok(())
        } else {
            Err(format!("unexpected length {}", bytes.len()))
        }
    });
}

#[test]
fn generated_region_kind_codes_stay_in_range() {
    check("kind_codes", &any_region_kind_code(), |code| {
        if *code <= 8 {
            Ok(())
        } else {
            Err(format!("code {code} out of range"))
        }
    });
    check("regions_aligned", &any_boot_region(), |region| {
        if region.start.is_multiple_of(PAGE_SIZE) && region.len != 0 {
            Ok(())
        } else {
            Err("misaligned or empty region".to_owned())
        }
    });
}
