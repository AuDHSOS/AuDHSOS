// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::symbolize`.

use std::path::Path;

use crate::symbolize::{Resolved, addresses_in, command, report};

#[test]
fn the_addresses_of_a_trap_report_are_found_once_each_in_order() {
    let output = "\
[trap] page fault (vector 14) at ip 0xffffffff80011360 sp 0xffffffff7effff00\n\
[trap] error code 0x2\n\
[trap] faulting address 0xdead_beef\n\
[trap] again at ip 0xffffffff80011360\n";
    assert_eq!(
        addresses_in(output),
        vec![0xffff_ffff_8001_1360, 0xffff_ffff_7eff_ff00]
    );
}

#[test]
fn short_numbers_and_user_addresses_are_left_alone() {
    let output = "error code 0x2 vector 0xe at 0x1000 and 0x0000000000001000";
    assert!(addresses_in(output).is_empty());
}

#[test]
fn output_without_an_address_yields_nothing() {
    assert!(addresses_in("").is_empty());
    assert!(addresses_in("boot complete").is_empty());
    assert!(addresses_in("0x").is_empty());
}

#[test]
fn a_resolved_address_reads_as_a_line() {
    let whole = Resolved {
        address: 0xffff_ffff_8001_1360,
        function: Some("kernel_entry".to_owned()),
        place: Some("crates/kernel/bin/src/main.rs:31".to_owned()),
    };
    assert_eq!(
        format!("{whole}"),
        "0xffffffff80011360 kernel_entry at crates/kernel/bin/src/main.rs:31"
    );
    let bare = Resolved {
        address: 0x10,
        function: None,
        place: None,
    };
    assert_eq!(format!("{bare}"), "0x0000000000000010 <unknown>");
    assert!(!format!("{bare:?}").is_empty());
    assert_ne!(whole, bare);
}

#[test]
fn a_report_over_a_file_that_is_not_there_says_nothing_and_fails_nothing() {
    report(
        Path::new("/definitely/missing/kernel"),
        "at ip 0xffffffff80011360",
    );
    report(Path::new("/definitely/missing/kernel"), "boot complete");
}

#[test]
fn the_subcommand_needs_a_file_and_an_address() {
    assert!(command(&[]).is_err());
    assert!(command(&["only-a-file".to_owned()]).is_err());
    assert!(
        command(&["/definitely/missing".to_owned(), "0x1000".to_owned()]).is_err(),
        "a file that is not there is an error"
    );
}
