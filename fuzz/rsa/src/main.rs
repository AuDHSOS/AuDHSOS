// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A key and a signature out of the same bytes, verified: no input may
//! panic, and none may run away.
//!
//! The second half is the one worth stating. Everything this target
//! reaches is bounded before it runs — the modulus by `MAX_LIMBS`, the
//! exponent by the sixty-four bits it is read into, the encodings by the
//! width of the key — so a signature cannot ask for work that a key does
//! not pay for in advance. A verification that took a long time would be
//! a bound that is missing, and the engine would find it as a timeout
//! rather than as a crash.
//!
//! Nothing here checks that a signature verifies. A random string almost
//! never is one, and asserting that it is not would be asserting that the
//! fuzzer is unlucky. What is asserted is that a well-formed key is read
//! back as the key it was built from, which is the one property the input
//! can be made to have.

use crypto_rsa::{HashId, PublicKey};

/// The widest modulus the arithmetic holds.
const MAX_MODULUS: usize = 512;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    check(bytes);
});

/// Everything a caller does with bytes that claim to be a key and a
/// signature.
fn check(bytes: &[u8]) {
    // The first byte says which of the six schemes to try, the second how
    // wide the modulus is in limbs.
    let Some((&scheme, rest)) = bytes.split_first() else {
        return;
    };
    let Some((&limbs, rest)) = rest.split_first() else {
        return;
    };
    let width = usize::from(limbs)
        .saturating_mul(8)
        .min(MAX_MODULUS)
        .min(rest.len());
    let (modulus, rest) = rest.split_at(width);

    let Some((&exponent_len, rest)) = rest.split_first() else {
        return;
    };
    let (exponent, rest) = rest.split_at(usize::from(exponent_len).min(rest.len()));

    // A signature is as wide as the key; whatever is left is the message.
    let (signature, message) = rest.split_at(modulus.len().min(rest.len()));

    let Ok(key) = PublicKey::new(modulus, exponent) else {
        return;
    };

    // The key reads back as what it was built from. `size` is the width of
    // the modulus and `bits` is eight times it, because a key whose top
    // bit is clear is refused above.
    assert_eq!(key.size(), modulus.len(), "the key changed width");
    assert_eq!(
        key.bits(),
        modulus.len().saturating_mul(8),
        "the modulus is not as wide as its encoding"
    );

    // The low two bits of the selector choose the hash and its top bit
    // chooses the encoding, so a corpus entry says which of the six
    // schemes it was made for and the mutator can move between them one
    // bit at a time.
    let hash = match scheme & 0x03 {
        0 => HashId::Sha256,
        1 => HashId::Sha384,
        _ => HashId::Sha512,
    };
    if scheme & 0x80 == 0 {
        let _ = key.verify_pkcs1(hash, message, signature);
    } else {
        let _ = key.verify_pss(hash, message, signature);
    }

    // Both encodings on every input, so that a corpus entry made for one
    // of them still walks the other. The message is the empty one here,
    // which is the shortest thing either encoding hashes.
    let _ = key.verify_pkcs1(hash, &[], signature);
    let _ = key.verify_pss(hash, &[], signature);
}
