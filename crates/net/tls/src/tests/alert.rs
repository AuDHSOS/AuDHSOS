// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a peer is told, and what it means when a peer says it.

use crate::alert::{Alert, FATAL, WARNING};
use crate::error::TlsError;

/// Every error this crate can end a connection with.
const ERRORS: [TlsError; 18] = [
    TlsError::RecordOverflow,
    TlsError::UnknownContentType,
    TlsError::UnexpectedMessage,
    TlsError::BadRecord,
    TlsError::SequenceExhausted,
    TlsError::BufferTooSmall,
    TlsError::BadDerivation,
    TlsError::TranscriptNotStarted,
    TlsError::Decode,
    TlsError::Encode,
    TlsError::UnsupportedVersion,
    TlsError::UnsupportedSuite,
    TlsError::MissingExtension,
    TlsError::UnexpectedExtension,
    TlsError::IllegalParameter,
    TlsError::BadCertificate,
    TlsError::CertificateExpired,
    TlsError::UnknownAuthority,
];

/// Every alert this crate knows.
const ALERTS: [Alert; 15] = [
    Alert::CloseNotify,
    Alert::UnexpectedMessage,
    Alert::BadRecordMac,
    Alert::RecordOverflow,
    Alert::HandshakeFailure,
    Alert::BadCertificate,
    Alert::CertificateExpired,
    Alert::UnknownCa,
    Alert::IllegalParameter,
    Alert::ProtocolVersion,
    Alert::DecodeError,
    Alert::DecryptError,
    Alert::MissingExtension,
    Alert::UnsupportedExtension,
    Alert::InternalError,
];

#[test]
fn every_error_names_an_alert_and_never_the_closing_one() {
    for error in ERRORS {
        let alert = Alert::for_error(error);
        assert_ne!(
            alert,
            Alert::CloseNotify,
            "{error:?} is a failure, and close notify says nothing failed"
        );
        assert_eq!(alert.level(), FATAL, "{error:?}");
    }
    assert_eq!(
        Alert::for_error(TlsError::BadSignature),
        Alert::DecryptError
    );
    assert_eq!(
        Alert::for_error(TlsError::NoSharedSecret),
        Alert::InternalError
    );
}

#[test]
fn an_alert_is_the_code_point_it_is_named_by() {
    for alert in ALERTS {
        assert_eq!(Alert::from_code(alert.code()), Some(alert), "{alert:?}");
    }
    assert_eq!(Alert::CloseNotify.level(), WARNING);
    assert_eq!(Alert::CloseNotify.code(), 0);

    for code in [1u8, 21, 99, 255] {
        assert_eq!(Alert::from_code(code), None, "code {code}");
    }
}

#[test]
fn the_errors_of_the_later_parts_render_a_message() {
    for error in [
        TlsError::BadCertificate,
        TlsError::CertificateExpired,
        TlsError::UnknownAuthority,
        TlsError::BadSignature,
        TlsError::NoSharedSecret,
        TlsError::Decode,
        TlsError::Encode,
        TlsError::UnsupportedVersion,
        TlsError::UnsupportedSuite,
        TlsError::MissingExtension,
        TlsError::UnexpectedExtension,
        TlsError::IllegalParameter,
    ] {
        assert!(!format!("{error}").is_empty());
        assert!(!format!("{error:?}").is_empty());
    }
}
