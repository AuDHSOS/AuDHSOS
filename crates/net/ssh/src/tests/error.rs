// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a refusal says.

use crypto_rng::{EntropyError, RngError};

use crate::error::SshError;

#[test]
fn every_refusal_renders_a_sentence_of_its_own() {
    let rendered: Vec<String> = [
        SshError::OutOfBounds {
            needed: 8,
            available: 2,
        },
        SshError::Mpint,
        SshError::Negative,
        SshError::NameList,
        SshError::PacketLength(13),
        SshError::PaddingLength(3),
        SshError::PayloadLength(40_000),
        SshError::BlockSize(4),
        SshError::Message(21),
        SshError::Identification,
        SshError::Negotiation("cipher"),
        SshError::KeyExchangeFailed,
        SshError::Tag,
        SshError::Rng(RngError::Entropy(EntropyError::Unavailable)),
    ]
    .iter()
    .map(|error| format!("{error}"))
    .collect();

    for sentence in &rendered {
        assert!(!sentence.is_empty());
    }
    let mut unique = rendered.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), rendered.len());
    assert_eq!(
        rendered.first().map(String::as_str),
        Some("8 bytes were needed and 2 are left")
    );
}
