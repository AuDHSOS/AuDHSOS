// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The handshake messages, read against the ones a real server sent.
//!
//! The server side of the trace of RFC 8448 is decoded here message by
//! message. That is worth more than decoding messages this crate wrote
//! itself: it is the only check available that the reader agrees with an
//! implementation that was not this one.

use crate::codec::{Reader, Writer};
use crate::error::TlsError;
use crate::handshake::{
    CertificateChain, CertificateVerify, ClientHelloParams, EncryptedExtensions, HandshakeType,
    RETRY_RANDOM, ServerHello, read_key_update, read_message, write_client_hello, write_finished,
    write_key_update,
};
use crate::suite::CipherSuite;
use crate::tests::trace;
use crate::tests::{hex, unhex};

#[test]
fn the_server_hello_of_the_trace_says_what_the_document_says() {
    let bytes = unhex(trace::SERVER_HELLO);
    let (kind, body, used) = read_message(&bytes)
        .expect("a well formed message")
        .expect("the message is complete");
    assert_eq!(kind, HandshakeType::ServerHello);
    assert_eq!(used, bytes.len());

    let hello = ServerHello::parse(body).expect("the server of the trace is well behaved");
    assert_eq!(hello.suite, CipherSuite::Aes128GcmSha256);
    assert!(!hello.is_retry);
    assert!(!hello.is_downgrade());
    assert_eq!(hello.random.len(), 32);
    assert!(
        hello.session_id.is_empty(),
        "the client of the trace sent no session identifier, so the server echoes none"
    );

    let share = hello.key_share.expect("the server agreed");
    assert_eq!(share.len(), 32);
    assert_eq!(
        hex(share),
        "c9828876112095fe66762bdbf7c672e156d6cc253b833df1dd69b1b04e751f0f"
    );
}

#[test]
fn the_encrypted_extensions_of_the_trace_carry_no_protocol() {
    let bytes = unhex(trace::ENCRYPTED_EXTENSIONS);
    let (kind, body, _) = read_message(&bytes)
        .expect("a well formed message")
        .expect("the message is complete");
    assert_eq!(kind, HandshakeType::EncryptedExtensions);

    let extensions = EncryptedExtensions::parse(body).expect("well formed");
    assert_eq!(extensions.alpn, None, "the trace negotiates no protocol");
}

#[test]
fn the_certificate_of_the_trace_holds_one_certificate() {
    let bytes = unhex(trace::CERTIFICATE);
    let (kind, body, _) = read_message(&bytes)
        .expect("a well formed message")
        .expect("the message is complete");
    assert_eq!(kind, HandshakeType::Certificate);

    let chain = CertificateChain::parse(body).expect("well formed");
    let certificates: Vec<&[u8]> = chain
        .certificates()
        .map(|entry| entry.expect("the entries are well formed"))
        .collect();
    assert_eq!(certificates.len(), 1);

    let certificate = certificates.first().expect("there is one");
    assert_eq!(certificate.len(), 432);
    assert_eq!(
        certificate.first(),
        Some(&0x30),
        "a certificate is a sequence"
    );
}

#[test]
fn the_certificate_verify_of_the_trace_names_the_scheme_the_document_says() {
    let bytes = unhex(trace::CERTIFICATE_VERIFY);
    let (kind, body, _) = read_message(&bytes)
        .expect("a well formed message")
        .expect("the message is complete");
    assert_eq!(kind, HandshakeType::CertificateVerify);

    let verify = CertificateVerify::parse(body).expect("well formed");
    // 0x0804 is rsa_pss_rsae_sha256. The reader is neutral about schemes;
    // refusing one is the business of the state machine. The signature
    // itself is checked in `super::replay`, where the transcript it was
    // made over exists.
    assert_eq!(verify.scheme, 0x0804);
    assert_eq!(verify.signature.len(), 128);
}

#[test]
fn the_two_finished_messages_of_the_trace_carry_a_hash_each() {
    for (name, message) in [
        ("the server's", trace::SERVER_FINISHED),
        ("the client's", trace::CLIENT_FINISHED),
    ] {
        let bytes = unhex(message);
        let (kind, body, used) = read_message(&bytes)
            .expect("a well formed message")
            .expect("the message is complete");
        assert_eq!(kind, HandshakeType::Finished, "{name}");
        assert_eq!(body.len(), 32, "{name}");
        assert_eq!(used, bytes.len());
    }
}

#[test]
fn a_ticket_is_recognised_and_left_alone() {
    let bytes = unhex(trace::NEW_SESSION_TICKET);
    let (kind, body, used) = read_message(&bytes)
        .expect("a well formed message")
        .expect("the message is complete");
    assert_eq!(kind, HandshakeType::NewSessionTicket);
    assert_eq!(used, bytes.len());
    assert!(!body.is_empty(), "the body is skipped, not parsed");
}

#[test]
fn a_message_that_has_not_arrived_in_full_is_not_a_message_yet() {
    let bytes = unhex(trace::SERVER_HELLO);
    for prefix in 0..bytes.len() {
        assert_eq!(
            read_message(bytes.get(..prefix).unwrap_or(&[])),
            Ok(None),
            "{prefix} bytes"
        );
    }
    assert!(read_message(&bytes).expect("well formed").is_some());
}

#[test]
fn a_type_this_client_never_handles_is_refused() {
    for byte in [0u8, 3, 5, 13, 25, 254] {
        let message = [byte, 0x00, 0x00, 0x00];
        assert_eq!(
            read_message(&message),
            Err(TlsError::UnexpectedMessage),
            "type {byte}"
        );
    }
    for (byte, expected) in [
        (1u8, HandshakeType::ClientHello),
        (2, HandshakeType::ServerHello),
        (4, HandshakeType::NewSessionTicket),
        (8, HandshakeType::EncryptedExtensions),
        (11, HandshakeType::Certificate),
        (15, HandshakeType::CertificateVerify),
        (20, HandshakeType::Finished),
        (24, HandshakeType::KeyUpdate),
    ] {
        assert_eq!(HandshakeType::from_byte(byte), Ok(expected));
        assert_eq!(expected.to_byte(), byte);
    }
}

#[test]
fn the_client_hello_this_client_writes_is_the_one_it_was_asked_for() {
    let mut random = [0u8; 32];
    for (slot, value) in random.iter_mut().zip(0u8..) {
        *slot = value;
    }
    let mut session = [0u8; 32];
    for (slot, value) in session.iter_mut().zip(0x40u8..) {
        *slot = value;
    }
    let share: [u8; 32] = unhex(trace::CLIENT_PUBLIC_KEY)
        .try_into()
        .expect("the key is thirty-two bytes");

    let params = ClientHelloParams {
        random: &random,
        session_id: &session,
        suites: &[
            CipherSuite::Aes128GcmSha256,
            CipherSuite::Aes256GcmSha384,
            CipherSuite::ChaCha20Poly1305Sha256,
        ],
        key_share: &share,
        server_name: Some("server"),
        alpn: &[b"h2"],
    };

    let mut out = [0u8; 512];
    let length = write_client_hello(&params, &mut out).expect("the buffer is big enough");
    assert_eq!(
        hex(out.get(..length).unwrap_or(&[])),
        trace::OUR_CLIENT_HELLO.replace([' ', '\n', '\\'], "")
    );

    // And it frames as a message of the type it claims.
    let (kind, body, used) = read_message(out.get(..length).unwrap_or(&[]))
        .expect("a well formed message")
        .expect("the message is complete");
    assert_eq!(kind, HandshakeType::ClientHello);
    assert_eq!(used, length);
    assert_eq!(body.len(), length - 4);
}

#[test]
fn a_client_hello_without_a_name_or_a_protocol_leaves_those_out() {
    let random = [0u8; 32];
    let share = [0u8; 32];
    let params = ClientHelloParams {
        random: &random,
        session_id: &[],
        suites: &[CipherSuite::Aes128GcmSha256],
        key_share: &share,
        server_name: None,
        alpn: &[],
    };
    let mut with_none = [0u8; 512];
    let short = write_client_hello(&params, &mut with_none).expect("the buffer is big enough");

    let named = ClientHelloParams {
        server_name: Some("example.test"),
        alpn: &[b"http/1.1"],
        ..params
    };
    let mut with_both = [0u8; 512];
    let long = write_client_hello(&named, &mut with_both).expect("the buffer is big enough");
    assert!(long > short, "the two extensions take room");
}

#[test]
fn a_client_hello_that_does_not_fit_is_refused() {
    let random = [0u8; 32];
    let share = [0u8; 32];
    let params = ClientHelloParams {
        random: &random,
        session_id: &[],
        suites: &[CipherSuite::Aes128GcmSha256],
        key_share: &share,
        server_name: None,
        alpn: &[],
    };
    let mut small = [0u8; 32];
    assert_eq!(
        write_client_hello(&params, &mut small),
        Err(TlsError::BufferTooSmall)
    );
}

#[test]
fn the_messages_this_client_writes_itself_round_trip() {
    let mut out = [0u8; 64];
    let length = write_finished(&[0xAB; 32], &mut out).expect("the buffer is big enough");
    let (kind, body, _) = read_message(out.get(..length).unwrap_or(&[]))
        .expect("well formed")
        .expect("complete");
    assert_eq!(kind, HandshakeType::Finished);
    assert_eq!(body, &[0xAB; 32]);

    for request in [false, true] {
        let length = write_key_update(request, &mut out).expect("the buffer is big enough");
        let (kind, body, _) = read_message(out.get(..length).unwrap_or(&[]))
            .expect("well formed")
            .expect("complete");
        assert_eq!(kind, HandshakeType::KeyUpdate);
        assert_eq!(read_key_update(body), Ok(request));
    }

    assert_eq!(read_key_update(&[2]), Err(TlsError::IllegalParameter));
    assert_eq!(read_key_update(&[]), Err(TlsError::Decode));
    assert_eq!(read_key_update(&[0, 0]), Err(TlsError::Decode));
}

/// A `ServerHello` body, assembled from parts so that each rule can be
/// broken one at a time.
struct Hello<'a> {
    /// The random, which decides whether this is a retry.
    random: [u8; 32],
    /// The compression method, which must be zero.
    compression: u8,
    /// The suite code point.
    suite: u16,
    /// The extensions, already encoded.
    extensions: &'a [u8],
}

impl Hello<'_> {
    /// The body of the message.
    fn encode(&self) -> Vec<u8> {
        let mut buffer = [0u8; 512];
        let mut writer = Writer::new(&mut buffer);
        writer.u16(0x0303).expect("room");
        writer.bytes(&self.random).expect("room");
        writer.vector8(|id| id.bytes(&[])).expect("room");
        writer.u16(self.suite).expect("room");
        writer.u8(self.compression).expect("room");
        writer
            .vector16(|extensions| extensions.bytes(self.extensions))
            .expect("room");
        writer.written().to_vec()
    }
}

/// An extension with the given code point and body.
fn extension(kind: u16, body: &[u8]) -> Vec<u8> {
    let mut buffer = [0u8; 256];
    let mut writer = Writer::new(&mut buffer);
    writer.u16(kind).expect("room");
    writer.vector16(|value| value.bytes(body)).expect("room");
    writer.written().to_vec()
}

/// The version extension a server sends when it agrees.
fn version_extension(version: u16) -> Vec<u8> {
    extension(43, &version.to_be_bytes())
}

/// A key share extension with a full public value.
fn share_extension(group: u16) -> Vec<u8> {
    let mut body = [0u8; 128];
    let mut writer = Writer::new(&mut body);
    writer.u16(group).expect("room");
    writer
        .vector16(|value| value.bytes(&[0x11; 32]))
        .expect("room");
    extension(51, writer.written())
}

/// A well formed hello, to be broken by the caller.
fn good_hello() -> Hello<'static> {
    Hello {
        random: [0x22; 32],
        compression: 0,
        suite: 0x1301,
        extensions: &[],
    }
}

#[test]
fn a_server_hello_that_breaks_a_rule_is_refused() {
    let versions = version_extension(0x0304);
    let share = share_extension(0x001D);
    let both = [versions.clone(), share.clone()].concat();

    // The shape that is accepted, so that each rejection differs from it
    // in one thing only.
    let hello = Hello {
        extensions: &both,
        ..good_hello()
    };
    assert!(ServerHello::parse(&hello.encode()).is_ok());

    let no_version = Hello {
        extensions: &share,
        ..good_hello()
    };
    assert_eq!(
        ServerHello::parse(&no_version.encode()).err(),
        Some(TlsError::UnsupportedVersion)
    );

    let old_version = [version_extension(0x0303), share.clone()].concat();
    let old = Hello {
        extensions: &old_version,
        ..good_hello()
    };
    assert_eq!(
        ServerHello::parse(&old.encode()).err(),
        Some(TlsError::UnsupportedVersion)
    );

    let no_share = Hello {
        extensions: &versions,
        ..good_hello()
    };
    assert_eq!(
        ServerHello::parse(&no_share.encode()).err(),
        Some(TlsError::MissingExtension)
    );

    let other_group = [versions.clone(), share_extension(0x0017)].concat();
    let wrong_group = Hello {
        extensions: &other_group,
        ..good_hello()
    };
    assert_eq!(
        ServerHello::parse(&wrong_group.encode()).err(),
        Some(TlsError::IllegalParameter)
    );

    let twice = [versions.clone(), share.clone(), share.clone()].concat();
    let repeated = Hello {
        extensions: &twice,
        ..good_hello()
    };
    assert_eq!(
        ServerHello::parse(&repeated.encode()).err(),
        Some(TlsError::UnexpectedExtension)
    );

    let unknown = [both.clone(), extension(0x1234, &[])].concat();
    let stranger = Hello {
        extensions: &unknown,
        ..good_hello()
    };
    assert_eq!(
        ServerHello::parse(&stranger.encode()).err(),
        Some(TlsError::UnexpectedExtension),
        "a server may send only what was offered"
    );

    let compressed = Hello {
        compression: 1,
        extensions: &both,
        ..good_hello()
    };
    assert_eq!(
        ServerHello::parse(&compressed.encode()).err(),
        Some(TlsError::IllegalParameter)
    );

    for code in [0x1304u16, 0x009C, 0x0000] {
        let suite = Hello {
            suite: code,
            extensions: &both,
            ..good_hello()
        };
        assert_eq!(
            ServerHello::parse(&suite.encode()).err(),
            Some(TlsError::UnsupportedSuite),
            "suite {code:#06x}"
        );
    }
}

#[test]
fn a_retry_is_recognised_and_must_name_a_group() {
    let versions = version_extension(0x0304);
    let group = extension(51, &0x001Du16.to_be_bytes());

    let both = [versions.clone(), group].concat();
    let retry = Hello {
        random: RETRY_RANDOM,
        extensions: &both,
        ..good_hello()
    };
    let encoded = retry.encode();
    let parsed = ServerHello::parse(&encoded).expect("a well formed retry");
    assert!(parsed.is_retry);
    assert_eq!(parsed.retry_group, Some(0x001D));
    assert_eq!(parsed.key_share, None);

    let without = Hello {
        random: RETRY_RANDOM,
        extensions: &versions,
        ..good_hello()
    };
    assert_eq!(
        ServerHello::parse(&without.encode()).err(),
        Some(TlsError::MissingExtension)
    );
}

#[test]
fn the_sentinel_of_an_older_version_is_seen() {
    let versions = version_extension(0x0304);
    let share = share_extension(0x001D);
    let both = [versions, share].concat();

    for tail in [
        [0x44u8, 0x4F, 0x57, 0x4E, 0x47, 0x52, 0x44, 0x01],
        [0x44, 0x4F, 0x57, 0x4E, 0x47, 0x52, 0x44, 0x00],
    ] {
        let mut random = [0x22u8; 32];
        for (slot, byte) in random.iter_mut().skip(24).zip(tail) {
            *slot = byte;
        }
        let hello = Hello {
            random,
            extensions: &both,
            ..good_hello()
        };
        let encoded = hello.encode();
        let parsed = ServerHello::parse(&encoded).expect("well formed");
        assert!(
            parsed.is_downgrade(),
            "a server that would have spoken an older version says so in its random"
        );
    }

    let ordinary = Hello {
        extensions: &both,
        ..good_hello()
    };
    let encoded = ordinary.encode();
    let parsed = ServerHello::parse(&encoded).expect("well formed");
    assert!(!parsed.is_downgrade());
}

#[test]
fn a_malformed_message_is_refused_rather_than_guessed_at() {
    assert_eq!(ServerHello::parse(&[]).err(), Some(TlsError::Decode));
    assert_eq!(
        ServerHello::parse(&[0x03, 0x03]).err(),
        Some(TlsError::Decode)
    );
    assert_eq!(
        EncryptedExtensions::parse(&[]).err(),
        Some(TlsError::Decode)
    );
    assert_eq!(CertificateChain::parse(&[]).err(), Some(TlsError::Decode));
    assert_eq!(
        CertificateVerify::parse(&[0x08]).err(),
        Some(TlsError::Decode)
    );

    // A certificate message that a client did not ask for carries a
    // context, and a server must not send one.
    let mut buffer = [0u8; 32];
    let mut writer = Writer::new(&mut buffer);
    writer
        .vector8(|context| context.bytes(&[0x01]))
        .expect("room");
    writer.vector24(|entries| entries.bytes(&[])).expect("room");
    assert_eq!(
        CertificateChain::parse(writer.written()).err(),
        Some(TlsError::IllegalParameter)
    );
}

#[test]
fn the_reader_and_the_writer_agree_about_every_width() {
    let mut buffer = [0u8; 64];
    let mut writer = Writer::new(&mut buffer);
    assert!(writer.is_empty());
    writer.u8(0x01).expect("room");
    writer.u16(0x0203).expect("room");
    writer
        .vector8(|body| body.bytes(&[0xAA, 0xBB]))
        .expect("room");
    writer.vector16(|body| body.bytes(&[0xCC])).expect("room");
    writer
        .vector24(|body| body.bytes(&[0xDD, 0xEE]))
        .expect("room");
    let written = writer.written().to_vec();

    let mut reader = Reader::new(&written);
    assert_eq!(reader.u8(), Ok(0x01));
    assert_eq!(reader.u16(), Ok(0x0203));
    assert_eq!(reader.vector8(), Ok(&[0xAAu8, 0xBB][..]));
    assert_eq!(reader.vector16(), Ok(&[0xCCu8][..]));
    assert_eq!(reader.vector24(), Ok(&[0xDDu8, 0xEE][..]));
    assert!(reader.is_empty());
    assert_eq!(reader.finish(), Ok(()));

    let mut short = Reader::new(&[0x00]);
    assert_eq!(short.u16(), Err(TlsError::Decode));
    assert_eq!(short.u24(), Err(TlsError::Decode));
    assert_eq!(Reader::new(&[0x05, 0x01]).vector8(), Err(TlsError::Decode));
    assert_eq!(Reader::new(&[0x01]).finish(), Err(TlsError::Decode));

    let mut tiny = [0u8; 2];
    let mut cramped = Writer::new(&mut tiny);
    assert_eq!(
        cramped.vector8(|body| body.bytes(&[0x01, 0x02, 0x03])),
        Err(TlsError::BufferTooSmall)
    );
}

#[test]
fn a_vector_longer_than_its_length_can_say_is_refused() {
    let mut buffer = vec![0u8; 1024];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        writer.vector8(|body| body.bytes(&[0x00; 256])),
        Err(TlsError::Encode)
    );
}

#[test]
fn the_client_hello_of_the_trace_offers_the_key_the_document_names() {
    // The client of the trace is not this one, so its message is read
    // here by hand rather than by a decoder this crate does not need.
    let bytes = unhex(trace::CLIENT_HELLO);
    let (kind, body, used) = read_message(&bytes)
        .expect("a well formed message")
        .expect("the message is complete");
    assert_eq!(kind, HandshakeType::ClientHello);
    assert_eq!(used, bytes.len());

    let mut reader = Reader::new(body);
    assert_eq!(reader.u16(), Ok(0x0303), "the legacy version");
    let _random = reader.take(32).expect("the random is there");
    assert_eq!(reader.vector8(), Ok(&[][..]), "no session identifier");
    let suites = reader.vector16().expect("the suites are there");
    assert_eq!(suites.len(), 6, "three suites");
    assert_eq!(reader.vector8(), Ok(&[0u8][..]), "the null compression");

    let mut extensions = Reader::new(reader.vector16().expect("the extensions are there"));
    reader.finish().expect("nothing follows the extensions");

    let mut share = None;
    while !extensions.is_empty() {
        let kind = extensions.u16().expect("an extension has a type");
        let body = extensions.vector16().expect("an extension has a body");
        if kind == 51 {
            let mut value = Reader::new(body);
            let mut list = Reader::new(value.vector16().expect("the shares are a list"));
            assert_eq!(list.u16(), Ok(0x001D), "the group is x25519");
            share = Some(list.vector16().expect("the value is there").to_vec());
        }
    }
    assert_eq!(
        hex(&share.expect("the client offered a share")),
        trace::CLIENT_PUBLIC_KEY
    );
}

/// D-82, the half of it that offers. The `ClientHello` carries all nine
/// code points, the three `rsa_pkcs1_*` among them, because RFC 8446
/// section 4.2.3 gives those three exactly one meaning — that the client
/// can verify a certificate signed that way — and nearly every chain on
/// the public web is signed that way. The same section forbids them in a
/// `CertificateVerify`, which `super::machine` checks.
#[test]
fn the_client_hello_offers_the_nine_schemes_this_client_can_verify() {
    let random = [0u8; 32];
    let share = [0u8; 32];
    let params = ClientHelloParams {
        random: &random,
        session_id: &[],
        suites: &[CipherSuite::Aes128GcmSha256],
        key_share: &share,
        server_name: None,
        alpn: &[],
    };
    let mut out = [0u8; 512];
    let length = write_client_hello(&params, &mut out).expect("the buffer is big enough");
    let written = hex(out.get(..length).unwrap_or(&[]));

    // The extension: `000d`, a length of twenty, a list of eighteen, and
    // the nine code points in the order the writer emits them.
    assert!(
        written.contains("000d00140012040305030807080408050806040105010601"),
        "the signature_algorithms extension is not the nine schemes: {written}"
    );
    for scheme in [
        "0403", "0503", "0807", "0804", "0805", "0806", "0401", "0501", "0601",
    ] {
        assert!(
            written.contains(scheme),
            "the scheme {scheme} is not offered"
        );
    }
}
