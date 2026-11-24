// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why an exchange ended without a complete response.

use core::fmt;

use audhsos_tls::TlsError;
use net_http::HttpError;

/// Why an exchange ended without a complete response. `E` is the error of
/// the caller's transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpsError<E> {
    /// The transport failed.
    Transport(E),
    /// The TLS connection failed, the certificate path included.
    Tls(TlsError),
    /// The request could not be written, or the response is malformed.
    Http(HttpError),
    /// The handler refused the response.
    Refused,
    /// The peer closed before the response was complete.
    Closed,
    /// A buffer is below its minimum, or bytes arrived that no buffer holds.
    BufferTooSmall,
}

impl<E: fmt::Display> fmt::Display for HttpsError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpsError::Transport(error) => write!(f, "transport: {error}"),
            HttpsError::Tls(error) => write!(f, "tls: {error}"),
            HttpsError::Http(error) => write!(f, "http: {error}"),
            HttpsError::Refused => f.write_str("the handler refused the response"),
            HttpsError::Closed => f.write_str("the peer closed before the response was complete"),
            HttpsError::BufferTooSmall => f.write_str("a buffer is too small"),
        }
    }
}
