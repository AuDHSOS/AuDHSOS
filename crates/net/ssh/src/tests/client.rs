// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The client driven end to end against a server built here.
//!
//! No document publishes a complete SSH handshake with the keys that made
//! it, so what the state machine is checked against is a server written
//! over the same layers: the greeting, both messages of the key exchange,
//! the signature over the exchange hash, the six keys, the authentication
//! and one session channel. A handshake against a server this project did
//! not write is the interop acceptance of step S8, which needs the network
//! on the machine.

use crypto_ec::ed25519;
use crypto_hash::Sha256;
use crypto_rng::ChaChaRng;
use crypto_rng::doubles::CountingEntropy;

use crate::auth::{self, ClientKey, PK_OK, Response};
use crate::channel::{EXIT_STATUS, SESSION, STDERR};
use crate::cipher::KEY_BYTES;
use crate::client::{Buffers, Config, Connection, Event, MIN_INCOMING, MIN_OUTGOING};
use crate::error::SshError;
use crate::exchange::{Ephemeral, HashInput, INIT, Method, REPLY, exchange_hash};
use crate::hostkey::{self, Fingerprint};
use crate::ident;
use crate::kex::{
    CIPHER_CHACHA20_POLY1305, COMPRESSION_NONE, HOST_KEY_ED25519, KEX_CURVE25519, KexInit, Proposal,
};
use crate::keys::{self, Key};
use crate::msg;
use crate::packet::{Decoded, Decoder, Encoder};
use crate::tests::ssh_string;
use crate::wire::{Reader, Writer};

/// The host's private key.
const HOST_SECRET: [u8; 32] = [0x21; 32];
/// The client's.
const CLIENT_SECRET: [u8; 32] = [0x22; 32];
/// The user this client authenticates as.
const USER: &str = "audhsos";
/// What the command writes to its standard output.
const OUTPUT: &[u8] = b"AuDHSOS\n";
/// What it writes to its standard error.
const DIAGNOSTIC: &[u8] = b"warning\n";

/// The server's identification string.
const SERVER_ID: &str = "SSH-2.0-TestServer_1.0";

/// What the server offers, which is what this client offers.
const SERVER_PROPOSAL: Proposal<'static> = Proposal {
    kex: &[KEX_CURVE25519],
    host_key: &[HOST_KEY_ED25519],
    encryption: &[CIPHER_CHACHA20_POLY1305],
    mac: &[],
    compression: &[COMPRESSION_NONE],
};

/// A generator that never runs out, seeded so that a run repeats.
fn generator(seed: u8) -> ChaChaRng<CountingEntropy> {
    ChaChaRng::new(CountingEntropy::new(seed)).expect("the source delivers")
}

/// Where the server stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Waiting for the client's identification string.
    Identification,
    /// Waiting for its `SSH_MSG_KEXINIT`.
    KexInit,
    /// Waiting for the first message of the method.
    KexInitMessage,
    /// Waiting for its `SSH_MSG_NEWKEYS`.
    NewKeys,
    /// Serving the authentication and the channel.
    Running,
}

/// A server for one connection, over the same layers the client uses.
struct Server {
    /// The packet layer for what it sends.
    encoder: Encoder,
    /// The packet layer for what arrives.
    decoder: Decoder,
    /// The padding and the cookie come from here.
    rng: ChaChaRng<CountingEntropy>,
    /// Where it stands.
    step: Step,
    /// Bytes from the client that have not been read.
    inbox: Vec<u8>,
    /// Bytes for the client.
    outbox: Vec<u8>,
    /// The client's identification string.
    client_id: String,
    /// The client's `SSH_MSG_KEXINIT` payload.
    client_kexinit: Vec<u8>,
    /// Its own.
    kexinit: Vec<u8>,
    /// The key for what it sends, once the exchange is done.
    next_key: Option<[u8; KEY_BYTES]>,
    /// The session identifier.
    session_id: Option<[u8; 32]>,
    /// The client's channel number.
    channel: Option<u32>,
    /// What the client sent on the channel.
    received: Vec<u8>,
    /// Whether the client answered a global request that asked for a
    /// reply.
    answered: bool,
    /// What this server was told to do and what it still owes.
    doing: Doing,
}

/// What the server was told to do, and what it still owes.
#[derive(Clone, Copy, Debug, Default)]
struct Doing {
    /// Ask for a re-exchange before the command's output goes out.
    rekeys: bool,
    /// Refuse the client's key.
    refuses: bool,
    /// Send a global request once authentication is through, the way
    /// OpenSSH sends `hostkeys-00@openssh.com`, and whether it asks for a
    /// reply.
    global_request: Option<bool>,
    /// The output is owed, because a re-exchange is running.
    deferred: bool,
}

impl Server {
    /// A server that has said nothing yet.
    fn new() -> Server {
        Server {
            encoder: Encoder::new(),
            decoder: Decoder::new(),
            rng: generator(0x40),
            step: Step::Identification,
            inbox: Vec::new(),
            outbox: Vec::new(),
            client_id: String::new(),
            client_kexinit: Vec::new(),
            kexinit: Vec::new(),
            next_key: None,
            session_id: None,
            channel: None,
            received: Vec::new(),
            answered: false,
            doing: Doing::default(),
        }
    }

    /// Frames one payload for the client.
    fn send(&mut self, payload: &[u8]) {
        let mut out = [0u8; 4096];
        let len = self
            .encoder
            .encode(payload, &mut self.rng, &mut out)
            .expect("the payload fits");
        self.outbox
            .extend_from_slice(out.get(..len).unwrap_or_default());
    }

    /// Takes bytes from the client and answers with what it has to say.
    fn exchange(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.inbox.extend_from_slice(bytes);
        loop {
            if self.step == Step::Identification {
                let Ok(ident::Greeting::Identification { line, length }) = ident::read(&self.inbox)
                else {
                    break;
                };
                self.client_id = line.to_owned();
                self.inbox.drain(..length);
                self.greet();
                self.step = Step::KexInit;
                continue;
            }
            let mut frame = self.inbox.clone();
            let (payload, length) = match self.decoder.decode(&mut frame) {
                Ok(Decoded::Packet { payload, length }) => (payload.to_vec(), length),
                Ok(Decoded::Incomplete { .. }) => break,
                Err(error) => panic!("the client sent something this server cannot read: {error}"),
            };
            self.inbox.drain(..length);
            self.handle(&payload);
        }
        core::mem::take(&mut self.outbox)
    }

    /// Its identification string and its `SSH_MSG_KEXINIT`.
    fn greet(&mut self) {
        self.outbox.extend_from_slice(SERVER_ID.as_bytes());
        self.outbox.extend_from_slice(b"\r\n");
        let mut payload = [0u8; 1024];
        let len = SERVER_PROPOSAL
            .write(&mut self.rng, &mut payload)
            .expect("the proposal fits");
        self.kexinit = payload.get(..len).unwrap_or_default().to_vec();
        let kexinit = self.kexinit.clone();
        self.send(&kexinit);
    }

    /// What one packet from the client means.
    fn handle(&mut self, payload: &[u8]) {
        let number = payload.first().copied().unwrap_or_default();
        match self.step {
            Step::Identification => {}
            Step::KexInit => {
                assert_eq!(number, msg::KEXINIT, "the client sent no KEXINIT");
                KexInit::read(payload).expect("the KEXINIT reads");
                self.client_kexinit = payload.to_vec();
                self.step = Step::KexInitMessage;
            }
            Step::KexInitMessage => {
                assert_eq!(number, INIT, "the client sent no key exchange init");
                self.reply(payload);
                self.step = Step::NewKeys;
            }
            Step::NewKeys => {
                assert_eq!(number, msg::NEWKEYS, "the client sent no NEWKEYS");
                let key = self.next_key.take().expect("the keys were derived");
                self.decoder.set_cipher(&key);
                self.step = Step::Running;
                if self.doing.deferred {
                    self.doing.deferred = false;
                    self.send_output();
                }
            }
            Step::Running => self.serve(payload),
        }
    }

    /// The reply of the method, the keys, and its own `SSH_MSG_NEWKEYS`.
    fn reply(&mut self, payload: &[u8]) {
        let mut reader = Reader::new(payload);
        reader.read_byte().expect("the number is there");
        let client_public = reader.read_string().expect("Q_C is a string").to_vec();

        let ephemeral = Ephemeral::new(Method::Curve25519, &mut self.rng).expect("a pair");
        let mut shared = [0u8; 32];
        let shared_len = ephemeral
            .shared_secret(&client_public, &mut shared)
            .expect("the client's value is a point");
        let host_key = host_key_blob();
        let hash = exchange_hash(
            Method::Curve25519,
            &HashInput {
                client_id: &self.client_id,
                server_id: SERVER_ID,
                client_kexinit: &self.client_kexinit,
                server_kexinit: &self.kexinit,
                host_key: &host_key,
                client_public: &client_public,
                server_public: ephemeral.public(),
                shared: shared.get(..shared_len).unwrap_or_default(),
            },
        );
        let session_id = *self.session_id.get_or_insert(hash);

        let signature = ed25519::sign(&HOST_SECRET, &hash);
        let mut blob = Vec::new();
        blob.extend_from_slice(&ssh_string(HOST_KEY_ED25519.as_bytes()));
        blob.extend_from_slice(&ssh_string(&signature));

        let mut out = Vec::new();
        out.push(REPLY);
        out.extend_from_slice(&ssh_string(&host_key));
        out.extend_from_slice(&ssh_string(ephemeral.public()));
        out.extend_from_slice(&ssh_string(&blob));
        self.send(&out);
        self.send(&[msg::NEWKEYS]);

        let secret = shared.get(..shared_len).unwrap_or_default();
        let mut client_key = [0u8; KEY_BYTES];
        let mut server_key = [0u8; KEY_BYTES];
        keys::derive(
            secret,
            &hash,
            &session_id,
            Key::EncryptionClientToServer,
            &mut client_key,
        );
        keys::derive(
            secret,
            &hash,
            &session_id,
            Key::EncryptionServerToClient,
            &mut server_key,
        );
        self.encoder.set_cipher(&server_key);
        self.next_key = Some(client_key);
    }

    /// The authentication and the channel.
    fn serve(&mut self, payload: &[u8]) {
        let number = payload.first().copied().unwrap_or_default();
        match number {
            msg::SERVICE_REQUEST => {
                let service = auth::read_service_accept(&accept_of(payload))
                    .expect("the request names a service")
                    .to_vec();
                assert_eq!(service, auth::SERVICE_USERAUTH.as_bytes());
                let mut out = Vec::new();
                out.push(msg::SERVICE_ACCEPT);
                out.extend_from_slice(&ssh_string(&service));
                self.send(&out);
            }
            msg::USERAUTH_REQUEST => self.authenticate(payload),
            msg::CHANNEL_OPEN => self.open(payload),
            msg::CHANNEL_REQUEST => self.start(payload),
            msg::KEXINIT => {
                self.client_kexinit = payload.to_vec();
                self.step = Step::KexInitMessage;
            }
            msg::CHANNEL_DATA => {
                let mut reader = Reader::new(payload);
                reader.read_byte().expect("the number is there");
                reader.read_u32().expect("the channel");
                let data = reader.read_string().expect("the data");
                self.received.extend_from_slice(data);
            }
            msg::REQUEST_FAILURE => self.answered = true,
            msg::CHANNEL_CLOSE | msg::DISCONNECT | msg::CHANNEL_EOF => {}
            other => panic!("the client sent message {other}"),
        }
    }

    /// The `publickey` query and the request that authenticates.
    fn authenticate(&mut self, payload: &[u8]) {
        let mut reader = Reader::new(payload);
        reader.read_byte().expect("the number is there");
        let user = reader.read_string().expect("the user name").to_vec();
        let service = reader.read_string().expect("the service").to_vec();
        let method = reader.read_string().expect("the method").to_vec();
        let signed = reader.read_boolean().expect("the boolean");
        let algorithm = reader.read_string().expect("the algorithm").to_vec();
        let blob = reader.read_string().expect("the key blob").to_vec();

        assert_eq!(user, USER.as_bytes());
        assert_eq!(service, auth::SERVICE_CONNECTION.as_bytes());
        assert_eq!(method, auth::METHOD_PUBLICKEY.as_bytes());
        assert_eq!(algorithm, HOST_KEY_ED25519.as_bytes());

        if !signed {
            let mut out = Vec::new();
            out.push(PK_OK);
            out.extend_from_slice(&ssh_string(&algorithm));
            out.extend_from_slice(&ssh_string(&blob));
            self.send(&out);
            return;
        }

        // What a server checks: the session identifier, then the fields of
        // the request as they stand, under the key the request carries.
        let fields = payload.get(..reader.position()).unwrap_or_default();
        let mut data = ssh_string(&self.session_id.expect("there is a session"));
        data.extend_from_slice(fields);
        let signature = reader.read_string().expect("the signature").to_vec();
        let key = hostkey::HostKey::parse(&blob).expect("the blob is a key");
        let mut value = [0u8; 64];
        let mut inner = Reader::new(&signature);
        inner.read_string().expect("the algorithm");
        value.copy_from_slice(inner.read_string().expect("the signature value"));
        assert!(
            ed25519::verify(key.public_key(), &data, &value).is_ok(),
            "the client's signature does not verify"
        );

        if self.doing.refuses {
            let mut out = vec![msg::USERAUTH_FAILURE];
            out.extend_from_slice(&ssh_string(b"publickey"));
            out.push(0);
            self.send(&out);
            return;
        }
        self.send(&[msg::USERAUTH_SUCCESS]);
        if let Some(wants_reply) = self.doing.global_request {
            let mut out = vec![msg::GLOBAL_REQUEST];
            out.extend_from_slice(&ssh_string(b"hostkeys-00@openssh.com"));
            out.push(u8::from(wants_reply));
            out.extend_from_slice(&ssh_string(b"a key blob this client does not read"));
            self.send(&out);
        }
    }

    /// The channel open, confirmed.
    fn open(&mut self, payload: &[u8]) {
        let mut reader = Reader::new(payload);
        reader.read_byte().expect("the number is there");
        let kind = reader.read_string().expect("the channel type").to_vec();
        assert_eq!(kind, SESSION.as_bytes());
        let client_channel = reader.read_u32().expect("the client's number");
        self.channel = Some(client_channel);
        reader.read_u32().expect("the window");
        reader.read_u32().expect("the maximum packet size");

        let mut out = Vec::new();
        out.push(msg::CHANNEL_OPEN_CONFIRMATION);
        out.extend_from_slice(&client_channel.to_be_bytes());
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&4096u32.to_be_bytes());
        out.extend_from_slice(&1024u32.to_be_bytes());
        self.send(&out);
    }

    /// The `exec` request, and everything the command says.
    fn start(&mut self, payload: &[u8]) {
        let mut reader = Reader::new(payload);
        reader.read_byte().expect("the number is there");
        reader.read_u32().expect("the channel");
        let kind = reader.read_string().expect("the request type").to_vec();
        assert_eq!(kind, b"exec");
        reader.read_boolean().expect("the reply flag");
        let command = reader.read_string().expect("the command").to_vec();
        assert_eq!(command, b"uname");

        let channel = self.channel.expect("the channel is open");
        self.send(&channel_message(msg::CHANNEL_SUCCESS, channel, &[]));

        if self.doing.rekeys {
            // A re-exchange the server starts: nothing above the transport
            // goes out until the new keys are in use (RFC 4253, section 9).
            self.doing.deferred = true;
            let mut payload = [0u8; 1024];
            let len = SERVER_PROPOSAL
                .write(&mut self.rng, &mut payload)
                .expect("the proposal fits");
            self.kexinit = payload.get(..len).unwrap_or_default().to_vec();
            let kexinit = self.kexinit.clone();
            self.send(&kexinit);
            return;
        }
        self.send_output();
    }

    /// What the command wrote, its status, and the end of the channel.
    fn send_output(&mut self) {
        let channel = self.channel.expect("the channel is open");
        self.send(&channel_message(
            msg::CHANNEL_DATA,
            channel,
            &ssh_string(OUTPUT),
        ));
        let mut extended = STDERR.to_be_bytes().to_vec();
        extended.extend_from_slice(&ssh_string(DIAGNOSTIC));
        self.send(&channel_message(
            msg::CHANNEL_EXTENDED_DATA,
            channel,
            &extended,
        ));

        let mut status = ssh_string(EXIT_STATUS.as_bytes());
        status.push(0);
        status.extend_from_slice(&0u32.to_be_bytes());
        self.send(&channel_message(msg::CHANNEL_REQUEST, channel, &status));
        self.send(&channel_message(msg::CHANNEL_EOF, channel, &[]));
        self.send(&channel_message(msg::CHANNEL_CLOSE, channel, &[]));
    }
}

/// The blob of the server's host key.
fn host_key_blob() -> Vec<u8> {
    let mut blob = [0u8; hostkey::BLOB_LEN];
    let len = hostkey::write_blob(&ed25519::public_key(&HOST_SECRET), &mut blob)
        .expect("the buffer is long enough");
    blob.get(..len).unwrap_or_default().to_vec()
}

/// One channel message: the number, the channel, and what follows.
fn channel_message(number: u8, channel: u32, tail: &[u8]) -> Vec<u8> {
    let mut out = vec![number];
    out.extend_from_slice(&channel.to_be_bytes());
    out.extend_from_slice(tail);
    out
}

/// A `SSH_MSG_SERVICE_REQUEST` read as the accept it has the shape of.
fn accept_of(payload: &[u8]) -> Vec<u8> {
    let mut out = vec![msg::SERVICE_ACCEPT];
    out.extend_from_slice(payload.get(1..).unwrap_or_default());
    out
}

/// The rule that admits this server's host key.
fn rule() -> Fingerprint {
    Fingerprint::new(Sha256::digest(&host_key_blob()))
}

/// What one run of the client against the server produced.
struct Run {
    /// The bytes of the command's standard output.
    output: Vec<u8>,
    /// The bytes of its standard error.
    diagnostic: Vec<u8>,
    /// Its exit status, if it reported one.
    status: Option<u32>,
    /// Whether the connection ended by the server's close.
    closed: bool,
    /// What the server read off the channel.
    received: Vec<u8>,
    /// Whether the client answered a global request that asked for a
    /// reply.
    answered: bool,
}

/// Drives a client against a fresh server.
fn run(config: &Config<'_, Fingerprint>) -> Result<Run, SshError> {
    drive(config, Server::new(), &[])
}

/// Drives a client against `server`, sending `input` on the channel once
/// the command is running.
fn drive(config: &Config<'_, Fingerprint>, server: Server, input: &[u8]) -> Result<Run, SshError> {
    let mut rng = generator(0x10);
    let mut incoming = vec![0u8; MIN_INCOMING];
    let mut outgoing = vec![0u8; MIN_OUTGOING];
    let mut client = Connection::new(
        config,
        &mut rng,
        Buffers {
            incoming: &mut incoming,
            outgoing: &mut outgoing,
        },
        0,
    )?;
    let mut server = server;
    let mut result = Run {
        output: Vec::new(),
        diagnostic: Vec::new(),
        status: None,
        closed: false,
        received: Vec::new(),
        answered: false,
    };

    let mut wire = Vec::new();
    for _ in 0..64 {
        let event = client.poll(0)?;
        match event {
            Event::WantsWrite => {
                let mut buffer = [0u8; 4096];
                let len = client.write_ssh(&mut buffer);
                let answer = server.exchange(buffer.get(..len).unwrap_or_default());
                wire.extend_from_slice(&answer);
            }
            Event::WantsRead => {
                if wire.is_empty() {
                    let mut buffer = [0u8; 4096];
                    let len = client.write_ssh(&mut buffer);
                    if len == 0 {
                        return Ok(result);
                    }
                    let answer = server.exchange(buffer.get(..len).unwrap_or_default());
                    wire.extend_from_slice(&answer);
                    continue;
                }
                let taken = client.read_ssh(&wire);
                wire.drain(..taken);
            }
            Event::Started => {
                if !input.is_empty() {
                    let taken = client.send(input)?;
                    assert_eq!(taken, input.len(), "the window took everything");
                    client.finish()?;
                }
            }
            Event::ExitSignal => {}
            Event::Data { stderr, len } => {
                let mut buffer = vec![0u8; len];
                let taken = client.recv(&mut buffer);
                let bytes = buffer.get(..taken).unwrap_or_default();
                if stderr {
                    result.diagnostic.extend_from_slice(bytes);
                } else {
                    result.output.extend_from_slice(bytes);
                }
            }
            Event::ExitStatus(status) => result.status = Some(status),
            Event::Closed => {
                // What the client wrote last — its own close, and the data
                // it sent — is still in its buffer, and the server reads it
                // before anything is judged.
                loop {
                    let mut buffer = [0u8; 4096];
                    let len = client.write_ssh(&mut buffer);
                    if len == 0 {
                        break;
                    }
                    server.exchange(buffer.get(..len).unwrap_or_default());
                }
                result.closed = true;
                result.received = server.received.clone();
                result.answered = server.answered;
                return Ok(result);
            }
        }
    }
    panic!("the client and the server did not finish");
}

/// The configuration every test runs with.
fn config<'a>(key: &'a ClientKey, trust: &'a Fingerprint) -> Config<'a, Fingerprint> {
    Config {
        user: USER,
        key,
        trust,
        command: Some(b"uname"),
        window: 4096,
        max_packet: 1024,
    }
}

#[test]
fn the_client_reaches_a_command_and_reads_what_it_wrote() {
    let key = ClientKey::new(CLIENT_SECRET);
    let trust = rule();

    let result = run(&config(&key, &trust)).expect("the handshake completes");

    assert_eq!(result.output, OUTPUT);
    assert_eq!(result.diagnostic, DIAGNOSTIC);
    assert_eq!(result.status, Some(0));
    assert!(result.closed);
}

#[test]
fn a_host_key_no_rule_admits_ends_the_connection() {
    let key = ClientKey::new(CLIENT_SECRET);
    let mut blob = [0u8; hostkey::BLOB_LEN];
    let len = hostkey::write_blob(&ed25519::public_key(&[0x99; 32]), &mut blob)
        .expect("the buffer is long enough");
    let trust = Fingerprint::new(Sha256::digest(blob.get(..len).unwrap_or_default()));

    let refused = run(&config(&key, &trust));

    assert!(matches!(refused, Err(SshError::HostKeyRejected)));
}

#[test]
fn what_the_client_sends_reaches_the_command() {
    let key = ClientKey::new(CLIENT_SECRET);
    let trust = rule();

    let result =
        drive(&config(&key, &trust), Server::new(), b"input\n").expect("the handshake completes");

    assert_eq!(result.received, b"input\n");
    assert_eq!(result.output, OUTPUT);
    assert!(result.closed);
}

#[test]
fn a_re_exchange_the_server_starts_changes_the_keys_and_not_the_session() {
    let key = ClientKey::new(CLIENT_SECRET);
    let trust = rule();
    let mut server = Server::new();
    server.doing.rekeys = true;

    let result = drive(&config(&key, &trust), server, &[]).expect("the handshake completes");

    assert_eq!(result.output, OUTPUT);
    assert_eq!(result.diagnostic, DIAGNOSTIC);
    assert_eq!(result.status, Some(0));
    assert!(result.closed);
}

#[test]
fn a_server_that_refuses_the_key_ends_the_connection() {
    let key = ClientKey::new(CLIENT_SECRET);
    let trust = rule();
    let mut server = Server::new();
    server.doing.refuses = true;

    let refused = drive(&config(&key, &trust), server, &[]);

    assert!(matches!(refused, Err(SshError::Authentication)));
}

#[test]
fn the_buffers_are_the_ones_the_document_makes_mandatory() {
    let key = ClientKey::new(CLIENT_SECRET);
    let trust = rule();
    let settings = config(&key, &trust);
    let mut rng = generator(0x10);
    let mut incoming = vec![0u8; MIN_INCOMING - 1];
    let mut outgoing = vec![0u8; MIN_OUTGOING];

    let short = Connection::new(
        &settings,
        &mut rng,
        Buffers {
            incoming: &mut incoming,
            outgoing: &mut outgoing,
        },
        0,
    );

    assert!(matches!(short, Err(SshError::OutOfBounds { .. })));
}

#[test]
fn a_user_name_longer_than_the_buffer_is_refused() {
    let key = ClientKey::new(CLIENT_SECRET);
    let trust = rule();
    let long = "x".repeat(65);
    let settings = Config {
        user: &long,
        key: &key,
        trust: &trust,
        command: None,
        window: 4096,
        max_packet: 1024,
    };
    let mut rng = generator(0x10);
    let mut incoming = vec![0u8; MIN_INCOMING];
    let mut outgoing = vec![0u8; MIN_OUTGOING];

    let refused = Connection::new(
        &settings,
        &mut rng,
        Buffers {
            incoming: &mut incoming,
            outgoing: &mut outgoing,
        },
        0,
    );

    assert!(matches!(refused, Err(SshError::OutOfBounds { .. })));
}

#[test]
fn the_answer_a_server_gives_a_query_is_leave_to_sign_and_nothing_else() {
    let key = ClientKey::new(CLIENT_SECRET);
    let mut blob = [0u8; hostkey::BLOB_LEN];
    let len = key
        .write_blob(&mut blob)
        .expect("the buffer is long enough");
    let mine = blob.get(..len).unwrap_or_default().to_vec();
    let mut payload = vec![PK_OK];
    payload.extend_from_slice(&ssh_string(HOST_KEY_ED25519.as_bytes()));
    payload.extend_from_slice(&ssh_string(&mine));

    let response = Response::read(&payload).expect("the answer reads");

    assert!(response.is_leave_to_sign(&mine));
}

#[test]
fn the_writer_and_the_reader_of_this_test_agree_with_the_crate() {
    let mut out = [0u8; 64];
    let mut writer = Writer::new(&mut out);
    writer.write_string(b"session").expect("there is room");
    let len = writer.position();

    assert_eq!(out.get(..len), Some(ssh_string(b"session").as_slice()));
}

#[test]
fn a_global_request_that_wants_no_reply_is_ignored_and_the_command_runs() {
    let key = ClientKey::new(CLIENT_SECRET);
    let trust = rule();
    let mut server = Server::new();
    server.doing.global_request = Some(false);

    let result = drive(&config(&key, &trust), server, &[]).expect("the handshake completes");

    assert_eq!(result.output, OUTPUT);
    assert_eq!(result.status, Some(0));
    assert!(!result.answered, "nothing was asked and nothing was sent");
}

#[test]
fn a_global_request_that_wants_a_reply_is_refused_and_the_command_runs() {
    let key = ClientKey::new(CLIENT_SECRET);
    let trust = rule();
    let mut server = Server::new();
    server.doing.global_request = Some(true);

    let result = drive(&config(&key, &trust), server, &[]).expect("the handshake completes");

    assert_eq!(result.output, OUTPUT);
    assert_eq!(result.status, Some(0));
    assert!(
        result.answered,
        "the client answered SSH_MSG_REQUEST_FAILURE"
    );
}
