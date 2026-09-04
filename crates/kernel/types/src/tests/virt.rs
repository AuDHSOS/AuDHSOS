// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::virt`.

#![allow(clippy::arithmetic_side_effects)]

use crate::strategies::{any_page, any_page_range, any_virt_addr};
use crate::virt::{Page, PageRange, VirtAddr};
use crate::{Alignment, Error};
use audhsos_abi::layout::{KERNEL_SPACE_START, USER_SPACE_END};
use test_support::generators::{pair, range};
use test_support::property::check;

const HOLE_START: u64 = USER_SPACE_END;
const HOLE_END: u64 = KERNEL_SPACE_START;

#[test]
fn canonical_boundaries() {
    assert_eq!(VirtAddr::new(0), Ok(VirtAddr::ZERO));
    assert_eq!(VirtAddr::new(HOLE_START - 1), Ok(VirtAddr::USER_MAX));
    assert_eq!(
        VirtAddr::new(HOLE_START),
        Err(Error::NonCanonical(HOLE_START))
    );
    assert_eq!(
        VirtAddr::new(HOLE_END - 1),
        Err(Error::NonCanonical(HOLE_END - 1))
    );
    assert_eq!(VirtAddr::new(HOLE_END), Ok(VirtAddr::KERNEL_MIN));
    assert_eq!(VirtAddr::new(u64::MAX), Ok(VirtAddr::MAX));
    assert_eq!(VirtAddr::new(1 << 47), Err(Error::NonCanonical(1 << 47)));
    assert!(VirtAddr::USER_MAX.is_user() && !VirtAddr::USER_MAX.is_kernel());
    assert!(VirtAddr::KERNEL_MIN.is_kernel() && !VirtAddr::KERNEL_MIN.is_user());
}

#[test]
fn checked_add_stops_at_the_top_of_each_half() {
    assert_eq!(VirtAddr::USER_MAX.checked_add(1), None);
    assert_eq!(VirtAddr::USER_MAX.checked_add(0), Some(VirtAddr::USER_MAX));
    assert_eq!(
        VirtAddr::ZERO.checked_add(HOLE_START - 1),
        Some(VirtAddr::USER_MAX)
    );
    assert_eq!(VirtAddr::ZERO.checked_add(HOLE_END), None);
    assert_eq!(VirtAddr::MAX.checked_add(1), None);
    assert_eq!(
        VirtAddr::KERNEL_MIN.checked_add(u64::MAX - HOLE_END),
        Some(VirtAddr::MAX)
    );
    assert_eq!(VirtAddr::KERNEL_MIN.checked_sub(1), None);
    assert_eq!(VirtAddr::ZERO.checked_sub(1), None);
    assert_eq!(
        VirtAddr::MAX.checked_sub(u64::MAX - HOLE_END),
        Some(VirtAddr::KERNEL_MIN)
    );
}

#[test]
fn align_up_at_the_hole_and_at_the_top() {
    let page = Alignment::PAGE;
    assert_eq!(
        VirtAddr::USER_MAX.align_up(page),
        Err(Error::CrossesCanonicalHole)
    );
    assert_eq!(VirtAddr::MAX.align_up(page), Err(Error::Overflow));
    assert_eq!(VirtAddr::MAX.align_up(Alignment::BYTE), Ok(VirtAddr::MAX));
    let last_user_page = VirtAddr::new(HOLE_START - 4096).unwrap();
    assert_eq!(last_user_page.align_up(page), Ok(last_user_page));
    assert_eq!(
        VirtAddr::new(HOLE_START - 4095).unwrap().align_up(page),
        Err(Error::CrossesCanonicalHole)
    );
    assert_eq!(
        VirtAddr::new(4095)
            .unwrap()
            .align_up(page)
            .unwrap()
            .as_u64(),
        4096
    );
    assert_eq!(
        VirtAddr::new(HOLE_END + 1).unwrap().align_down(page),
        VirtAddr::KERNEL_MIN
    );
}

#[test]
fn last_page_rounding_does_not_overflow() {
    let last_page = VirtAddr::MAX.page();
    assert_eq!(last_page.start().as_u64(), u64::MAX - 4095);
    assert_eq!(last_page.last_address(), VirtAddr::MAX);
    assert_eq!(last_page.checked_add(1), None);
    let range = PageRange::from_addresses(last_page.start(), VirtAddr::MAX).unwrap();
    assert_eq!(range.count(), 1);
    assert_eq!(range.end_address(), None);
    assert_eq!(range.last(), Some(last_page));
}

#[test]
fn pages_round_trip_and_reject_unaligned_starts() {
    let addr = VirtAddr::new(0x7FFF_1234_5678).unwrap();
    assert_eq!(addr.offset_in_page(), 0x678);
    assert_eq!(addr.page().start().as_u64(), 0x7FFF_1234_5000);
    assert!(matches!(
        Page::from_start(addr),
        Err(Error::Unaligned { .. })
    ));
    let page = Page::from_start(addr.page().start()).unwrap();
    assert_eq!(Page::containing(addr), page);
    assert_eq!(page.number(), 0x0007_FFF1_2345);
    assert!(page.is_user());
}

#[test]
fn page_arithmetic_stays_in_the_half() {
    let last_user = VirtAddr::USER_MAX.page();
    assert_eq!(last_user.checked_add(1), None);
    assert_eq!(last_user.checked_add(0), Some(last_user));
    assert_eq!(
        last_user.checked_sub(last_user.number()),
        Some(VirtAddr::ZERO.page())
    );
    assert_eq!(VirtAddr::KERNEL_MIN.page().checked_sub(1), None);
    assert_eq!(VirtAddr::ZERO.page().checked_add(u64::MAX), None);
}

#[test]
fn ranges_empty_single_crossing_and_overflowing() {
    let first = VirtAddr::ZERO.page();
    let empty = PageRange::new(first, 0).unwrap();
    assert!(empty.is_empty() && empty.last().is_none() && empty.iter().count() == 0);
    let single = PageRange::new(first, 1).unwrap();
    assert_eq!(single.bytes(), 4096);
    assert_eq!(single.end_address(), VirtAddr::new(4096).ok());
    assert!(single.contains(first));
    let user_pages = HOLE_START >> 12;
    let whole_user_half = PageRange::new(first, user_pages).unwrap();
    assert_eq!(whole_user_half.last(), Some(VirtAddr::USER_MAX.page()));
    assert_eq!(whole_user_half.end_address(), None);
    assert_eq!(
        PageRange::new(first, user_pages + 1),
        Err(Error::CrossesCanonicalHole)
    );
    let kernel_pages = (u64::MAX - HOLE_END + 1) >> 12;
    let whole_kernel_half = PageRange::new(VirtAddr::KERNEL_MIN.page(), kernel_pages).unwrap();
    assert_eq!(whole_kernel_half.last(), Some(VirtAddr::MAX.page()));
    assert_eq!(
        PageRange::new(VirtAddr::KERNEL_MIN.page(), kernel_pages + 1),
        Err(Error::Overflow)
    );
    assert_eq!(
        PageRange::new(VirtAddr::KERNEL_MIN.page(), u64::MAX),
        Err(Error::Overflow)
    );
}

#[test]
fn from_addresses_rounds_outward_and_validates() {
    let range = PageRange::from_addresses(
        VirtAddr::new(0x1001).unwrap(),
        VirtAddr::new(0x2FFF).unwrap(),
    )
    .unwrap();
    assert_eq!(range.start().number(), 1);
    assert_eq!(range.count(), 2);
    let exact = PageRange::from_addresses(
        VirtAddr::new(0x1000).unwrap(),
        VirtAddr::new(0x2000).unwrap(),
    )
    .unwrap();
    assert_eq!(exact.count(), 1);
    let empty = PageRange::from_addresses(
        VirtAddr::new(0x1000).unwrap(),
        VirtAddr::new(0x1000).unwrap(),
    )
    .unwrap();
    assert!(empty.is_empty());
    assert_eq!(
        PageRange::from_addresses(
            VirtAddr::new(0x2000).unwrap(),
            VirtAddr::new(0x1000).unwrap()
        ),
        Err(Error::Overflow)
    );
    assert_eq!(
        PageRange::from_addresses(VirtAddr::USER_MAX, VirtAddr::KERNEL_MIN),
        Err(Error::CrossesCanonicalHole)
    );
    let user_end = PageRange::from_addresses(
        VirtAddr::new(HOLE_START - 8192).unwrap(),
        VirtAddr::USER_MAX,
    )
    .unwrap();
    assert_eq!(user_end.count(), 2);
    assert_eq!(user_end.end_address(), None);
}

#[test]
fn overlap_and_iteration() {
    let a = PageRange::new(VirtAddr::new(0x10_000).unwrap().page(), 4).unwrap();
    let touching = PageRange::new(VirtAddr::new(0x14_000).unwrap().page(), 2).unwrap();
    let inside = PageRange::new(VirtAddr::new(0x11_000).unwrap().page(), 1).unwrap();
    assert!(!a.overlaps(touching));
    assert!(a.overlaps(inside) && inside.overlaps(a));
    let empty = PageRange::new(VirtAddr::new(0x11_000).unwrap().page(), 0).unwrap();
    assert!(!a.overlaps(empty) && !empty.overlaps(a));
    let above = PageRange::new(VirtAddr::new(0x20_000).unwrap().page(), 1).unwrap();
    assert!(!above.overlaps(a) && !a.overlaps(above));
    assert!(!a.contains(VirtAddr::new(0xF_000).unwrap().page()));
    let numbers: Vec<u64> = a.into_iter().map(Page::number).collect();
    assert_eq!(numbers, vec![0x10, 0x11, 0x12, 0x13]);
    assert_eq!(a.iter().size_hint(), (4, Some(4)));
    let kernel = PageRange::new(VirtAddr::KERNEL_MIN.page(), 2).unwrap();
    assert!(!kernel.is_user());
    assert_eq!(kernel.iter().count(), 2);
}

#[test]
fn debug_and_display_are_hexadecimal() {
    let addr = VirtAddr::new(0xABC).unwrap();
    assert_eq!(format!("{addr:?}"), "VirtAddr(0xabc)");
    assert_eq!(format!("{addr}"), "0xabc");
    assert_eq!(format!("{:?}", addr.page()), "Page(0x0)");
}

#[test]
fn property_every_generated_address_is_canonical_and_round_trips() {
    check("virt_canonical", &any_virt_addr(), |&addr| {
        if VirtAddr::new(addr.as_u64()) != Ok(addr) {
            return Err("does not round-trip".into());
        }
        if addr.is_user() == addr.is_kernel() {
            return Err("address is in both or neither half".into());
        }
        if addr.page().start() > addr || addr.page().last_address() < addr {
            return Err("page does not contain the address".into());
        }
        Ok(())
    });
}

#[test]
fn property_checked_add_agrees_with_wide_arithmetic_and_keeps_the_half() {
    let generator = pair(any_virt_addr(), range(0u64..=u64::MAX));
    check("virt_checked_add", &generator, |&(addr, bytes)| {
        let wide = u128::from(addr.as_u64()) + u128::from(bytes);
        let limit = if addr.is_user() {
            u128::from(HOLE_START)
        } else {
            1u128 << 64
        };
        let expected = if wide < limit { Some(wide) } else { None };
        if addr.checked_add(bytes).map(|a| u128::from(a.as_u64())) != expected {
            return Err(format!("checked_add({addr}, {bytes:#x}) disagrees"));
        }
        Ok(())
    });
}

#[test]
fn property_page_ranges_stay_in_one_half() {
    let generator = pair(any_page_range(), any_page());
    check("page_range_half", &generator, |&(range, page)| {
        if let Some(last) = range.last()
            && last.is_user() != range.is_user()
        {
            return Err("range spans both halves".into());
        }
        if range.contains(page) && page.is_user() != range.is_user() {
            return Err("contains a page of the other half".into());
        }
        if range.contains(page)
            != (range.start() <= page && range.last().is_some_and(|l| page <= l))
        {
            return Err("contains disagrees with ordering".into());
        }
        Ok(())
    });
}
