// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The handshake messages against arbitrary bytes: the name-lists of
//! `SSH_MSG_KEXINIT`, the reply of the key exchange, the host key and its
//! signature, and the identification string.
//!
//! These are the places where a byte from the network chooses a length: a
//! name-list says how many names follow, an `mpint` says how wide a number
//! is, and a blob says how long a key is. Nothing here may be read as
//! longer than what arrived, and nothing a peer sends may be accepted as a
//! host key without a rule that admits it.

use audhsos_ssh::exchange::{Method, Reply};
use audhsos_ssh::hostkey::{Fingerprint, HostKey, Trust};
use audhsos_ssh::kex::{CLIENT, KexInit, negotiate};
use audhsos_ssh::{Greeting, ident};

/// A fingerprint nobody's key hashes to, which is the point: no input may
/// be admitted as a host key under it.
const UNKNOWN: [u8; 32] = [0x7e; 32];

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    greeting(bytes);
    kexinit(bytes);
    reply(bytes);
    host_key(bytes);
});

/// The identification string, which arrives before any packet.
fn greeting(bytes: &[u8]) {
    if let Ok(Greeting::Identification { line, length }) = ident::read(bytes) {
        assert!(length <= bytes.len(), "a greeting longer than the input");
        assert!(
            line.len() < length,
            "a line no shorter than the bytes it took"
        );
        assert!(line.len() <= ident::MAX_LEN, "a line above the maximum");
        assert!(
            line.starts_with(ident::PREFIX),
            "a line that is no identification"
        );
    }
}

/// The negotiation: ten name-lists, and the rule that chooses from them.
fn kexinit(bytes: &[u8]) {
    let Ok(message) = KexInit::read(bytes) else {
        return;
    };
    assert_eq!(message.cookie.len(), 16, "a cookie that is not sixteen bytes");
    for name in message.kex.iter() {
        assert!(!name.is_empty(), "a name of no length");
        assert!(!name.contains(','), "a name that holds a separator");
    }
    // What is chosen, if anything, is a name both sides offered.
    if let Ok(choice) = negotiate(&CLIENT, &message) {
        assert!(
            CLIENT.kex.contains(&choice.kex),
            "a method this client does not offer"
        );
        assert!(
            message.kex.contains(choice.kex),
            "a method the peer does not offer"
        );
        assert!(
            CLIENT.host_key.contains(&choice.host_key),
            "a host key algorithm this client does not offer"
        );
        assert!(choice.mac_c2s.is_none(), "a MAC beside an AEAD");
    }
}

/// The second message of the key exchange, under both methods.
fn reply(bytes: &[u8]) {
    for method in [Method::Curve25519, Method::Group14] {
        let Ok(reply) = Reply::read(bytes, method) else {
            continue;
        };
        let total = reply
            .host_key
            .len()
            .saturating_add(reply.public.len())
            .saturating_add(reply.signature.len());
        assert!(total <= bytes.len(), "a reply longer than the input");
        if method == Method::Curve25519 {
            assert!(
                reply.public.len() <= bytes.len(),
                "a public value longer than the input"
            );
        }
    }
}

/// The host key blob and the signature over the exchange hash.
fn host_key(bytes: &[u8]) {
    let Ok(key) = HostKey::parse(bytes) else {
        return;
    };
    assert_eq!(bytes.len(), 51, "a key blob of another length");
    assert_eq!(
        key.fingerprint().len(),
        32,
        "a fingerprint that is not a digest"
    );
    assert!(
        !Fingerprint::new(UNKNOWN).accepts(&key),
        "a key admitted by a rule that names another"
    );
    // A blob is never also a signature for the same key: the two carry
    // values of different lengths under the same name.
    assert!(
        key.verify(&[0u8; 32], bytes).is_err(),
        "a key blob read as a signature over a hash of zeros"
    );
}
