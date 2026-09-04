// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::timer`.

use crate::timer::TimerError;

#[test]
fn error_has_a_message() {
    assert!(
        TimerError::UnsupportedFrequency(7)
            .to_string()
            .contains('7')
    );
}
