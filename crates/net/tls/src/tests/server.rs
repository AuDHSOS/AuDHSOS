// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The client of this crate against the server of this crate.
//!
//! `tests/machine.rs` drives the client against a server written in that
//! file, message by message, so that a test can break one rule at a time.
//! This one drives it against [`crate::server`], which is the server the
//! acceptance run of Phase 15 starts on the development machine (D-148):
//! what it checks is that the two halves of this crate reach each other
//! over nothing but the bytes they exchange.

use audhsos_time::CivilTime;
use audhsos_x509::builder::{MAX_CERTIFICATE, Params, TestKey, build};
use audhsos_x509::{TrustAnchor, TrustAnchors};
use crypto_rng::doubles::ScriptedRng;

use crate::client::{self, Buffers, Event, MIN_HANDSHAKE, MIN_INCOMING, MIN_OUTGOING};
use crate::codec::Writer;
use crate::config::ClientConfig;
use crate::error::TlsError;
use crate::handshake::{ECDSA_SECP256R1_SHA256, ED25519, GROUP_X25519, HandshakeType};
use crate::record::{self, ContentType, HEADER_LEN, MAX_PLAINTEXT};
use crate::server::{self, ServerConfig};

/// The name the server answers to.
const NAME: &str = "example.test";
/// The secret of the authority both sides know.
const ROOT_SECRET: [u8; 32] = [0x41; 32];
/// The secret of the server's own key.
const LEAF_SECRET: [u8; 32] = [0x42; 32];
/// The server's private value of the key exchange.
const EPHEMERAL: [u8; 32] = [0x43; 32];
/// What the client says once the handshake is through.
const REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: example.test\r\n\r\n";
/// What the server answers.
const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi";

/// A certificate and the bytes it lives in.
struct Built {
    /// The encoding.
    bytes: [u8; MAX_CERTIFICATE],
    /// How much of it is the certificate.
    length: usize,
}

impl Built {
    /// The encoded certificate.
    fn as_slice(&self) -> &[u8] {
        self.bytes.get(..self.length).unwrap_or(&[])
    }
}

/// A moment inside the window of both certificates.
fn now() -> CivilTime {
    CivilTime {
        year: 2025,
        month: 6,
        day: 15,
        hour: 12,
        minute: 0,
        second: 0,
    }
}

/// The window they carry.
fn window() -> (CivilTime, CivilTime) {
    (
        CivilTime {
            year: 2020,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
        CivilTime {
            year: 2030,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            second: 59,
        },
    )
}

/// Builds one certificate.
fn certificate(params: &Params<'_>, subject: TestKey, issuer: TestKey) -> Built {
    let mut bytes = [0u8; MAX_CERTIFICATE];
    let length = build(params, subject, issuer, &mut bytes).expect("the parameters fit");
    Built { bytes, length }
}

/// The root both sides know.
fn root() -> Built {
    let (from, until) = window();
    certificate(
        &Params::authority("Root", "Root", None, from, until),
        TestKey::Ed25519(ROOT_SECRET),
        TestKey::Ed25519(ROOT_SECRET),
    )
}

/// The server's certificate under that root.
fn leaf() -> Built {
    let (from, until) = window();
    let names = [NAME];
    certificate(
        &Params::leaf("Root", NAME, &names, from, until),
        TestKey::Ed25519(LEAF_SECRET),
        TestKey::Ed25519(ROOT_SECRET),
    )
}

/// The buffers of one side.
struct Space {
    /// Bytes from the transport.
    incoming: Vec<u8>,
    /// Bytes for the transport.
    outgoing: Vec<u8>,
    /// Handshake messages.
    handshake: Vec<u8>,
}

impl Space {
    /// Space enough for either side.
    fn new() -> Space {
        Space {
            incoming: vec![0u8; MIN_INCOMING],
            outgoing: vec![0u8; MIN_OUTGOING],
            handshake: vec![0u8; MIN_HANDSHAKE],
        }
    }

    /// The buffers, as the constructors take them.
    fn buffers(&mut self) -> Buffers<'_> {
        Buffers {
            incoming: &mut self.incoming,
            outgoing: &mut self.outgoing,
            handshake: &mut self.handshake,
        }
    }
}

/// Everything the client needs to know.
fn client_config<'a>(anchors: &'a [TrustAnchor<'a>]) -> ClientConfig<'a> {
    ClientConfig::new(NAME, TrustAnchors::new(anchors), now())
}

/// What the server presents.
fn server_config<'a>(chain: &'a [&'a [u8]]) -> ServerConfig<'a> {
    ServerConfig {
        ephemeral: EPHEMERAL,
        ..ServerConfig::new(chain, TestKey::Ed25519(LEAF_SECRET), ED25519)
    }
}

/// Moves what one side wrote to the other, and says how many bytes moved.
fn carry(from: &mut impl Writes, to: &mut impl Reads) -> usize {
    let mut wire = vec![0u8; MAX_PLAINTEXT + 512];
    let taken = from.take(&mut wire).expect("the writer answers");
    let mut moved = 0usize;
    while moved < taken {
        let piece = wire.get(moved..taken).unwrap_or(&[]);
        let given = to.give(piece).expect("the reader answers");
        if given == 0 {
            break;
        }
        moved = moved.saturating_add(given);
    }
    moved
}

/// One end that writes bytes for the transport.
trait Writes {
    /// Takes what is waiting.
    fn take(&mut self, out: &mut [u8]) -> Result<usize, TlsError>;
}

/// One end that reads bytes from the transport.
trait Reads {
    /// Gives it bytes.
    fn give(&mut self, input: &[u8]) -> Result<usize, TlsError>;
}

impl Writes for client::Connection<'_> {
    fn take(&mut self, out: &mut [u8]) -> Result<usize, TlsError> {
        self.write_tls(out)
    }
}

impl Reads for client::Connection<'_> {
    fn give(&mut self, input: &[u8]) -> Result<usize, TlsError> {
        self.read_tls(input)
    }
}

impl Writes for server::Connection<'_> {
    fn take(&mut self, out: &mut [u8]) -> Result<usize, TlsError> {
        self.write_tls(out)
    }
}

impl Reads for server::Connection<'_> {
    fn give(&mut self, input: &[u8]) -> Result<usize, TlsError> {
        self.read_tls(input)
    }
}

#[test]
fn the_client_and_the_server_of_this_crate_reach_each_other() {
    let root = root();
    let leaf = leaf();
    let chain: [&[u8]; 2] = [leaf.as_slice(), root.as_slice()];
    let anchors = [TrustAnchor::from_certificate(root.as_slice()).expect("a well formed root")];

    let client_config = client_config(&anchors);
    let server_config = server_config(&chain);
    let script: Vec<u8> = (0u8..=255).collect();
    let mut rng = ScriptedRng::new(&script);

    let mut client_room = Space::new();
    let mut server_room = Space::new();
    let mut client = client::Connection::new(&client_config, &mut rng, client_room.buffers())
        .expect("the client starts");
    let mut server =
        server::Connection::new(&server_config, server_room.buffers()).expect("the server starts");

    // The handshake: each side is polled and what it wrote is carried to
    // the other, until both say they are through.
    for _ in 0..8 {
        client.poll().expect("the client makes progress");
        carry(&mut client, &mut server);
        server.poll().expect("the server makes progress");
        carry(&mut server, &mut client);
        if client.is_handshaked() && server.is_handshaked() {
            break;
        }
    }
    assert!(client.is_handshaked(), "the client finished the handshake");
    assert!(server.is_handshaked(), "the server finished the handshake");

    // A request one way and a response the other.
    client.send(REQUEST).expect("the request is sealed");
    carry(&mut client, &mut server);
    let mut read = vec![0u8; MAX_PLAINTEXT];
    let length = server.recv(&mut read).expect("the request opens");
    assert_eq!(read.get(..length), Some(REQUEST));

    server.send(RESPONSE).expect("the response is sealed");
    carry(&mut server, &mut client);
    let length = client.recv(&mut read).expect("the response opens");
    assert_eq!(read.get(..length), Some(RESPONSE));

    // And the close, from the server. A `close_notify` of the connection
    // arrives as a record of application data, so it is `recv` that opens
    // it and `poll` that then says the peer is done.
    server.close().expect("the close notify is sealed");
    carry(&mut server, &mut client);
    assert_eq!(client.recv(&mut read).expect("the alert opens"), 0);
    assert_eq!(
        client.poll().expect("the client reads it"),
        Event::PeerClosed
    );
}

#[test]
fn the_server_chooses_the_protocol_the_client_offers_and_no_other() {
    let root = root();
    let leaf = leaf();
    let chain: [&[u8]; 2] = [leaf.as_slice(), root.as_slice()];
    let anchors = [TrustAnchor::from_certificate(root.as_slice()).expect("a well formed root")];

    let offered: [&[u8]; 1] = [b"http/1.1"];
    let client_config = ClientConfig {
        alpn: &offered,
        ..client_config(&anchors)
    };
    let server_config = ServerConfig {
        alpn: b"http/1.1",
        ephemeral: EPHEMERAL,
        ..ServerConfig::new(&chain, TestKey::Ed25519(LEAF_SECRET), ED25519)
    };

    let script: Vec<u8> = (0u8..=255).collect();
    let mut rng = ScriptedRng::new(&script);
    let mut client_room = Space::new();
    let mut server_room = Space::new();
    let mut client = client::Connection::new(&client_config, &mut rng, client_room.buffers())
        .expect("the client starts");
    let mut server =
        server::Connection::new(&server_config, server_room.buffers()).expect("the server starts");

    for _ in 0..8 {
        client.poll().expect("the client makes progress");
        carry(&mut client, &mut server);
        server.poll().expect("the server makes progress");
        carry(&mut server, &mut client);
        if client.is_handshaked() && server.is_handshaked() {
            break;
        }
    }
    assert_eq!(client.alpn(), Some(&b"http/1.1"[..]));
    assert_eq!(server.alpn(), Some(&b"http/1.1"[..]));
}

#[test]
fn a_server_with_no_chain_and_one_with_too_short_a_buffer_are_refused() {
    let empty: [&[u8]; 0] = [];
    let config = server_config(&empty);
    let mut space = Space::new();
    assert_eq!(
        server::Connection::new(&config, space.buffers()).err(),
        Some(TlsError::BadCertificate)
    );

    let authority = root();
    let chain: [&[u8]; 1] = [authority.as_slice()];
    let config = server_config(&chain);
    let mut incoming = vec![0u8; MIN_INCOMING - 1];
    let mut outgoing = vec![0u8; MIN_OUTGOING];
    let mut handshake = vec![0u8; MIN_HANDSHAKE];
    let buffers = Buffers {
        incoming: &mut incoming,
        outgoing: &mut outgoing,
        handshake: &mut handshake,
    };
    assert_eq!(
        server::Connection::new(&config, buffers).err(),
        Some(TlsError::BufferTooSmall)
    );
}

#[test]
fn a_client_that_asked_for_another_name_refuses_the_chain() {
    let authority = root();
    let server_certificate = leaf();
    let chain: [&[u8]; 2] = [server_certificate.as_slice(), authority.as_slice()];
    let anchors =
        [TrustAnchor::from_certificate(authority.as_slice()).expect("a well formed root")];

    // The chain reaches the anchor, so what is wrong is the certificate
    // and not the authority: `bad_certificate` and not `unknown_ca`.
    let client_config = ClientConfig::new("somewhere.else", TrustAnchors::new(&anchors), now());
    let server_config = server_config(&chain);
    let script: Vec<u8> = (0u8..=255).collect();
    let mut rng = ScriptedRng::new(&script);
    let mut client_room = Space::new();
    let mut server_room = Space::new();
    let mut client = client::Connection::new(&client_config, &mut rng, client_room.buffers())
        .expect("the client starts");
    let mut server =
        server::Connection::new(&server_config, server_room.buffers()).expect("the server starts");

    let mut outcome = Ok(Event::WantsRead);
    for _ in 0..8 {
        outcome = client.poll();
        if outcome.is_err() {
            break;
        }
        carry(&mut client, &mut server);
        if server.poll().is_err() {
            break;
        }
        carry(&mut server, &mut client);
    }
    assert_eq!(outcome, Err(TlsError::BadCertificate));
}

/// The code point of the suite this server prefers.
const AES128: u16 = 0x1301;
/// The code point of the suite it takes when that one is not offered.
const CHACHA20: u16 = 0x1303;

/// What a hand-written `ClientHello` says, so that one test can change
/// one field of it and leave the rest alone.
#[derive(Clone, Copy)]
struct Offer {
    /// The suites, as code points.
    suites: &'static [u16],
    /// The versions of `supported_versions`, or `None` for no extension.
    versions: Option<&'static [u16]>,
    /// The group of the one key share, or `None` for no extension.
    group: Option<u16>,
    /// The schemes of `signature_algorithms`, or `None` for no extension.
    schemes: Option<&'static [u16]>,
    /// The protocols of the ALPN extension, or `None` for no extension.
    protocols: Option<&'static [&'static [u8]]>,
}

impl Default for Offer {
    fn default() -> Offer {
        Offer {
            suites: &[AES128],
            versions: Some(&[0x0304]),
            group: Some(GROUP_X25519),
            schemes: Some(&[ED25519]),
            protocols: None,
        }
    }
}

/// Writes the record of a `ClientHello` that says what `offer` says.
fn hello(offer: Offer) -> Vec<u8> {
    let mut body = vec![0u8; 1024];
    let length = {
        let mut writer = Writer::new(&mut body);
        writer
            .u8(HandshakeType::ClientHello.to_byte())
            .expect("space");
        writer
            .vector24(|message| {
                message.u16(0x0303)?;
                message.bytes(&[0x11; 32])?;
                message.vector8(|id| id.bytes(&[0x22; 32]))?;
                message.vector16(|suites| {
                    for suite in offer.suites {
                        suites.u16(*suite)?;
                    }
                    Ok(())
                })?;
                message.vector8(|compression| compression.u8(0))?;
                message.vector16(|extensions| {
                    if let Some(versions) = offer.versions {
                        extensions.u16(43)?;
                        extensions.vector16(|value| {
                            value.vector8(|list| {
                                for version in versions {
                                    list.u16(*version)?;
                                }
                                Ok(())
                            })
                        })?;
                    }
                    if let Some(group) = offer.group {
                        extensions.u16(51)?;
                        extensions.vector16(|value| {
                            value.vector16(|shares| {
                                shares.u16(group)?;
                                shares.vector16(|point| point.bytes(&[0x33; 32]))
                            })
                        })?;
                    }
                    if let Some(schemes) = offer.schemes {
                        extensions.u16(13)?;
                        extensions.vector16(|value| {
                            value.vector16(|list| {
                                for scheme in schemes {
                                    list.u16(*scheme)?;
                                }
                                Ok(())
                            })
                        })?;
                    }
                    if let Some(protocols) = offer.protocols {
                        extensions.u16(16)?;
                        extensions.vector16(|value| {
                            value.vector16(|list| {
                                for protocol in protocols {
                                    list.vector8(|name| name.bytes(protocol))?;
                                }
                                Ok(())
                            })
                        })?;
                    }
                    Ok(())
                })
            })
            .expect("space");
        writer.len()
    };
    let message = body.get(..length).unwrap_or(&[]);

    let mut out = vec![0u8; HEADER_LEN.saturating_add(length)];
    record::write_header(ContentType::Handshake, length, &mut out).expect("the length fits");
    let space = out.get_mut(HEADER_LEN..).expect("space");
    space.copy_from_slice(message);
    out
}

/// Starts a server, hands it `record`, and answers what it makes of it.
fn against(record: &[u8]) -> Result<Event, TlsError> {
    let root = root();
    let leaf = leaf();
    let chain: [&[u8]; 2] = [leaf.as_slice(), root.as_slice()];
    let config = server_config(&chain);
    let mut space = Space::new();
    let mut server = server::Connection::new(&config, space.buffers()).expect("the server starts");
    server.read_tls(record).expect("the bytes are taken");
    server.poll()
}

#[test]
fn a_hello_this_server_can_answer_is_answered() {
    assert_eq!(against(&hello(Offer::default())), Ok(Event::WantsWrite));
}

#[test]
fn a_hello_without_the_extensions_this_server_needs_is_refused() {
    let no_versions = Offer {
        versions: None,
        ..Offer::default()
    };
    assert_eq!(
        against(&hello(no_versions)),
        Err(TlsError::MissingExtension)
    );

    let no_share = Offer {
        group: None,
        ..Offer::default()
    };
    assert_eq!(against(&hello(no_share)), Err(TlsError::MissingExtension));

    let no_schemes = Offer {
        schemes: None,
        ..Offer::default()
    };
    assert_eq!(against(&hello(no_schemes)), Err(TlsError::MissingExtension));
}

#[test]
fn a_hello_that_offers_another_version_another_suite_or_another_group_is_refused() {
    let older = Offer {
        versions: Some(&[0x0303]),
        ..Offer::default()
    };
    assert_eq!(against(&hello(older)), Err(TlsError::UnsupportedVersion));

    let strange_suite = Offer {
        suites: &[0x1234],
        ..Offer::default()
    };
    assert_eq!(
        against(&hello(strange_suite)),
        Err(TlsError::UnsupportedSuite)
    );

    // A client that offers only P-256 would need a `HelloRetryRequest`,
    // which this server does not send.
    let other_group = Offer {
        group: Some(0x0017),
        ..Offer::default()
    };
    assert_eq!(
        against(&hello(other_group)),
        Err(TlsError::MissingExtension)
    );
}

#[test]
fn a_client_that_would_not_take_this_server_s_signature_is_refused() {
    // The server signs with Ed25519; a client that accepts only P-256
    // gets `handshake_failure`, which is what RFC 8446, section 4.4.2.2
    // asks for.
    let other = Offer {
        schemes: Some(&[ECDSA_SECP256R1_SHA256]),
        ..Offer::default()
    };
    assert_eq!(against(&hello(other)), Err(TlsError::UnsupportedSuite));
}

#[test]
fn the_server_takes_the_second_suite_when_the_first_is_not_offered() {
    let second = Offer {
        suites: &[CHACHA20],
        ..Offer::default()
    };
    assert_eq!(against(&hello(second)), Ok(Event::WantsWrite));
}

#[test]
fn a_protocol_this_server_does_not_offer_is_not_chosen() {
    let root = root();
    let leaf = leaf();
    let chain: [&[u8]; 2] = [leaf.as_slice(), root.as_slice()];
    let config = ServerConfig {
        alpn: b"http/1.1",
        ephemeral: EPHEMERAL,
        ..ServerConfig::new(&chain, TestKey::Ed25519(LEAF_SECRET), ED25519)
    };
    let mut space = Space::new();
    let mut server = server::Connection::new(&config, space.buffers()).expect("the server starts");
    let offered: &[&[u8]] = &[b"h2"];
    let record = hello(Offer {
        protocols: Some(offered),
        ..Offer::default()
    });
    server.read_tls(&record).expect("the bytes are taken");
    assert_eq!(server.poll(), Ok(Event::WantsWrite));
    assert_eq!(server.alpn(), None);
}

#[test]
fn a_record_that_belongs_to_a_later_moment_is_refused() {
    // Application data before there are keys to open it under.
    let mut record = vec![0u8; HEADER_LEN + 4];
    record::write_header(ContentType::ApplicationData, 4, &mut record).expect("the length fits");
    assert_eq!(against(&record), Err(TlsError::UnexpectedMessage));

    // A compatibility record that carries something else.
    let mut other = vec![0u8; HEADER_LEN + 1];
    record::write_header(ContentType::ChangeCipherSpec, 1, &mut other).expect("the length fits");
    other[HEADER_LEN] = 0x02;
    assert_eq!(against(&other), Err(TlsError::UnexpectedMessage));

    // A `Finished` before the `ClientHello`.
    let mut finished = vec![0u8; HEADER_LEN + 4];
    record::write_header(ContentType::Handshake, 4, &mut finished).expect("the length fits");
    finished[HEADER_LEN] = HandshakeType::Finished.to_byte();
    assert_eq!(against(&finished), Err(TlsError::UnexpectedMessage));
}

#[test]
fn an_alert_of_the_client_is_taken_and_a_fatal_one_ends_the_connection() {
    let mut closed = vec![0u8; HEADER_LEN + 2];
    record::write_header(ContentType::Alert, 2, &mut closed).expect("the length fits");
    closed[HEADER_LEN] = crate::alert::WARNING;
    closed[HEADER_LEN + 1] = crate::alert::Alert::CloseNotify.code();
    assert_eq!(against(&closed), Ok(Event::PeerClosed));

    let mut fatal = vec![0u8; HEADER_LEN + 2];
    record::write_header(ContentType::Alert, 2, &mut fatal).expect("the length fits");
    fatal[HEADER_LEN] = crate::alert::FATAL;
    fatal[HEADER_LEN + 1] = crate::alert::Alert::HandshakeFailure.code();
    assert_eq!(
        against(&fatal),
        Err(TlsError::PeerAlert(
            crate::alert::Alert::HandshakeFailure.code()
        ))
    );
}

#[test]
fn a_server_before_its_handshake_sends_nothing_and_stays_ended_afterwards() {
    let root = root();
    let leaf = leaf();
    let chain: [&[u8]; 2] = [leaf.as_slice(), root.as_slice()];
    let config = server_config(&chain);
    let mut space = Space::new();
    let mut server = server::Connection::new(&config, space.buffers()).expect("the server starts");

    assert_eq!(server.send(b"too early"), Err(TlsError::UnexpectedMessage));
    let mut small = vec![0u8; 8];
    assert_eq!(server.recv(&mut small), Err(TlsError::BufferTooSmall));

    // A hello it cannot answer poisons it, and every later call answers
    // with the same reason.
    let record = hello(Offer {
        suites: &[0x1234],
        ..Offer::default()
    });
    server.read_tls(&record).expect("the bytes are taken");
    assert_eq!(server.poll(), Err(TlsError::UnsupportedSuite));
    assert_eq!(server.poll(), Err(TlsError::UnsupportedSuite));
    assert_eq!(server.send(b"anything"), Err(TlsError::UnsupportedSuite));
    let mut out = vec![0u8; MAX_PLAINTEXT];
    assert_eq!(server.recv(&mut out), Err(TlsError::UnsupportedSuite));

    // And it wrote the alert that says why.
    let mut wire = vec![0u8; 64];
    let written = server.write_tls(&mut wire).expect("the alert is waiting");
    assert!(written > 0, "the client is told");
}

#[test]
fn the_server_hands_its_flight_over_in_as_many_pieces_as_the_caller_takes() {
    let root = root();
    let leaf = leaf();
    let chain: [&[u8]; 2] = [leaf.as_slice(), root.as_slice()];
    let config = server_config(&chain);
    let mut space = Space::new();
    let mut server = server::Connection::new(&config, space.buffers()).expect("the server starts");
    server
        .read_tls(&hello(Offer::default()))
        .expect("the bytes are taken");
    assert_eq!(server.poll(), Ok(Event::WantsWrite));

    // A caller with space for sixteen bytes gets sixteen at a time, and the
    // rest stays where it is.
    let mut piece = [0u8; 16];
    let mut moved = 0usize;
    loop {
        let taken = server.write_tls(&mut piece).expect("the writer answers");
        if taken == 0 {
            break;
        }
        moved = moved.saturating_add(taken);
    }
    assert!(moved > 16, "the flight is longer than one piece");
    assert_eq!(server.poll(), Ok(Event::WantsRead));
}

#[test]
fn a_reader_that_is_full_takes_what_fits_and_no_more() {
    let root = root();
    let leaf = leaf();
    let chain: [&[u8]; 2] = [leaf.as_slice(), root.as_slice()];
    let config = server_config(&chain);
    let mut space = Space::new();
    let mut server = server::Connection::new(&config, space.buffers()).expect("the server starts");

    let flood = vec![0u8; MIN_INCOMING + 64];
    let taken = server.read_tls(&flood).expect("the bytes are taken");
    assert_eq!(taken, MIN_INCOMING);
    assert_eq!(server.read_tls(&flood), Ok(0));
}

#[test]
fn an_alert_of_one_byte_is_no_alert() {
    let mut short = vec![0u8; HEADER_LEN + 1];
    record::write_header(ContentType::Alert, 1, &mut short).expect("the length fits");
    short[HEADER_LEN] = crate::alert::WARNING;
    assert_eq!(against(&short), Err(TlsError::BadRecord));
}

#[test]
fn a_protocol_list_that_is_no_list_chooses_nothing() {
    let root = root();
    let leaf = leaf();
    let chain: [&[u8]; 2] = [leaf.as_slice(), root.as_slice()];
    let config = ServerConfig {
        alpn: b"http/1.1",
        ephemeral: EPHEMERAL,
        ..ServerConfig::new(&chain, TestKey::Ed25519(LEAF_SECRET), ED25519)
    };
    let mut space = Space::new();
    let mut server = server::Connection::new(&config, space.buffers()).expect("the server starts");

    // The extension is there and its length is one no list has: the
    // server answers without a protocol rather than failing.
    let broken = broken_alpn();
    server.read_tls(&broken).expect("the bytes are taken");
    assert_eq!(server.poll(), Ok(Event::WantsWrite));
    assert_eq!(server.alpn(), None);
}

/// A `ClientHello` whose ALPN extension carries a length no list has.
fn broken_alpn() -> Vec<u8> {
    let mut record = hello(Offer {
        protocols: Some(&[b"http/1.1"]),
        ..Offer::default()
    });
    // The last bytes of the message are the ALPN extension; a length of
    // one where the list is longer makes the vector unreadable.
    let last = record.len().saturating_sub(10);
    if let Some(slot) = record.get_mut(last) {
        *slot = 0xff;
    }
    record
}

#[test]
fn a_close_of_the_client_reaches_the_server_after_the_handshake() {
    let root = root();
    let leaf = leaf();
    let chain: [&[u8]; 2] = [leaf.as_slice(), root.as_slice()];
    let anchors = [TrustAnchor::from_certificate(root.as_slice()).expect("a well formed root")];
    let client_config = client_config(&anchors);
    let server_config = server_config(&chain);

    let script: Vec<u8> = (0u8..=255).collect();
    let mut rng = ScriptedRng::new(&script);
    let mut client_room = Space::new();
    let mut server_room = Space::new();
    let mut client = client::Connection::new(&client_config, &mut rng, client_room.buffers())
        .expect("the client starts");
    let mut server =
        server::Connection::new(&server_config, server_room.buffers()).expect("the server starts");
    for _ in 0..8 {
        client.poll().expect("the client makes progress");
        carry(&mut client, &mut server);
        server.poll().expect("the server makes progress");
        carry(&mut server, &mut client);
        if client.is_handshaked() && server.is_handshaked() {
            break;
        }
    }
    assert!(server.is_handshaked(), "the handshake is through");

    // The client closes, which arrives as a record of application data
    // and is what `recv` opens.
    client.close().expect("the close notify is sealed");
    carry(&mut client, &mut server);
    let mut out = vec![0u8; MAX_PLAINTEXT];
    assert_eq!(server.recv(&mut out), Ok(0));
    assert_eq!(server.poll(), Ok(Event::PeerClosed));
}

#[test]
fn a_server_that_closes_before_it_has_keys_says_so_and_stays_closed() {
    let root = root();
    let leaf = leaf();
    let chain: [&[u8]; 2] = [leaf.as_slice(), root.as_slice()];
    let config = server_config(&chain);
    let mut space = Space::new();
    let mut server = server::Connection::new(&config, space.buffers()).expect("the server starts");

    // There are no application keys yet, so there is nothing to seal the
    // `close_notify` under.
    assert_eq!(server.close(), Err(TlsError::UnexpectedMessage));
    assert_eq!(server.poll(), Ok(Event::WantsRead));
    assert!(!server.is_handshaked());
}

#[test]
fn a_chain_longer_than_this_server_sends_is_refused() {
    let authority = root();
    let certificate = authority.as_slice();
    let chain: [&[u8]; 5] = [
        certificate,
        certificate,
        certificate,
        certificate,
        certificate,
    ];
    let config = server_config(&chain);
    let mut space = Space::new();
    assert_eq!(
        server::Connection::new(&config, space.buffers()).err(),
        Some(TlsError::BadCertificate)
    );
}
