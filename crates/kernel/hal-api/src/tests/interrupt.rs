// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::interrupt`.

use crate::interrupt::{InterruptError, InterruptLine, Vector};

#[test]
fn device_vectors_start_at_thirty_two() {
    assert_eq!(Vector::new(31), Err(InterruptError::ReservedVector(31)));
    assert_eq!(Vector::new(32).map(Vector::number), Ok(32));
    assert_eq!(Vector::new(255).map(Vector::number), Ok(255));
    assert_eq!(InterruptLine::new(4).number(), 4);
}

#[test]
fn errors_have_messages() {
    for error in [
        InterruptError::ReservedVector(1),
        InterruptError::UnknownLine(2),
        InterruptError::AlreadyRouted(3),
    ] {
        assert!(!error.to_string().is_empty());
    }
}
