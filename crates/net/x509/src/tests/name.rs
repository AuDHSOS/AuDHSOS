// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Name matching, by the rules of RFC 6125.

use audhsos_der::Tag;

use crate::algorithm::DNS_NAME_TAG;
use crate::name::{ServerName, matches_entry};

/// Whether a presented domain name matches a reference one.
fn dns(presented: &str, reference: &str) -> bool {
    matches_entry(
        DNS_NAME_TAG,
        presented.as_bytes(),
        ServerName::Dns(reference),
    )
}

#[test]
fn a_name_matches_itself_whatever_its_case() {
    assert!(dns("example.test", "example.test"));
    assert!(dns("EXAMPLE.test", "example.TEST"));
    assert!(dns("www.example.test", "www.example.test"));
    assert!(!dns("example.test", "example.tes"));
    assert!(!dns("example.test", "www.example.test"));
    assert!(!dns("www.example.test", "example.test"));
}

#[test]
fn a_trailing_dot_is_the_same_name() {
    assert!(dns("example.test.", "example.test"));
    assert!(dns("example.test", "example.test."));
    assert!(dns("example.test.", "example.test."));
}

#[test]
fn a_wildcard_stands_for_exactly_one_label() {
    assert!(dns("*.example.test", "www.example.test"));
    assert!(dns("*.example.test", "anything.example.test"));

    assert!(
        !dns("*.example.test", "example.test"),
        "a wildcard is not nothing"
    );
    assert!(
        !dns("*.example.test", "a.b.example.test"),
        "a wildcard is not two labels"
    );
    assert!(!dns("*.example.test", "www.example.other"));
}

#[test]
fn a_star_that_is_not_the_whole_leftmost_label_is_not_a_wildcard() {
    assert!(!dns("w*.example.test", "www.example.test"));
    assert!(!dns("w*.example.test", "w*.example.test"));
    assert!(!dns("www.*.test", "www.example.test"));
    assert!(!dns("*", "example.test"));
    assert!(!dns("*.", "example.test"));
    assert!(!dns("example.*", "example.test"));
}

#[test]
fn an_empty_name_matches_nothing() {
    assert!(!dns("", "example.test"));
    assert!(!dns("example.test", ""));
    assert!(!dns(".", "example.test"));
    assert!(!dns("example.test", "."));
}

#[test]
fn an_address_matches_only_an_address_entry() {
    let address = [192u8, 0, 2, 1];
    let ip_tag = Tag::context(7, false);

    assert!(matches_entry(ip_tag, &address, ServerName::Ip(&address)));
    assert!(!matches_entry(
        ip_tag,
        &[192, 0, 2, 2],
        ServerName::Ip(&address)
    ));

    // A name entry never matches an address, and the reverse.
    assert!(!matches_entry(
        DNS_NAME_TAG,
        b"192.0.2.1",
        ServerName::Ip(&address)
    ));
    assert!(!matches_entry(
        ip_tag,
        &address,
        ServerName::Dns("192.0.2.1")
    ));

    // Nor does any other kind of entry.
    assert!(!matches_entry(
        Tag::context(1, false),
        b"someone@example.test",
        ServerName::Dns("example.test")
    ));
}
