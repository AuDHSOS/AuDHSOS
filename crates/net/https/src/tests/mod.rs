// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `exchange` against the test server of `audhsos-tls`, over an in-memory
//! transport that answers a scripted response.

use audhsos_time::CivilTime;
use audhsos_tls::client::{Buffers as TlsBuffers, MIN_HANDSHAKE, MIN_INCOMING, MIN_OUTGOING};
use audhsos_tls::config::ClientConfig;
use audhsos_tls::handshake::ED25519;
use audhsos_tls::record::MAX_PLAINTEXT;
use audhsos_tls::server::{Connection as Server, ServerConfig};
use audhsos_tls::{TlsError, client};
use audhsos_x509::builder::{MAX_CERTIFICATE, Params, TestKey, build};
use audhsos_x509::{TrustAnchor, TrustAnchors};
use crypto_rng::doubles::ScriptedRng;
use net_http::{Head, HttpError, Method, Request};

use crate::error::HttpsError;
use crate::exchange::{Buffers, Handler, Refused, Transport, exchange};

/// The name the server answers to.
const NAME: &str = "example.test";
/// The secret of the root.
const ROOT_SECRET: [u8; 32] = [0x41; 32];
/// The secret of the server's key.
const LEAF_SECRET: [u8; 32] = [0x42; 32];

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

/// A moment of the given year.
const fn moment(year: i32, month: u8, day: u8) -> CivilTime {
    CivilTime {
        year,
        month,
        day,
        hour: 12,
        minute: 0,
        second: 0,
    }
}

/// One certificate, valid from 2020 to 2030.
fn certificate(params: &Params<'_>, subject: TestKey) -> Built {
    let mut bytes = [0u8; MAX_CERTIFICATE];
    let length = build(params, subject, TestKey::Ed25519(ROOT_SECRET), &mut bytes)
        .expect("the parameters fit");
    Built { bytes, length }
}

/// The root and the server's certificate under it.
fn chain() -> (Built, Built) {
    let (from, until) = (moment(2020, 1, 1), moment(2030, 12, 31));
    let root = certificate(
        &Params::authority("Root", "Root", None, from, until),
        TestKey::Ed25519(ROOT_SECRET),
    );
    let names = [NAME];
    let leaf = certificate(
        &Params::leaf("Root", NAME, &names, from, until),
        TestKey::Ed25519(LEAF_SECRET),
    );
    (root, leaf)
}

/// What the scripted server does once the request is complete.
#[derive(Clone, Copy)]
enum Ending {
    /// Sends the response and nothing more.
    Open,
    /// Sends the response, then `close_notify`.
    CloseNotify,
    /// Sends the response, then ends the transport without `close_notify`.
    Drop,
}

/// The buffers of the server side.
struct ServerRoom {
    /// Bytes from the client.
    incoming: Vec<u8>,
    /// Bytes for the client.
    outgoing: Vec<u8>,
    /// Handshake messages.
    handshake: Vec<u8>,
}

impl ServerRoom {
    /// Room enough for the server.
    fn new() -> ServerRoom {
        ServerRoom {
            incoming: vec![0u8; MIN_INCOMING],
            outgoing: vec![0u8; MIN_OUTGOING],
            handshake: vec![0u8; MIN_HANDSHAKE],
        }
    }
}

/// An in-memory transport with the test server at its other end.
struct Pipe<'a> {
    /// The server.
    server: Server<'a>,
    /// What the server answers once the request is complete.
    response: &'a [u8],
    /// What the server does after the response.
    ending: Ending,
    /// The request as the server opened it.
    request: Vec<u8>,
    /// Whether the request was complete and the response began.
    answered: bool,
    /// How much of the response is sealed.
    sealed: usize,
    /// Whether the transport has ended.
    dropped: bool,
    /// The most bytes one `recv` returns.
    piece: usize,
    /// A failure `send` answers with, once.
    fail_send: Option<&'static str>,
}

impl<'a> Pipe<'a> {
    /// A pipe to a server of `config` answering `response`.
    fn new(config: &'a ServerConfig<'a>, room: &'a mut ServerRoom, response: &'a [u8]) -> Pipe<'a> {
        let buffers = TlsBuffers {
            incoming: &mut room.incoming,
            outgoing: &mut room.outgoing,
            handshake: &mut room.handshake,
        };
        Pipe {
            server: Server::new(config, buffers).expect("the server starts"),
            response,
            ending: Ending::CloseNotify,
            request: Vec::new(),
            answered: false,
            sealed: 0,
            dropped: false,
            piece: usize::MAX,
            fail_send: None,
        }
    }

    /// Seals the next record of the response once the request is
    /// complete, and the ending after the last one.
    fn answer(&mut self) -> Result<(), TlsError> {
        if !self.answered {
            if !request_complete(&self.request) {
                return Ok(());
            }
            self.answered = true;
        }
        let Some(rest) = self.response.get(self.sealed..) else {
            return Ok(());
        };
        if rest.is_empty() {
            self.sealed = usize::MAX;
            match self.ending {
                Ending::Open => {}
                Ending::CloseNotify => self.server.close()?,
                Ending::Drop => self.dropped = true,
            }
            return Ok(());
        }
        let taken = self.server.send(rest)?;
        self.sealed = self.sealed.saturating_add(taken);
        Ok(())
    }
}

/// Whether `request` holds a whole head and the body its length names.
fn request_complete(request: &[u8]) -> bool {
    let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(request.get(..end).unwrap_or(&[])).to_ascii_lowercase();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    request.len() >= end.saturating_add(4).saturating_add(length)
}

impl Transport for Pipe<'_> {
    type Error = &'static str;

    fn send(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        if let Some(error) = self.fail_send.take() {
            return Err(error);
        }
        let mut rest = bytes;
        while !rest.is_empty() {
            let taken = self.server.read_tls(rest).map_err(|_| "server full")?;
            rest = rest.get(taken..).unwrap_or(&[]);
            self.server.poll().map_err(|_| "server failed")?;
            let mut plaintext = vec![0u8; MAX_PLAINTEXT];
            while self.server.is_handshaked() {
                let opened = self
                    .server
                    .recv(&mut plaintext)
                    .map_err(|_| "server failed")?;
                if opened == 0 {
                    break;
                }
                self.request
                    .extend_from_slice(plaintext.get(..opened).unwrap_or(&[]));
            }
        }
        self.answer().map_err(|_| "server failed")
    }

    fn recv(&mut self, buffer: &mut [u8]) -> Result<usize, &'static str> {
        let room = buffer.len().min(self.piece);
        let target = buffer.get_mut(..room).unwrap_or(&mut []);
        let mut written = self.server.write_tls(target).map_err(|_| "server failed")?;
        if written == 0 && self.answered {
            self.answer().map_err(|_| "server failed")?;
            written = self.server.write_tls(target).map_err(|_| "server failed")?;
        }
        if written == 0 && !self.dropped && !self.answered {
            return Err("the client waits for a server with nothing to say");
        }
        Ok(written)
    }
}

/// The response as a handler received it.
#[derive(Default)]
struct Collected {
    /// The status.
    status: u16,
    /// The `Content-Type` field.
    content_type: Option<String>,
    /// The body.
    body: Vec<u8>,
    /// Refuse the head.
    refuse_head: bool,
    /// Refuse the body beyond this many bytes.
    limit: Option<usize>,
}

impl Handler for Collected {
    fn head(&mut self, head: &Head<'_>) -> Result<(), Refused> {
        self.status = head.status.get();
        self.content_type = head.header("content-type").map(str::to_owned);
        if self.refuse_head {
            Err(Refused)
        } else {
            Ok(())
        }
    }

    fn body(&mut self, piece: &[u8]) -> Result<(), Refused> {
        self.body.extend_from_slice(piece);
        match self.limit {
            Some(limit) if self.body.len() > limit => Err(Refused),
            _ => Ok(()),
        }
    }
}

/// The buffers of the client side.
struct ClientRoom {
    /// TLS incoming.
    incoming: Vec<u8>,
    /// TLS outgoing.
    outgoing: Vec<u8>,
    /// TLS handshake.
    handshake: Vec<u8>,
    /// The request.
    request: Vec<u8>,
    /// One record's plaintext.
    plaintext: Vec<u8>,
    /// The response head.
    head: Vec<u8>,
    /// Transport bytes in.
    receiving: Vec<u8>,
    /// Transport bytes out.
    sending: Vec<u8>,
}

impl ClientRoom {
    /// Room of the usual sizes, with `receiving` transport bytes per read.
    fn new(receiving: usize) -> ClientRoom {
        ClientRoom {
            incoming: vec![0u8; MIN_INCOMING],
            outgoing: vec![0u8; MIN_OUTGOING],
            handshake: vec![0u8; MIN_HANDSHAKE],
            request: vec![0u8; 4096],
            plaintext: vec![0u8; MAX_PLAINTEXT],
            head: vec![0u8; 4096],
            receiving: vec![0u8; receiving],
            sending: vec![0u8; 4096],
        }
    }

    /// The buffers, as `exchange` takes them.
    fn buffers(&mut self) -> Buffers<'_> {
        Buffers {
            tls: client::Buffers {
                incoming: &mut self.incoming,
                outgoing: &mut self.outgoing,
                handshake: &mut self.handshake,
            },
            request: &mut self.request,
            plaintext: &mut self.plaintext,
            head: &mut self.head,
            receiving: &mut self.receiving,
            sending: &mut self.sending,
        }
    }
}

/// The outcome of one exchange and what each side saw.
struct Run {
    /// What `exchange` returned.
    result: Result<(), HttpsError<&'static str>>,
    /// What the handler received.
    collected: Collected,
    /// The request as the server opened it.
    request: Vec<u8>,
}

/// The knobs of one run.
struct Setup<'a> {
    /// The name the client asks for.
    name: &'a str,
    /// The request.
    request: Request<'a>,
    /// The scripted response.
    response: &'a [u8],
    /// What the server does after it.
    ending: Ending,
    /// Transport bytes per read on the client side.
    receiving: usize,
    /// The most bytes one transport read returns.
    piece: usize,
    /// The handler.
    collected: Collected,
    /// A failure of the transport's first send.
    fail_send: Option<&'static str>,
}

impl Default for Setup<'_> {
    fn default() -> Self {
        Setup {
            name: NAME,
            request: Request::get("/@hello", NAME),
            response: b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nhi",
            ending: Ending::CloseNotify,
            receiving: 4096,
            piece: usize::MAX,
            collected: Collected::default(),
            fail_send: None,
        }
    }
}

/// Runs one exchange of `setup`.
fn run(setup: Setup<'_>) -> Run {
    let (root, leaf) = chain();
    let anchors = [TrustAnchor::from_certificate(root.as_slice()).expect("a well formed root")];
    let config = ClientConfig::new(
        setup.name,
        TrustAnchors::new(&anchors),
        moment(2026, 11, 24),
    );
    let certificates: [&[u8]; 2] = [leaf.as_slice(), root.as_slice()];
    let server_config = ServerConfig::new(&certificates, TestKey::Ed25519(LEAF_SECRET), ED25519);
    let mut server_room = ServerRoom::new();
    let mut pipe = Pipe::new(&server_config, &mut server_room, setup.response);
    pipe.ending = setup.ending;
    pipe.piece = setup.piece;
    pipe.fail_send = setup.fail_send;
    let script: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let mut rng = ScriptedRng::new(&script);
    let mut client_room = ClientRoom::new(setup.receiving);
    let mut collected = setup.collected;
    let result = exchange(
        &config,
        &mut rng,
        client_room.buffers(),
        &mut pipe,
        &setup.request,
        &mut collected,
    );
    Run {
        result,
        collected,
        request: pipe.request,
    }
}

#[test]
fn a_get_receives_the_head_and_the_body() {
    let run = run(Setup::default());
    assert_eq!(run.result, Ok(()));
    assert_eq!(run.collected.status, 200);
    assert_eq!(run.collected.content_type.as_deref(), Some("text/plain"));
    assert_eq!(run.collected.body, b"hi");
    assert_eq!(
        run.request,
        b"GET /@hello HTTP/1.1\r\nHost: example.test\r\n\r\n"
    );
}

#[test]
fn a_post_sends_its_fields_and_body() {
    let headers = [("Content-Type", "application/activity+json")];
    let run = run(Setup {
        request: Request {
            method: Method::Post,
            target: "/inbox",
            host: NAME,
            headers: &headers,
            body: b"{}",
        },
        response: b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n",
        ending: Ending::Open,
        ..Setup::default()
    });
    assert_eq!(run.result, Ok(()));
    assert_eq!(run.collected.status, 202);
    assert_eq!(
        run.request,
        b"POST /inbox HTTP/1.1\r\nHost: example.test\r\nContent-Type: application/activity+json\r\nContent-Length: 2\r\n\r\n{}"
    );
}

#[test]
fn a_chunked_body_over_many_records_and_one_byte_reads_arrives_whole() {
    let body: Vec<u8> = (0u8..=250).cycle().take(40_000).collect();
    let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for chunk in body.chunks(7000) {
        response.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        response.extend_from_slice(chunk);
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(b"0\r\n\r\n");
    for (receiving, piece) in [(65_536, usize::MAX), (1, 1), (4096, 777)] {
        let run = run(Setup {
            response: &response,
            ending: Ending::Open,
            receiving,
            piece,
            ..Setup::default()
        });
        assert_eq!(run.result, Ok(()), "receiving {receiving}, piece {piece}");
        assert_eq!(run.collected.body, body);
    }
}

#[test]
fn a_body_that_ends_at_close_needs_close_notify() {
    let response: &[u8] = b"HTTP/1.1 200 OK\r\n\r\nuntil the end";
    let closed = run(Setup {
        response,
        ..Setup::default()
    });
    assert_eq!(closed.result, Ok(()));
    assert_eq!(closed.collected.body, b"until the end");

    let dropped = run(Setup {
        response,
        ending: Ending::Drop,
        ..Setup::default()
    });
    assert_eq!(dropped.result, Err(HttpsError::Closed));
}

#[test]
fn a_body_shorter_than_its_length_is_truncated() {
    let run = run(Setup {
        response: b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nhi",
        ..Setup::default()
    });
    assert_eq!(run.result, Err(HttpsError::Http(HttpError::Truncated)));
}

#[test]
fn the_handler_refuses_the_head_or_a_body_too_long() {
    let head = run(Setup {
        collected: Collected {
            refuse_head: true,
            ..Collected::default()
        },
        ..Setup::default()
    });
    assert_eq!(head.result, Err(HttpsError::Refused));

    let body = run(Setup {
        collected: Collected {
            limit: Some(1),
            ..Collected::default()
        },
        ..Setup::default()
    });
    assert_eq!(body.result, Err(HttpsError::Refused));
}

#[test]
fn a_certificate_for_another_name_is_refused_before_the_request() {
    let run = run(Setup {
        name: "other.test",
        ..Setup::default()
    });
    assert!(
        matches!(run.result, Err(HttpsError::Tls(_))),
        "{:?}",
        run.result
    );
    assert!(run.request.is_empty());
}

#[test]
fn a_malformed_response_is_an_http_error() {
    let run = run(Setup {
        response: b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nhi",
        ..Setup::default()
    });
    assert!(
        matches!(run.result, Err(HttpsError::Http(_))),
        "{:?}",
        run.result
    );
}

#[test]
fn an_invalid_request_is_refused_before_the_handshake() {
    let run = run(Setup {
        request: Request::get("no-slash", NAME),
        fail_send: Some("never reached"),
        ..Setup::default()
    });
    assert!(
        matches!(run.result, Err(HttpsError::Http(_))),
        "{:?}",
        run.result
    );
}

#[test]
fn a_transport_failure_is_reported() {
    let run = run(Setup {
        fail_send: Some("broken pipe"),
        ..Setup::default()
    });
    assert_eq!(run.result, Err(HttpsError::Transport("broken pipe")));
}

/// A transport that takes every byte and answers the end of the stream.
struct Silent;

impl Transport for Silent {
    type Error = &'static str;

    fn send(&mut self, _: &[u8]) -> Result<(), &'static str> {
        Ok(())
    }

    fn recv(&mut self, _: &mut [u8]) -> Result<usize, &'static str> {
        Ok(0)
    }
}

#[test]
fn a_transport_that_ends_during_the_handshake_is_closed() {
    let (root, _) = chain();
    let anchors = [TrustAnchor::from_certificate(root.as_slice()).expect("a well formed root")];
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), moment(2026, 11, 24));
    let script: Vec<u8> = (0u8..=255).collect();
    let mut rng = ScriptedRng::new(&script);
    let mut client_room = ClientRoom::new(64);
    let result = exchange(
        &config,
        &mut rng,
        client_room.buffers(),
        &mut Silent,
        &Request::get("/", NAME),
        &mut Collected::default(),
    );
    assert_eq!(result, Err(HttpsError::Closed));
}

#[test]
fn buffers_below_their_minimum_are_refused() {
    let (root, _) = chain();
    let anchors = [TrustAnchor::from_certificate(root.as_slice()).expect("a well formed root")];
    let config = ClientConfig::new(NAME, TrustAnchors::new(&anchors), moment(2026, 11, 24));
    let script: Vec<u8> = (0u8..=255).collect();
    for shrink in 0..4 {
        let mut client_room = ClientRoom::new(64);
        match shrink {
            0 => client_room.plaintext.truncate(MAX_PLAINTEXT - 1),
            1 => client_room.receiving.clear(),
            2 => client_room.sending.clear(),
            _ => client_room.handshake.truncate(16),
        }
        let mut rng = ScriptedRng::new(&script);
        let mut silent = Silent;
        let result = exchange(
            &config,
            &mut rng,
            client_room.buffers(),
            &mut silent,
            &Request::get("/", NAME),
            &mut Collected::default(),
        );
        let expected = if shrink == 3 {
            Err(HttpsError::Tls(TlsError::BufferTooSmall))
        } else {
            Err(HttpsError::BufferTooSmall)
        };
        assert_eq!(result, expected, "shrink {shrink}");
    }
}

#[test]
fn errors_name_their_layer() {
    let cases: [(HttpsError<&str>, &str); 6] = [
        (HttpsError::Transport("reset"), "transport: reset"),
        (HttpsError::Tls(TlsError::BadCertificate), "tls: "),
        (HttpsError::Http(HttpError::Truncated), "http: "),
        (HttpsError::Refused, "the handler refused the response"),
        (
            HttpsError::Closed,
            "the peer closed before the response was complete",
        ),
        (HttpsError::BufferTooSmall, "a buffer is too small"),
    ];
    for (error, start) in cases {
        assert!(error.to_string().starts_with(start), "{error}");
    }
}
