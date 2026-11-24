// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One request and its response over one TLS connection.

use audhsos_tls::client::{self, Connection, Event};
use audhsos_tls::config::ClientConfig;
use audhsos_tls::record::MAX_PLAINTEXT;
use crypto_rng::Rng;
use net_http::{Decoder, Event as HttpEvent, Head, Request};
use net_wire::Writer;

use crate::error::HttpsError;

/// How many header fields of a response the decoder holds.
pub const MAX_FIELDS: usize = 64;

/// The byte stream an exchange runs over, usually a TCP connection.
pub trait Transport {
    /// Why the transport failed.
    type Error;

    /// Sends all of `bytes`.
    ///
    /// # Errors
    ///
    /// The transport's own.
    fn send(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;

    /// Receives bytes into `buffer` and returns how many. Zero is the end
    /// of the stream.
    ///
    /// # Errors
    ///
    /// The transport's own.
    fn recv(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error>;
}

/// A handler's refusal of a response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused;

/// What receives the response.
pub trait Handler {
    /// Takes the head of the response.
    ///
    /// # Errors
    ///
    /// [`Refused`] ends the exchange.
    fn head(&mut self, head: &Head<'_>) -> Result<(), Refused>;

    /// Takes a piece of the body, in order.
    ///
    /// # Errors
    ///
    /// [`Refused`] ends the exchange.
    fn body(&mut self, piece: &[u8]) -> Result<(), Refused>;
}

/// The buffers of one exchange, all owned by the caller.
pub struct Buffers<'a> {
    /// The buffers of the TLS connection.
    pub tls: client::Buffers<'a>,
    /// Where the request is written; at least the size of the request.
    pub request: &'a mut [u8],
    /// Where a record opens; at least [`MAX_PLAINTEXT`] bytes.
    pub plaintext: &'a mut [u8],
    /// Where the response head is assembled.
    pub head: &'a mut [u8],
    /// Where transport bytes arrive; at least one byte.
    pub receiving: &'a mut [u8],
    /// Where TLS bytes wait for the transport; at least one byte.
    pub sending: &'a mut [u8],
}

/// Sends `request` over `transport` and hands the response to `handler`.
///
/// # Errors
///
/// [`HttpsError`] names the layer that failed. `handler` may have received
/// a head and part of a body before an error.
pub fn exchange<E>(
    config: &ClientConfig<'_>,
    rng: &mut dyn Rng,
    buffers: Buffers<'_>,
    transport: &mut dyn Transport<Error = E>,
    request: &Request<'_>,
    handler: &mut dyn Handler,
) -> Result<(), HttpsError<E>> {
    let Buffers {
        tls,
        request: request_buffer,
        plaintext,
        head,
        receiving,
        sending,
    } = buffers;
    if plaintext.len() < MAX_PLAINTEXT || receiving.is_empty() || sending.is_empty() {
        return Err(HttpsError::BufferTooSmall);
    }
    let mut writer = Writer::new(request_buffer);
    request.write(&mut writer).map_err(HttpsError::Http)?;
    let outgoing = writer.finish();

    let mut connection = Connection::new(config, rng, tls).map_err(HttpsError::Tls)?;
    let mut decoder = Decoder::<MAX_FIELDS>::new(head, request.method);
    let mut sent = 0usize;
    let mut start = 0usize;
    let mut end = 0usize;
    loop {
        let pending = receiving.get(start..end).unwrap_or(&[]);
        start = start.saturating_add(connection.read_tls(pending).map_err(HttpsError::Tls)?);

        if connection.is_handshaked() {
            if let Some(rest) = outgoing.get(sent..).filter(|rest| !rest.is_empty()) {
                let sealed = connection.send(rest).map_err(HttpsError::Tls)?;
                sent = sent.saturating_add(sealed);
                flush(&mut connection, transport, sending)?;
                continue;
            }
            let opened = connection.recv(plaintext).map_err(HttpsError::Tls)?;
            if opened != 0 {
                deliver(
                    &mut decoder,
                    plaintext.get(..opened).unwrap_or(&[]),
                    handler,
                )?;
                if decoder.is_done() {
                    break;
                }
                continue;
            }
        }

        match connection.poll().map_err(HttpsError::Tls)? {
            Event::WantsWrite => flush(&mut connection, transport, sending)?,
            Event::PeerClosed => {
                decoder.finish().map_err(HttpsError::Http)?;
                return Ok(());
            }
            Event::WantsRead | Event::Handshaked => {
                if start < end {
                    // `poll` and `recv` consumed every whole record, so the
                    // connection takes the next bytes or no record fits.
                    let pending = receiving.get(start..end).unwrap_or(&[]);
                    let taken = connection.read_tls(pending).map_err(HttpsError::Tls)?;
                    if taken == 0 {
                        return Err(HttpsError::BufferTooSmall);
                    }
                    start = start.saturating_add(taken);
                    continue;
                }
                let received = transport.recv(receiving).map_err(HttpsError::Transport)?;
                if received == 0 {
                    return Err(HttpsError::Closed);
                }
                start = 0;
                end = received;
            }
        }
    }
    let _ = connection.close();
    let _ = flush(&mut connection, transport, sending);
    Ok(())
}

/// Hands every byte the connection has written to the transport.
fn flush<E>(
    connection: &mut Connection<'_>,
    transport: &mut dyn Transport<Error = E>,
    sending: &mut [u8],
) -> Result<(), HttpsError<E>> {
    loop {
        let length = connection.write_tls(sending).map_err(HttpsError::Tls)?;
        if length == 0 {
            return Ok(());
        }
        transport
            .send(sending.get(..length).unwrap_or(&[]))
            .map_err(HttpsError::Transport)?;
    }
}

/// Decodes `plaintext` and hands what it holds to `handler`.
fn deliver<E>(
    decoder: &mut Decoder<'_, MAX_FIELDS>,
    plaintext: &[u8],
    handler: &mut dyn Handler,
) -> Result<(), HttpsError<E>> {
    let mut rest = plaintext;
    loop {
        let (used, event) = decoder.feed(rest).map_err(HttpsError::Http)?;
        match event {
            HttpEvent::Head => {
                let head = decoder.head().ok_or(HttpsError::Closed)?;
                handler.head(&head).map_err(|Refused| HttpsError::Refused)?;
            }
            HttpEvent::Body(piece) => handler.body(piece).map_err(|Refused| HttpsError::Refused)?,
            HttpEvent::NeedMore | HttpEvent::Done => {}
        }
        rest = rest.get(used..).unwrap_or(&[]);
        if used == 0 {
            return Ok(());
        }
    }
}
