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
        let alert = Alert::for_error(error).expect("a rule this client applied names an alert");
        assert_ne!(
            alert,
            Alert::CloseNotify,
            "{error:?} is a failure, and close notify says nothing failed"
        );
        assert_eq!(alert.level(), FATAL, "{error:?}");
    }
    assert_eq!(
        Alert::for_error(TlsError::BadSignature),
        Some(Alert::DecryptError)
    );
    assert_eq!(
        Alert::for_error(TlsError::NoSharedSecret),
        Some(Alert::InternalError)
    );
}

#[test]
fn the_peers_own_alert_is_the_one_error_with_nothing_to_send() {
    // RFC 8446 section 6.2: a fatal alert closes the connection on both
    // sides, so there is nobody left to tell. Every other error names an
    // alert, and this one names none, whatever code the peer sent.
    for code in [10u8, 40, 47, 80, 0xFF] {
        assert_eq!(Alert::for_error(TlsError::PeerAlert(code)), None, "{code}");
    }
}

#[test]
fn an_alert_this_client_received_says_which_one_it_was() {
    // The point of carrying the code: `handshake_failure` is a server
    // saying it could not agree, and it used to read as a complaint about
    // the record the alert arrived in.
    assert_eq!(
        format!("{}", TlsError::PeerAlert(40)),
        "the peer sent the alert handshake_failure"
    );
    assert_eq!(
        format!("{}", TlsError::PeerAlert(48)),
        "the peer sent the alert unknown_ca"
    );

    // A code this client does not know is still reported as a number
    // rather than as something else entirely.
    assert_eq!(
        format!("{}", TlsError::PeerAlert(112)),
        "the peer sent alert 112, which is not one this client knows"
    );
    assert!(!format!("{:?}", TlsError::PeerAlert(40)).is_empty());
}

#[test]
fn every_alert_has_the_name_the_standard_gives_it() {
    let mut seen: Vec<&str> = Vec::new();
    for alert in ALERTS {
        let name = alert.name();
        assert!(!name.is_empty(), "{alert:?}");
        assert!(!seen.contains(&name), "{name} names two alerts");
        seen.push(name);
    }
    assert_eq!(Alert::HandshakeFailure.name(), "handshake_failure");
    assert_eq!(Alert::CloseNotify.name(), "close_notify");
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
