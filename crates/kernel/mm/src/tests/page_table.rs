// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::page_table`, covering the entry items of the catalog
//! 6.6.4.

#![allow(clippy::arithmetic_side_effects)]

use kernel_types::{PhysAddr, PhysFrame, VirtAddr};
use test_support::generators::pair;
use test_support::property::check;

use crate::page_table::{
    CachePolicy, EntryError, EntryFormat, PageTable, Permissions, X86_ADDRESS_MASK, X86_DIRTY,
    X86_GLOBAL, X86_HUGE, X86_NO_CACHE, X86_NO_EXECUTE, X86_PRESENT, X86_RESERVED_MASK, X86_USER,
    X86_WRITABLE, X86_WRITE_THROUGH, X86Entry,
};
use crate::strategies::{any_permissions, any_user_page_range};

fn frame(number: u64) -> PhysFrame {
    PhysFrame::from_number(number).unwrap()
}

fn page(address: u64) -> kernel_types::Page {
    VirtAddr::new(address).unwrap().page()
}

const ALL_PERMISSIONS: [Permissions; 8] = [
    Permissions {
        write: false,
        execute: false,
        user: false,
    },
    Permissions {
        write: true,
        execute: false,
        user: false,
    },
    Permissions {
        write: false,
        execute: true,
        user: false,
    },
    Permissions {
        write: true,
        execute: true,
        user: false,
    },
    Permissions {
        write: false,
        execute: false,
        user: true,
    },
    Permissions {
        write: true,
        execute: false,
        user: true,
    },
    Permissions {
        write: false,
        execute: true,
        user: true,
    },
    Permissions {
        write: true,
        execute: true,
        user: true,
    },
];

#[test]
fn the_named_permission_sets_are_what_they_say() {
    assert_eq!(
        Permissions::READ_ONLY,
        Permissions {
            write: false,
            execute: false,
            user: false
        }
    );
    const { assert!(Permissions::READ_WRITE.write && !Permissions::READ_WRITE.execute) };
    const { assert!(Permissions::READ_EXECUTE.execute && !Permissions::READ_EXECUTE.write) };
    assert!(!Permissions::READ_WRITE.is_write_execute());
    assert!(
        Permissions {
            write: true,
            execute: true,
            user: false
        }
        .is_write_execute()
    );
    assert!(Permissions::READ_ONLY.for_user().user);
    assert_eq!(Permissions::default(), Permissions::READ_ONLY);
    assert_eq!(CachePolicy::default(), CachePolicy::WriteBack);
}

#[test]
fn every_permission_and_cache_combination_round_trips() {
    for perms in ALL_PERMISSIONS {
        for cache in [CachePolicy::WriteBack, CachePolicy::Uncached] {
            for global in [false, true] {
                let entry = X86Entry::leaf(frame(9), perms, cache, global);
                assert!(entry.is_present());
                assert_eq!(entry.permissions(), perms);
                assert_eq!(entry.frame(), Some(frame(9)));
                assert_eq!(entry.has(X86_GLOBAL), global);
                assert_eq!(
                    entry.has(X86_NO_CACHE),
                    cache == CachePolicy::Uncached,
                    "the cache policy shows in the cache bits"
                );
                assert_eq!(entry.has(X86_WRITE_THROUGH), cache == CachePolicy::Uncached);
                assert_eq!(entry.validate(), Ok(()));
            }
        }
    }
}

#[test]
fn a_read_only_entry_is_neither_writable_nor_executable() {
    let entry = X86Entry::leaf(
        frame(1),
        Permissions::READ_ONLY,
        CachePolicy::WriteBack,
        false,
    );
    assert!(!entry.has(X86_WRITABLE));
    assert!(entry.has(X86_NO_EXECUTE));
    assert!(!entry.has(X86_USER));
    assert!(entry.has(X86_PRESENT));
}

#[test]
fn permissions_can_be_changed_without_touching_the_address_or_the_cache() {
    let original = X86Entry::leaf(
        frame(0x1234),
        Permissions::READ_ONLY,
        CachePolicy::Uncached,
        true,
    );
    for perms in ALL_PERMISSIONS {
        let changed = original.with_permissions(perms);
        assert_eq!(changed.permissions(), perms);
        assert_eq!(changed.frame(), original.frame());
        assert_eq!(changed.has(X86_NO_CACHE), original.has(X86_NO_CACHE));
        assert_eq!(changed.has(X86_GLOBAL), original.has(X86_GLOBAL));
    }
}

#[test]
fn a_table_entry_is_present_writable_and_reachable_from_user_mode() {
    let entry = X86Entry::table(frame(7));
    assert!(entry.has(X86_PRESENT) && entry.has(X86_WRITABLE) && entry.has(X86_USER));
    assert!(!entry.has(X86_NO_EXECUTE));
    assert_eq!(entry.frame(), Some(frame(7)));
    assert_eq!(entry.validate(), Ok(()));
}

#[test]
fn only_the_address_bits_twelve_to_fifty_one_carry_the_frame() {
    let highest = PhysFrame::from_start(PhysAddr::new(X86_ADDRESS_MASK).unwrap()).unwrap();
    let entry = X86Entry::leaf(
        highest,
        Permissions::READ_WRITE,
        CachePolicy::WriteBack,
        false,
    );
    assert_eq!(entry.frame(), Some(highest));
    assert_eq!(entry.as_u64() & X86_ADDRESS_MASK, X86_ADDRESS_MASK);

    let noisy = X86Entry::from_raw(X86_PRESENT | X86_DIRTY | 0x0000_0000_0009_9000);
    assert_eq!(
        noisy.frame(),
        Some(frame(0x99)),
        "the flag bits are masked out of the address"
    );
    assert_eq!(X86_ADDRESS_MASK, 0x000F_FFFF_FFFF_F000);
}

#[test]
fn an_entry_that_is_not_present_carries_no_frame_and_needs_no_validation() {
    let empty = X86Entry::EMPTY;
    assert!(!empty.is_present());
    assert_eq!(empty.frame(), None);
    assert_eq!(empty.validate(), Ok(()));
    assert_eq!(X86Entry::default(), empty);
    let garbage = X86Entry::from_raw(X86_RESERVED_MASK | X86_HUGE);
    assert_eq!(
        garbage.validate(),
        Ok(()),
        "bits of an absent entry are software-defined"
    );
    assert_eq!(garbage.frame(), None);
}

#[test]
fn reserved_bits_and_large_pages_are_reported_instead_of_being_ignored() {
    for bit in 52..=62u32 {
        let raw = X86_PRESENT | (1u64 << bit);
        assert_eq!(
            X86Entry::from_raw(raw).validate(),
            Err(EntryError::ReservedBits(raw)),
            "bit {bit} must be reported"
        );
    }
    let huge = X86Entry::from_raw(X86_PRESENT | X86_HUGE);
    assert_eq!(huge.validate(), Err(EntryError::HugePage));
    let no_execute = X86Entry::from_raw(X86_PRESENT | X86_NO_EXECUTE);
    assert_eq!(
        no_execute.validate(),
        Ok(()),
        "bit 63 is the no-execute bit, not a reserved bit"
    );
    assert!(!format!("{:?}", X86Entry::from_raw(3)).is_empty());
    for error in [EntryError::ReservedBits(1), EntryError::HugePage] {
        assert!(!format!("{error}").is_empty());
    }
}

#[test]
fn the_index_of_a_page_is_the_nine_bit_slice_of_its_page_number() {
    assert_eq!(X86Entry::LEVELS, 4);
    assert_eq!(X86Entry::INDEX_BITS, 9);
    assert_eq!(X86Entry::ENTRIES, 512);
    let address = page(0x0000_7FBF_DFEF_F000_u64 & 0x0000_7FFF_FFFF_F000);
    for level in 0..X86Entry::LEVELS {
        let expected = usize::try_from((address.number() >> (9 * level)) & 0x1FF).unwrap();
        assert_eq!(X86Entry::index(level, address), expected);
    }
    assert_eq!(X86Entry::index(0, page(0)), 0);
    assert_eq!(X86Entry::index(0, page(0x1000)), 1);
    assert_eq!(X86Entry::index(1, page(0x20_0000)), 1);
    assert_eq!(X86Entry::index(2, page(0x4000_0000)), 1);
    assert_eq!(X86Entry::index(3, page(0x80_0000_0000)), 1);
    assert_eq!(
        X86Entry::index(99, page(0x1000)),
        0,
        "a level above the tree yields the index zero"
    );
}

#[test]
fn a_new_table_is_empty_and_reports_its_entries() {
    let mut table: PageTable<X86Entry> = PageTable::new();
    assert!(table.is_unused());
    assert_eq!(table.entry(0), X86Entry::EMPTY);
    assert_eq!(table.entry(511), X86Entry::EMPTY);
    assert_eq!(
        table.entry(512),
        X86Entry::EMPTY,
        "an index out of range reports the empty entry"
    );
    assert!(table.set_entry(511, X86Entry::table(frame(3))));
    assert!(!table.is_unused());
    assert_eq!(table.entry(511).frame(), Some(frame(3)));
    assert!(
        !table.set_entry(512, X86Entry::table(frame(4))),
        "an index out of range changes nothing"
    );
    assert_eq!(table, PageTable::<X86Entry>::default().tap(511, frame(3)));
}

/// Builds the expected table for the comparison above.
trait Tap {
    fn tap(self, index: usize, frame: PhysFrame) -> Self;
}

impl Tap for PageTable<X86Entry> {
    fn tap(mut self, index: usize, frame: PhysFrame) -> Self {
        self.set_entry(index, X86Entry::table(frame));
        self
    }
}

#[test]
fn property_leaf_entries_round_trip_their_permissions() {
    check(
        "leaf_round_trip",
        &pair(any_permissions(), any_user_page_range()),
        |(perms, pages)| {
            let target = frame(pages.start().number() & 0xFFFF);
            let entry = X86Entry::leaf(target, *perms, CachePolicy::WriteBack, false);
            if entry.permissions() != *perms {
                return Err(format!("{perms:?} did not round-trip"));
            }
            if entry.frame() != Some(target) {
                return Err("the frame did not round-trip".to_owned());
            }
            entry.validate().map_err(|error| error.to_string())
        },
    );
}
