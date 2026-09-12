// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The message numbers, against the table that assigns them.

use crate::msg;

#[test]
fn the_numbers_are_the_ones_rfc_4250_assigns() {
    // Section 4.1.2, transcribed a second time so that the constant and
    // the test do not share one slip.
    let table: [(&str, u8); 9] = [
        ("SSH_MSG_DISCONNECT", 1),
        ("SSH_MSG_IGNORE", 2),
        ("SSH_MSG_UNIMPLEMENTED", 3),
        ("SSH_MSG_DEBUG", 4),
        ("SSH_MSG_SERVICE_REQUEST", 5),
        ("SSH_MSG_SERVICE_ACCEPT", 6),
        ("SSH_MSG_EXT_INFO", 7),
        ("SSH_MSG_KEXINIT", 20),
        ("SSH_MSG_NEWKEYS", 21),
    ];
    let held: [(&str, u8); 9] = [
        ("SSH_MSG_DISCONNECT", msg::DISCONNECT),
        ("SSH_MSG_IGNORE", msg::IGNORE),
        ("SSH_MSG_UNIMPLEMENTED", msg::UNIMPLEMENTED),
        ("SSH_MSG_DEBUG", msg::DEBUG),
        ("SSH_MSG_SERVICE_REQUEST", msg::SERVICE_REQUEST),
        ("SSH_MSG_SERVICE_ACCEPT", msg::SERVICE_ACCEPT),
        ("SSH_MSG_EXT_INFO", msg::EXT_INFO),
        ("SSH_MSG_KEXINIT", msg::KEXINIT),
        ("SSH_MSG_NEWKEYS", msg::NEWKEYS),
    ];
    assert_eq!(held, table);

    // Section 4.1.1: the ranges the numbers fall in.
    assert!((1..=19).contains(&msg::DISCONNECT));
    assert!((1..=19).contains(&msg::EXT_INFO));
    assert!((20..=29).contains(&msg::KEXINIT));
    assert!((20..=29).contains(&msg::NEWKEYS));
}
