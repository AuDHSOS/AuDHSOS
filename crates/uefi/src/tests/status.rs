// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::status`.

use crate::status::{ERROR_BIT, Status};

#[test]
fn success_is_zero_and_every_error_carries_the_high_bit() {
    assert_eq!(Status::SUCCESS.0, 0);
    assert!(Status::SUCCESS.is_success() && !Status::SUCCESS.is_error());
    let errors = [
        (Status::LOAD_ERROR, 1),
        (Status::INVALID_PARAMETER, 2),
        (Status::UNSUPPORTED, 3),
        (Status::BUFFER_TOO_SMALL, 5),
        (Status::NOT_READY, 6),
        (Status::OUT_OF_RESOURCES, 9),
        (Status::NOT_FOUND, 14),
    ];
    for (status, code) in errors {
        assert!(status.is_error(), "{status} is an error");
        assert!(!status.is_success());
        assert_eq!(status.0, code | ERROR_BIT);
    }
}

#[test]
fn a_warning_is_neither_success_nor_error() {
    let warning = Status(1);
    assert!(!warning.is_success());
    assert!(!warning.is_error());
    assert!(format!("{warning}").contains("status 1"));
    assert!(format!("{}", Status::NOT_FOUND).contains("error 14"));
}

#[test]
fn ok_passes_the_value_through_on_success_only() {
    assert_eq!(Status::SUCCESS.ok(7), Ok(7));
    assert_eq!(Status::NOT_FOUND.ok(7), Err(Status::NOT_FOUND));
    assert!(Status::SUCCESS < Status::NOT_FOUND);
}
