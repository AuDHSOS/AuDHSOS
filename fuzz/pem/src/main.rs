// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! PEM against arbitrary bytes: no input may panic, everything an accepted
//! block points at lies where it was told to, and what the parser accepts
//! re-encodes to a text that decodes to the same bytes.
//!
//! The last of those is what strictness is for. A decoder that accepted
//! two texts for one block would let a certificate be written twice and
//! compared once.

#![cfg_attr(fuzzing, no_main)]

use audhsos_encoding::base64;
use audhsos_encoding::pem::{self, LINE};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    block(bytes);
    quantum(bytes);
});

/// Everything a caller does with bytes that claim to carry a block.
fn block(bytes: &[u8]) {
    let mut out = vec![0u8; bytes.len().max(1)];
    let Ok(parsed) = pem::decode(bytes, &mut out) else {
        return;
    };
    let label = parsed.label.to_owned();
    let body = parsed.bytes.to_vec();

    assert!(!body.is_empty(), "an accepted block with no bytes");
    assert!(
        body.len() <= bytes.len(),
        "a block longer than the text it came from"
    );
    assert!(
        !label.contains('-'),
        "a label carrying the character that frames it"
    );

    // The canonical text of an accepted block is itself accepted, and
    // means the same thing.
    let needed = pem::encoded_len(&label, body.len()).expect("a length that fits");
    let mut text = vec![0u8; needed];
    let written = pem::encode(&label, &body, &mut text).expect("a buffer of the right size");
    let text = text.get(..written).expect("what was written");
    for line in text.split(|byte| *byte == b'\n') {
        assert!(line.len() <= LINE + 11 + label.len(), "an over-long line");
    }

    let mut again = vec![0u8; text.len()];
    let second = pem::decode(text, &mut again).expect("the canonical form of an accepted block");
    assert_eq!(second.label, label, "the label changed on re-encoding");
    assert_eq!(second.bytes, body, "the bytes changed on re-encoding");
}

/// The Base64 layer on its own, because a block reaches it only for an
/// input that happens to carry a frame.
fn quantum(bytes: &[u8]) {
    let mut out = vec![0u8; bytes.len()];
    let Ok(written) = base64::decode(bytes, &mut out) else {
        return;
    };
    let decoded = out.get(..written).expect("what was written");
    let needed = base64::encoded_len(decoded.len()).expect("a length that fits");
    let mut text = vec![0u8; needed];
    let count = base64::encode(decoded, &mut text).expect("a buffer of the right size");
    assert_eq!(
        text.get(..count),
        Some(bytes),
        "a strict decoder accepted a text that is not the canonical one"
    );
}
