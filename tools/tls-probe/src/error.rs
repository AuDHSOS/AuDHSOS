// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a probe run stopped.
//!
//! The crates under test are `no_std` and their errors do not implement
//! `std::error::Error`, so the probe carries one enum that names the layer
//! a failure came from. Which layer refused is the whole point of the
//! exercise, so it is not flattened into a string.

use std::fmt;

use audhsos_encoding::EncodingError;
use audhsos_time::TimeError;
use audhsos_tls::TlsError;
use audhsos_x509::X509Error;
use crypto_rng::RngError;

/// Why a probe run stopped.
#[derive(Debug)]
pub enum ProbeError {
    /// The socket, the device, or a file.
    Io(std::io::Error),
    /// The anchor is not a PEM block or a certificate.
    Anchor(AnchorError),
    /// The generator, or the source under it.
    Rng(RngError),
    /// The clock could not be turned into a civil time.
    Time(TimeError),
    /// The protocol refused something.
    Tls(TlsError),
    /// The response is not HTTP this client reads.
    Http(HttpError),
    /// The command line does not name a run.
    Usage(&'static str),
}

/// Why an anchor could not be read.
#[derive(Debug)]
pub enum AnchorError {
    /// The file is neither DER nor a PEM block.
    Encoding(EncodingError),
    /// The bytes are not a certificate this crate reads.
    Certificate(X509Error),
    /// The PEM block is labelled something other than `CERTIFICATE`.
    WrongLabel,
}

/// Why a response could not be read.
#[derive(Debug)]
pub enum HttpError {
    /// The head is not terminated by a blank line.
    NoHead,
    /// The status line does not have three parts.
    BadStatusLine,
    /// The status code is not three digits.
    BadStatus,
    /// A header line carries no colon.
    BadHeader,
    /// A chunk length is not hexadecimal, or a chunk is truncated.
    BadChunk,
    /// The head is not UTF-8. Only the head is required to be; the body is
    /// handed on as bytes.
    NotText,
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProbeError::Io(error) => write!(f, "transport: {error}"),
            ProbeError::Anchor(error) => write!(f, "anchor: {error}"),
            ProbeError::Rng(error) => write!(f, "randomness: {error}"),
            ProbeError::Time(error) => write!(f, "clock: {error}"),
            ProbeError::Tls(error) => write!(f, "tls: {error}"),
            ProbeError::Http(error) => write!(f, "http: {error}"),
            ProbeError::Usage(message) => write!(f, "usage: {message}"),
        }
    }
}

impl fmt::Display for AnchorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnchorError::Encoding(error) => write!(f, "{error}"),
            AnchorError::Certificate(error) => write!(f, "{error}"),
            AnchorError::WrongLabel => f.write_str("the PEM block is not a certificate"),
        }
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpError::NoHead => f.write_str("the head is not terminated"),
            HttpError::BadStatusLine => f.write_str("the status line is malformed"),
            HttpError::BadStatus => f.write_str("the status code is not three digits"),
            HttpError::BadHeader => f.write_str("a header line carries no colon"),
            HttpError::BadChunk => f.write_str("a chunk is malformed or truncated"),
            HttpError::NotText => f.write_str("the head is not UTF-8"),
        }
    }
}

impl From<std::io::Error> for ProbeError {
    fn from(error: std::io::Error) -> ProbeError {
        ProbeError::Io(error)
    }
}

impl From<AnchorError> for ProbeError {
    fn from(error: AnchorError) -> ProbeError {
        ProbeError::Anchor(error)
    }
}

impl From<RngError> for ProbeError {
    fn from(error: RngError) -> ProbeError {
        ProbeError::Rng(error)
    }
}

impl From<TimeError> for ProbeError {
    fn from(error: TimeError) -> ProbeError {
        ProbeError::Time(error)
    }
}

impl From<TlsError> for ProbeError {
    fn from(error: TlsError) -> ProbeError {
        ProbeError::Tls(error)
    }
}

impl From<HttpError> for ProbeError {
    fn from(error: HttpError) -> ProbeError {
        ProbeError::Http(error)
    }
}

impl From<X509Error> for AnchorError {
    fn from(error: X509Error) -> AnchorError {
        AnchorError::Certificate(error)
    }
}
