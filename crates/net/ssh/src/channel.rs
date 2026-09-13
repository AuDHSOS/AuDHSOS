// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The connection layer of RFC 4254: one `session` channel, its window,
//! and the requests that start a program on the other side.
//!
//! One channel per connection, which is what a client that runs a command
//! needs and what 14.11 fixes the buffers for. A [`Channel`] holds the two
//! windows and what each side has said about the end of the channel; it
//! moves no bytes and keeps no buffer, so what it costs is what its
//! fields say.
//!
//! Extended data — stderr — spends the same window as ordinary data
//! (section 5.2), which is why there is one window here and not two.

use crate::error::SshError;
use crate::msg;
use crate::wire::{Reader, Writer};

/// The one channel type of section 14.5 (RFC 4254, section 6.1).
pub const SESSION: &str = "session";

/// `SSH_EXTENDED_DATA_STDERR`, the one extended data type RFC 4254,
/// section 5.2, defines.
pub const STDERR: u32 = 1;

/// The request that starts a command (section 6.5).
pub const EXEC: &str = "exec";

/// The request that starts the user's login shell.
pub const SHELL: &str = "shell";

/// The request that passes one environment variable (section 6.4), which
/// a server is free to ignore.
pub const ENV: &str = "env";

/// The request that carries the exit status of the command (section
/// 6.10).
pub const EXIT_STATUS: &str = "exit-status";

/// The request that says the command died of a signal.
pub const EXIT_SIGNAL: &str = "exit-signal";

/// One `session` channel and what it may still send and receive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Channel {
    /// This side's channel number, which every message from the peer
    /// names.
    local: u32,
    /// The peer's number, once it has confirmed the open.
    remote: Option<u32>,
    /// What the peer may still send to this side.
    local_window: u32,
    /// What this side may still send.
    remote_window: u32,
    /// The largest data payload the peer will take (section 5.2).
    remote_max_packet: u32,
    /// The largest this side takes, which its transport must be able to
    /// receive.
    local_max_packet: u32,
    /// What this side has said about the end of the channel.
    sent: Ending,
    /// What the peer has said.
    received: Ending,
}

/// What one side has said about the end of a channel (RFC 4254, section
/// 5.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
struct Ending {
    /// It will send no more data, and the channel stays open.
    eof: bool,
    /// It is done with the channel.
    close: bool,
}

impl Channel {
    /// A channel this side has a number and a window for, before the open
    /// is sent.
    ///
    /// `max_packet` is what this side advertises, and section 5.2 forbids
    /// advertising more than the transport will receive.
    #[must_use]
    pub const fn new(local: u32, window: u32, max_packet: u32) -> Channel {
        Channel {
            local,
            remote: None,
            local_window: window,
            remote_window: 0,
            remote_max_packet: 0,
            local_max_packet: max_packet,
            sent: Ending {
                eof: false,
                close: false,
            },
            received: Ending {
                eof: false,
                close: false,
            },
        }
    }

    /// This side's channel number.
    #[must_use]
    pub const fn local(&self) -> u32 {
        self.local
    }

    /// The peer's number, once the open was confirmed.
    #[must_use]
    pub const fn remote(&self) -> Option<u32> {
        self.remote
    }

    /// Whether the peer confirmed the open.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.remote.is_some()
    }

    /// Whether the channel is closed for this side, which section 5.3
    /// makes both a close sent and a close received.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.sent.close && self.received.close
    }

    /// Whether the peer said it will send no more data.
    #[must_use]
    pub const fn eof_received(&self) -> bool {
        self.received.eof
    }

    /// What this side may still send.
    #[must_use]
    pub const fn remote_window(&self) -> u32 {
        self.remote_window
    }

    /// What the peer may still send.
    #[must_use]
    pub const fn local_window(&self) -> u32 {
        self.local_window
    }

    /// Writes `SSH_MSG_CHANNEL_OPEN` for a session (section 6.1).
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when `out` is too small.
    pub fn write_open(&self, out: &mut [u8]) -> Result<usize, SshError> {
        let mut writer = Writer::new(out);
        writer.write_byte(msg::CHANNEL_OPEN)?;
        writer.write_string(SESSION.as_bytes())?;
        writer.write_u32(self.local)?;
        writer.write_u32(self.local_window)?;
        writer.write_u32(self.local_max_packet)?;
        Ok(writer.position())
    }

    /// Writes `SSH_MSG_CHANNEL_DATA` and spends the window it costs.
    ///
    /// # Errors
    ///
    /// [`SshError::Channel`] before the open is confirmed and after a
    /// close or an end of file was sent; [`SshError::Window`] for more
    /// data than the peer granted or than its maximum packet size takes;
    /// [`SshError::OutOfBounds`] when `out` is too small, in which case
    /// no window is spent.
    pub fn write_data(&mut self, data: &[u8], out: &mut [u8]) -> Result<usize, SshError> {
        self.write_data_message(None, data, out)
    }

    /// Writes `SSH_MSG_CHANNEL_EXTENDED_DATA` of `kind`, which spends the
    /// same window as ordinary data.
    ///
    /// # Errors
    ///
    /// Those of [`Channel::write_data`].
    pub fn write_extended_data(
        &mut self,
        kind: u32,
        data: &[u8],
        out: &mut [u8],
    ) -> Result<usize, SshError> {
        self.write_data_message(Some(kind), data, out)
    }

    /// Writes `SSH_MSG_CHANNEL_EOF` (section 5.3), which spends no
    /// window.
    ///
    /// # Errors
    ///
    /// [`SshError::Channel`] before the open is confirmed and after a
    /// close was sent, and [`SshError::OutOfBounds`] when `out` is too
    /// small.
    pub fn write_eof(&mut self, out: &mut [u8]) -> Result<usize, SshError> {
        let remote = self.sendable()?;
        let len = write_channel_message(msg::CHANNEL_EOF, remote, out)?;
        self.sent.eof = true;
        Ok(len)
    }

    /// Writes `SSH_MSG_CHANNEL_CLOSE`, which a party must answer with one
    /// of its own unless it has sent one already.
    ///
    /// # Errors
    ///
    /// [`SshError::Channel`] before the open is confirmed and when a
    /// close was already sent, and [`SshError::OutOfBounds`] when `out`
    /// is too small.
    pub fn write_close(&mut self, out: &mut [u8]) -> Result<usize, SshError> {
        let remote = self.remote.ok_or(SshError::Channel)?;
        if self.sent.close {
            return Err(SshError::Channel);
        }
        let len = write_channel_message(msg::CHANNEL_CLOSE, remote, out)?;
        self.sent.close = true;
        Ok(len)
    }

    /// Writes `SSH_MSG_CHANNEL_WINDOW_ADJUST` and grants the peer `bytes`
    /// more.
    ///
    /// # Errors
    ///
    /// [`SshError::Channel`] before the open is confirmed,
    /// [`SshError::Window`] when the window would pass 2^32 - 1, and
    /// [`SshError::OutOfBounds`] when `out` is too small, in which case
    /// nothing is granted.
    pub fn write_window_adjust(&mut self, bytes: u32, out: &mut [u8]) -> Result<usize, SshError> {
        let remote = self.remote.ok_or(SshError::Channel)?;
        let granted = self
            .local_window
            .checked_add(bytes)
            .ok_or(SshError::Window)?;
        let mut writer = Writer::new(out);
        writer.write_byte(msg::CHANNEL_WINDOW_ADJUST)?;
        writer.write_u32(remote)?;
        writer.write_u32(bytes)?;
        self.local_window = granted;
        Ok(writer.position())
    }

    /// Writes the `exec` request of section 6.5, which starts one command.
    ///
    /// # Errors
    ///
    /// Those of [`Channel::write_shell`].
    pub fn write_exec(
        &self,
        command: &[u8],
        want_reply: bool,
        out: &mut [u8],
    ) -> Result<usize, SshError> {
        let mut writer = self.request(EXEC, want_reply, out)?;
        writer.write_string(command)?;
        Ok(writer.position())
    }

    /// Writes the `shell` request, which starts the user's login shell.
    ///
    /// # Errors
    ///
    /// [`SshError::Channel`] before the open is confirmed and after a
    /// close was sent, and [`SshError::OutOfBounds`] when `out` is too
    /// small.
    pub fn write_shell(&self, want_reply: bool, out: &mut [u8]) -> Result<usize, SshError> {
        let writer = self.request(SHELL, want_reply, out)?;
        Ok(writer.position())
    }

    /// Writes the `env` request of section 6.4, which a server is free to
    /// ignore.
    ///
    /// # Errors
    ///
    /// Those of [`Channel::write_shell`].
    pub fn write_env(
        &self,
        name: &[u8],
        value: &[u8],
        want_reply: bool,
        out: &mut [u8],
    ) -> Result<usize, SshError> {
        let mut writer = self.request(ENV, want_reply, out)?;
        writer.write_string(name)?;
        writer.write_string(value)?;
        Ok(writer.position())
    }

    /// Takes what arrived for this channel: the open confirmation, the
    /// window the peer grants, the data it sends, and the end of the
    /// channel.
    ///
    /// The caller keeps the event; what the channel keeps is what the
    /// event changed about it.
    ///
    /// # Errors
    ///
    /// [`SshError::Channel`] for a message that names another channel, a
    /// second open confirmation, or data on a channel that is not open;
    /// [`SshError::Window`] for data past what this side granted or for a
    /// credit that would take the window past 2^32 - 1.
    pub fn apply(&mut self, message: &Message<'_>) -> Result<(), SshError> {
        if message.channel != self.local {
            return Err(SshError::Channel);
        }
        match message.event {
            Event::Opened {
                remote,
                window,
                max_packet,
            } => {
                if self.remote.is_some() {
                    return Err(SshError::Channel);
                }
                self.remote = Some(remote);
                self.remote_window = window;
                self.remote_max_packet = max_packet;
            }
            Event::OpenFailed { .. } => {
                if self.remote.is_some() {
                    return Err(SshError::Channel);
                }
                self.sent.close = true;
                self.received.close = true;
            }
            Event::WindowAdjust(bytes) => {
                self.expect_open()?;
                self.remote_window = self
                    .remote_window
                    .checked_add(bytes)
                    .ok_or(SshError::Window)?;
            }
            Event::Data(data) | Event::ExtendedData { data, .. } => {
                self.spend_local(data.len())?;
            }
            Event::Eof => {
                self.expect_open()?;
                self.received.eof = true;
            }
            Event::Close => {
                self.expect_open()?;
                self.received.close = true;
            }
            Event::Success
            | Event::Failure
            | Event::ExitStatus(_)
            | Event::ExitSignal { .. }
            | Event::Request { .. } => self.expect_open()?,
        }
        Ok(())
    }

    /// The peer's number, for a message that may still be sent.
    fn sendable(&self) -> Result<u32, SshError> {
        let remote = self.remote.ok_or(SshError::Channel)?;
        if self.sent.close {
            return Err(SshError::Channel);
        }
        Ok(remote)
    }

    /// The channel is open, for an event that presumes it.
    const fn expect_open(&self) -> Result<(), SshError> {
        if self.remote.is_none() || self.is_closed() {
            return Err(SshError::Channel);
        }
        Ok(())
    }

    /// Takes `len` bytes off what the peer may send.
    fn spend_local(&mut self, len: usize) -> Result<(), SshError> {
        self.expect_open()?;
        let spent = u32::try_from(len).map_err(|_| SshError::Window)?;
        self.local_window = self
            .local_window
            .checked_sub(spent)
            .ok_or(SshError::Window)?;
        Ok(())
    }

    /// The head of a channel request (section 5.4), which spends no
    /// window.
    fn request<'a>(
        &self,
        kind: &str,
        want_reply: bool,
        out: &'a mut [u8],
    ) -> Result<Writer<'a>, SshError> {
        let remote = self.sendable()?;
        let mut writer = Writer::new(out);
        writer.write_byte(msg::CHANNEL_REQUEST)?;
        writer.write_u32(remote)?;
        writer.write_string(kind.as_bytes())?;
        writer.write_boolean(want_reply)?;
        Ok(writer)
    }

    /// Both data messages, which differ in one field and in nothing else.
    fn write_data_message(
        &mut self,
        kind: Option<u32>,
        data: &[u8],
        out: &mut [u8],
    ) -> Result<usize, SshError> {
        let remote = self.sendable()?;
        if self.sent.eof {
            return Err(SshError::Channel);
        }
        let len = u32::try_from(data.len()).map_err(|_| SshError::Window)?;
        if len > self.remote_window || len > self.remote_max_packet {
            return Err(SshError::Window);
        }
        let mut writer = Writer::new(out);
        if let Some(code) = kind {
            writer.write_byte(msg::CHANNEL_EXTENDED_DATA)?;
            writer.write_u32(remote)?;
            writer.write_u32(code)?;
        } else {
            writer.write_byte(msg::CHANNEL_DATA)?;
            writer.write_u32(remote)?;
        }
        writer.write_string(data)?;
        self.remote_window = self.remote_window.saturating_sub(len);
        Ok(writer.position())
    }
}

/// One `uint32` message: the end of file and the close.
fn write_channel_message(number: u8, remote: u32, out: &mut [u8]) -> Result<usize, SshError> {
    let mut writer = Writer::new(out);
    writer.write_byte(number)?;
    writer.write_u32(remote)?;
    Ok(writer.position())
}

/// What arrived for one channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Message<'a> {
    /// The channel it names, which for everything but an open is this
    /// side's number.
    pub channel: u32,
    /// What it says.
    pub event: Event<'a>,
}

/// What a peer sends about a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event<'a> {
    /// `SSH_MSG_CHANNEL_OPEN_CONFIRMATION`: the peer's number, and what
    /// this side may send.
    Opened {
        /// The peer's channel number, which every message to it carries.
        remote: u32,
        /// The window it grants.
        window: u32,
        /// The largest data payload it will take.
        max_packet: u32,
    },
    /// `SSH_MSG_CHANNEL_OPEN_FAILURE`, whose reason codes are in
    /// [`crate::msg::open`].
    OpenFailed {
        /// Why the peer refused.
        reason: u32,
        /// Its text, which a client may show and must treat as bytes.
        description: &'a [u8],
        /// The language tag.
        language: &'a [u8],
    },
    /// `SSH_MSG_CHANNEL_WINDOW_ADJUST`: bytes this side may send beyond
    /// what it could.
    WindowAdjust(u32),
    /// `SSH_MSG_CHANNEL_DATA`.
    Data(&'a [u8]),
    /// `SSH_MSG_CHANNEL_EXTENDED_DATA`, of which [`STDERR`] is the one
    /// type a session has.
    ExtendedData {
        /// The data type code.
        kind: u32,
        /// The bytes, which spend the same window as ordinary data.
        data: &'a [u8],
    },
    /// `SSH_MSG_CHANNEL_EOF`: the peer will send no more data, and the
    /// channel stays open.
    Eof,
    /// `SSH_MSG_CHANNEL_CLOSE`.
    Close,
    /// `SSH_MSG_CHANNEL_SUCCESS`, the answer to a request that asked for
    /// one.
    Success,
    /// `SSH_MSG_CHANNEL_FAILURE`.
    Failure,
    /// The `exit-status` request of section 6.10.
    ExitStatus(u32),
    /// The `exit-signal` request, which says the command died.
    ExitSignal {
        /// The signal name without its `SIG` prefix.
        signal: &'a [u8],
        /// Whether the command dumped core.
        core_dumped: bool,
        /// The text of the error.
        message: &'a [u8],
        /// The language tag.
        language: &'a [u8],
    },
    /// A channel request this client does not act on, kept so that one
    /// that asked for a reply can be answered.
    Request {
        /// The request type.
        kind: &'a [u8],
        /// Whether the peer wants an answer.
        want_reply: bool,
    },
}

impl<'a> Message<'a> {
    /// Reads one channel message from a packet payload.
    ///
    /// # Errors
    ///
    /// [`SshError::Message`] for a number the connection layer does not
    /// send and [`SshError::OutOfBounds`] when the payload ends early.
    pub fn read(payload: &'a [u8]) -> Result<Message<'a>, SshError> {
        let mut reader = Reader::new(payload);
        let number = reader.read_byte()?;
        let channel = reader.read_u32()?;
        let event = match number {
            msg::CHANNEL_OPEN_CONFIRMATION => Event::Opened {
                remote: reader.read_u32()?,
                window: reader.read_u32()?,
                max_packet: reader.read_u32()?,
            },
            msg::CHANNEL_OPEN_FAILURE => Event::OpenFailed {
                reason: reader.read_u32()?,
                description: reader.read_string()?,
                language: reader.read_string()?,
            },
            msg::CHANNEL_WINDOW_ADJUST => Event::WindowAdjust(reader.read_u32()?),
            msg::CHANNEL_DATA => Event::Data(reader.read_string()?),
            msg::CHANNEL_EXTENDED_DATA => Event::ExtendedData {
                kind: reader.read_u32()?,
                data: reader.read_string()?,
            },
            msg::CHANNEL_EOF => Event::Eof,
            msg::CHANNEL_CLOSE => Event::Close,
            msg::CHANNEL_SUCCESS => Event::Success,
            msg::CHANNEL_FAILURE => Event::Failure,
            msg::CHANNEL_REQUEST => read_request(&mut reader)?,
            other => return Err(SshError::Message(other)),
        };
        Ok(Message { channel, event })
    }
}

/// The three channel requests a server sends a client: the exit status,
/// the exit signal, and everything else.
fn read_request<'a>(reader: &mut Reader<'a>) -> Result<Event<'a>, SshError> {
    let kind = reader.read_string()?;
    let want_reply = reader.read_boolean()?;
    if kind == EXIT_STATUS.as_bytes() {
        return Ok(Event::ExitStatus(reader.read_u32()?));
    }
    if kind == EXIT_SIGNAL.as_bytes() {
        return Ok(Event::ExitSignal {
            signal: reader.read_string()?,
            core_dumped: reader.read_boolean()?,
            message: reader.read_string()?,
            language: reader.read_string()?,
        });
    }
    Ok(Event::Request { kind, want_reply })
}
