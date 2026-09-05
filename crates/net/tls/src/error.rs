// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why the protocol refused something.

use core::fmt;

/// Why the protocol refused something.
///
/// Each variant is one rule. The alert a peer is told about is derived
/// from the variant, so that the reason a connection failed and the reason
/// the peer is given cannot drift apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TlsError {
    /// A record is longer than the protocol allows.
    RecordOverflow,
    /// A record carries a content type that is not one of the four.
    UnknownContentType,
    /// A record arrived that this state does not expect.
    UnexpectedMessage,
    /// A record failed to decrypt, or its tag did not belong to it.
    BadRecord,
    /// The sequence number of a key epoch is exhausted.
    SequenceExhausted,
    /// A buffer the caller supplied is too small for what must go in it.
    BufferTooSmall,
    /// A derivation asked for more than the construction can produce.
    BadDerivation,
    /// The transcript was asked for a hash before its algorithm was known.
    TranscriptNotStarted,
    /// A message does not have the shape its type prescribes.
    Decode,
    /// A message cannot be written as it was described.
    Encode,
    /// The peer named a version this client does not speak.
    UnsupportedVersion,
    /// The peer chose a cipher suite that was not offered.
    UnsupportedSuite,
    /// A message lacks an extension it must carry.
    MissingExtension,
    /// A message carries an extension twice, or one that was not offered.
    UnexpectedExtension,
    /// A field carries a value the protocol forbids here.
    IllegalParameter,
    /// A certificate is malformed, or is not one this client can read.
    BadCertificate,
    /// A certificate is outside its validity window.
    CertificateExpired,
    /// The chain reaches nothing this client trusts, or the name is not
    /// the one that was asked for.
    UnknownAuthority,
    /// A signature does not verify.
    BadSignature,
    /// The key exchange produced nothing usable.
    NoSharedSecret,
}

impl fmt::Display for TlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TlsError::RecordOverflow => f.write_str("the record is longer than allowed"),
            TlsError::UnknownContentType => f.write_str("the content type is not one of the four"),
            TlsError::UnexpectedMessage => f.write_str("the record was not expected here"),
            TlsError::BadRecord => f.write_str("the record does not decrypt"),
            TlsError::SequenceExhausted => f.write_str("the sequence number is exhausted"),
            TlsError::BufferTooSmall => f.write_str("the buffer is too small"),
            TlsError::BadDerivation => f.write_str("the derivation asked for too much"),
            TlsError::TranscriptNotStarted => {
                f.write_str("the transcript has no hash algorithm yet")
            }
            TlsError::Decode => f.write_str("the message does not have the expected shape"),
            TlsError::Encode => f.write_str("the message cannot be written as described"),
            TlsError::UnsupportedVersion => f.write_str("the peer named another version"),
            TlsError::UnsupportedSuite => {
                f.write_str("the peer chose a suite that was not offered")
            }
            TlsError::MissingExtension => f.write_str("a required extension is absent"),
            TlsError::UnexpectedExtension => f.write_str("an extension repeats or was not offered"),
            TlsError::IllegalParameter => f.write_str("a field carries a forbidden value"),
            TlsError::BadCertificate => f.write_str("the certificate cannot be read"),
            TlsError::CertificateExpired => f.write_str("the certificate is outside its window"),
            TlsError::UnknownAuthority => {
                f.write_str("the chain reaches nothing this client trusts")
            }
            TlsError::BadSignature => f.write_str("the signature does not verify"),
            TlsError::NoSharedSecret => f.write_str("the key exchange produced nothing usable"),
        }
    }
}
