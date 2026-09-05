// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The I/O the TLS client does not do.
//!
//! `audhsos_tls::client::Connection` moves no bytes: `read_tls` takes what
//! arrived, `poll` makes what progress it can, `write_tls` hands back what
//! to send. This module is the other half — a socket, and the loop that
//! turns those three calls into a conversation. Nothing here knows what a
//! record is.

use std::io::{Read, Write};
use std::net::TcpStream;

use audhsos_tls::client::{Connection, Event};
use audhsos_tls::record::{MAX_PLAINTEXT, MAX_RECORD};

use crate::error::ProbeError;

/// A socket, and the bytes read from it that the client has not taken.
pub struct Wire {
    /// The socket.
    stream: TcpStream,
    /// Bytes that arrived but did not fit in the client's incoming buffer.
    pending: Vec<u8>,
    /// Where a read from the socket, and a write to it, are staged.
    scratch: Vec<u8>,
}

impl Wire {
    /// A wire over `stream`.
    #[must_use]
    pub fn new(stream: TcpStream) -> Wire {
        Wire {
            stream,
            pending: Vec::new(),
            scratch: vec![0u8; MAX_RECORD],
        }
    }

    /// Runs the handshake to completion.
    ///
    /// # Errors
    ///
    /// Whatever the protocol refused, or whatever the socket reported. A
    /// peer that closes before the handshake is done is
    /// [`std::io::ErrorKind::UnexpectedEof`].
    pub fn handshake(&mut self, connection: &mut Connection<'_>) -> Result<(), ProbeError> {
        loop {
            self.flush(connection)?;
            self.feed(connection)?;
            match connection.poll()? {
                Event::Handshaked => return Ok(()),
                // Something was produced; the next turn sends it.
                Event::WantsWrite => {}
                Event::WantsRead => {
                    // While bytes are still queued the client has room to
                    // make progress without the socket; a record never
                    // exceeds the incoming buffer, so this terminates.
                    if self.pending.is_empty() {
                        self.fill()?;
                    }
                }
                Event::PeerClosed => {
                    return Err(ProbeError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "the peer closed during the handshake",
                    )));
                }
            }
        }
    }

    /// Sends `request` and reads until the peer closes.
    ///
    /// # Errors
    ///
    /// Whatever the protocol refused, or whatever the socket reported.
    pub fn exchange(
        &mut self,
        connection: &mut Connection<'_>,
        request: &[u8],
    ) -> Result<Vec<u8>, ProbeError> {
        let mut rest = request;
        while !rest.is_empty() {
            let sent = connection.send(rest)?;
            rest = rest.get(sent..).unwrap_or(&[]);
        }
        self.flush(connection)?;

        let mut body = Vec::new();
        let mut plaintext = vec![0u8; MAX_PLAINTEXT];
        loop {
            self.feed(connection)?;
            let taken = connection.recv(&mut plaintext)?;
            if taken != 0 {
                body.extend_from_slice(plaintext.get(..taken).unwrap_or(&[]));
                continue;
            }
            // A `KeyUpdate` in the stream is answered from `recv`, so a
            // turn that delivered nothing may still owe the peer bytes.
            self.flush(connection)?;
            if !connection.is_handshaked() {
                // `close_notify` arrived, or the connection ended.
                return Ok(body);
            }
            if !self.pending.is_empty() {
                continue;
            }
            if self.fill()? == 0 {
                // A close of the transport without `close_notify`. RFC
                // 8446 forbids it; the bytes that did arrive are still
                // reported, and the caller decides what they are worth.
                return Ok(body);
            }
        }
    }

    /// Sends `close_notify`, best effort.
    ///
    /// # Errors
    ///
    /// Whatever the protocol refused.
    pub fn close(&mut self, connection: &mut Connection<'_>) -> Result<(), ProbeError> {
        connection.close()?;
        self.flush(connection)
    }

    /// Hands everything the client has produced to the socket.
    fn flush(&mut self, connection: &mut Connection<'_>) -> Result<(), ProbeError> {
        loop {
            let taken = connection.write_tls(&mut self.scratch)?;
            if taken == 0 {
                return Ok(());
            }
            self.stream
                .write_all(self.scratch.get(..taken).unwrap_or(&[]))?;
        }
    }

    /// Hands the client as much of what arrived as it will take.
    fn feed(&mut self, connection: &mut Connection<'_>) -> Result<(), ProbeError> {
        while !self.pending.is_empty() {
            let taken = connection.read_tls(&self.pending)?;
            if taken == 0 {
                return Ok(());
            }
            self.pending.drain(..taken);
        }
        Ok(())
    }

    /// Blocks on the socket once, and says how many bytes arrived.
    fn fill(&mut self) -> Result<usize, ProbeError> {
        let read = self.stream.read(&mut self.scratch)?;
        self.pending
            .extend_from_slice(self.scratch.get(..read).unwrap_or(&[]));
        Ok(read)
    }
}
