// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The client: the states it goes through, and the interface a caller
//! drives it with.
//!
//! The connection moves no bytes. `read_tls` takes what arrived, `poll`
//! makes what progress it can, `write_tls` hands back what to send. What
//! opens the socket, waits on it, and closes it is the caller's business.
//!
//! Invariants: the transcript receives every handshake message exactly
//! once, in the order it appeared; a message is dispatched only in the
//! state that expects it, and any other message ends the connection; once
//! an error has ended a connection it is remembered, so every later call
//! returns the same error rather than a different one.

use audhsos_der::Reader as DerReader;
use audhsos_x509::{Certificate, ServerName, SubjectPublicKey, verify_chain};
use crypto_ct::ct_eq;
use crypto_ec::x25519;
use crypto_rng::Rng;

use crate::alert::Alert;
use crate::codec::Writer;
use crate::config::ClientConfig;
use crate::error::TlsError;
use crate::handshake::{
    CertificateChain, CertificateVerify, ClientHelloParams, ECDSA_SECP256R1_SHA256,
    ECDSA_SECP384R1_SHA384, ED25519, EncryptedExtensions, HandshakeType, RSA_PKCS1_SHA256,
    RSA_PKCS1_SHA384, RSA_PKCS1_SHA512, RSA_PSS_RSAE_SHA256, RSA_PSS_RSAE_SHA384,
    RSA_PSS_RSAE_SHA512, ServerHello, read_key_update, read_message, write_client_hello,
    write_finished,
};
use crate::keys::{Schedule, finished_key, next_traffic_secret, traffic_keys, verify_data};
use crate::protection::RecordProtection;
use crate::record::{self, ContentType, HEADER_LEN, MAX_PLAINTEXT, MAX_RECORD};
use crate::secret::Secret;
use crate::suite::CipherSuite;
use crate::transcript::Transcript;

/// The most a subject public key of the four supported kinds occupies,
/// wrapped in the information that names its algorithm.
///
/// RSA at four thousand and ninety-six bits is the widest at 550 bytes:
/// fifteen for the algorithm identifier, five hundred and twenty-six for
/// the inner sequence of two integers, five for the bit string around it,
/// four for the outer sequence. P-384 takes 120, P-256 91, and Ed25519
/// fewer still.
///
/// The alternative, borrowing the leaf's `spki_bytes` instead of copying
/// them, would tie a borrow across two handshake messages to save the
/// bytes, and is not worth it.
const MAX_SPKI: usize = 550;

/// The context string of a signature a server makes over the handshake.
const SERVER_CONTEXT: &[u8] = b"TLS 1.3, server CertificateVerify";

/// What the caller should do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Nothing can happen until more bytes arrive.
    WantsRead,
    /// There are bytes to send.
    WantsWrite,
    /// The handshake is done and application data may flow.
    Handshaked,
    /// The peer closed its side.
    PeerClosed,
}

/// Where in the handshake the client is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Waiting for the server's answer.
    WaitServerHello,
    /// Waiting for the extensions that did not fit in it.
    WaitEncryptedExtensions,
    /// Waiting for the certificate chain.
    WaitCertificate,
    /// Waiting for the signature over the handshake.
    WaitCertificateVerify,
    /// Waiting for the server to finish.
    WaitFinished,
    /// Done; application data may flow.
    Connected,
    /// Closed, by this side or the other.
    Closed,
}

/// The buffers a connection works in, all owned by the caller.
pub struct Buffers<'a> {
    /// Where bytes from the transport are put.
    pub incoming: &'a mut [u8],
    /// Where bytes for the transport are taken from.
    pub outgoing: &'a mut [u8],
    /// Where handshake messages are reassembled.
    pub handshake: &'a mut [u8],
}

/// The smallest incoming buffer that can hold any record.
pub const MIN_INCOMING: usize = MAX_RECORD;
/// The smallest outgoing buffer that can hold a record and a flight.
pub const MIN_OUTGOING: usize = MAX_RECORD;
/// The smallest buffer a handshake message can be reassembled in.
pub const MIN_HANDSHAKE: usize = 16_384;

/// Everything about a connection but its buffers.
struct Machine {
    /// Where the handshake stands.
    state: State,
    /// The running hash.
    transcript: Transcript,
    /// The suite, once the server has chosen one.
    suite: Option<CipherSuite>,
    /// The schedule, once it has begun.
    schedule: Option<Schedule>,
    /// The private value of the key exchange.
    private_key: [u8; 32],
    /// Protection for what this client sends during the handshake.
    client_handshake: Option<RecordProtection>,
    /// Protection for what the server sends during the handshake.
    server_handshake: Option<RecordProtection>,
    /// Protection for what this client sends afterwards.
    client_application: Option<RecordProtection>,
    /// Protection for what the server sends afterwards.
    server_application: Option<RecordProtection>,
    /// The secret the client's application keys came from.
    client_secret: Option<Secret>,
    /// The secret the server's application keys came from.
    server_secret: Option<Secret>,
    /// The key this client's `Finished` is authenticated with.
    client_finished: Option<Secret>,
    /// The key the server's `Finished` is authenticated with.
    server_finished: Option<Secret>,
    /// The server's public key information, as it was encoded.
    server_spki: [u8; MAX_SPKI],
    /// How much of it there is.
    server_spki_len: usize,
    /// The protocol the server chose.
    alpn: [u8; 32],
    /// How much of it there is.
    alpn_len: usize,
    /// The error that ended the connection, if one did.
    poison: Option<TlsError>,
    /// Whether the peer has closed its side.
    peer_closed: bool,
}

/// A client connection.
pub struct Connection<'a> {
    /// What the caller decided.
    config: &'a ClientConfig<'a>,
    /// The protocol state.
    machine: Machine,
    /// Bytes from the transport.
    incoming: &'a mut [u8],
    /// How many of them there are.
    incoming_len: usize,
    /// Bytes for the transport.
    outgoing: &'a mut [u8],
    /// How many of them there are.
    outgoing_len: usize,
    /// Handshake messages being reassembled.
    handshake: &'a mut [u8],
    /// How much of it is filled.
    handshake_len: usize,
}

impl<'a> Connection<'a> {
    /// A connection that has written its first flight.
    ///
    /// # Errors
    ///
    /// [`TlsError::BufferTooSmall`] when a buffer is below its minimum,
    /// and whatever the generator reports.
    pub fn new(
        config: &'a ClientConfig<'a>,
        rng: &mut dyn Rng,
        buffers: Buffers<'a>,
    ) -> Result<Connection<'a>, TlsError> {
        if buffers.incoming.len() < MIN_INCOMING
            || buffers.outgoing.len() < MIN_OUTGOING
            || buffers.handshake.len() < MIN_HANDSHAKE
        {
            return Err(TlsError::BufferTooSmall);
        }

        let mut private_key = [0u8; 32];
        let mut random = [0u8; 32];
        let mut session_id = [0u8; 32];
        rng.fill(&mut private_key)
            .map_err(|_| TlsError::NoSharedSecret)?;
        rng.fill(&mut random)
            .map_err(|_| TlsError::NoSharedSecret)?;
        rng.fill(&mut session_id)
            .map_err(|_| TlsError::NoSharedSecret)?;
        let public_key = x25519::base_point(&private_key);

        let Buffers {
            incoming,
            outgoing,
            handshake,
        } = buffers;
        let mut connection = Connection {
            config,
            machine: Machine {
                state: State::WaitServerHello,
                transcript: Transcript::new(),
                suite: None,
                schedule: None,
                private_key,
                client_handshake: None,
                server_handshake: None,
                client_application: None,
                server_application: None,
                client_secret: None,
                server_secret: None,
                client_finished: None,
                server_finished: None,
                server_spki: [0u8; MAX_SPKI],
                server_spki_len: 0,
                alpn: [0u8; 32],
                alpn_len: 0,
                poison: None,
                peer_closed: false,
            },
            incoming,
            incoming_len: 0,
            outgoing,
            outgoing_len: 0,
            handshake,
            handshake_len: 0,
        };

        let params = ClientHelloParams {
            random: &random,
            session_id: &session_id,
            suites: config.suites,
            key_share: &public_key,
            server_name: Some(config.server_name),
            alpn: config.alpn,
        };
        let mut message = [0u8; 1024];
        let length = write_client_hello(&params, &mut message)?;
        let hello = message.get(..length).ok_or(TlsError::BufferTooSmall)?;
        connection.machine.transcript.update(hello);
        connection.write_plain(ContentType::Handshake, hello)?;
        // The compatibility record of appendix D.4, which makes a
        // middlebox that expects a resumption see one.
        connection.write_plain(ContentType::ChangeCipherSpec, &[0x01])?;
        Ok(connection)
    }

    /// The protocol the server chose, if any.
    #[must_use]
    pub fn alpn(&self) -> Option<&[u8]> {
        self.machine
            .alpn
            .get(..self.machine.alpn_len)
            .filter(|chosen| !chosen.is_empty())
    }

    /// Whether the handshake is done.
    #[must_use]
    pub const fn is_handshaked(&self) -> bool {
        matches!(self.machine.state, State::Connected)
    }

    /// Takes bytes from the transport, and says how many it took.
    ///
    /// # Errors
    ///
    /// The error that ended the connection, if one did.
    pub fn read_tls(&mut self, input: &[u8]) -> Result<usize, TlsError> {
        self.machine.check()?;
        let room = self.incoming.len().saturating_sub(self.incoming_len);
        let taken = room.min(input.len());
        let target = self
            .incoming
            .get_mut(self.incoming_len..self.incoming_len.wrapping_add(taken))
            .ok_or(TlsError::BufferTooSmall)?;
        for (slot, byte) in target.iter_mut().zip(input) {
            *slot = *byte;
        }
        self.incoming_len = self.incoming_len.wrapping_add(taken);
        Ok(taken)
    }

    /// Hands bytes to the transport, and says how many it handed over.
    ///
    /// This works while a connection is poisoned, so that the alert which
    /// says why can still be sent.
    ///
    /// # Errors
    ///
    /// Never; the result is a `Result` so that the shape of the interface
    /// does not change when it can.
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
    /// Whatever ended the connection. The peer is told about it in an
    /// alert that `write_tls` will hand over.
    pub fn poll(&mut self) -> Result<Event, TlsError> {
        self.machine.check()?;
        while self.machine.state != State::Closed {
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
        if self.machine.peer_closed {
            return Ok(Event::PeerClosed);
        }
        if self.machine.state == State::Connected {
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
        self.machine.check()?;
        if self.machine.state != State::Connected {
            return Err(TlsError::UnexpectedMessage);
        }
        let taken = plaintext.len().min(MAX_PLAINTEXT);
        let piece = plaintext.get(..taken).unwrap_or(&[]);
        let result = write_protected(
            self.machine.client_application.as_mut(),
            ContentType::ApplicationData,
            piece,
            self.outgoing,
            &mut self.outgoing_len,
        );
        match result {
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
    /// `out` must be able to hold a whole record's plaintext.
    ///
    /// # Errors
    ///
    /// [`TlsError::BufferTooSmall`] for a buffer below the record limit,
    /// and whatever ended the connection.
    pub fn recv(&mut self, out: &mut [u8]) -> Result<usize, TlsError> {
        self.machine.check()?;
        if out.len() < MAX_PLAINTEXT {
            return Err(TlsError::BufferTooSmall);
        }
        match self.step_data(out) {
            Ok(Some(length)) => Ok(length),
            Ok(None) => Ok(0),
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
        self.machine.check()?;
        let result = write_protected(
            self.machine.client_application.as_mut(),
            ContentType::Alert,
            &[crate::alert::WARNING, Alert::CloseNotify.code()],
            self.outgoing,
            &mut self.outgoing_len,
        );
        self.machine.state = State::Closed;
        result
    }

    /// Writes a record that is not protected, which only the first flight
    /// is.
    fn write_plain(&mut self, kind: ContentType, body: &[u8]) -> Result<(), TlsError> {
        let start = self.outgoing_len;
        let room = self
            .outgoing
            .get_mut(start..)
            .ok_or(TlsError::BufferTooSmall)?;
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
        self.outgoing_len = start.wrapping_add(HEADER_LEN).wrapping_add(body.len());
        Ok(())
    }

    /// Remembers the error and tells the peer about it, when there is
    /// anything to tell.
    ///
    /// The alert goes out under the keys of the epoch the connection has
    /// reached: the application keys once they exist, the handshake keys
    /// before that, and in the clear only while there are no keys at all,
    /// which is the window between the first flight and the server's
    /// answer. An alert the peer cannot open is no alert.
    ///
    /// An error the peer's own alert caused sends nothing.
    /// [`Alert::for_error`] answers `None` for it, and RFC 8446 section
    /// 6.2 is why: a fatal alert closes the connection on both sides at
    /// once, so the peer is not waiting to hear back. The connection is
    /// still poisoned with the reason, which is what the caller reads.
    fn fail(&mut self, error: TlsError) {
        if self.machine.poison.is_none() {
            self.machine.poison = Some(error);
            if let Some(alert) = Alert::for_error(error) {
                let body = [alert.level(), alert.code()];
                let keys = if self.machine.client_application.is_some() {
                    self.machine.client_application.as_mut()
                } else {
                    self.machine.client_handshake.as_mut()
                };
                let sent = write_protected(
                    keys,
                    ContentType::Alert,
                    &body,
                    self.outgoing,
                    &mut self.outgoing_len,
                );
                if sent.is_err() {
                    let _ = self.write_plain(ContentType::Alert, &body);
                }
            }
        }
        self.machine.state = State::Closed;
    }
}

/// Writes a protected record, or a plain one when there are no keys yet.
fn write_protected(
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

impl Machine {
    /// Refuses to go on once something has gone wrong.
    const fn check(&self) -> Result<(), TlsError> {
        match self.poison {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Connection<'_> {
    /// Processes one record of the handshake. Says whether it did.
    fn step(&mut self) -> Result<bool, TlsError> {
        if self.machine.state == State::Connected {
            // Everything after the handshake is driven by `recv`, because
            // only decrypting a record says whether it is data or a
            // message, and a record can be decrypted once.
            return Ok(false);
        }
        let available = self.incoming.get(..self.incoming_len).unwrap_or(&[]);
        let Some((header, _, used)) = record::read(available)? else {
            return Ok(false);
        };

        let Connection {
            config,
            machine,
            incoming,
            incoming_len,
            outgoing,
            outgoing_len,
            handshake,
            handshake_len,
        } = self;

        {
            let record = incoming.get_mut(..used).ok_or(TlsError::BadRecord)?;
            let (head, body) = record.split_at_mut(HEADER_LEN);
            let mut header_copy = [0u8; HEADER_LEN];
            for (slot, byte) in header_copy.iter_mut().zip(head.iter()) {
                *slot = *byte;
            }

            let (kind, plaintext) = match (header.content_type, machine.server_handshake.as_mut()) {
                (ContentType::ChangeCipherSpec, _) => {
                    // RFC 8446 section 5: the compatibility record carries
                    // the single byte one, and anything else it carries is
                    // an unexpected message.
                    if body.len() != 1 || body.first() != Some(&0x01) {
                        return Err(TlsError::UnexpectedMessage);
                    }
                    (ContentType::ChangeCipherSpec, &[][..])
                }
                (ContentType::ApplicationData, Some(keys)) => keys.open(&header_copy, body)?,
                (ContentType::ApplicationData, None) => {
                    return Err(TlsError::UnexpectedMessage);
                }
                (kind, _) => (kind, &body[..]),
            };

            match kind {
                ContentType::ChangeCipherSpec => {}
                ContentType::Alert => {
                    machine.take_alert(plaintext)?;
                }
                ContentType::Handshake => {
                    let end = handshake_len.wrapping_add(plaintext.len());
                    let room = handshake
                        .get_mut(*handshake_len..end)
                        .ok_or(TlsError::BufferTooSmall)?;
                    for (slot, byte) in room.iter_mut().zip(plaintext) {
                        *slot = *byte;
                    }
                    *handshake_len = end;
                }
                ContentType::ApplicationData => return Err(TlsError::UnexpectedMessage),
            }
        }

        incoming.copy_within(used..*incoming_len, 0);
        *incoming_len = incoming_len.saturating_sub(used);

        machine.drain_messages(config, handshake, handshake_len, outgoing, outgoing_len)?;
        Ok(true)
    }

    /// Processes records until one carries application data.
    fn step_data(&mut self, out: &mut [u8]) -> Result<Option<usize>, TlsError> {
        loop {
            if self.machine.state != State::Connected {
                return Ok(None);
            }
            let available = self.incoming.get(..self.incoming_len).unwrap_or(&[]);
            let Some((header, _, used)) = record::read(available)? else {
                return Ok(None);
            };

            let Connection {
                machine,
                incoming,
                incoming_len,
                outgoing,
                outgoing_len,
                ..
            } = self;

            let mut delivered = None;
            {
                let record = incoming.get_mut(..used).ok_or(TlsError::BadRecord)?;
                let (head, body) = record.split_at_mut(HEADER_LEN);
                let mut header_copy = [0u8; HEADER_LEN];
                for (slot, byte) in header_copy.iter_mut().zip(head.iter()) {
                    *slot = *byte;
                }

                // RFC 8446 section 5: the window for the compatibility
                // record closes with the peer's `Finished`, and one that
                // arrives after it is an unexpected record.
                if header.content_type == ContentType::ChangeCipherSpec {
                    return Err(TlsError::UnexpectedMessage);
                }
                {
                    let keys = machine
                        .server_application
                        .as_mut()
                        .ok_or(TlsError::UnexpectedMessage)?;
                    let (kind, plaintext) = keys.open(&header_copy, body)?;
                    match kind {
                        ContentType::ApplicationData => {
                            let room = out
                                .get_mut(..plaintext.len())
                                .ok_or(TlsError::BufferTooSmall)?;
                            for (slot, byte) in room.iter_mut().zip(plaintext) {
                                *slot = *byte;
                            }
                            delivered = Some(plaintext.len());
                        }
                        ContentType::Alert => machine.take_alert(plaintext)?,
                        ContentType::Handshake => {
                            machine.after_handshake(plaintext, outgoing, outgoing_len)?;
                        }
                        ContentType::ChangeCipherSpec => {
                            return Err(TlsError::UnexpectedMessage);
                        }
                    }
                }
            }

            incoming.copy_within(used..*incoming_len, 0);
            *incoming_len = incoming_len.saturating_sub(used);
            if let Some(length) = delivered {
                return Ok(Some(length));
            }
            if self.machine.peer_closed {
                return Ok(None);
            }
        }
    }
}

impl Machine {
    /// Reads an alert, which either closes the connection or ends it.
    ///
    /// RFC 8446 section 6: in TLS 1.3 the level says nothing, and every
    /// alert but `close_notify` is fatal whatever level it carries. So the
    /// code is read and the level is not. A code this client does not know
    /// is fatal too, and is reported as the number the peer sent rather
    /// than as a message about the record it arrived in.
    fn take_alert(&mut self, body: &[u8]) -> Result<(), TlsError> {
        let [_level, description] = body else {
            return Err(TlsError::Decode);
        };
        if Alert::from_code(*description) == Some(Alert::CloseNotify) {
            self.peer_closed = true;
            self.state = State::Closed;
            return Ok(());
        }
        Err(TlsError::PeerAlert(*description))
    }

    /// Consumes every complete handshake message that has arrived.
    fn drain_messages(
        &mut self,
        config: &ClientConfig<'_>,
        buffer: &mut [u8],
        filled: &mut usize,
        outgoing: &mut [u8],
        used: &mut usize,
    ) -> Result<(), TlsError> {
        loop {
            let available = buffer.get(..*filled).unwrap_or(&[]);
            let Some((kind, _, length)) = read_message(available)? else {
                return Ok(());
            };
            let whole = available.get(..length).ok_or(TlsError::Decode)?;
            let (_, body, _) = read_message(whole)?.ok_or(TlsError::Decode)?;
            self.handle(config, kind, body, whole, outgoing, used)?;

            buffer.copy_within(length..*filled, 0);
            *filled = filled.saturating_sub(length);
        }
    }

    /// One handshake message, in the state that expects it.
    fn handle(
        &mut self,
        config: &ClientConfig<'_>,
        kind: HandshakeType,
        body: &[u8],
        whole: &[u8],
        outgoing: &mut [u8],
        used: &mut usize,
    ) -> Result<(), TlsError> {
        match (self.state, kind) {
            (State::WaitServerHello, HandshakeType::ServerHello) => {
                self.take_server_hello(body, whole)
            }
            (State::WaitEncryptedExtensions, HandshakeType::EncryptedExtensions) => {
                let extensions = EncryptedExtensions::parse(body)?;
                if let Some(chosen) = extensions.alpn {
                    let room = self.alpn.get_mut(..chosen.len()).ok_or(TlsError::Decode)?;
                    for (slot, byte) in room.iter_mut().zip(chosen) {
                        *slot = *byte;
                    }
                    self.alpn_len = chosen.len();
                }
                self.transcript.update(whole);
                self.state = State::WaitCertificate;
                Ok(())
            }
            (State::WaitCertificate, HandshakeType::Certificate) => {
                self.take_certificate(config, body)?;
                self.transcript.update(whole);
                self.state = State::WaitCertificateVerify;
                Ok(())
            }
            (State::WaitCertificateVerify, HandshakeType::CertificateVerify) => {
                self.take_certificate_verify(body)?;
                self.transcript.update(whole);
                self.state = State::WaitFinished;
                Ok(())
            }
            (State::WaitFinished, HandshakeType::Finished) => {
                self.take_finished(body, whole, outgoing, used)
            }
            _ => Err(TlsError::UnexpectedMessage),
        }
    }

    /// A message that arrives once the handshake is done.
    fn after_handshake(
        &mut self,
        plaintext: &[u8],
        outgoing: &mut [u8],
        used: &mut usize,
    ) -> Result<(), TlsError> {
        let mut rest = plaintext;
        while let Some((kind, body, length)) = read_message(rest)? {
            match kind {
                HandshakeType::NewSessionTicket => {
                    // This client resumes nothing, so a ticket is read for
                    // its shape and dropped.
                }
                HandshakeType::KeyUpdate => {
                    let asked = read_key_update(body)?;
                    self.update_server_keys()?;
                    if asked {
                        self.update_client_keys(outgoing, used)?;
                    }
                }
                _ => return Err(TlsError::UnexpectedMessage),
            }
            rest = rest.get(length..).unwrap_or(&[]);
        }
        Ok(())
    }
}

/// The longest chain this client reads, the leaf included.
const MAX_CHAIN: usize = 8;

impl Machine {
    /// The server's answer: the suite, the shared value, and the keys that
    /// follow from them.
    fn take_server_hello(&mut self, body: &[u8], whole: &[u8]) -> Result<(), TlsError> {
        let hello = ServerHello::parse(body)?;
        if hello.is_downgrade() {
            // The server put the sentinel of an older version in its
            // random, which means something is speaking for it.
            return Err(TlsError::IllegalParameter);
        }
        if hello.is_retry {
            // This client offers one group, so a retry can only ask for a
            // group it already sent or one it did not offer, and RFC 8446
            // forbids both.
            return Err(TlsError::IllegalParameter);
        }

        let share = hello.key_share.ok_or(TlsError::MissingExtension)?;
        let peer: &[u8; 32] = share.try_into().map_err(|_| TlsError::IllegalParameter)?;
        let shared =
            x25519::x25519(&self.private_key, peer).map_err(|_| TlsError::NoSharedSecret)?;
        self.private_key = [0u8; 32];

        let suite = hello.suite;
        self.suite = Some(suite);
        self.transcript.update(whole);
        let transcript = self.transcript.checked_hash(suite)?;

        let mut schedule = Schedule::new(suite);
        schedule.advance(&shared)?;
        let client = schedule.derive(b"c hs traffic", transcript.as_bytes())?;
        let server = schedule.derive(b"s hs traffic", transcript.as_bytes())?;

        self.client_finished = Some(finished_key(suite, client.as_bytes())?);
        self.server_finished = Some(finished_key(suite, server.as_bytes())?);
        self.client_handshake = Some(RecordProtection::new(traffic_keys(
            suite,
            client.as_bytes(),
        )?)?);
        self.server_handshake = Some(RecordProtection::new(traffic_keys(
            suite,
            server.as_bytes(),
        )?)?);
        self.schedule = Some(schedule);
        self.state = State::WaitEncryptedExtensions;
        Ok(())
    }

    /// The chain, which must reach an anchor and carry the name.
    fn take_certificate(&mut self, config: &ClientConfig<'_>, body: &[u8]) -> Result<(), TlsError> {
        let chain = CertificateChain::parse(body)?;
        let mut entries = chain.certificates();
        let first = entries.next().ok_or(TlsError::BadCertificate)??;
        let leaf = Certificate::parse(first).map_err(|_| TlsError::BadCertificate)?;

        let mut list = [leaf; MAX_CHAIN];
        let mut count = 0usize;
        for entry in entries {
            let bytes = entry?;
            // RFC 8446 section 4.4.2: the certificates after the first are
            // an aid to path building, and the list may hold ones that
            // belong to no path. A server that sends its own root sends a
            // certificate whose key this client cannot read, and refusing
            // the message for it would refuse every such server. An entry
            // that does not parse is passed over instead: it is a path
            // this client could not have taken either way. The leaf still
            // has to parse, and the path still has to reach an anchor
            // through signatures that verify.
            let Ok(parsed) = Certificate::parse(bytes) else {
                continue;
            };
            let slot = list.get_mut(count).ok_or(TlsError::BadCertificate)?;
            *slot = parsed;
            count = count.wrapping_add(1);
        }
        let intermediates = list.get(..count).ok_or(TlsError::BadCertificate)?;

        verify_chain(
            &leaf,
            intermediates,
            &config.anchors,
            ServerName::Dns(config.server_name),
            config.now,
        )
        .map_err(chain_error)?;

        let spki = leaf.spki_bytes;
        let room = self
            .server_spki
            .get_mut(..spki.len())
            .ok_or(TlsError::BadCertificate)?;
        for (slot, byte) in room.iter_mut().zip(spki) {
            *slot = *byte;
        }
        self.server_spki_len = spki.len();
        Ok(())
    }

    /// The signature over everything up to the certificate.
    fn take_certificate_verify(&self, body: &[u8]) -> Result<(), TlsError> {
        let verify = CertificateVerify::parse(body)?;
        let suite = self.suite.ok_or(TlsError::UnexpectedMessage)?;
        let transcript = self.transcript.checked_hash(suite)?;

        // RFC 8446 section 4.4.3: sixty-four spaces, the context, a zero,
        // and the transcript hash.
        let mut content = [0u8; 160];
        let mut writer = Writer::new(&mut content);
        for _ in 0..64 {
            writer.u8(0x20)?;
        }
        writer.bytes(SERVER_CONTEXT)?;
        writer.u8(0x00)?;
        writer.bytes(transcript.as_bytes())?;

        let spki = self
            .server_spki
            .get(..self.server_spki_len)
            .ok_or(TlsError::BadCertificate)?;
        let mut reader = DerReader::new(spki);
        let key = SubjectPublicKey::parse(&mut reader).map_err(|_| TlsError::BadCertificate)?;

        // RFC 8446 section 4.2.3: an ECDSA scheme names the curve as well
        // as the hash, so the scheme and the key the certificate carries
        // have to be the pair the code point stands for. X.509 binds no
        // curve to `ecdsa-with-SHA384` — a P-256 key may sign with it, and
        // `audhsos-x509` keeps that freedom for a chain — but the protocol
        // takes it away here, and a scheme that does not match the key is a
        // field inconsistent with another field rather than a signature
        // worth trying.
        //
        // The three `rsa_pkcs1_*` schemes are the sharper case, and they
        // have an arm of their own so that the refusal is visible rather
        // than a fall-through. This client offers them one message
        // earlier, because RFC 8446 section 4.2.3 gives them exactly one
        // meaning — that a certificate may be signed that way — and the
        // same section forbids them here (D-82). The offer is about the
        // chain and the refusal is about this signature.
        let algorithm = match (verify.scheme, key) {
            (ECDSA_SECP256R1_SHA256, SubjectPublicKey::EcdsaP256(_)) => {
                audhsos_x509::SignatureAlgorithm::EcdsaSha256
            }
            (ECDSA_SECP384R1_SHA384, SubjectPublicKey::EcdsaP384(_)) => {
                audhsos_x509::SignatureAlgorithm::EcdsaSha384
            }
            (ED25519, SubjectPublicKey::Ed25519(_)) => audhsos_x509::SignatureAlgorithm::Ed25519,
            (RSA_PSS_RSAE_SHA256, SubjectPublicKey::Rsa { .. }) => {
                audhsos_x509::SignatureAlgorithm::RsaPssSha256
            }
            (RSA_PSS_RSAE_SHA384, SubjectPublicKey::Rsa { .. }) => {
                audhsos_x509::SignatureAlgorithm::RsaPssSha384
            }
            (RSA_PSS_RSAE_SHA512, SubjectPublicKey::Rsa { .. }) => {
                audhsos_x509::SignatureAlgorithm::RsaPssSha512
            }
            (RSA_PKCS1_SHA256 | RSA_PKCS1_SHA384 | RSA_PKCS1_SHA512, _) => {
                return Err(TlsError::IllegalParameter);
            }
            _ => return Err(TlsError::IllegalParameter),
        };

        key.verify(algorithm, writer.written(), verify.signature)
            .map_err(|_| TlsError::BadSignature)
    }

    /// The server's `Finished`, and everything that follows it: the
    /// application keys and this client's own `Finished`.
    fn take_finished(
        &mut self,
        body: &[u8],
        whole: &[u8],
        outgoing: &mut [u8],
        used: &mut usize,
    ) -> Result<(), TlsError> {
        let suite = self.suite.ok_or(TlsError::UnexpectedMessage)?;
        let transcript = self.transcript.checked_hash(suite)?;
        let server_key = self
            .server_finished
            .as_ref()
            .ok_or(TlsError::UnexpectedMessage)?;
        let expected = verify_data(suite, server_key.as_bytes(), transcript.as_bytes());
        if !ct_eq(expected.as_bytes(), body).is_true() {
            return Err(TlsError::BadSignature);
        }
        self.transcript.update(whole);

        // The application secrets are derived over everything up to and
        // including what the server just said.
        let after_server = self.transcript.checked_hash(suite)?;
        let schedule = self.schedule.as_mut().ok_or(TlsError::UnexpectedMessage)?;
        schedule.advance_to_master()?;
        let client_secret = schedule.derive(b"c ap traffic", after_server.as_bytes())?;
        let server_secret = schedule.derive(b"s ap traffic", after_server.as_bytes())?;

        // This client's `Finished` is authenticated with its handshake key
        // over the same transcript, and travels under the handshake keys.
        let client_key = self
            .client_finished
            .as_ref()
            .ok_or(TlsError::UnexpectedMessage)?;
        let code = verify_data(suite, client_key.as_bytes(), after_server.as_bytes());
        let mut message = [0u8; 128];
        let length = write_finished(code.as_bytes(), &mut message)?;
        let finished = message.get(..length).ok_or(TlsError::BufferTooSmall)?;
        write_protected(
            self.client_handshake.as_mut(),
            ContentType::Handshake,
            finished,
            outgoing,
            used,
        )?;
        self.transcript.update(finished);

        self.client_application = Some(RecordProtection::new(traffic_keys(
            suite,
            client_secret.as_bytes(),
        )?)?);
        self.server_application = Some(RecordProtection::new(traffic_keys(
            suite,
            server_secret.as_bytes(),
        )?)?);
        self.client_secret = Some(client_secret);
        self.server_secret = Some(server_secret);
        self.client_handshake = None;
        self.server_handshake = None;
        self.state = State::Connected;
        Ok(())
    }

    /// Moves the server's keys on, as a `KeyUpdate` says to.
    fn update_server_keys(&mut self) -> Result<(), TlsError> {
        let suite = self.suite.ok_or(TlsError::UnexpectedMessage)?;
        let current = self
            .server_secret
            .as_ref()
            .ok_or(TlsError::UnexpectedMessage)?;
        let next = next_traffic_secret(suite, current.as_bytes())?;
        self.server_application = Some(RecordProtection::new(traffic_keys(
            suite,
            next.as_bytes(),
        )?)?);
        self.server_secret = Some(next);
        Ok(())
    }

    /// Moves this client's keys on, and says so.
    fn update_client_keys(
        &mut self,
        outgoing: &mut [u8],
        used: &mut usize,
    ) -> Result<(), TlsError> {
        let suite = self.suite.ok_or(TlsError::UnexpectedMessage)?;
        let mut message = [0u8; 16];
        let length = crate::handshake::write_key_update(false, &mut message)?;
        let update = message.get(..length).ok_or(TlsError::BufferTooSmall)?;
        write_protected(
            self.client_application.as_mut(),
            ContentType::Handshake,
            update,
            outgoing,
            used,
        )?;

        let current = self
            .client_secret
            .as_ref()
            .ok_or(TlsError::UnexpectedMessage)?;
        let next = next_traffic_secret(suite, current.as_bytes())?;
        self.client_application = Some(RecordProtection::new(traffic_keys(
            suite,
            next.as_bytes(),
        )?)?);
        self.client_secret = Some(next);
        Ok(())
    }
}

/// The error a failed path check becomes.
const fn chain_error(error: audhsos_x509::X509Error) -> TlsError {
    match error {
        audhsos_x509::X509Error::NotYetValid | audhsos_x509::X509Error::Expired => {
            TlsError::CertificateExpired
        }
        audhsos_x509::X509Error::NameMismatch
        | audhsos_x509::X509Error::NoTrustAnchor
        | audhsos_x509::X509Error::NotAnAuthority
        | audhsos_x509::X509Error::PathLengthExceeded
        | audhsos_x509::X509Error::NotForCertificateSigning
        | audhsos_x509::X509Error::NotForServerAuthentication
        | audhsos_x509::X509Error::ChainTooLong => TlsError::UnknownAuthority,
        audhsos_x509::X509Error::SignatureFailed => TlsError::BadSignature,
        _ => TlsError::BadCertificate,
    }
}
