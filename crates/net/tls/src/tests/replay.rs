// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The handshake of RFC 8448, section 3, replayed.
//!
//! Every value the document prints is reproduced here from the pieces this
//! crate is built of: the transcript, the schedule, the record protection,
//! and the message readers. It is the only check available that this
//! implementation agrees with one that was not this one.
//!
//! One thing the replay does not do, and why. It does not run the state
//! machine, because reproducing the trace byte for byte would need this
//! client to write the `ClientHello` of another implementation, extension
//! for extension, and that `ClientHello` goes into the transcript that
//! every later secret depends on. Carrying four extensions this client has
//! no use for, so that its bytes match a document, would be the wrong
//! trade; the state machine is driven end to end against a server in
//! `super::machine` instead.
//!
//! What it does do, and did not before step R3 of section 11.15, is check
//! the server's signature. The trace signs with `rsa_pss_rsae_sha256`
//! under the thousand-and-twenty-four-bit key of section 2 of the same
//! document. That key is below what a certificate may carry (D-79), so no
//! chain here is walked with it; the signature is verified through
//! `crypto-rsa` directly, which is the one RSA vector this repository has
//! that it did not make itself.

use audhsos_x509::{Certificate, SignatureAlgorithm};

use crate::handshake::{CertificateChain, CertificateVerify, HandshakeType, read_message};
use crate::keys::{Schedule, finished_key, traffic_keys, verify_data};
use crate::protection::RecordProtection;
use crate::record::{ContentType, HEADER_LEN};
use crate::suite::CipherSuite;
use crate::tests::{hex, trace, unhex};
use crate::transcript::Transcript;

/// The suite the trace negotiates.
const SUITE: CipherSuite = CipherSuite::Aes128GcmSha256;

/// A constant of the trace as bytes.
fn bytes(text: &str) -> Vec<u8> {
    unhex(text)
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one handshake, replayed in the order the document prints it"
)]
fn the_trace_of_the_document_is_reproduced() {
    // The two hellos, as they went over the wire.
    let mut transcript = Transcript::new();
    transcript.update(&bytes(trace::CLIENT_HELLO));
    transcript.update(&bytes(trace::SERVER_HELLO));

    // The schedule, from its start to the handshake stage.
    let mut schedule = Schedule::new(SUITE);
    assert_eq!(hex(schedule.secret()), joined(trace::EARLY_SECRET), "early");

    schedule
        .advance(&bytes(trace::SHARED_SECRET))
        .expect("the shared value is well formed");
    assert_eq!(
        hex(schedule.secret()),
        joined(trace::HANDSHAKE_SECRET),
        "handshake"
    );

    let after_hellos = transcript.hash(SUITE);
    let client_handshake = schedule
        .derive(b"c hs traffic", after_hellos.as_bytes())
        .expect("the derivation fits");
    let server_handshake = schedule
        .derive(b"s hs traffic", after_hellos.as_bytes())
        .expect("the derivation fits");
    assert_eq!(
        hex(client_handshake.as_bytes()),
        joined(trace::CLIENT_HANDSHAKE_SECRET)
    );
    assert_eq!(
        hex(server_handshake.as_bytes()),
        joined(trace::SERVER_HANDSHAKE_SECRET)
    );

    // The keys the server's flight is under.
    let keys = traffic_keys(SUITE, server_handshake.as_bytes()).expect("the derivation fits");
    assert_eq!(hex(keys.key()), joined(trace::SERVER_HANDSHAKE_KEY));
    assert_eq!(hex(keys.iv()), joined(trace::SERVER_HANDSHAKE_IV));

    // The flight itself, opened.
    let record = bytes(trace::SERVER_FLIGHT_RECORD);
    let (head, tail) = record.split_at(HEADER_LEN);
    let mut body = tail.to_vec();
    let mut protection = RecordProtection::new(keys).expect("the key fits the cipher");
    let (kind, flight) = protection
        .open(head, &mut body)
        .expect("the flight opens under the keys the document names");
    assert_eq!(kind, ContentType::Handshake);

    // The four messages inside it.
    let mut rest = flight;
    let mut messages = Vec::new();
    while let Some((kind, message_body, length)) =
        read_message(rest).expect("the flight is well formed")
    {
        messages.push((
            kind,
            message_body.to_vec(),
            rest.get(..length).unwrap_or(&[]).to_vec(),
        ));
        rest = rest.get(length..).unwrap_or(&[]);
    }
    let kinds: Vec<HandshakeType> = messages.iter().map(|(kind, _, _)| *kind).collect();
    assert_eq!(
        kinds,
        vec![
            HandshakeType::EncryptedExtensions,
            HandshakeType::Certificate,
            HandshakeType::CertificateVerify,
            HandshakeType::Finished,
        ]
    );
    for (index, expected) in [
        (0, trace::ENCRYPTED_EXTENSIONS),
        (1, trace::CERTIFICATE),
        (2, trace::CERTIFICATE_VERIFY),
        (3, trace::SERVER_FINISHED),
    ] {
        let (_, _, whole) = messages.get(index).expect("the message is there");
        assert_eq!(hex(whole), joined(expected), "message {index}");
    }

    // The signature is over everything up to and including the
    // certificate, so those two messages go into the transcript first.
    for index in 0..2 {
        let (_, _, whole) = messages.get(index).expect("the message is there");
        transcript.update(whole);
    }
    let before_verify = transcript.hash(SUITE);
    check_the_signature(&messages, before_verify.as_bytes());

    // Then the `CertificateVerify` itself, before the server's code is
    // checked.
    let (_, _, whole_verify) = messages.get(2).expect("the message is there");
    transcript.update(whole_verify);
    let before_finished = transcript.hash(SUITE);
    let server_finished =
        finished_key(SUITE, server_handshake.as_bytes()).expect("the derivation fits");
    assert_eq!(
        hex(server_finished.as_bytes()),
        joined(trace::SERVER_FINISHED_KEY)
    );
    let expected = verify_data(
        SUITE,
        server_finished.as_bytes(),
        before_finished.as_bytes(),
    );
    let (_, finished_body, whole_finished) = messages.get(3).expect("the message is there");
    assert_eq!(
        hex(expected.as_bytes()),
        hex(finished_body),
        "the server's code is the one this schedule computes"
    );

    // The application stage.
    transcript.update(whole_finished);
    let after_finished = transcript.hash(SUITE);
    schedule.advance_to_master().expect("the derivation fits");
    assert_eq!(
        hex(schedule.secret()),
        joined(trace::MASTER_SECRET),
        "master"
    );

    let client_application = schedule
        .derive(b"c ap traffic", after_finished.as_bytes())
        .expect("the derivation fits");
    let server_application = schedule
        .derive(b"s ap traffic", after_finished.as_bytes())
        .expect("the derivation fits");
    assert_eq!(
        hex(client_application.as_bytes()),
        joined(trace::CLIENT_APPLICATION_SECRET)
    );
    assert_eq!(
        hex(server_application.as_bytes()),
        joined(trace::SERVER_APPLICATION_SECRET)
    );

    let application =
        traffic_keys(SUITE, client_application.as_bytes()).expect("the derivation fits");
    assert_eq!(
        hex(application.key()),
        joined(trace::CLIENT_APPLICATION_KEY)
    );
    assert_eq!(hex(application.iv()), joined(trace::CLIENT_APPLICATION_IV));

    // And the client's own `Finished`, which the document also prints.
    let client_finished =
        finished_key(SUITE, client_handshake.as_bytes()).expect("the derivation fits");
    assert_eq!(
        hex(client_finished.as_bytes()),
        joined(trace::CLIENT_FINISHED_KEY)
    );
    let code = verify_data(SUITE, client_finished.as_bytes(), after_finished.as_bytes());
    let sent = bytes(trace::CLIENT_FINISHED);
    let (_, body, _) = read_message(&sent).expect("well formed").expect("complete");
    assert_eq!(
        hex(code.as_bytes()),
        hex(body),
        "the client's code is the one this schedule computes"
    );
}

#[test]
fn the_client_record_of_the_trace_opens_under_the_keys_it_names() {
    let keys =
        traffic_keys(SUITE, &unhex(trace::CLIENT_HANDSHAKE_SECRET)).expect("the derivation fits");
    let mut protection = RecordProtection::new(keys).expect("the key fits the cipher");

    let record = bytes(trace::CLIENT_FINISHED_RECORD);
    let (head, tail) = record.split_at(HEADER_LEN);
    let mut body = tail.to_vec();
    let (kind, plaintext) = protection.open(head, &mut body).expect("the record opens");
    assert_eq!(kind, ContentType::Handshake);
    assert_eq!(hex(plaintext), joined(trace::CLIENT_FINISHED));
}

/// A constant without the wrapping the source needed.
fn joined(text: &str) -> String {
    text.replace([' ', '\n', '\\'], "")
}

/// The server's `CertificateVerify`, checked against the key its
/// certificate carries.
///
/// This is the seam section 11.11 named as open: RFC 8448 signs with
/// `rsa_pss_rsae_sha256`, and until `crypto-rsa` existed there was
/// nothing here that could tell a good signature from a bad one.
fn check_the_signature(messages: &[(HandshakeType, Vec<u8>, Vec<u8>)], transcript: &[u8]) {
    let (_, certificate_body, _) = messages.get(1).expect("the certificate is there");
    let chain = CertificateChain::parse(certificate_body).expect("the message is well formed");
    let leaf = chain
        .certificates()
        .next()
        .expect("the chain has an entry")
        .expect("the entry is well formed");
    let certificate = Certificate::parse(leaf).expect("the certificate of the document parses");

    let (_, verify_body, _) = messages.get(2).expect("the certificate verify is there");
    let verify = CertificateVerify::parse(verify_body).expect("the message is well formed");
    assert_eq!(verify.scheme, 0x0804, "rsa_pss_rsae_sha256");

    // RFC 8446 section 4.4.3: sixty-four spaces, the context, a zero, and
    // the transcript hash.
    let mut content = vec![0x20u8; 64];
    content.extend_from_slice(b"TLS 1.3, server CertificateVerify");
    content.push(0x00);
    content.extend_from_slice(transcript);

    certificate
        .spki
        .verify(SignatureAlgorithm::RsaPssSha256, &content, verify.signature)
        .expect("the signature of the document verifies under the key of the document");

    // And it is a signature over that content and no other.
    let mut changed = content;
    if let Some(last) = changed.last_mut() {
        *last ^= 0x01;
    }
    assert!(
        certificate
            .spki
            .verify(SignatureAlgorithm::RsaPssSha256, &changed, verify.signature)
            .is_err(),
        "a signature over another transcript"
    );
}
