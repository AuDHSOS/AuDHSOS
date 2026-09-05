// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The error type: every variant says what a container refused.

use crate::error::CollectionError;

#[test]
fn every_variant_renders_a_message() {
    for (error, text) in [
        (CollectionError::Full, "the container is full"),
        (
            CollectionError::Index(9),
            "the index 9 is outside the container",
        ),
        (
            CollectionError::NotLinked(3),
            "the node 3 is not in this list",
        ),
        (
            CollectionError::AlreadyLinked(3),
            "the node 3 is already in a list",
        ),
        (
            CollectionError::ListId,
            "a list cannot be identified by `NONE`",
        ),
    ] {
        assert_eq!(error.to_string(), text);
    }
}

#[test]
fn two_errors_of_the_same_kind_and_value_are_equal() {
    assert_eq!(CollectionError::Index(3), CollectionError::Index(3));
    assert_ne!(CollectionError::Index(3), CollectionError::Index(4));
    assert_ne!(
        CollectionError::NotLinked(3),
        CollectionError::AlreadyLinked(3)
    );
}
