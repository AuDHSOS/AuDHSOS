// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The client driven end to end against a server built in this file.
//!
//! The replay of RFC 8448 checks that this implementation agrees with a
//! foreign one; it cannot drive the state machine, for the reason that
//! file gives. This one drives the machine through a whole connection —
//! handshake, data both ways, key update, close — against a server that
//! uses the same pieces. The two tests cover different halves: one that
//! the arithmetic matches the world, the other that the machine does what
//! the arithmetic allows.

use audhsos_der::Timestamp;
use audhsos_x509::builder::{Params, TestKey, build};
use audhsos_x509::{Certificate, TrustAnchor, TrustAnchors};
use crypto_ec::x25519;
use crypto_rng::doubles::ScriptedRng;
use test_support::generators::bytes;
use test_support::property::check;

use crate::alert::Alert;
use crate::client::{Buffers, Connection, Event, MIN_HANDSHAKE, MIN_INCOMING, MIN_OUTGOING};
use crate::codec::{Reader, Writer};
use crate::config::ClientConfig;
use crate::error::TlsError;
use crate::handshake::{HandshakeType, read_message, write_finished};
use crate::keys::{Schedule, finished_key, traffic_keys, verify_data};
use crate::protection::RecordProtection;
use crate::record::{self, ContentType, HEADER_LEN};
use crate::suite::CipherSuite;
use crate::transcript::Transcript;

/// The name the server is asked for and answers to.
const NAME: &str = "example.test";
/// The secret of the authority the test trusts.
const ROOT_SECRET: [u8; 32] = [0x11; 32];
/// The secret of the server's own key.
const LEAF_SECRET: [u8; 32] = [0x22; 32];
/// The server's ephemeral private value.
const SERVER_EPHEMERAL: [u8; 32] = [0x33; 32];
/// The suite the server chooses.
const SUITE: CipherSuite = CipherSuite::Aes128GcmSha256;

/// What the server's chain and its `CertificateVerify` are signed with.
///
/// Most of this file uses Ed25519, because it is the shorter of the two
/// and the connection does not care. One test uses the other, so that the
/// path through `p256` is walked by a whole handshake and not only by the
/// unit tests underneath it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Authority {
    /// Ed25519 throughout, signature scheme `0x0807`.
    Ed25519,
    /// P-256 with SHA-256 throughout, signature scheme `0x0403`.
    EcdsaP256,
}

impl Authority {
    /// The root's key.
    fn root_key(self) -> TestKey {
        match self {
            Authority::Ed25519 => TestKey::Ed25519(ROOT_SECRET),
            Authority::EcdsaP256 => TestKey::EcdsaSha256(ROOT_SECRET),
        }
    }

    /// The server's key.
    fn leaf_key(self) -> TestKey {
        match self {
            Authority::Ed25519 => TestKey::Ed25519(LEAF_SECRET),
            Authority::EcdsaP256 => TestKey::EcdsaSha256(LEAF_SECRET),
        }
    }

    /// The code point the `CertificateVerify` names.
    fn scheme(self) -> u16 {
        match self {
            Authority::Ed25519 => 0x0807,
            Authority::EcdsaP256 => 0x0403,
        }
    }
}

/// The moment every window in this file contains.
fn now() -> Timestamp {
    Timestamp {
        year: 2025,
        month: 6,
        day: 15,
        hour: 12,
        minute: 0,
        second: 0,
    }
}

/// A window that contains it.
fn window() -> (Timestamp, Timestamp) {
    (
        Timestamp {
            year: 2020,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
        Timestamp {
            year: 2030,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            second: 59,
        },
    )
}

/// A certificate and the bytes it lives in.
struct Built {
    /// The encoding.
    bytes: [u8; 1024],
    /// How much of it is the certificate.
    length: usize,
}

impl Built {
    /// The encoded certificate.
    fn as_slice(&self) -> &[u8] {
        self.bytes.get(..self.length).unwrap_or(&[])
    }
}

/// Builds one certificate.
fn certificate(params: &Params<'_>, subject: TestKey, issuer: TestKey) -> Built {
    let mut bytes = [0u8; 1024];
    let length = build(params, subject, issuer, &mut bytes).expect("the parameters fit");
    Built { bytes, length }
}

/// The root the test trusts, signing with `kind`.
fn root_of(kind: Authority) -> Built {
    let (from, until) = window();
    certificate(
        &Params::authority("Root", "Root", None, from, until),
        kind.root_key(),
        kind.root_key(),
    )
}

/// The root of the tests that do not care which key it is.
fn root() -> Built {
    root_of(Authority::Ed25519)
}

/// The server's certificate under that root.
fn leaf() -> Built {
    leaf_of(Authority::Ed25519)
}

/// The server's certificate, under that root.
fn leaf_of(kind: Authority) -> Built {
    let (from, until) = window();
    let names = [NAME];
    certificate(
        &Params::leaf("Root", NAME, &names, from, until),
        kind.leaf_key(),
        kind.root_key(),
    )
}

/// The server side of one connection.
struct Server {
    /// The running hash, which must match the client's.
    transcript: Transcript,
    /// The schedule.
    schedule: Schedule,
    /// What the server sends the handshake under.
    write_handshake: RecordProtection,
    /// What the client sends the handshake under.
    read_handshake: RecordProtection,
    /// What the server sends data under.
    write_application: Option<RecordProtection>,
    /// What the client sends data under.
    read_application: Option<RecordProtection>,
    /// The key the client's `Finished` must carry.
    client_finished: Vec<u8>,
    /// The transcript the client's `Finished` is over.
    expected_transcript: Vec<u8>,
}

impl Server {
    /// Answers a `ClientHello` record with the whole server flight.
    fn answer(hello_record: &[u8], flight: &mut Vec<u8>) -> Server {
        Server::answer_as(Authority::Ed25519, hello_record, flight)
    }

    /// The same, with the chain and the signature of one `Authority`.
    #[expect(
        clippy::too_many_lines,
        reason = "one server flight, written in the order it goes on the wire"
    )]
    fn answer_as(authority: Authority, hello_record: &[u8], flight: &mut Vec<u8>) -> Server {
        let (header, body, _) = record::read(hello_record)
            .expect("the client's record is well formed")
            .expect("it is complete");
        assert_eq!(header.content_type, ContentType::Handshake);
        let (kind, hello, _) = read_message(body).expect("well formed").expect("complete");
        assert_eq!(kind, HandshakeType::ClientHello);

        // Read what the server needs out of it.
        let mut reader = Reader::new(hello);
        let _version = reader.u16().expect("a version");
        let _random = reader.take(32).expect("a random");
        let session_id = reader.vector8().expect("a session identifier").to_vec();
        let suites = reader.vector16().expect("the suites");
        assert!(
            suites
                .as_chunks::<2>()
                .0
                .iter()
                .any(|pair| *pair == SUITE.code().to_be_bytes()),
            "the client offers the suite this server picks"
        );
        let _compression = reader.vector8().expect("the compression methods");
        let mut extensions = Reader::new(reader.vector16().expect("the extensions"));

        let mut share = None;
        while !extensions.is_empty() {
            let kind = extensions.u16().expect("an extension type");
            let value = extensions.vector16().expect("an extension body");
            if kind == 51 {
                let mut list = Reader::new(Reader::new(value).vector16().expect("the shares"));
                assert_eq!(list.u16(), Ok(0x001D));
                share = Some(list.vector16().expect("the value").to_vec());
            }
        }
        let peer: [u8; 32] = share
            .expect("the client offered a share")
            .try_into()
            .expect("thirty-two bytes");

        // The server's own share, and the value they agree on.
        let public = x25519::base_point(&SERVER_EPHEMERAL);
        let shared = x25519::x25519(&SERVER_EPHEMERAL, &peer).expect("the peer value is usable");

        // The answer.
        let mut buffer = [0u8; 256];
        let mut writer = Writer::new(&mut buffer);
        writer
            .u8(HandshakeType::ServerHello.to_byte())
            .expect("room");
        writer
            .vector24(|body| {
                body.u16(0x0303)?;
                body.bytes(&[0x44; 32])?;
                body.vector8(|id| id.bytes(&session_id))?;
                body.u16(SUITE.code())?;
                body.u8(0)?;
                body.vector16(|extensions| {
                    extensions.u16(43)?;
                    extensions.vector16(|value| value.u16(0x0304))?;
                    extensions.u16(51)?;
                    extensions.vector16(|value| {
                        value.u16(0x001D)?;
                        value.vector16(|point| point.bytes(&public))
                    })
                })
            })
            .expect("room");
        let server_hello = writer.written().to_vec();

        let mut transcript = Transcript::new();
        transcript.update(body);
        transcript.update(&server_hello);

        let mut schedule = Schedule::new(SUITE);
        schedule.advance(&shared).expect("the value is well formed");
        let after_hellos = transcript.hash(SUITE);
        let client_secret = schedule
            .derive(b"c hs traffic", after_hellos.as_bytes())
            .expect("the derivation fits");
        let server_secret = schedule
            .derive(b"s hs traffic", after_hellos.as_bytes())
            .expect("the derivation fits");

        let mut server = Server {
            transcript,
            schedule,
            write_handshake: RecordProtection::new(
                traffic_keys(SUITE, server_secret.as_bytes()).expect("fits"),
            )
            .expect("fits"),
            read_handshake: RecordProtection::new(
                traffic_keys(SUITE, client_secret.as_bytes()).expect("fits"),
            )
            .expect("fits"),
            write_application: None,
            read_application: None,
            client_finished: finished_key(SUITE, client_secret.as_bytes())
                .expect("fits")
                .as_bytes()
                .to_vec(),
            expected_transcript: Vec::new(),
        };

        // The `ServerHello` goes out unprotected, and behind it the
        // compatibility record a server in this mode sends. Every
        // handshake in this file carries it, so every one of them checks
        // that the client passes over it.
        push_plain(flight, ContentType::Handshake, &server_hello);
        push_plain(flight, ContentType::ChangeCipherSpec, &[0x01]);

        // The flight the client must decrypt.
        let leaf = leaf_of(authority);
        let mut messages = Vec::new();
        messages.extend_from_slice(&encrypted_extensions());
        messages.extend_from_slice(&certificate_message(leaf.as_slice()));
        server.transcript.update(&messages);

        let signature_transcript = server.transcript.hash(SUITE);
        let verify = certificate_verify(authority, signature_transcript.as_bytes());
        messages.extend_from_slice(&verify);
        server.transcript.update(&verify);

        let before_finished = server.transcript.hash(SUITE);
        let code = verify_data(
            SUITE,
            finished_key(SUITE, server_secret.as_bytes())
                .expect("fits")
                .as_bytes(),
            before_finished.as_bytes(),
        );
        let mut finished = [0u8; 128];
        let length = write_finished(code.as_bytes(), &mut finished).expect("room");
        let finished = finished.get(..length).unwrap_or(&[]).to_vec();
        messages.extend_from_slice(&finished);
        server.transcript.update(&finished);

        let after_finished = server.transcript.hash(SUITE);
        server.expected_transcript = after_finished.as_bytes().to_vec();
        server.schedule.advance_to_master().expect("fits");
        let client_application = server
            .schedule
            .derive(b"c ap traffic", after_finished.as_bytes())
            .expect("fits");
        let server_application = server
            .schedule
            .derive(b"s ap traffic", after_finished.as_bytes())
            .expect("fits");
        server.write_application = Some(
            RecordProtection::new(
                traffic_keys(SUITE, server_application.as_bytes()).expect("fits"),
            )
            .expect("fits"),
        );
        server.read_application = Some(
            RecordProtection::new(
                traffic_keys(SUITE, client_application.as_bytes()).expect("fits"),
            )
            .expect("fits"),
        );

        let mut record = vec![0u8; 4096];
        let length = server
            .write_handshake
            .seal(ContentType::Handshake, &messages, &mut record)
            .expect("room");
        flight.extend_from_slice(record.get(..length).unwrap_or(&[]));
        server
    }

    /// Reads the client's `Finished` and checks it.
    fn take_client_finished(&mut self, record: &[u8]) {
        let (_, body, _) = record::read(record)
            .expect("well formed")
            .expect("complete");
        let head = record.get(..HEADER_LEN).unwrap_or(&[]).to_vec();
        let mut body = body.to_vec();
        let (kind, plaintext) = self
            .read_handshake
            .open(&head, &mut body)
            .expect("the client's record opens under the keys the server derived");
        assert_eq!(kind, ContentType::Handshake);

        let (kind, code, _) = read_message(plaintext)
            .expect("well formed")
            .expect("complete");
        assert_eq!(kind, HandshakeType::Finished);
        let expected = verify_data(SUITE, &self.client_finished, &self.expected_transcript);
        assert_eq!(
            expected.as_bytes(),
            code,
            "the client's code is the one the server computes"
        );
    }

    /// Seals application data for the client.
    fn send(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let mut record = vec![0u8; 4096];
        let length = self
            .write_application
            .as_mut()
            .expect("the handshake is done")
            .seal(ContentType::ApplicationData, plaintext, &mut record)
            .expect("room");
        record.get(..length).unwrap_or(&[]).to_vec()
    }

    /// Opens application data from the client.
    fn receive(&mut self, record: &[u8]) -> (ContentType, Vec<u8>) {
        let (_, body, _) = record::read(record)
            .expect("well formed")
            .expect("complete");
        let head = record.get(..HEADER_LEN).unwrap_or(&[]).to_vec();
        let mut body = body.to_vec();
        let (kind, plaintext) = self
            .read_application
            .as_mut()
            .expect("the handshake is done")
            .open(&head, &mut body)
            .expect("the record opens");
        (kind, plaintext.to_vec())
    }
}

/// An empty `EncryptedExtensions`.
fn encrypted_extensions() -> Vec<u8> {
    let mut buffer = [0u8; 16];
    let mut writer = Writer::new(&mut buffer);
    writer
        .u8(HandshakeType::EncryptedExtensions.to_byte())
        .expect("room");
    writer
        .vector24(|body| body.vector16(|_| Ok(())))
        .expect("room");
    writer.written().to_vec()
}

/// A `Certificate` message carrying one certificate.
fn certificate_message(certificate: &[u8]) -> Vec<u8> {
    let mut buffer = vec![0u8; 2048];
    let mut writer = Writer::new(&mut buffer);
    writer
        .u8(HandshakeType::Certificate.to_byte())
        .expect("room");
    writer
        .vector24(|body| {
            body.vector8(|context| context.bytes(&[]))?;
            body.vector24(|entries| {
                entries.vector24(|data| data.bytes(certificate))?;
                entries.vector16(|_| Ok(()))
            })
        })
        .expect("room");
    writer.written().to_vec()
}

/// A `CertificateVerify` over the transcript, signed with the leaf key.
fn certificate_verify(kind: Authority, transcript: &[u8]) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend(core::iter::repeat_n(0x20u8, 64));
    content.extend_from_slice(b"TLS 1.3, server CertificateVerify");
    content.push(0x00);
    content.extend_from_slice(transcript);
    let mut signature = [0u8; 128];
    let length = kind
        .leaf_key()
        .sign(&content, &mut signature)
        .expect("the key signs and the signature fits");
    let signature = signature.get(..length).unwrap_or(&[]).to_vec();

    let mut buffer = [0u8; 256];
    let mut writer = Writer::new(&mut buffer);
    writer
        .u8(HandshakeType::CertificateVerify.to_byte())
        .expect("room");
    writer
        .vector24(|body| {
            body.u16(kind.scheme())?;
            body.vector16(|value| value.bytes(&signature))
        })
        .expect("room");
    writer.written().to_vec()
}

/// Appends an unprotected record.
fn push_plain(out: &mut Vec<u8>, kind: ContentType, body: &[u8]) {
    let mut header = [0u8; HEADER_LEN];
    record::write_header(kind, body.len(), &mut header).expect("the length fits");
    out.extend_from_slice(&header);
    out.extend_from_slice(body);
}

/// Everything a connection needs to exist.
struct Fixture {
    /// The buffers.
    incoming: Vec<u8>,
    /// The buffers.
    outgoing: Vec<u8>,
    /// The buffers.
    handshake: Vec<u8>,
    /// The randomness the client draws from.
    script: Vec<u8>,
}

impl Fixture {
    /// A fixture with room and a script.
    fn new() -> Fixture {
        Fixture {
            incoming: vec![0u8; MIN_INCOMING],
            outgoing: vec![0u8; MIN_OUTGOING],
            handshake: vec![0u8; MIN_HANDSHAKE],
            script: (0u8..=255).cycle().take(256).collect(),
        }
    }
}

/// Runs a whole connection and returns what both sides said.
#[test]
fn a_connection_is_made_data_passes_both_ways_and_it_closes() {
    let root = root();
    let parsed = Certificate::parse(root.as_slice()).expect("a well formed root");
    let anchors = [TrustAnchor {
        subject: parsed.subject,
        spki: parsed.spki_bytes,
    }];
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());

    let mut fixture = Fixture::new();
    let mut rng = ScriptedRng::new(&fixture.script);
    let mut client = Connection::new(
        &config,
        &mut rng,
        Buffers {
            incoming: &mut fixture.incoming,
            outgoing: &mut fixture.outgoing,
            handshake: &mut fixture.handshake,
        },
    )
    .expect("the buffers are big enough");

    // The client's first flight: the hello and the compatibility record.
    let mut wire = vec![0u8; 4096];
    let written = client
        .write_tls(&mut wire)
        .expect("the buffer is big enough");
    let first = wire.get(..written).unwrap_or(&[]).to_vec();
    let (_, _, hello_len) = record::read(&first)
        .expect("well formed")
        .expect("complete");
    let hello_record = first.get(..hello_len).unwrap_or(&[]);

    let mut flight = Vec::new();
    let mut server = Server::answer(hello_record, &mut flight);

    // The client reads the flight and finishes.
    let taken = client.read_tls(&flight).expect("room");
    assert_eq!(taken, flight.len());
    assert_eq!(client.poll(), Ok(Event::WantsWrite));
    assert!(client.is_handshaked(), "the handshake is done");

    let written = client.write_tls(&mut wire).expect("room");
    server.take_client_finished(wire.get(..written).unwrap_or(&[]));

    // Data from the client to the server.
    let sent = client.send(b"a request").expect("the connection is up");
    assert_eq!(sent, 9);
    let written = client.write_tls(&mut wire).expect("room");
    let (kind, plaintext) = server.receive(wire.get(..written).unwrap_or(&[]));
    assert_eq!(kind, ContentType::ApplicationData);
    assert_eq!(plaintext, b"a request");

    // And back.
    let answer = server.send(b"an answer");
    client.read_tls(&answer).expect("room");
    let mut out = vec![0u8; 16_384];
    let length = client.recv(&mut out).expect("the record opens");
    assert_eq!(out.get(..length), Some(&b"an answer"[..]));

    // Nothing more has arrived.
    assert_eq!(client.recv(&mut out), Ok(0));

    // The client closes, and the server sees why.
    client.close().expect("the connection is up");
    let written = client.write_tls(&mut wire).expect("room");
    let (kind, plaintext) = server.receive(wire.get(..written).unwrap_or(&[]));
    assert_eq!(kind, ContentType::Alert);
    assert_eq!(plaintext, &[1, 0], "a warning that says close notify");
}

/// The same connection with the other signature algorithm, so that a
/// whole handshake walks the `p256` path and not only its unit tests.
#[test]
fn a_connection_is_made_when_the_server_signs_with_p256() {
    let root = root_of(Authority::EcdsaP256);
    let parsed = Certificate::parse(root.as_slice()).expect("a well formed root");
    let anchors = [TrustAnchor {
        subject: parsed.subject,
        spki: parsed.spki_bytes,
    }];
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());

    let mut fixture = Fixture::new();
    let mut rng = ScriptedRng::new(&fixture.script);
    let mut client = Connection::new(
        &config,
        &mut rng,
        Buffers {
            incoming: &mut fixture.incoming,
            outgoing: &mut fixture.outgoing,
            handshake: &mut fixture.handshake,
        },
    )
    .expect("the buffers are big enough");

    let mut wire = vec![0u8; 4096];
    let written = client.write_tls(&mut wire).expect("room");
    let first = wire.get(..written).unwrap_or(&[]).to_vec();
    let (_, _, hello_len) = record::read(&first)
        .expect("well formed")
        .expect("complete");

    let mut flight = Vec::new();
    let mut server = Server::answer_as(
        Authority::EcdsaP256,
        first.get(..hello_len).unwrap_or(&[]),
        &mut flight,
    );

    let taken = client.read_tls(&flight).expect("room");
    assert_eq!(taken, flight.len());
    assert_eq!(client.poll(), Ok(Event::WantsWrite));
    assert!(client.is_handshaked(), "the handshake is done");

    let written = client.write_tls(&mut wire).expect("room");
    server.take_client_finished(wire.get(..written).unwrap_or(&[]));

    let answer = server.send(b"an answer");
    client.read_tls(&answer).expect("room");
    let mut out = vec![0u8; 16_384];
    let length = client.recv(&mut out).expect("the record opens");
    assert_eq!(out.get(..length), Some(&b"an answer"[..]));
}

#[test]
fn a_server_that_closes_is_noticed() {
    let root = root();
    let parsed = Certificate::parse(root.as_slice()).expect("a well formed root");
    let anchors = [TrustAnchor {
        subject: parsed.subject,
        spki: parsed.spki_bytes,
    }];
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());

    let mut fixture = Fixture::new();
    let mut rng = ScriptedRng::new(&fixture.script);
    let mut client = Connection::new(
        &config,
        &mut rng,
        Buffers {
            incoming: &mut fixture.incoming,
            outgoing: &mut fixture.outgoing,
            handshake: &mut fixture.handshake,
        },
    )
    .expect("the buffers are big enough");

    let mut wire = vec![0u8; 4096];
    let written = client.write_tls(&mut wire).expect("room");
    let first = wire.get(..written).unwrap_or(&[]).to_vec();
    let (_, _, hello_len) = record::read(&first)
        .expect("well formed")
        .expect("complete");

    let mut flight = Vec::new();
    let mut server = Server::answer(first.get(..hello_len).unwrap_or(&[]), &mut flight);
    client.read_tls(&flight).expect("room");
    client.poll().expect("the handshake completes");
    let written = client.write_tls(&mut wire).expect("room");
    server.take_client_finished(wire.get(..written).unwrap_or(&[]));

    // The server says goodbye.
    let mut record = vec![0u8; 128];
    let length = server
        .write_application
        .as_mut()
        .expect("the handshake is done")
        .seal(ContentType::Alert, &[1, 0], &mut record)
        .expect("room");
    client
        .read_tls(record.get(..length).unwrap_or(&[]))
        .expect("room");

    let mut out = vec![0u8; 16_384];
    assert_eq!(client.recv(&mut out), Ok(0));
    assert_eq!(client.poll(), Ok(Event::PeerClosed));
}

#[test]
fn a_chain_that_does_not_reach_the_anchor_ends_the_connection() {
    // The client trusts a different root than the one that signed.
    let (from, until) = window();
    let stranger = certificate(
        &Params::authority("Elsewhere", "Elsewhere", None, from, until),
        TestKey::Ed25519([0x77; 32]),
        TestKey::Ed25519([0x77; 32]),
    );
    let parsed = Certificate::parse(stranger.as_slice()).expect("well formed");
    let anchors = [TrustAnchor {
        subject: parsed.subject,
        spki: parsed.spki_bytes,
    }];
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());

    let mut fixture = Fixture::new();
    let mut rng = ScriptedRng::new(&fixture.script);
    let mut client = Connection::new(
        &config,
        &mut rng,
        Buffers {
            incoming: &mut fixture.incoming,
            outgoing: &mut fixture.outgoing,
            handshake: &mut fixture.handshake,
        },
    )
    .expect("the buffers are big enough");

    let mut wire = vec![0u8; 4096];
    let written = client.write_tls(&mut wire).expect("room");
    let first = wire.get(..written).unwrap_or(&[]).to_vec();
    let (_, _, hello_len) = record::read(&first)
        .expect("well formed")
        .expect("complete");
    let mut flight = Vec::new();
    let _server = Server::answer(first.get(..hello_len).unwrap_or(&[]), &mut flight);

    client.read_tls(&flight).expect("room");
    assert_eq!(client.poll(), Err(TlsError::UnknownAuthority));

    // And it stays ended, with the same reason.
    assert_eq!(client.poll(), Err(TlsError::UnknownAuthority));
    assert_eq!(client.send(b"anything"), Err(TlsError::UnknownAuthority));
    let mut out = vec![0u8; 16_384];
    assert_eq!(client.recv(&mut out), Err(TlsError::UnknownAuthority));

    // The peer is told, and the alert can still be collected.
    let written = client.write_tls(&mut wire).expect("room");
    assert!(written > 0, "an alert is waiting to be sent");
}

#[test]
fn a_message_in_the_wrong_place_ends_the_connection() {
    let root = root();
    let parsed = Certificate::parse(root.as_slice()).expect("well formed");
    let anchors = [TrustAnchor {
        subject: parsed.subject,
        spki: parsed.spki_bytes,
    }];
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());

    let mut fixture = Fixture::new();
    let mut rng = ScriptedRng::new(&fixture.script);
    let mut client = Connection::new(
        &config,
        &mut rng,
        Buffers {
            incoming: &mut fixture.incoming,
            outgoing: &mut fixture.outgoing,
            handshake: &mut fixture.handshake,
        },
    )
    .expect("the buffers are big enough");

    // A `Finished` where a `ServerHello` belongs.
    let mut message = [0u8; 64];
    let length = write_finished(&[0u8; 32], &mut message).expect("room");
    let mut flight = Vec::new();
    push_plain(
        &mut flight,
        ContentType::Handshake,
        message.get(..length).unwrap_or(&[]),
    );

    client.read_tls(&flight).expect("room");
    assert_eq!(client.poll(), Err(TlsError::UnexpectedMessage));
}

#[test]
fn buffers_below_their_minimum_are_refused() {
    let root = root();
    let parsed = Certificate::parse(root.as_slice()).expect("well formed");
    let anchors = [TrustAnchor {
        subject: parsed.subject,
        spki: parsed.spki_bytes,
    }];
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());
    let script: Vec<u8> = (0u8..=255).collect();

    let mut small = vec![0u8; 16];
    let mut incoming = vec![0u8; MIN_INCOMING];
    let mut outgoing = vec![0u8; MIN_OUTGOING];
    let mut handshake = vec![0u8; MIN_HANDSHAKE];
    let mut rng = ScriptedRng::new(&script);
    assert_eq!(
        Connection::new(
            &config,
            &mut rng,
            Buffers {
                incoming: &mut small,
                outgoing: &mut outgoing,
                handshake: &mut handshake,
            },
        )
        .err(),
        Some(TlsError::BufferTooSmall)
    );

    let mut rng = ScriptedRng::new(&script);
    assert_eq!(
        Connection::new(
            &config,
            &mut rng,
            Buffers {
                incoming: &mut incoming,
                outgoing: &mut outgoing,
                handshake: &mut small,
            },
        )
        .err(),
        Some(TlsError::BufferTooSmall)
    );
}

/// A connection that has completed its handshake, with its server.
struct Connected<'a> {
    /// The client.
    client: Connection<'a>,
    /// The server it talked to.
    server: Server,
}

/// Runs a handshake and returns both sides.
fn connected<'a>(
    config: &'a ClientConfig<'a>,
    fixture: &'a mut Fixture,
    script: &'a [u8],
) -> Connected<'a> {
    let mut rng = ScriptedRng::new(script);
    let mut client = Connection::new(
        config,
        &mut rng,
        Buffers {
            incoming: &mut fixture.incoming,
            outgoing: &mut fixture.outgoing,
            handshake: &mut fixture.handshake,
        },
    )
    .expect("the buffers are big enough");

    let mut wire = vec![0u8; 4096];
    let written = client.write_tls(&mut wire).expect("room");
    let first = wire.get(..written).unwrap_or(&[]).to_vec();
    let (_, _, hello_len) = record::read(&first)
        .expect("well formed")
        .expect("complete");

    let mut flight = Vec::new();
    let mut server = Server::answer(first.get(..hello_len).unwrap_or(&[]), &mut flight);
    client.read_tls(&flight).expect("room");
    client.poll().expect("the handshake completes");
    let written = client.write_tls(&mut wire).expect("room");
    server.take_client_finished(wire.get(..written).unwrap_or(&[]));
    Connected { client, server }
}

/// The anchors the trusted root stands for.
fn trusted(root: &Built) -> [TrustAnchor<'_>; 1] {
    let parsed = Certificate::parse(root.as_slice()).expect("a well formed root");
    [TrustAnchor {
        subject: parsed.subject,
        spki: parsed.spki_bytes,
    }]
}

#[test]
fn a_key_update_moves_both_sides_on() {
    let root = root();
    let anchors = trusted(&root);
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());
    let mut fixture = Fixture::new();
    let script: Vec<u8> = (0u8..=255).collect();
    let Connected {
        mut client,
        mut server,
    } = connected(&config, &mut fixture, &script);

    // The server updates its keys and asks the client to do the same.
    let mut message = [0u8; 16];
    let length = crate::handshake::write_key_update(true, &mut message).expect("room");
    let update = message.get(..length).unwrap_or(&[]).to_vec();
    let mut record = vec![0u8; 128];
    let sealed = server
        .write_application
        .as_mut()
        .expect("the handshake is done")
        .seal(ContentType::Handshake, &update, &mut record)
        .expect("room");
    client
        .read_tls(record.get(..sealed).unwrap_or(&[]))
        .expect("room");

    let mut out = vec![0u8; 16_384];
    assert_eq!(client.recv(&mut out), Ok(0), "an update carries no data");

    // The client answered with one of its own, which the server reads
    // under the keys it had before its own change.
    let mut wire = vec![0u8; 4096];
    let written = client.write_tls(&mut wire).expect("room");
    assert!(
        written > 0,
        "the client answers an update that asks for one"
    );
    let (kind, plaintext) = server.receive(wire.get(..written).unwrap_or(&[]));
    assert_eq!(kind, ContentType::Handshake);
    assert_eq!(
        crate::handshake::read_key_update(
            crate::handshake::read_message(&plaintext)
                .expect("well formed")
                .expect("complete")
                .1
        ),
        Ok(false),
        "the client does not ask for another in return"
    );
}

#[test]
fn a_ticket_after_the_handshake_is_ignored() {
    let root = root();
    let anchors = trusted(&root);
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());
    let mut fixture = Fixture::new();
    let script: Vec<u8> = (0u8..=255).collect();
    let Connected {
        mut client,
        mut server,
    } = connected(&config, &mut fixture, &script);

    // A ticket, which this client resumes nothing with.
    let mut ticket = vec![4u8, 0x00, 0x00, 0x08];
    ticket.extend_from_slice(&[0u8; 8]);
    let mut record = vec![0u8; 128];
    let sealed = server
        .write_application
        .as_mut()
        .expect("the handshake is done")
        .seal(ContentType::Handshake, &ticket, &mut record)
        .expect("room");
    client
        .read_tls(record.get(..sealed).unwrap_or(&[]))
        .expect("room");

    let mut out = vec![0u8; 16_384];
    assert_eq!(client.recv(&mut out), Ok(0));

    // And data still flows afterwards.
    let answer = server.send(b"still here");
    client.read_tls(&answer).expect("room");
    let length = client.recv(&mut out).expect("the record opens");
    assert_eq!(out.get(..length), Some(&b"still here"[..]));
}

#[test]
fn data_before_the_handshake_and_a_short_buffer_are_refused() {
    let root = root();
    let anchors = trusted(&root);
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());
    let mut fixture = Fixture::new();
    let script: Vec<u8> = (0u8..=255).collect();
    let mut rng = ScriptedRng::new(&script);
    let mut client = Connection::new(
        &config,
        &mut rng,
        Buffers {
            incoming: &mut fixture.incoming,
            outgoing: &mut fixture.outgoing,
            handshake: &mut fixture.handshake,
        },
    )
    .expect("the buffers are big enough");

    assert_eq!(client.send(b"too early"), Err(TlsError::UnexpectedMessage));
    let mut small = vec![0u8; 16];
    assert_eq!(client.recv(&mut small), Err(TlsError::BufferTooSmall));

    // The first flight comes out in pieces when the buffer is small.
    let mut piece = vec![0u8; 32];
    let mut total = 0usize;
    loop {
        let written = client.write_tls(&mut piece).expect("room");
        if written == 0 {
            break;
        }
        total = total.wrapping_add(written);
    }
    assert!(total > 32, "the flight was longer than one piece");
    assert_eq!(client.poll(), Ok(Event::WantsRead));
}

#[test]
fn a_record_that_does_not_open_ends_the_connection() {
    let root = root();
    let anchors = trusted(&root);
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());
    let mut fixture = Fixture::new();
    let script: Vec<u8> = (0u8..=255).collect();
    let Connected {
        mut client,
        mut server,
    } = connected(&config, &mut fixture, &script);

    let mut answer = server.send(b"a message");
    if let Some(byte) = answer.last_mut() {
        *byte ^= 0x01;
    }
    client.read_tls(&answer).expect("room");
    let mut out = vec![0u8; 16_384];
    assert_eq!(client.recv(&mut out), Err(TlsError::BadRecord));
    assert_eq!(
        client.recv(&mut out),
        Err(TlsError::BadRecord),
        "and it stays"
    );

    // The alert that says why goes out under the keys of the epoch the
    // connection had reached, so the server can read it.
    let mut wire = vec![0u8; 4096];
    let written = client.write_tls(&mut wire).expect("room");
    let (kind, body) = server.receive(wire.get(..written).unwrap_or(&[]));
    assert_eq!(kind, ContentType::Alert);
    assert_eq!(
        body,
        &[2, Alert::BadRecordMac.code()],
        "a fatal alert that names the record"
    );
}

#[test]
fn a_hello_that_retries_or_offers_an_older_version_is_refused() {
    let root = root();
    let anchors = trusted(&root);
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());

    for (name, random, expected) in [
        (
            "a retry",
            crate::handshake::RETRY_RANDOM,
            TlsError::IllegalParameter,
        ),
        (
            "a downgrade",
            downgrade_random(),
            TlsError::IllegalParameter,
        ),
    ] {
        let mut fixture = Fixture::new();
        let script: Vec<u8> = (0u8..=255).collect();
        let mut rng = ScriptedRng::new(&script);
        let mut client = Connection::new(
            &config,
            &mut rng,
            Buffers {
                incoming: &mut fixture.incoming,
                outgoing: &mut fixture.outgoing,
                handshake: &mut fixture.handshake,
            },
        )
        .expect("the buffers are big enough");

        let mut wire = vec![0u8; 4096];
        client.write_tls(&mut wire).expect("room");

        let mut flight = Vec::new();
        push_plain(&mut flight, ContentType::Handshake, &hello_with(random));
        client.read_tls(&flight).expect("room");
        assert_eq!(client.poll(), Err(expected), "{name}");
    }
}

/// A random that ends in the sentinel of an older version.
fn downgrade_random() -> [u8; 32] {
    let mut random = [0x55u8; 32];
    for (slot, byte) in random
        .iter_mut()
        .skip(24)
        .zip([0x44u8, 0x4F, 0x57, 0x4E, 0x47, 0x52, 0x44, 0x01])
    {
        *slot = byte;
    }
    random
}

/// A `ServerHello` with the given random, otherwise well formed.
fn hello_with(random: [u8; 32]) -> Vec<u8> {
    let public = x25519::base_point(&SERVER_EPHEMERAL);
    let mut buffer = [0u8; 256];
    let mut writer = Writer::new(&mut buffer);
    writer
        .u8(HandshakeType::ServerHello.to_byte())
        .expect("room");
    writer
        .vector24(|body| {
            body.u16(0x0303)?;
            body.bytes(&random)?;
            body.vector8(|id| id.bytes(&[]))?;
            body.u16(SUITE.code())?;
            body.u8(0)?;
            body.vector16(|extensions| {
                extensions.u16(43)?;
                extensions.vector16(|value| value.u16(0x0304))?;
                extensions.u16(51)?;
                extensions.vector16(|value| {
                    if random == crate::handshake::RETRY_RANDOM {
                        value.u16(0x001D)
                    } else {
                        value.u16(0x001D)?;
                        value.vector16(|point| point.bytes(&public))
                    }
                })
            })
        })
        .expect("room");
    writer.written().to_vec()
}

#[test]
fn an_alert_from_the_server_during_the_handshake_ends_it() {
    let root = root();
    let anchors = trusted(&root);
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());
    let mut fixture = Fixture::new();
    let script: Vec<u8> = (0u8..=255).collect();
    let mut rng = ScriptedRng::new(&script);
    let mut client = Connection::new(
        &config,
        &mut rng,
        Buffers {
            incoming: &mut fixture.incoming,
            outgoing: &mut fixture.outgoing,
            handshake: &mut fixture.handshake,
        },
    )
    .expect("the buffers are big enough");
    let mut wire = vec![0u8; 4096];
    client.write_tls(&mut wire).expect("room");

    // A handshake failure, which is not a closing alert.
    let mut flight = Vec::new();
    push_plain(&mut flight, ContentType::Alert, &[2, 40]);
    client.read_tls(&flight).expect("room");
    assert_eq!(client.poll(), Err(TlsError::UnexpectedMessage));

    // Closing before there are keys is refused rather than sent in clear.
    assert_eq!(client.close(), Err(TlsError::UnexpectedMessage));
}

#[test]
fn more_bytes_than_the_buffer_holds_are_taken_in_pieces() {
    let root = root();
    let anchors = trusted(&root);
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());
    let mut fixture = Fixture::new();
    let script: Vec<u8> = (0u8..=255).collect();
    let mut rng = ScriptedRng::new(&script);
    let mut client = Connection::new(
        &config,
        &mut rng,
        Buffers {
            incoming: &mut fixture.incoming,
            outgoing: &mut fixture.outgoing,
            handshake: &mut fixture.handshake,
        },
    )
    .expect("the buffers are big enough");

    let flood = vec![0x16u8; MIN_INCOMING + 100];
    let taken = client.read_tls(&flood).expect("room for some of it");
    assert_eq!(taken, MIN_INCOMING, "what fits is taken and no more");
    assert_eq!(client.read_tls(&flood), Ok(0), "and then nothing does");
}

/// The window for the compatibility record closes with the server's
/// `Finished`, so one that arrives after it is an unexpected record.
#[test]
fn a_compatibility_record_after_the_handshake_is_refused() {
    let root = root();
    let anchors = trusted(&root);
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());
    let mut fixture = Fixture::new();
    let script: Vec<u8> = (0u8..=255).collect();
    let Connected { mut client, .. } = connected(&config, &mut fixture, &script);

    let mut late = Vec::new();
    push_plain(&mut late, ContentType::ChangeCipherSpec, &[0x01]);
    client.read_tls(&late).expect("room");
    let mut out = vec![0u8; 16_384];
    assert_eq!(client.recv(&mut out), Err(TlsError::UnexpectedMessage));
}

/// And so is a message that belongs to the handshake that is over.
#[test]
fn a_handshake_message_after_the_handshake_is_refused() {
    let root = root();
    let anchors = trusted(&root);
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());
    let mut fixture = Fixture::new();
    let script: Vec<u8> = (0u8..=255).collect();
    let Connected {
        mut client,
        mut server,
    } = connected(&config, &mut fixture, &script);

    let stray = certificate_message(leaf().as_slice());
    let mut record = vec![0u8; 2048];
    let sealed = server
        .write_application
        .as_mut()
        .expect("the handshake is done")
        .seal(ContentType::Handshake, &stray, &mut record)
        .expect("room");
    client
        .read_tls(record.get(..sealed).unwrap_or(&[]))
        .expect("room");
    let mut out = vec![0u8; 16_384];
    assert_eq!(client.recv(&mut out), Err(TlsError::UnexpectedMessage));
}

/// The compatibility record carries one byte, and that byte is one.
#[test]
fn a_compatibility_record_that_says_anything_else_is_refused() {
    let root = root();
    let anchors = trusted(&root);
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());

    for body in [&[0x02u8][..], &[][..], &[0x01, 0x01][..]] {
        let mut fixture = Fixture::new();
        let script: Vec<u8> = (0u8..=255).collect();
        let mut rng = ScriptedRng::new(&script);
        let mut client = Connection::new(
            &config,
            &mut rng,
            Buffers {
                incoming: &mut fixture.incoming,
                outgoing: &mut fixture.outgoing,
                handshake: &mut fixture.handshake,
            },
        )
        .expect("the buffers are big enough");

        let mut wire = vec![0u8; 4096];
        client.write_tls(&mut wire).expect("the hello goes out");

        let mut record = Vec::new();
        push_plain(&mut record, ContentType::ChangeCipherSpec, body);
        client.read_tls(&record).expect("room");
        assert_eq!(
            client.poll(),
            Err(TlsError::UnexpectedMessage),
            "a compatibility record of {body:?}"
        );
    }
}

/// Whatever arrives from the transport, the connection either makes sense
/// of it or ends; it never panics, and once it has ended it stays ended.
#[test]
fn property_any_bytes_from_the_transport_end_in_an_error_or_a_state() {
    check("tls_read_any_bytes", &bytes(0..=512), |input| {
        let root = root();
        let parsed = Certificate::parse(root.as_slice()).expect("a well formed root");
        let anchors = [TrustAnchor {
            subject: parsed.subject,
            spki: parsed.spki_bytes,
        }];
        let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), now());

        let mut fixture = Fixture::new();
        let mut rng = ScriptedRng::new(&fixture.script);
        let mut client = Connection::new(
            &config,
            &mut rng,
            Buffers {
                incoming: &mut fixture.incoming,
                outgoing: &mut fixture.outgoing,
                handshake: &mut fixture.handshake,
            },
        )
        .expect("the buffers are big enough");

        let mut wire = vec![0u8; 4096];
        client.write_tls(&mut wire).expect("the hello goes out");

        let taken = client.read_tls(input).map_err(|error| error.to_string())?;
        if taken != input.len() {
            return Err("the buffer holds a whole record and more".to_owned());
        }

        let first = client.poll();
        let again = client.poll();
        match first {
            // An error poisons the connection: the next call says the same.
            Err(error) => {
                if again != Err(error) {
                    return Err("a poisoned connection changed its mind".to_owned());
                }
            }
            // Nothing usable arrived, so nothing may have happened.
            Ok(event) => {
                if event != Event::WantsRead || again != Ok(Event::WantsRead) {
                    return Err("random bytes completed a handshake".to_owned());
                }
            }
        }
        Ok(())
    });
}
