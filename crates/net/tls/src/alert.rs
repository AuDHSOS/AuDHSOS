// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Alerts: what a peer is told when something goes wrong, and what it
//! means when a peer says it.
//!
//! Every alert this client sends is derived from the error that caused it,
//! so the reason a connection failed and the reason the peer is given
//! cannot drift apart. The mapping is one function and it is tested
//! exhaustively. It answers `None` for exactly one error: the one that
//! says the peer already sent an alert of its own.

use crate::error::TlsError;

/// The level of an alert. TLS 1.3 has one that matters.
pub const FATAL: u8 = 2;
/// The other level, which only `close_notify` uses.
pub const WARNING: u8 = 1;

/// What an alert says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Alert {
    /// The connection is closing, and nothing is wrong.
    CloseNotify,
    /// A message arrived that the state did not expect.
    UnexpectedMessage,
    /// A record did not decrypt or its tag did not belong to it.
    BadRecordMac,
    /// A record was longer than the protocol allows.
    RecordOverflow,
    /// The peer offered nothing this client can use.
    HandshakeFailure,
    /// A certificate is malformed.
    BadCertificate,
    /// A certificate has expired or is not valid yet.
    CertificateExpired,
    /// A certificate chain reaches nothing this client trusts.
    UnknownCa,
    /// A field carries a value the protocol forbids.
    IllegalParameter,
    /// The peer named a version this client does not speak.
    ProtocolVersion,
    /// A message is malformed in a way no other alert names.
    DecodeError,
    /// A signature did not verify.
    DecryptError,
    /// An extension is missing that the message must carry.
    MissingExtension,
    /// An extension appeared that was not offered.
    UnsupportedExtension,
    /// The client cannot go on for a reason of its own.
    InternalError,
}

impl Alert {
    /// The code point of the alert.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Alert::CloseNotify => 0,
            Alert::UnexpectedMessage => 10,
            Alert::BadRecordMac => 20,
            Alert::RecordOverflow => 22,
            Alert::HandshakeFailure => 40,
            Alert::BadCertificate => 42,
            Alert::CertificateExpired => 45,
            Alert::UnknownCa => 48,
            Alert::DecodeError => 50,
            Alert::DecryptError => 51,
            Alert::ProtocolVersion => 70,
            Alert::InternalError => 80,
            Alert::IllegalParameter => 47,
            Alert::MissingExtension => 109,
            Alert::UnsupportedExtension => 110,
        }
    }

    /// The level an alert is sent at. Only the closing one is not fatal.
    #[must_use]
    pub const fn level(self) -> u8 {
        match self {
            Alert::CloseNotify => WARNING,
            _ => FATAL,
        }
    }

    /// The name RFC 8446 gives the alert.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Alert::CloseNotify => "close_notify",
            Alert::UnexpectedMessage => "unexpected_message",
            Alert::BadRecordMac => "bad_record_mac",
            Alert::RecordOverflow => "record_overflow",
            Alert::HandshakeFailure => "handshake_failure",
            Alert::BadCertificate => "bad_certificate",
            Alert::CertificateExpired => "certificate_expired",
            Alert::UnknownCa => "unknown_ca",
            Alert::IllegalParameter => "illegal_parameter",
            Alert::ProtocolVersion => "protocol_version",
            Alert::DecodeError => "decode_error",
            Alert::DecryptError => "decrypt_error",
            Alert::MissingExtension => "missing_extension",
            Alert::UnsupportedExtension => "unsupported_extension",
            Alert::InternalError => "internal_error",
        }
    }

    /// The alert a peer should be told about this error, when there is one
    /// to tell.
    ///
    /// [`TlsError::PeerAlert`] answers `None`, and it is the only error
    /// that does. RFC 8446 section 6.2 has both sides close the connection
    /// on a fatal alert; a client that answered one would be writing into
    /// a connection the peer has already given up on, and saying
    /// `unexpected_message` about a message that was perfectly expected.
    #[must_use]
    pub const fn for_error(error: TlsError) -> Option<Alert> {
        let alert = match error {
            TlsError::RecordOverflow => Alert::RecordOverflow,
            TlsError::UnknownContentType | TlsError::UnexpectedMessage => Alert::UnexpectedMessage,
            TlsError::BadRecord | TlsError::SequenceExhausted => Alert::BadRecordMac,
            TlsError::Decode | TlsError::Encode => Alert::DecodeError,
            TlsError::UnsupportedVersion => Alert::ProtocolVersion,
            TlsError::UnsupportedSuite => Alert::HandshakeFailure,
            TlsError::MissingExtension => Alert::MissingExtension,
            TlsError::UnexpectedExtension => Alert::UnsupportedExtension,
            TlsError::IllegalParameter => Alert::IllegalParameter,
            TlsError::BadCertificate => Alert::BadCertificate,
            TlsError::CertificateExpired => Alert::CertificateExpired,
            TlsError::UnknownAuthority => Alert::UnknownCa,
            TlsError::BadSignature => Alert::DecryptError,
            TlsError::BufferTooSmall
            | TlsError::BadDerivation
            | TlsError::TranscriptNotStarted
            | TlsError::NoSharedSecret => Alert::InternalError,
            TlsError::PeerAlert(_) => return None,
        };
        Some(alert)
    }

    /// The alert a peer sent, when it is one this client knows.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Alert> {
        match code {
            0 => Some(Alert::CloseNotify),
            10 => Some(Alert::UnexpectedMessage),
            20 => Some(Alert::BadRecordMac),
            22 => Some(Alert::RecordOverflow),
            40 => Some(Alert::HandshakeFailure),
            42 => Some(Alert::BadCertificate),
            45 => Some(Alert::CertificateExpired),
            47 => Some(Alert::IllegalParameter),
            48 => Some(Alert::UnknownCa),
            50 => Some(Alert::DecodeError),
            51 => Some(Alert::DecryptError),
            70 => Some(Alert::ProtocolVersion),
            80 => Some(Alert::InternalError),
            109 => Some(Alert::MissingExtension),
            110 => Some(Alert::UnsupportedExtension),
            _ => None,
        }
    }
}
