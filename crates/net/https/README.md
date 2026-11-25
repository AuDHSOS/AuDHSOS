# net-https

One HTTPS exchange: `audhsos-tls` carries a request that `net-http`
writes and a response that `net-http` decodes. The caller supplies the
transport through the trait `Transport` and receives the response through
the trait `Handler`. The crate opens no connection, reads no clock, and
allocates nothing; the caller owns every buffer.

`exchange` runs one request on one connection and closes it:

1. `Connection::new` writes the first flight; `poll` drives the handshake.
2. `send` seals the request once `Connection::is_handshaked` holds.
3. `recv` opens each record; `Decoder::feed` decodes the plaintext.
4. A complete message or the peer's `close_notify` ends the exchange.

End of the transport before `close_notify`:

- Message complete by `Content-Length` or the last chunk: `Ok`, because
  `exchange` returns before it reads further (RFC 9112, section 9.8).
- Any other message, a body that ends at close included:
  `HttpsError::Closed`, because a receiver of a transport-level close
  cannot know that all data arrived (RFC 8446, section 6.1).

A handler refuses a response with `Refused`, which ends the exchange; a
caller bounds the response size this way.
