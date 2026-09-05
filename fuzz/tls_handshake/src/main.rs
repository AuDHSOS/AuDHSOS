// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The handshake readers against arbitrary bytes: no input may panic, and
//! a message that frames must say how long it is.
//!
//! Two passes. The first treats the input as the stream of messages a
//! server sends and reads it message by message; the second hands the
//! whole input to every reader, so that a body which never frames is
//! parsed anyway. Without the second pass the fuzzer would have to guess
//! four bytes of header before it reached a parser at all.


use audhsos_tls::handshake::{
    CertificateChain, CertificateVerify, EncryptedExtensions, HandshakeType, ServerHello,
    read_key_update, read_message,
};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    // The stream, as it arrives from the record layer.
    let mut rest = bytes;
    while let Ok(Some((kind, body, length))) = read_message(rest) {
        assert!(length <= rest.len(), "a message longer than the input");
        assert!(length > 0, "a message that consumed nothing");
        read_body(kind, body);
        rest = rest.get(length..).unwrap_or(&[]);
    }

    // Every reader against the whole input.
    for kind in [
        HandshakeType::ServerHello,
        HandshakeType::EncryptedExtensions,
        HandshakeType::Certificate,
        HandshakeType::CertificateVerify,
        HandshakeType::KeyUpdate,
    ] {
        read_body(kind, bytes);
    }
});

/// One message body, read as the type it claims to be.
fn read_body(kind: HandshakeType, body: &[u8]) {
    match kind {
        HandshakeType::ServerHello => {
            if let Ok(hello) = ServerHello::parse(body) {
                if let Some(share) = hello.key_share {
                    assert!(share.len() <= body.len(), "a share larger than the body");
                }
            }
        }
        HandshakeType::EncryptedExtensions => {
            if let Ok(extensions) = EncryptedExtensions::parse(body) {
                if let Some(protocol) = extensions.alpn {
                    assert!(
                        protocol.len() <= body.len(),
                        "a protocol name larger than the body"
                    );
                }
            }
        }
        HandshakeType::Certificate => {
            if let Ok(chain) = CertificateChain::parse(body) {
                for entry in chain.certificates() {
                    let Ok(certificate) = entry else {
                        break;
                    };
                    assert!(
                        certificate.len() <= body.len(),
                        "a certificate larger than the message"
                    );
                }
            }
        }
        HandshakeType::CertificateVerify => {
            if let Ok(verify) = CertificateVerify::parse(body) {
                assert!(
                    verify.signature.len() <= body.len(),
                    "a signature larger than the message"
                );
            }
        }
        HandshakeType::KeyUpdate => {
            let _ = read_key_update(body);
        }
        _ => {}
    }
}
