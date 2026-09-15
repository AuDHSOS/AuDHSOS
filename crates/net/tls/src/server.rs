// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The server side of one connection, for the tests and for the
//! acceptance run of Phase 15 (D-148).
//!
//! This exists so that a client can be driven against something. It is
//! not a server of this system: it answers one connection at a time, it
//! asks for no client certificate, it sends no session ticket, it answers
//! no key update, and it never sends a `HelloRetryRequest` — a client that
//! offers no X25519 share is refused rather than asked again. What it does
//! do is what the client of this crate needs to reach: one TLS 1.3
//! handshake over X25519, a chain the caller hands it, application data
//! both ways, and `close_notify`.
//!
//! It is sans-I/O like [`crate::client`], with the same four calls —
//! [`Connection::read_tls`], [`Connection::write_tls`],
//! [`Connection::poll`], [`Connection::recv`] — and the same
//! [`Buffers`].

use audhsos_x509::builder::TestKey;
use crypto_ct::ct_eq;
use crypto_ec::x25519;

use crate::alert::Alert;
use crate::client::{Buffers, Event, MIN_HANDSHAKE, MIN_INCOMING, MIN_OUTGOING};
use crate::codec::{Reader, Writer};
use crate::error::TlsError;
use crate::handshake::{GROUP_X25519, HandshakeType, VERSION_TLS13, read_message, write_finished};
use crate::keys::{Schedule, finished_key, traffic_keys, verify_data};
use crate::protection::RecordProtection;
use crate::record::{self, ContentType, HEADER_LEN, MAX_PLAINTEXT};
use crate::secret::Secret;
use crate::suite::CipherSuite;
use crate::transcript::Transcript;

/// The extension that carries the versions a client offers.
const SUPPORTED_VERSIONS: u16 = 43;
/// The extension that carries the shares a client offers.
const KEY_SHARE: u16 = 51;
/// The extension that carries the signature schemes a client accepts.
const SIGNATURE_ALGORITHMS: u16 = 13;
/// The extension that carries the protocols a client offers.
const ALPN: u16 = 16;

/// The suites this server answers with, in the order it prefers them.
const SUITES: [CipherSuite; 3] = [
    CipherSuite::Aes128GcmSha256,
    CipherSuite::Aes256GcmSha384,
    CipherSuite::ChaCha20Poly1305Sha256,
];

/// The longest chain this server sends.
pub const MAX_CHAIN: usize = 4;

/// The longest signature a `CertificateVerify` carries, which is an RSA
/// signature of a 4096-bit key.
const MAX_SIGNATURE: usize = 512;

/// The bytes signed before the transcript: sixty-four spaces, the label,
/// and one zero (RFC 8446, section 4.4.3).
const CONTEXT: &[u8] = b"TLS 1.3, server CertificateVerify";

/// The widest transcript hash this crate takes, which is SHA-384's.
const MAX_HASH: usize = 48;

/// What a server needs to know before it answers.
#[derive(Clone, Copy, Debug)]
pub struct ServerConfig<'a> {
    /// The chain it presents, the leaf first.
    pub chain: &'a [&'a [u8]],
    /// The key of the leaf, which signs the `CertificateVerify`.
    pub key: TestKey,
    /// The signature scheme that key signs with, as TLS numbers it.
    pub scheme: u16,
    /// The protocol to choose, where the client offers it.
    pub alpn: &'a [u8],
    /// The private value of its key exchange. It is a parameter because
    /// this crate reads no clock and draws no randomness of its own; a
    /// caller that wants a fresh one draws it from its generator.
    pub ephemeral: [u8; 32],
}

impl<'a> ServerConfig<'a> {
    /// A configuration that offers no protocol.
    #[must_use]
    pub const fn new(chain: &'a [&'a [u8]], key: TestKey, scheme: u16) -> ServerConfig<'a> {
        ServerConfig {
            chain,
            key,
            scheme,
            alpn: &[],
            ephemeral: [0x33; 32],
        }
    }
}

/// Where in the handshake the server is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Waiting for the `ClientHello`.
    WaitClientHello,
    /// The flight is written; waiting for the client's `Finished`.
    WaitFinished,
    /// Done; application data may flow.
    Connected,
    /// Closed, by this side or the other.
    Closed,
}

/// The server side of one connection.
pub struct Connection<'a> {
    /// What the caller decided.
    config: &'a ServerConfig<'a>,
    /// Where the handshake stands.
    state: State,
    /// The suite the `ClientHello` and this server agreed on.
    suite: CipherSuite,
    /// The running hash.
    transcript: Transcript,
    /// What this side sends the handshake under.
    write_handshake: Option<RecordProtection>,
    /// What the client sends the handshake under.
    read_handshake: Option<RecordProtection>,
    /// What this side sends data under.
    write_application: Option<RecordProtection>,
    /// What the client sends data under.
    read_application: Option<RecordProtection>,
    /// The key the client's `Finished` is authenticated with.
    client_finished: Option<Secret>,
    /// The transcript that `Finished` is over.
    expected: Secret,
    /// The protocol this server chose, if any.
    chosen: usize,
    /// Whether the client closed its side.
    peer_closed: bool,
    /// The error that ended the connection, if one did.
    poison: Option<TlsError>,
    /// Bytes from the transport.
    incoming: &'a mut [u8],
    /// How many of them there are.
    incoming_len: usize,
    /// Bytes for the transport.
    outgoing: &'a mut [u8],
    /// How many of them there are.
    outgoing_len: usize,
    /// Where the flight is built and the client's messages reassembled.
    handshake: &'a mut [u8],
    /// How much of it is filled.
    handshake_len: usize,
}

impl<'a> Connection<'a> {
    /// A connection that has said nothing yet.
    ///
    /// # Errors
    ///
    /// [`TlsError::BufferTooSmall`] when a buffer is below its minimum,
    /// and [`TlsError::BadCertificate`] for a chain longer than
    /// [`MAX_CHAIN`] or with nothing in it.
    pub const fn new(
        config: &'a ServerConfig<'a>,
        buffers: Buffers<'a>,
    ) -> Result<Connection<'a>, TlsError> {
        if buffers.incoming.len() < MIN_INCOMING
            || buffers.outgoing.len() < MIN_OUTGOING
            || buffers.handshake.len() < MIN_HANDSHAKE
        {
            return Err(TlsError::BufferTooSmall);
        }
        if config.chain.is_empty() || config.chain.len() > MAX_CHAIN {
            return Err(TlsError::BadCertificate);
        }
        let Buffers {
            incoming,
            outgoing,
            handshake,
        } = buffers;
        Ok(Connection {
            config,
            state: State::WaitClientHello,
            suite: CipherSuite::Aes128GcmSha256,
            transcript: Transcript::new(),
            write_handshake: None,
            read_handshake: None,
            write_application: None,
            read_application: None,
            client_finished: None,
            expected: Secret::zero(0),
            chosen: 0,
            peer_closed: false,
            poison: None,
            incoming,
            incoming_len: 0,
            outgoing,
            outgoing_len: 0,
            handshake,
            handshake_len: 0,
        })
    }

    /// The protocol this server chose, if the client offered the one it
    /// was configured with.
    #[must_use]
    pub fn alpn(&self) -> Option<&[u8]> {
        self.config
            .alpn
            .get(..self.chosen)
            .filter(|it| !it.is_empty())
    }

    /// Whether the handshake is done.
    #[must_use]
    pub const fn is_handshaked(&self) -> bool {
        matches!(self.state, State::Connected)
    }

    /// Takes bytes from the transport, and says how many it took.
    ///
    /// # Errors
    ///
    /// The error that ended the connection, if one did.
    pub fn read_tls(&mut self, input: &[u8]) -> Result<usize, TlsError> {
        self.check()?;
        let room = self.incoming.len().saturating_sub(self.incoming_len);
        let taken = room.min(input.len());
        let end = self.incoming_len.wrapping_add(taken);
        let target = self
            .incoming
            .get_mut(self.incoming_len..end)
            .ok_or(TlsError::BufferTooSmall)?;
        for (slot, byte) in target.iter_mut().zip(input) {
            *slot = *byte;
        }
        self.incoming_len = end;
        Ok(taken)
    }

    /// Hands bytes to the transport, and says how many it handed over.
    ///
    /// # Errors
    ///
    /// Never; the result is a `Result` so that the shape of the interface
    /// matches the client's.
    pub fn write_tls(&mut self, output: &mut [u8]) -> Result<usize, TlsError> {
        let taken = output.len().min(self.outgoing_len);
        for (slot, byte) in output.iter_mut().zip(self.outgoing.iter()) {
            *slot = *byte;
        }
        self.outgoing.copy_within(taken..self.outgoing_len, 0);
        self.outgoing_len = self.outgoing_len.saturating_sub(taken);
        Ok(taken)
    }

    /// Makes what progress the bytes at hand allow.
    ///
    /// # Errors
    ///
    /// Whatever ended the connection. The client is told about it in an
    /// alert that `write_tls` will hand over.
    pub fn poll(&mut self) -> Result<Event, TlsError> {
        self.check()?;
        while self.state != State::Closed && self.state != State::Connected {
            match self.step() {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    self.fail(error);
                    return Err(error);
                }
            }
        }
        if self.outgoing_len != 0 {
            return Ok(Event::WantsWrite);
        }
        if self.peer_closed {
            return Ok(Event::PeerClosed);
        }
        if self.state == State::Connected {
            return Ok(Event::Handshaked);
        }
        Ok(Event::WantsRead)
    }

    /// Sends application data, and says how much it took.
    ///
    /// # Errors
    ///
    /// [`TlsError::UnexpectedMessage`] before the handshake is done, and
    /// whatever ended the connection.
    pub fn send(&mut self, plaintext: &[u8]) -> Result<usize, TlsError> {
        self.check()?;
        if self.state != State::Connected {
            return Err(TlsError::UnexpectedMessage);
        }
        let taken = plaintext.len().min(MAX_PLAINTEXT);
        let piece = plaintext.get(..taken).unwrap_or(&[]);
        match seal_into(
            self.write_application.as_mut(),
            ContentType::ApplicationData,
            piece,
            self.outgoing,
            &mut self.outgoing_len,
        ) {
            Ok(()) => Ok(taken),
            Err(error) => {
                self.fail(error);
                Err(error)
            }
        }
    }

    /// Takes application data that has arrived, and says how much it put
    /// in `out`. Zero means none had arrived.
    ///
    /// # Errors
    ///
    /// [`TlsError::BufferTooSmall`] for a buffer below the record limit,
    /// and whatever ended the connection.
    pub fn recv(&mut self, out: &mut [u8]) -> Result<usize, TlsError> {
        self.check()?;
        if out.len() < MAX_PLAINTEXT {
            return Err(TlsError::BufferTooSmall);
        }
        match self.step_data(out) {
            Ok(length) => Ok(length),
            Err(error) => {
                self.fail(error);
                Err(error)
            }
        }
    }

    /// Closes this side of the connection.
    ///
    /// # Errors
    ///
    /// Whatever ended the connection before.
    pub fn close(&mut self) -> Result<(), TlsError> {
        self.check()?;
        let result = seal_into(
            self.write_application.as_mut(),
            ContentType::Alert,
            &[crate::alert::WARNING, Alert::CloseNotify.code()],
            self.outgoing,
            &mut self.outgoing_len,
        );
        self.state = State::Closed;
        result
    }

    /// Refuses to go on once something has gone wrong.
    const fn check(&self) -> Result<(), TlsError> {
        match self.poison {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Remembers the error and tells the client about it.
    fn fail(&mut self, error: TlsError) {
        if self.poison.is_none() {
            self.poison = Some(error);
            if let Some(alert) = Alert::for_error(error) {
                let body = [alert.level(), alert.code()];
                let keys = if self.write_application.is_some() {
                    self.write_application.as_mut()
                } else {
                    self.write_handshake.as_mut()
                };
                let sent = seal_into(
                    keys,
                    ContentType::Alert,
                    &body,
                    self.outgoing,
                    &mut self.outgoing_len,
                );
                if sent.is_err() {
                    let _ = write_plain(
                        ContentType::Alert,
                        &body,
                        self.outgoing,
                        &mut self.outgoing_len,
                    );
                }
            }
        }
        self.state = State::Closed;
    }

    /// Processes one record of the handshake. Says whether it did.
    fn step(&mut self) -> Result<bool, TlsError> {
        let available = self.incoming.get(..self.incoming_len).unwrap_or(&[]);
        let Some((header, _, used)) = record::read(available)? else {
            return Ok(false);
        };

        // The record is opened in place and copied out, because what
        // happens next takes the whole server: the flight is written into
        // `self.handshake` and the transcript is updated. The frame this
        // costs is the reason the module says it runs on a development
        // machine and not on the target.
        let mut message = [0u8; MIN_HANDSHAKE];
        let (kind, message_len) = {
            let record = self.incoming.get_mut(..used).ok_or(TlsError::BadRecord)?;
            let (head, body) = record.split_at_mut(HEADER_LEN);
            let mut header_copy = [0u8; HEADER_LEN];
            for (slot, byte) in header_copy.iter_mut().zip(head.iter()) {
                *slot = *byte;
            }
            let (opened_kind, plaintext) = match (header.content_type, self.read_handshake.as_mut())
            {
                (ContentType::ChangeCipherSpec, _) => {
                    // RFC 8446, section 5: the compatibility record carries
                    // the single byte one and nothing else.
                    if body.len() != 1 || body.first() != Some(&0x01) {
                        return Err(TlsError::UnexpectedMessage);
                    }
                    (ContentType::ChangeCipherSpec, &[][..])
                }
                (ContentType::ApplicationData, Some(keys)) => keys.open(&header_copy, body)?,
                (ContentType::ApplicationData, None) => return Err(TlsError::UnexpectedMessage),
                (plain, _) => (plain, &body[..]),
            };
            let room = message
                .get_mut(..plaintext.len())
                .ok_or(TlsError::BufferTooSmall)?;
            for (slot, byte) in room.iter_mut().zip(plaintext) {
                *slot = *byte;
            }
            (opened_kind, plaintext.len())
        };
        self.incoming.copy_within(used..self.incoming_len, 0);
        self.incoming_len = self.incoming_len.saturating_sub(used);

        let plaintext = message.get(..message_len).unwrap_or(&[]);
        match kind {
            ContentType::ChangeCipherSpec => Ok(true),
            ContentType::Alert => {
                self.take_alert(plaintext)?;
                Ok(true)
            }
            ContentType::Handshake => {
                self.take_handshake(plaintext)?;
                Ok(true)
            }
            ContentType::ApplicationData => Err(TlsError::UnexpectedMessage),
        }
    }

    /// Takes what the client said in an alert.
    fn take_alert(&mut self, body: &[u8]) -> Result<(), TlsError> {
        let level = body.first().copied().ok_or(TlsError::BadRecord)?;
        let code = body.get(1).copied().ok_or(TlsError::BadRecord)?;
        if code == Alert::CloseNotify.code() {
            self.peer_closed = true;
            return Ok(());
        }
        if level == crate::alert::FATAL {
            return Err(TlsError::PeerAlert(code));
        }
        Ok(())
    }

    /// Takes one handshake message of the client's.
    fn take_handshake(&mut self, plaintext: &[u8]) -> Result<(), TlsError> {
        let Some((kind, body, _)) = read_message(plaintext)? else {
            return Err(TlsError::BadRecord);
        };
        match (self.state, kind) {
            (State::WaitClientHello, HandshakeType::ClientHello) => self.answer(plaintext, body),
            (State::WaitFinished, HandshakeType::Finished) => self.take_finished(body),
            _ => Err(TlsError::UnexpectedMessage),
        }
    }

    /// Checks the client's `Finished` and opens the connection.
    fn take_finished(&mut self, code: &[u8]) -> Result<(), TlsError> {
        let key = self
            .client_finished
            .as_ref()
            .ok_or(TlsError::UnexpectedMessage)?;
        let expected = verify_data(self.suite, key.as_bytes(), self.expected.as_bytes());
        if !ct_eq(expected.as_bytes(), code).is_true() {
            return Err(TlsError::BadSignature);
        }
        self.state = State::Connected;
        Ok(())
    }

    /// Reads one record of application data into `out`.
    fn step_data(&mut self, out: &mut [u8]) -> Result<usize, TlsError> {
        if self.state != State::Connected {
            return Err(TlsError::UnexpectedMessage);
        }
        let available = self.incoming.get(..self.incoming_len).unwrap_or(&[]);
        let Some((header, _, used)) = record::read(available)? else {
            return Ok(0);
        };
        if header.content_type != ContentType::ApplicationData {
            return Err(TlsError::UnexpectedMessage);
        }

        let (kind, length) = {
            let record = self.incoming.get_mut(..used).ok_or(TlsError::BadRecord)?;
            let (head, body) = record.split_at_mut(HEADER_LEN);
            let mut header_copy = [0u8; HEADER_LEN];
            for (slot, byte) in header_copy.iter_mut().zip(head.iter()) {
                *slot = *byte;
            }
            let keys = self
                .read_application
                .as_mut()
                .ok_or(TlsError::UnexpectedMessage)?;
            let (opened, plaintext) = keys.open(&header_copy, body)?;
            let room = out
                .get_mut(..plaintext.len())
                .ok_or(TlsError::BufferTooSmall)?;
            for (slot, byte) in room.iter_mut().zip(plaintext) {
                *slot = *byte;
            }
            (opened, plaintext.len())
        };
        self.incoming.copy_within(used..self.incoming_len, 0);
        self.incoming_len = self.incoming_len.saturating_sub(used);

        match kind {
            ContentType::ApplicationData => Ok(length),
            ContentType::Alert => {
                self.take_alert(out.get(..length).unwrap_or(&[]))?;
                Ok(0)
            }
            // A `KeyUpdate` or a ticket is a message this server does not
            // answer, and a `ClientHello` here is out of place.
            ContentType::Handshake | ContentType::ChangeCipherSpec => {
                Err(TlsError::UnexpectedMessage)
            }
        }
    }
}

/// Writes an unprotected record.
fn write_plain(
    kind: ContentType,
    body: &[u8],
    outgoing: &mut [u8],
    used: &mut usize,
) -> Result<(), TlsError> {
    let start = *used;
    let room = outgoing.get_mut(start..).ok_or(TlsError::BufferTooSmall)?;
    if room.len() < HEADER_LEN.wrapping_add(body.len()) {
        return Err(TlsError::BufferTooSmall);
    }
    record::write_header(kind, body.len(), room)?;
    let target = room
        .get_mut(HEADER_LEN..HEADER_LEN.wrapping_add(body.len()))
        .ok_or(TlsError::BufferTooSmall)?;
    for (slot, byte) in target.iter_mut().zip(body) {
        *slot = *byte;
    }
    *used = start.wrapping_add(HEADER_LEN).wrapping_add(body.len());
    Ok(())
}

/// Writes a protected record.
fn seal_into(
    keys: Option<&mut RecordProtection>,
    kind: ContentType,
    body: &[u8],
    outgoing: &mut [u8],
    used: &mut usize,
) -> Result<(), TlsError> {
    let keys = keys.ok_or(TlsError::UnexpectedMessage)?;
    let room = outgoing.get_mut(*used..).ok_or(TlsError::BufferTooSmall)?;
    let length = keys.seal(kind, body, room)?;
    *used = used.wrapping_add(length);
    Ok(())
}

impl Connection<'_> {
    /// Answers a `ClientHello` with the whole server flight.
    ///
    /// `message` is the handshake message with its header, which is what
    /// the transcript is over; `body` is what follows the header.
    fn answer(&mut self, message: &[u8], body: &[u8]) -> Result<(), TlsError> {
        let hello = Hello::read(body, self.config.scheme)?;
        self.suite = hello.suite;
        self.chosen = self.chosen_protocol(hello.protocols);

        let public = x25519::base_point(&self.config.ephemeral);
        let shared = x25519::x25519(&self.config.ephemeral, &hello.share)
            .map_err(|_| TlsError::NoSharedSecret)?;

        // The `ServerHello`, in the clear, and the compatibility record
        // behind it (RFC 8446, section 5).
        let mut buffer = [0u8; 256];
        let server_hello = write_server_hello(self.suite, hello.session_id, &public, &mut buffer)?;
        self.transcript.update(message);
        self.transcript.update(server_hello);
        write_plain(
            ContentType::Handshake,
            server_hello,
            self.outgoing,
            &mut self.outgoing_len,
        )?;
        write_plain(
            ContentType::ChangeCipherSpec,
            &[0x01],
            self.outgoing,
            &mut self.outgoing_len,
        )?;

        // The keys of the handshake.
        let mut schedule = Schedule::new(self.suite);
        schedule.advance(&shared)?;
        let after_hellos = self.transcript.hash(self.suite);
        let client_secret = schedule.derive(b"c hs traffic", after_hellos.as_bytes())?;
        let server_secret = schedule.derive(b"s hs traffic", after_hellos.as_bytes())?;
        self.write_handshake = Some(RecordProtection::new(traffic_keys(
            self.suite,
            server_secret.as_bytes(),
        )?)?);
        self.read_handshake = Some(RecordProtection::new(traffic_keys(
            self.suite,
            client_secret.as_bytes(),
        )?)?);
        self.client_finished = Some(finished_key(self.suite, client_secret.as_bytes())?);

        // The flight the client decrypts: the extensions, the chain, the
        // signature over everything so far, and this side's `Finished`.
        self.handshake_len = 0;
        let extensions = self.write_encrypted_extensions()?;
        self.hash_message(extensions);
        let certificate = self.write_certificate()?;
        self.hash_message(certificate);

        let over = self.transcript.hash(self.suite);
        let verify = self.write_certificate_verify(over.as_bytes())?;
        self.hash_message(verify);

        let before_finished = self.transcript.hash(self.suite);
        let key = finished_key(self.suite, server_secret.as_bytes())?;
        let code = verify_data(self.suite, key.as_bytes(), before_finished.as_bytes());
        let finished = self.append(|out| write_finished(code.as_bytes(), out))?;
        self.hash_message(finished);

        let flight = self
            .handshake
            .get(..self.handshake_len)
            .ok_or(TlsError::BufferTooSmall)?;
        let room = self
            .outgoing
            .get_mut(self.outgoing_len..)
            .ok_or(TlsError::BufferTooSmall)?;
        let sealed = self
            .write_handshake
            .as_mut()
            .ok_or(TlsError::UnexpectedMessage)?
            .seal(ContentType::Handshake, flight, room)?;
        self.outgoing_len = self.outgoing_len.wrapping_add(sealed);
        self.handshake_len = 0;

        // The keys of the connection.
        let after_finished = self.transcript.hash(self.suite);
        self.expected = Secret::from_slice(after_finished.as_bytes());
        schedule.advance_to_master()?;
        let client_application = schedule.derive(b"c ap traffic", after_finished.as_bytes())?;
        let server_application = schedule.derive(b"s ap traffic", after_finished.as_bytes())?;
        self.write_application = Some(RecordProtection::new(traffic_keys(
            self.suite,
            server_application.as_bytes(),
        )?)?);
        self.read_application = Some(RecordProtection::new(traffic_keys(
            self.suite,
            client_application.as_bytes(),
        )?)?);
        self.state = State::WaitFinished;
        Ok(())
    }

    /// The protocol to answer with: as many bytes of the configured one as
    /// the client offered it, and none where it did not.
    fn chosen_protocol(&self, offered: Option<&[u8]>) -> usize {
        let wanted = self.config.alpn;
        if wanted.is_empty() {
            return 0;
        }
        let Some(offered) = offered else {
            return 0;
        };
        // The extension carries a list of names, each one a vector of its
        // own (RFC 7301, section 3.1).
        let Ok(list) = Reader::new(offered).vector16() else {
            return 0;
        };
        let mut reader = Reader::new(list);
        while !reader.is_empty() {
            match reader.vector8() {
                Ok(name) if name == wanted => return wanted.len(),
                Ok(_) => {}
                Err(_) => return 0,
            }
        }
        0
    }

    /// Writes one message into the flight and answers where it lies, so
    /// that the caller can hash it without borrowing the whole server.
    fn append<F>(&mut self, write: F) -> Result<(usize, usize), TlsError>
    where
        F: FnOnce(&mut [u8]) -> Result<usize, TlsError>,
    {
        let start = self.handshake_len;
        let room = self
            .handshake
            .get_mut(start..)
            .ok_or(TlsError::BufferTooSmall)?;
        let length = write(room)?;
        self.handshake_len = start.wrapping_add(length);
        Ok((start, self.handshake_len))
    }

    /// Adds the message at `range` of the flight to the transcript.
    fn hash_message(&mut self, range: (usize, usize)) {
        let (start, end) = range;
        let message = self.handshake.get(start..end).unwrap_or(&[]);
        self.transcript.update(message);
    }

    /// `EncryptedExtensions`, carrying the chosen protocol where there is
    /// one.
    fn write_encrypted_extensions(&mut self) -> Result<(usize, usize), TlsError> {
        let chosen = self.config.alpn.get(..self.chosen).unwrap_or(&[]);
        self.append(|out| {
            let mut writer = Writer::new(out);
            writer.u8(HandshakeType::EncryptedExtensions.to_byte())?;
            writer.vector24(|body| {
                body.vector16(|extensions| {
                    if chosen.is_empty() {
                        return Ok(());
                    }
                    extensions.u16(ALPN)?;
                    extensions.vector16(|value| {
                        value.vector16(|list| list.vector8(|name| name.bytes(chosen)))
                    })
                })
            })?;
            Ok(writer.len())
        })
    }

    /// The `Certificate` message, the leaf first and no extension on any
    /// entry.
    fn write_certificate(&mut self) -> Result<(usize, usize), TlsError> {
        let chain = self.config.chain;
        self.append(|out| {
            let mut writer = Writer::new(out);
            writer.u8(HandshakeType::Certificate.to_byte())?;
            writer.vector24(|body| {
                body.vector8(|context| context.bytes(&[]))?;
                body.vector24(|entries| {
                    for certificate in chain {
                        entries.vector24(|data| data.bytes(certificate))?;
                        entries.vector16(|_| Ok(()))?;
                    }
                    Ok(())
                })
            })?;
            Ok(writer.len())
        })
    }

    /// The `CertificateVerify` over the transcript so far.
    fn write_certificate_verify(&mut self, transcript: &[u8]) -> Result<(usize, usize), TlsError> {
        let mut content = [0x20u8; 64 + CONTEXT.len() + 1 + MAX_HASH];
        let mut writer = Writer::new(&mut content);
        writer.bytes(&[0x20; 64])?;
        writer.bytes(CONTEXT)?;
        writer.u8(0x00)?;
        writer.bytes(transcript)?;
        let signed_len = writer.len();

        let mut signature = [0u8; MAX_SIGNATURE];
        let content_bytes = content.get(..signed_len).ok_or(TlsError::Encode)?;
        let length = self
            .config
            .key
            .sign(content_bytes, &mut signature)
            .map_err(|_| TlsError::BadSignature)?;
        let signature = signature.get(..length).ok_or(TlsError::Encode)?;
        let scheme = self.config.scheme;

        self.append(|out| {
            let mut writer = Writer::new(out);
            writer.u8(HandshakeType::CertificateVerify.to_byte())?;
            writer.vector24(|body| {
                body.u16(scheme)?;
                body.vector16(|value| value.bytes(signature))
            })?;
            Ok(writer.len())
        })
    }
}

/// What this server reads out of a `ClientHello`.
struct Hello<'a> {
    /// The identifier to echo, which is the client's and not this side's.
    session_id: &'a [u8],
    /// The suite both sides have.
    suite: CipherSuite,
    /// The client's X25519 share.
    share: [u8; 32],
    /// The protocols it offered, as the extension carries them.
    protocols: Option<&'a [u8]>,
}

impl<'a> Hello<'a> {
    /// Reads what this server needs out of the message body, refusing a
    /// client that would not take `scheme` on the `CertificateVerify`.
    fn read(body: &'a [u8], scheme: u16) -> Result<Hello<'a>, TlsError> {
        let mut reader = Reader::new(body);
        let _legacy_version = reader.u16()?;
        let _random = reader.take(32)?;
        let session_id = reader.vector8()?;
        let suites = reader.vector16()?;
        let _compression = reader.vector8()?;
        let mut extensions = Reader::new(reader.vector16()?);

        let mut versions = None;
        let mut share = None;
        let mut schemes = None;
        let mut protocols = None;
        while !extensions.is_empty() {
            let kind = extensions.u16()?;
            let value = extensions.vector16()?;
            match kind {
                SUPPORTED_VERSIONS => versions = Some(value),
                KEY_SHARE => share = Some(value),
                SIGNATURE_ALGORITHMS => schemes = Some(value),
                ALPN => protocols = Some(value),
                _ => {}
            }
        }

        let versions = versions.ok_or(TlsError::MissingExtension)?;
        if !offers(Reader::new(versions).vector8()?, VERSION_TLS13) {
            return Err(TlsError::UnsupportedVersion);
        }
        let schemes = schemes.ok_or(TlsError::MissingExtension)?;
        if !offers(Reader::new(schemes).vector16()?, scheme) {
            // RFC 8446, section 4.4.2.2: a server with no signature the
            // client accepts sends `handshake_failure`, which is the
            // alert this error carries.
            return Err(TlsError::UnsupportedSuite);
        }

        Ok(Hello {
            session_id,
            suite: suite_of(suites)?,
            share: share_of(share.ok_or(TlsError::MissingExtension)?)?,
            protocols,
        })
    }
}

/// Whether a vector of code points holds `wanted`.
fn offers(list: &[u8], wanted: u16) -> bool {
    list.as_chunks::<2>()
        .0
        .iter()
        .any(|pair| u16::from_be_bytes(*pair) == wanted)
}

/// The first suite of [`SUITES`] the client offered.
fn suite_of(offered: &[u8]) -> Result<CipherSuite, TlsError> {
    for suite in SUITES {
        if offers(offered, suite.code()) {
            return Ok(suite);
        }
    }
    Err(TlsError::UnsupportedSuite)
}

/// The X25519 value out of the shares a client offered.
fn share_of(extension: &[u8]) -> Result<[u8; 32], TlsError> {
    let mut list = Reader::new(Reader::new(extension).vector16()?);
    while !list.is_empty() {
        let group = list.u16()?;
        let value = list.vector16()?;
        if group == GROUP_X25519 {
            return value.try_into().map_err(|_| TlsError::IllegalParameter);
        }
    }
    // A client that offers no X25519 share would need a
    // `HelloRetryRequest`, which this server does not send.
    Err(TlsError::MissingExtension)
}

/// Writes a `ServerHello` and answers the bytes of it.
fn write_server_hello<'a>(
    suite: CipherSuite,
    session_id: &[u8],
    public: &[u8; 32],
    out: &'a mut [u8],
) -> Result<&'a [u8], TlsError> {
    let mut writer = Writer::new(out);
    writer.u8(HandshakeType::ServerHello.to_byte())?;
    writer.vector24(|body| {
        body.u16(0x0303)?;
        body.bytes(&[0x44; 32])?;
        body.vector8(|id| id.bytes(session_id))?;
        body.u16(suite.code())?;
        body.u8(0)?;
        body.vector16(|extensions| {
            extensions.u16(SUPPORTED_VERSIONS)?;
            extensions.vector16(|value| value.u16(VERSION_TLS13))?;
            extensions.u16(KEY_SHARE)?;
            extensions.vector16(|value| {
                value.u16(GROUP_X25519)?;
                value.vector16(|point| point.bytes(public))
            })
        })
    })?;
    let length = writer.len();
    out.get(..length).ok_or(TlsError::Encode)
}
