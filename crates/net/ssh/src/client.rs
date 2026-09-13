// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The client: one state machine over the layers below it, with no I/O.
//!
//! It is given bytes that arrived and a buffer to write bytes into, and it
//! answers with what it wants sent, what it has to give its caller, and
//! what it is waiting for. Everything it needs from outside — the
//! generator, the private key, the rule that admits a host key, the moment
//! — it is given at construction or at the call (D-46, D-49).
//!
//! What it does, in order: the identification string of RFC 4253, section
//! 4.2; the negotiation and the key exchange of sections 7 and 8; the
//! authentication of RFC 4252 with `publickey`; one `session` channel of
//! RFC 4254 with `exec` or `shell`; and the re-exchange of section 9
//! whenever either side asks for one.
//!
//! The socket under it and the program around it are the rest of step S8
//! and wait on Phase 14.

use crypto_ct::wipe;
use crypto_rng::Rng;

use crate::auth::{self, ClientKey, Request, Response};
use crate::channel::{Channel, Event as ChannelEvent, Message, STDERR};
use crate::cipher::KEY_BYTES;
use crate::error::SshError;
use crate::exchange::{self, Ephemeral, HASH_LEN, HashInput, MAX_PUBLIC, Method, Reply};
use crate::hostkey::{self, SIGNATURE_BLOB_LEN, Trust};
use crate::ident;
use crate::kex::{self, CLIENT as PROPOSAL, KexInit};
use crate::keys::{self, Key};
use crate::msg::{self, Disconnect};
use crate::packet::{Decoded, Decoder, Encoder, MAX_PACKET};
use crate::rekey::{Answer, Rekey};

/// The smallest buffer that can hold any packet this client must receive
/// (RFC 4253, section 6.1).
pub const MIN_INCOMING: usize = MAX_PACKET;

/// The smallest buffer that can hold any packet it sends.
pub const MIN_OUTGOING: usize = MAX_PACKET;

/// The longest user name this client authenticates as, which bounds the
/// buffer the signature of RFC 4252, section 7, is taken over.
pub const MAX_USER: usize = 64;

/// The most channel data one [`Connection::send`] takes.
pub const MAX_SEND: usize = 4096;

/// The longest `SSH_MSG_KEXINIT` this client keeps, its own and the
/// peer's, because both go into the exchange hash as they stood.
const MAX_KEXINIT: usize = 4096;

/// Room for the signed data of the authentication request.
const MAX_SIGNED: usize = auth::signed_len(MAX_USER, auth::SERVICE_CONNECTION.len());

/// Room for a packet payload that is not channel data.
const MAX_CONTROL: usize = MAX_SIGNED + SIGNATURE_BLOB_LEN + 64;

/// Room for one channel data payload.
const MAX_DATA: usize = MAX_SEND + 16;

/// The channel number this client uses, of which it has one (14.11).
const CHANNEL: u32 = 0;

/// The bytes of a `SSH_MSG_CHANNEL_DATA` before its data: the message
/// number, the channel number, and the length field.
const DATA_HEAD: usize = 1 + 4 + 4;

/// The same for `SSH_MSG_CHANNEL_EXTENDED_DATA`, which has the data type
/// code between them.
const EXTENDED_HEAD: usize = DATA_HEAD + 4;

/// The bytes of a binary packet before its payload: the length field and
/// the padding length (RFC 4253, section 6).
const PACKET_HEAD: usize = 5;

/// What the caller should do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// Nothing more can happen until bytes arrive.
    WantsRead,
    /// There are bytes to send.
    WantsWrite,
    /// The command or the shell is running, and data may flow.
    Started,
    /// Bytes arrived on the channel. [`Connection::recv`] takes them.
    Data {
        /// Whether they came as extended data, which for a session is
        /// standard error (RFC 4254, section 5.2).
        stderr: bool,
        /// How many there are.
        len: usize,
    },
    /// The command ended and said so (RFC 4254, section 6.10).
    ExitStatus(u32),
    /// The command died of a signal.
    ExitSignal,
    /// The channel is closed and nothing more will arrive.
    Closed,
}

/// What the caller decided about this connection.
pub struct Config<'a, T: Trust> {
    /// The user to authenticate as, at most [`MAX_USER`] bytes.
    pub user: &'a str,
    /// The key to authenticate with.
    pub key: &'a ClientKey,
    /// Which host keys this client will talk to (14.10).
    pub trust: &'a T,
    /// The command to run, or nothing for the user's login shell.
    pub command: Option<&'a [u8]>,
    /// The window this side grants the peer.
    pub window: u32,
    /// The largest data payload this side takes, which its transport must
    /// be able to receive.
    pub max_packet: u32,
}

/// Where the client stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum State {
    /// Waiting for the peer's identification string.
    Identification,
    /// Waiting for its `SSH_MSG_KEXINIT`.
    KexInit,
    /// Waiting for the reply of the method that was chosen.
    KexReply,
    /// Waiting for its `SSH_MSG_NEWKEYS`.
    NewKeys,
    /// Waiting for `SSH_MSG_SERVICE_ACCEPT`.
    ServiceAccept,
    /// Waiting for leave to sign, or for the answer to the signed
    /// request.
    UserAuth,
    /// Waiting for the channel to be confirmed, or for the answer to
    /// `exec` or `shell`.
    ChannelOpen,
    /// The command is running.
    Session,
    /// Nothing more will happen.
    Closed,
}

/// The bytes a connection works in, both owned by the caller.
pub struct Buffers<'a> {
    /// Where bytes from the transport are put.
    pub incoming: &'a mut [u8],
    /// Where bytes for the transport are taken from.
    pub outgoing: &'a mut [u8],
}

/// Channel data sitting in the incoming buffer.
#[derive(Clone, Copy, Debug)]
struct Pending {
    /// Where it starts in the incoming buffer.
    start: usize,
    /// How many bytes it is.
    len: usize,
}

/// Where a payload sits in the incoming buffer.
#[derive(Clone, Copy, Debug)]
struct Span {
    /// Its first byte.
    at: usize,
    /// How many bytes it is.
    len: usize,
}

/// What one channel message means, without the bytes it borrowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Report {
    /// The channel is open.
    Opened,
    /// The peer refused it, or refused what was asked on it.
    Refused,
    /// A request this side made succeeded.
    Succeeded,
    /// The server answered the query with leave to sign.
    LeaveToSign,
    /// Channel data, of this length and this stream.
    Data {
        /// How many bytes.
        len: usize,
        /// Whether it is standard error.
        stderr: bool,
        /// Where the data starts inside the payload.
        head: usize,
    },
    /// The command's exit status.
    ExitStatus(u32),
    /// The command died of a signal.
    ExitSignal,
    /// The peer closed the channel.
    Close,
    /// Anything else, which changes only the channel's own state.
    Nothing,
}

/// Everything about a connection but its buffers and its parameters.
struct Machine {
    /// Where the client stands.
    state: State,
    /// The packet layer for what this side sends.
    encoder: Encoder,
    /// The packet layer for what arrives.
    decoder: Decoder,
    /// What the connection has carried, and whether an exchange runs.
    rekey: Rekey,
    /// The peer's identification string, which the exchange hash takes.
    server_id: [u8; ident::MAX_LEN],
    /// How much of it there is.
    server_id_len: usize,
    /// This side's `SSH_MSG_KEXINIT` payload.
    client_kexinit: [u8; MAX_KEXINIT],
    /// How much of it there is.
    client_kexinit_len: usize,
    /// The peer's.
    server_kexinit: [u8; MAX_KEXINIT],
    /// How much of it there is.
    server_kexinit_len: usize,
    /// The method this exchange runs, once it is chosen.
    method: Option<Method>,
    /// This side's ephemeral pair for the exchange that is running.
    ephemeral: Option<Ephemeral>,
    /// The key for what the peer sends, until its `SSH_MSG_NEWKEYS`
    /// arrives and takes it into use.
    next_server_key: Option<[u8; KEY_BYTES]>,
    /// The session identifier, which is the first exchange hash and never
    /// changes afterwards (RFC 4253, section 7.2).
    session_id: Option<[u8; HASH_LEN]>,
    /// The channel.
    channel: Channel,
    /// Channel data that arrived and has not been taken.
    pending: Option<Pending>,
    /// Whether this side has sent `SSH_MSG_CHANNEL_EOF`.
    finished: bool,
}

/// One client connection.
pub struct Connection<'a, R: Rng, T: Trust> {
    /// What the caller decided.
    config: &'a Config<'a, T>,
    /// The generator, for the cookie, the ephemeral secret and every
    /// packet's padding.
    rng: &'a mut R,
    /// The protocol state.
    machine: Machine,
    /// Bytes from the transport.
    incoming: &'a mut [u8],
    /// How many of them there are.
    incoming_len: usize,
    /// How many of those have been read.
    consumed: usize,
    /// Bytes for the transport.
    outgoing: &'a mut [u8],
    /// How many of them there are.
    outgoing_len: usize,
    /// How many of those have been handed over.
    written: usize,
}

impl<'a, R: Rng, T: Trust> Connection<'a, R, T> {
    /// A connection that has written its identification string and its
    /// first `SSH_MSG_KEXINIT`.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when a buffer is below its minimum or the
    /// user name is longer than [`MAX_USER`], and [`SshError::Rng`] when
    /// the generator has nothing.
    pub fn new(
        config: &'a Config<'a, T>,
        rng: &'a mut R,
        buffers: Buffers<'a>,
        now: u64,
    ) -> Result<Connection<'a, R, T>, SshError> {
        let Buffers { incoming, outgoing } = buffers;
        if incoming.len() < MIN_INCOMING || outgoing.len() < MIN_OUTGOING {
            return Err(SshError::OutOfBounds {
                needed: MIN_INCOMING,
                available: incoming.len().min(outgoing.len()),
            });
        }
        if config.user.len() > MAX_USER {
            return Err(SshError::OutOfBounds {
                needed: config.user.len(),
                available: MAX_USER,
            });
        }

        let mut connection = Connection {
            config,
            rng,
            machine: Machine {
                state: State::Identification,
                encoder: Encoder::new(),
                decoder: Decoder::new(),
                rekey: Rekey::new(now),
                server_id: [0u8; ident::MAX_LEN],
                server_id_len: 0,
                client_kexinit: [0u8; MAX_KEXINIT],
                client_kexinit_len: 0,
                server_kexinit: [0u8; MAX_KEXINIT],
                server_kexinit_len: 0,
                method: None,
                ephemeral: None,
                next_server_key: None,
                session_id: None,
                channel: Channel::new(CHANNEL, config.window, config.max_packet),
                pending: None,
                finished: false,
            },
            incoming,
            incoming_len: 0,
            consumed: 0,
            outgoing,
            outgoing_len: 0,
            written: 0,
        };

        connection.outgoing_len = ident::write(connection.outgoing)?;
        connection.send_kexinit()?;
        Ok(connection)
    }

    /// Takes bytes that arrived and answers with how many it took.
    ///
    /// What it does not take stays with the caller, which happens when the
    /// buffer holds packets that have not been read yet.
    pub fn read_ssh(&mut self, input: &[u8]) -> usize {
        if self.machine.pending.is_none() {
            self.compact();
        }
        let room = self.incoming.len().saturating_sub(self.incoming_len);
        let take = room.min(input.len());
        let end = self.incoming_len.saturating_add(take);
        let slot = self
            .incoming
            .get_mut(self.incoming_len..end)
            .unwrap_or(&mut []);
        slot.copy_from_slice(input.get(..take).unwrap_or(&[]));
        self.incoming_len = end;
        take
    }

    /// Hands bytes to the transport and answers with how many it wrote.
    pub fn write_ssh(&mut self, out: &mut [u8]) -> usize {
        let available = self.outgoing_len.saturating_sub(self.written);
        let take = available.min(out.len());
        let end = self.written.saturating_add(take);
        let source = self.outgoing.get(self.written..end).unwrap_or(&[]);
        let slot = out.get_mut(..take).unwrap_or(&mut []);
        slot.copy_from_slice(source);
        self.written = end;
        if self.written >= self.outgoing_len {
            self.outgoing_len = 0;
            self.written = 0;
        }
        take
    }

    /// Takes the channel data of the last [`Event::Data`] and answers with
    /// how many bytes it copied.
    ///
    /// A caller whose buffer is shorter than the event named takes what
    /// fits and loses the rest, so it passes one of at least that length.
    pub fn recv(&mut self, out: &mut [u8]) -> usize {
        let Some(pending) = self.machine.pending.take() else {
            return 0;
        };
        let end = pending.start.saturating_add(pending.len);
        let data = self.incoming.get(pending.start..end).unwrap_or(&[]);
        let take = data.len().min(out.len());
        let slot = out.get_mut(..take).unwrap_or(&mut []);
        slot.copy_from_slice(data.get(..take).unwrap_or(&[]));
        take
    }

    /// Sends channel data and answers with how many bytes it took, which
    /// is bounded by the window, by the peer's maximum packet size, by
    /// [`MAX_SEND`], and by the room left in the outgoing buffer.
    ///
    /// # Errors
    ///
    /// [`SshError::Channel`] when the command is not running, and whatever
    /// the packet layer reports.
    pub fn send(&mut self, data: &[u8]) -> Result<usize, SshError> {
        if self.machine.state != State::Session {
            return Err(SshError::Channel);
        }
        let window = usize::try_from(self.machine.channel.remote_window()).unwrap_or(usize::MAX);
        let room = self.room().saturating_sub(MAX_CONTROL);
        let take = data.len().min(window).min(MAX_SEND).min(room);
        if take == 0 {
            return Ok(0);
        }
        let mut payload = [0u8; MAX_DATA];
        let len = self
            .machine
            .channel
            .write_data(data.get(..take).unwrap_or(&[]), &mut payload)?;
        self.emit(&payload, len)?;
        Ok(take)
    }

    /// Says this side will send no more data (RFC 4254, section 5.3).
    ///
    /// # Errors
    ///
    /// [`SshError::Channel`] when the channel cannot take it, and whatever
    /// the packet layer reports.
    pub fn finish(&mut self) -> Result<(), SshError> {
        if self.machine.finished {
            return Ok(());
        }
        let mut payload = [0u8; MAX_CONTROL];
        let len = self.machine.channel.write_eof(&mut payload)?;
        self.emit(&payload, len)?;
        self.machine.finished = true;
        Ok(())
    }

    /// Closes the channel and ends the connection.
    ///
    /// A channel that is already closed is not an error: this says what is
    /// left to say and no more.
    ///
    /// # Errors
    ///
    /// Whatever the packet layer reports.
    pub fn close(&mut self) -> Result<(), SshError> {
        if self.machine.state == State::Closed {
            return Ok(());
        }
        let mut payload = [0u8; MAX_CONTROL];
        if let Ok(len) = self.machine.channel.write_close(&mut payload) {
            self.emit(&payload, len)?;
        }
        let len = Disconnect {
            reason: msg::disconnect::BY_APPLICATION,
            description: b"done",
            language: b"",
        }
        .write(&mut payload)?;
        self.emit(&payload, len)?;
        self.machine.state = State::Closed;
        Ok(())
    }

    /// The channel, for the caller that wants its windows.
    #[must_use]
    pub const fn channel(&self) -> &Channel {
        &self.machine.channel
    }

    /// The session identifier, once the first key exchange has made one.
    #[must_use]
    pub const fn session_id(&self) -> Option<&[u8; HASH_LEN]> {
        self.machine.session_id.as_ref()
    }

    /// Drives the connection as far as the bytes it has allow, and says
    /// what the caller should do next.
    ///
    /// `now` is the moment this is asked about, which is what decides
    /// whether a re-exchange is due (RFC 4253, section 9).
    ///
    /// # Errors
    ///
    /// Whatever the layer that found it reports.
    pub fn poll(&mut self, now: u64) -> Result<Event, SshError> {
        loop {
            if self.machine.state == State::Closed {
                return Ok(Event::Closed);
            }
            if self.machine.pending.is_some() {
                return Ok(self.wants());
            }
            if self.machine.state == State::Session && self.machine.rekey.due(now) {
                self.machine.rekey.ask()?;
                self.send_kexinit()?;
            }
            if let Some(event) = self.step(now)? {
                return Ok(event);
            }
        }
    }

    /// One line or one packet, and what it means.
    fn step(&mut self, now: u64) -> Result<Option<Event>, SshError> {
        if self.machine.state == State::Identification {
            return self.read_identification();
        }
        let start = self.consumed;
        let rest = self
            .incoming
            .get_mut(start..self.incoming_len)
            .unwrap_or(&mut []);
        let (payload, packet) = match self.machine.decoder.decode(rest)? {
            Decoded::Incomplete { .. } => return Ok(Some(self.wants())),
            Decoded::Packet { payload, length } => (
                Span {
                    at: start.saturating_add(PACKET_HEAD),
                    len: payload.len(),
                },
                length,
            ),
        };
        self.consumed = self.consumed.saturating_add(packet);
        self.machine.rekey.note(packet);
        self.handle(payload, now)
    }

    /// The peer's identification string, and the lines before it.
    fn read_identification(&mut self) -> Result<Option<Event>, SshError> {
        let rest = self
            .incoming
            .get(self.consumed..self.incoming_len)
            .unwrap_or(&[]);
        let (line, length) = match ident::read(rest)? {
            ident::Greeting::Incomplete => return Ok(Some(self.wants())),
            ident::Greeting::Identification { line, length } => (line, length),
        };
        let bytes = line.as_bytes();
        let len = bytes.len();
        let slot = self
            .machine
            .server_id
            .get_mut(..len)
            .ok_or(SshError::Identification)?;
        slot.copy_from_slice(bytes);
        self.machine.server_id_len = len;
        self.consumed = self.consumed.saturating_add(length);
        self.machine.state = State::KexInit;
        Ok(None)
    }

    /// What one packet means where the client stands.
    fn handle(&mut self, payload: Span, now: u64) -> Result<Option<Event>, SshError> {
        let number = at(self.incoming, payload).first().copied().unwrap_or(0);
        match number {
            msg::DISCONNECT => {
                self.machine.state = State::Closed;
                return Ok(Some(Event::Closed));
            }
            msg::IGNORE | msg::DEBUG | msg::UNIMPLEMENTED | msg::EXT_INFO => return Ok(None),
            _ => {}
        }
        if number == msg::KEXINIT && self.machine.state == State::Session {
            return self.take_kexinit(payload).map(|()| None);
        }
        match self.machine.state {
            State::KexInit => self.take_kexinit(payload).map(|()| None),
            State::KexReply => self.take_kex_reply(payload).map(|()| None),
            State::NewKeys => self.take_new_keys(payload, now).map(|()| None),
            State::ServiceAccept => self.take_service_accept(payload).map(|()| None),
            State::UserAuth => self.take_auth(payload).map(|()| None),
            State::ChannelOpen => self.take_channel(payload),
            State::Session => self.take_session(payload),
            State::Identification | State::Closed => Ok(None),
        }
    }

    /// The peer's `SSH_MSG_KEXINIT`, which either answers this side's or
    /// starts a re-exchange.
    fn take_kexinit(&mut self, payload: Span) -> Result<(), SshError> {
        let bytes = at(self.incoming, payload);
        let len = bytes.len();
        let slot = self
            .machine
            .server_kexinit
            .get_mut(..len)
            .ok_or(SshError::OutOfBounds {
                needed: len,
                available: MAX_KEXINIT,
            })?;
        slot.copy_from_slice(bytes);
        self.machine.server_kexinit_len = len;

        let method = {
            let server = KexInit::read(self.server_kexinit())?;
            let choice = kex::negotiate(&PROPOSAL, &server)?;
            Method::from_name(choice.kex).ok_or(SshError::Negotiation("key exchange"))?
        };
        self.machine.method = Some(method);
        if self.machine.rekey.peer_asked()? == Answer::SendKexInit {
            self.send_kexinit()?;
        }
        self.send_kex_init_message(method)?;
        self.machine.state = State::KexReply;
        Ok(())
    }

    /// The reply of the method: the host key, the peer's public value and
    /// the signature over the exchange hash.
    fn take_kex_reply(&mut self, payload: Span) -> Result<(), SshError> {
        let method = self.machine.method.ok_or(SshError::Exchange)?;
        let mut host_key = [0u8; hostkey::BLOB_LEN];
        let mut public = [0u8; MAX_PUBLIC];
        let mut signature = [0u8; SIGNATURE_BLOB_LEN];
        let (host_key_len, public_len, signature_len) = {
            let reply = Reply::read(at(self.incoming, payload), method)?;
            (
                copy_into(&mut host_key, reply.host_key)?,
                copy_into(&mut public, reply.public)?,
                copy_into(&mut signature, reply.signature)?,
            )
        };
        let host_key = host_key.get(..host_key_len).unwrap_or(&[]);
        let public = public.get(..public_len).unwrap_or(&[]);
        let signature = signature.get(..signature_len).unwrap_or(&[]);

        let mut shared = [0u8; MAX_PUBLIC];
        let (hash, shared_len) = {
            let ephemeral = self.machine.ephemeral.as_ref().ok_or(SshError::Exchange)?;
            let shared_len = ephemeral.shared_secret(public, &mut shared)?;
            let hash = exchange::exchange_hash(
                method,
                &HashInput {
                    client_id: ident::CLIENT,
                    server_id: self.server_id()?,
                    client_kexinit: self.client_kexinit(),
                    server_kexinit: self.server_kexinit(),
                    host_key,
                    client_public: ephemeral.public(),
                    server_public: public,
                    shared: shared.get(..shared_len).unwrap_or(&[]),
                },
            );
            (hash, shared_len)
        };

        hostkey::accept(host_key, &hash, signature, self.config.trust)?;

        let session_id = *self.machine.session_id.get_or_insert(hash);
        self.emit(&[msg::NEWKEYS], 1)?;

        let secret = shared.get(..shared_len).unwrap_or(&[]);
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
        wipe(&mut shared);
        self.machine.encoder.set_cipher(&client_key);
        wipe(&mut client_key);
        self.machine.next_server_key = Some(server_key);
        self.machine.ephemeral = None;
        self.machine.state = State::NewKeys;
        Ok(())
    }

    /// The peer's `SSH_MSG_NEWKEYS`, after which what it sends is under
    /// the new keys.
    fn take_new_keys(&mut self, payload: Span, now: u64) -> Result<(), SshError> {
        let number = at(self.incoming, payload).first().copied().unwrap_or(0);
        if number != msg::NEWKEYS {
            return Err(SshError::Message(number));
        }
        let mut key = self
            .machine
            .next_server_key
            .take()
            .ok_or(SshError::Exchange)?;
        self.machine.decoder.set_cipher(&key);
        wipe(&mut key);
        self.machine.rekey.settled(now)?;
        if self.machine.channel.is_open() {
            self.machine.state = State::Session;
            return Ok(());
        }
        let mut out = [0u8; MAX_CONTROL];
        let len = auth::write_service_request(auth::SERVICE_USERAUTH, &mut out)?;
        self.emit(&out, len)?;
        self.machine.state = State::ServiceAccept;
        Ok(())
    }

    /// `SSH_MSG_SERVICE_ACCEPT`, after which the authentication runs.
    fn take_service_accept(&mut self, payload: Span) -> Result<(), SshError> {
        let accepted = auth::read_service_accept(at(self.incoming, payload))?;
        if accepted != auth::SERVICE_USERAUTH.as_bytes() {
            return Err(SshError::Negotiation("service"));
        }
        let mut out = [0u8; MAX_CONTROL];
        let len = auth::write_query(&self.request(), self.config.key, &mut out)?;
        self.emit(&out, len)?;
        self.machine.state = State::UserAuth;
        Ok(())
    }

    /// What the server answers an authentication request with.
    fn take_auth(&mut self, payload: Span) -> Result<(), SshError> {
        let mut blob = [0u8; hostkey::BLOB_LEN];
        let blob_len = self.config.key.write_blob(&mut blob)?;
        let answer = {
            let response = Response::read(at(self.incoming, payload))?;
            match response {
                Response::Banner { .. } => Report::Nothing,
                Response::Success => Report::Succeeded,
                Response::Failure { .. } => Report::Refused,
                Response::PkOk { .. } => {
                    if response.is_leave_to_sign(blob.get(..blob_len).unwrap_or(&[])) {
                        Report::LeaveToSign
                    } else {
                        Report::Refused
                    }
                }
            }
        };
        match answer {
            Report::Refused => Err(SshError::Authentication),
            Report::Succeeded => {
                let mut out = [0u8; MAX_CONTROL];
                let len = self.machine.channel.write_open(&mut out)?;
                self.emit(&out, len)?;
                self.machine.state = State::ChannelOpen;
                Ok(())
            }
            Report::LeaveToSign => {
                let session_id = self.machine.session_id.ok_or(SshError::Exchange)?;
                let mut scratch = [0u8; MAX_SIGNED];
                let mut out = [0u8; MAX_CONTROL];
                let len = auth::write_publickey(
                    &self.request(),
                    self.config.key,
                    &session_id,
                    &mut scratch,
                    &mut out,
                )?;
                self.emit(&out, len)?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The channel's open and the answer to the request that starts a
    /// program.
    fn take_channel(&mut self, payload: Span) -> Result<Option<Event>, SshError> {
        let report = self.apply_channel(payload)?;
        match report {
            Report::Opened => {
                let mut out = [0u8; MAX_CONTROL];
                let len = match self.config.command {
                    Some(command) => self.machine.channel.write_exec(command, true, &mut out)?,
                    None => self.machine.channel.write_shell(true, &mut out)?,
                };
                self.emit(&out, len)?;
                Ok(None)
            }
            Report::Succeeded => {
                self.machine.state = State::Session;
                Ok(Some(Event::Started))
            }
            Report::Refused => {
                self.machine.state = State::Closed;
                Ok(Some(Event::Closed))
            }
            _ => Ok(None),
        }
    }

    /// What arrives while the command runs.
    fn take_session(&mut self, payload: Span) -> Result<Option<Event>, SshError> {
        match self.apply_channel(payload)? {
            Report::Data { len, stderr, head } => {
                self.machine.pending = Some(Pending {
                    start: payload.at.saturating_add(head),
                    len,
                });
                self.grant()?;
                Ok(Some(Event::Data { stderr, len }))
            }
            Report::ExitStatus(status) => Ok(Some(Event::ExitStatus(status))),
            Report::ExitSignal => Ok(Some(Event::ExitSignal)),
            Report::Close => {
                let mut out = [0u8; MAX_CONTROL];
                if let Ok(len) = self.machine.channel.write_close(&mut out) {
                    self.emit(&out, len)?;
                }
                self.machine.state = State::Closed;
                Ok(Some(Event::Closed))
            }
            _ => Ok(None),
        }
    }

    /// Reads one channel message, gives it to the channel, and answers
    /// with what it means without the bytes it borrowed.
    fn apply_channel(&mut self, payload: Span) -> Result<Report, SshError> {
        let message = Message::read(at(self.incoming, payload))?;
        let report = match message.event {
            ChannelEvent::Opened { .. } => Report::Opened,
            ChannelEvent::OpenFailed { .. } | ChannelEvent::Failure => Report::Refused,
            ChannelEvent::Success => Report::Succeeded,
            ChannelEvent::Data(data) => Report::Data {
                len: data.len(),
                stderr: false,
                head: DATA_HEAD,
            },
            ChannelEvent::ExtendedData { kind, data } => Report::Data {
                len: data.len(),
                stderr: kind == STDERR,
                head: EXTENDED_HEAD,
            },
            ChannelEvent::ExitStatus(status) => Report::ExitStatus(status),
            ChannelEvent::ExitSignal { .. } => Report::ExitSignal,
            ChannelEvent::Close => Report::Close,
            ChannelEvent::Eof | ChannelEvent::WindowAdjust(_) | ChannelEvent::Request { .. } => {
                Report::Nothing
            }
        };
        self.machine.channel.apply(&message)?;
        Ok(report)
    }

    /// Grants the peer what it has spent, once half the window is gone.
    fn grant(&mut self) -> Result<(), SshError> {
        let granted = self.config.window;
        let left = self.machine.channel.local_window();
        if left > granted / 2 {
            return Ok(());
        }
        let bytes = granted.saturating_sub(left);
        let mut out = [0u8; MAX_CONTROL];
        let len = self.machine.channel.write_window_adjust(bytes, &mut out)?;
        self.emit(&out, len)
    }

    /// Writes this side's `SSH_MSG_KEXINIT` and keeps it for the hash.
    fn send_kexinit(&mut self) -> Result<(), SshError> {
        let mut payload = [0u8; MAX_KEXINIT];
        let len = PROPOSAL.write(self.rng, &mut payload)?;
        let slot = self
            .machine
            .client_kexinit
            .get_mut(..len)
            .ok_or(SshError::OutOfBounds {
                needed: len,
                available: MAX_KEXINIT,
            })?;
        slot.copy_from_slice(payload.get(..len).unwrap_or(&[]));
        self.machine.client_kexinit_len = len;
        self.emit(&payload, len)
    }

    /// Writes the first message of the method that was chosen.
    fn send_kex_init_message(&mut self, method: Method) -> Result<(), SshError> {
        let ephemeral = Ephemeral::new(method, self.rng)?;
        let mut payload = [0u8; MAX_PUBLIC + 8];
        let len = ephemeral.write_init(&mut payload)?;
        self.machine.ephemeral = Some(ephemeral);
        self.emit(&payload, len)
    }

    /// Frames the first `len` bytes of `payload` into the outgoing buffer.
    fn emit(&mut self, payload: &[u8], len: usize) -> Result<(), SshError> {
        let body = payload.get(..len).unwrap_or(&[]);
        let room = self
            .outgoing
            .get_mut(self.outgoing_len..)
            .unwrap_or(&mut []);
        let written = self.machine.encoder.encode(body, &mut *self.rng, room)?;
        self.outgoing_len = self.outgoing_len.saturating_add(written);
        self.machine.rekey.note(written);
        Ok(())
    }

    /// The request every authentication message names.
    const fn request(&self) -> Request<'a> {
        Request {
            user: self.config.user,
            service: auth::SERVICE_CONNECTION,
        }
    }

    /// Whether the caller should send or read.
    const fn wants(&self) -> Event {
        if self.outgoing_len > self.written {
            Event::WantsWrite
        } else {
            Event::WantsRead
        }
    }

    /// Room left in the outgoing buffer.
    const fn room(&self) -> usize {
        self.outgoing.len().saturating_sub(self.outgoing_len)
    }

    /// Moves what has not been read to the front of the incoming buffer.
    fn compact(&mut self) {
        if self.consumed == 0 {
            return;
        }
        self.incoming
            .copy_within(self.consumed..self.incoming_len, 0);
        self.incoming_len = self.incoming_len.saturating_sub(self.consumed);
        self.consumed = 0;
    }

    /// The peer's identification string.
    fn server_id(&self) -> Result<&str, SshError> {
        let bytes = self
            .machine
            .server_id
            .get(..self.machine.server_id_len)
            .unwrap_or(&[]);
        core::str::from_utf8(bytes).map_err(|_| SshError::Identification)
    }

    /// This side's `SSH_MSG_KEXINIT` payload.
    fn client_kexinit(&self) -> &[u8] {
        self.machine
            .client_kexinit
            .get(..self.machine.client_kexinit_len)
            .unwrap_or(&[])
    }

    /// The peer's.
    fn server_kexinit(&self) -> &[u8] {
        self.machine
            .server_kexinit
            .get(..self.machine.server_kexinit_len)
            .unwrap_or(&[])
    }
}

/// A payload in a buffer.
fn at(bytes: &[u8], span: Span) -> &[u8] {
    let end = span.at.saturating_add(span.len);
    bytes.get(span.at..end).unwrap_or(&[])
}

/// Copies `source` into `out` and answers with its length.
fn copy_into(out: &mut [u8], source: &[u8]) -> Result<usize, SshError> {
    let available = out.len();
    let slot = out.get_mut(..source.len()).ok_or(SshError::OutOfBounds {
        needed: source.len(),
        available,
    })?;
    slot.copy_from_slice(source);
    Ok(source.len())
}
