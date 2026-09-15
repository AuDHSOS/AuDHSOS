// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The authentication exchange: the service request, the two forms of a
//! `publickey` request, what a server answers, and the extension that
//! may come with it.
//!
//! RFC 4252 publishes no vector either, so a request is read back field
//! by field with a reader of this crate and the signature is checked the
//! way a server checks it, with `crypto-ec`.

use crypto_ec::ed25519;

use crate::auth::{
    ClientKey, ExtInfo, METHOD_PUBLICKEY, PK_OK, Request, Response, SECRET_LEN, SERVER_SIG_ALGS,
    SERVICE_CONNECTION, SERVICE_USERAUTH, read_service_accept, request_len, signed_len,
    write_publickey, write_query, write_service_request,
};
use crate::error::SshError;
use crate::hostkey::BLOB_LEN;
use crate::kex::HOST_KEY_ED25519;
use crate::msg;
use crate::tests::ssh_string;
use crate::wire::{Reader, Writer};

/// The client's private key.
const CLIENT_SECRET: [u8; SECRET_LEN] = [0x66; SECRET_LEN];

/// The session identifier the signature is bound to.
const SESSION_ID: [u8; 32] = [0x77; 32];

/// The user this client authenticates as.
const USER: &str = "audhsos";

/// Room for the longest request these tests write.
const ROOM: usize = 512;

/// The request every test sends.
fn request() -> Request<'static> {
    Request {
        user: USER,
        service: SERVICE_CONNECTION,
    }
}

/// The key blob the requests carry.
fn key_blob(key: &ClientKey) -> Vec<u8> {
    let mut blob = [0u8; BLOB_LEN];
    let len = key
        .write_blob(&mut blob)
        .expect("the buffer is long enough");
    blob.get(..len).unwrap_or_default().to_vec()
}

/// The `SSH_MSG_USERAUTH_PK_OK` a server answers a query with.
fn pk_ok(algorithm: &str, blob: &[u8]) -> Vec<u8> {
    let mut payload = vec![PK_OK];
    payload.extend_from_slice(&ssh_string(algorithm.as_bytes()));
    payload.extend_from_slice(&ssh_string(blob));
    payload
}

#[test]
fn the_service_request_names_the_service_and_the_accept_answers_it() {
    let mut out = [0u8; ROOM];
    let len = write_service_request(SERVICE_USERAUTH, &mut out).expect("there is room");

    let mut reader = Reader::new(out.get(..len).unwrap_or_default());
    assert_eq!(reader.read_byte(), Ok(msg::SERVICE_REQUEST));
    assert_eq!(reader.read_string(), Ok(SERVICE_USERAUTH.as_bytes()));
    assert!(reader.is_empty());

    let mut accept = [0u8; ROOM];
    let mut writer = Writer::new(&mut accept);
    writer
        .write_byte(msg::SERVICE_ACCEPT)
        .expect("there is room");
    writer
        .write_string(SERVICE_USERAUTH.as_bytes())
        .expect("there is room");
    let written = writer.position();

    assert_eq!(
        read_service_accept(accept.get(..written).unwrap_or_default()),
        Ok(SERVICE_USERAUTH.as_bytes())
    );
}

#[test]
fn another_message_is_no_service_accept() {
    assert_eq!(
        read_service_accept(&[msg::SERVICE_REQUEST, 0, 0, 0, 0]),
        Err(SshError::Message(msg::SERVICE_REQUEST))
    );
    assert!(matches!(
        read_service_accept(&[]),
        Err(SshError::OutOfBounds { .. })
    ));
}

#[test]
fn the_query_carries_the_key_and_no_signature() {
    let key = ClientKey::new(CLIENT_SECRET);
    let mut out = [0u8; ROOM];
    let len = write_query(&request(), &key, &mut out).expect("there is room");

    let mut reader = Reader::new(out.get(..len).unwrap_or_default());
    assert_eq!(reader.read_byte(), Ok(msg::USERAUTH_REQUEST));
    assert_eq!(reader.read_string(), Ok(USER.as_bytes()));
    assert_eq!(reader.read_string(), Ok(SERVICE_CONNECTION.as_bytes()));
    assert_eq!(reader.read_string(), Ok(METHOD_PUBLICKEY.as_bytes()));
    assert_eq!(reader.read_boolean(), Ok(false));
    assert_eq!(reader.read_string(), Ok(HOST_KEY_ED25519.as_bytes()));
    assert_eq!(reader.read_string(), Ok(key_blob(&key).as_slice()));
    assert!(reader.is_empty());
}

#[test]
fn the_request_that_authenticates_is_the_query_signed() {
    let key = ClientKey::new(CLIENT_SECRET);
    let mut scratch = [0u8; ROOM];
    let mut out = [0u8; ROOM];
    let len = write_publickey(&request(), &key, &SESSION_ID, &mut scratch, &mut out)
        .expect("there is room");

    let payload = out.get(..len).unwrap_or_default();
    let mut reader = Reader::new(payload);
    assert_eq!(reader.read_byte(), Ok(msg::USERAUTH_REQUEST));
    assert_eq!(reader.read_string(), Ok(USER.as_bytes()));
    assert_eq!(reader.read_string(), Ok(SERVICE_CONNECTION.as_bytes()));
    assert_eq!(reader.read_string(), Ok(METHOD_PUBLICKEY.as_bytes()));
    assert_eq!(reader.read_boolean(), Ok(true));
    assert_eq!(reader.read_string(), Ok(HOST_KEY_ED25519.as_bytes()));
    assert_eq!(reader.read_string(), Ok(key_blob(&key).as_slice()));
    let fields = reader.position();

    let mut signature = Reader::new(reader.read_string().expect("the signature is a string"));
    assert_eq!(signature.read_string(), Ok(HOST_KEY_ED25519.as_bytes()));
    let value: [u8; 64] = signature
        .read_string()
        .expect("the signature is sixty-four octets")
        .try_into()
        .expect("the signature is sixty-four octets");
    assert!(signature.is_empty());
    assert!(reader.is_empty());

    // What a server verifies: the session identifier, then the fields of
    // the request as they stand.
    let mut signed = ssh_string(&SESSION_ID);
    signed.extend_from_slice(payload.get(..fields).unwrap_or_default());

    assert_eq!(ed25519::verify(key.public_key(), &signed, &value), Ok(()));
}

#[test]
fn a_signature_of_one_connection_is_worthless_on_another() {
    let key = ClientKey::new(CLIENT_SECRET);
    let mut scratch = [0u8; ROOM];
    let mut first = [0u8; ROOM];
    let mut second = [0u8; ROOM];

    let one = write_publickey(&request(), &key, &SESSION_ID, &mut scratch, &mut first)
        .expect("there is room");
    let mut other_session = SESSION_ID;
    other_session[0] ^= 0x01;
    let two = write_publickey(&request(), &key, &other_session, &mut scratch, &mut second)
        .expect("there is room");

    assert_eq!(one, two);
    assert_ne!(first.get(..one), second.get(..two));
}

#[test]
fn the_lengths_are_what_the_two_requests_need() {
    let key = ClientKey::new(CLIENT_SECRET);
    let mut scratch = [0u8; ROOM];
    let mut out = [0u8; ROOM];
    let len = write_publickey(&request(), &key, &SESSION_ID, &mut scratch, &mut out)
        .expect("there is room");

    let signed = signed_len(USER.len(), SERVICE_CONNECTION.len());
    assert_eq!(len, request_len(USER.len(), SERVICE_CONNECTION.len()));

    let mut exact = vec![0u8; signed];
    let mut room = [0u8; ROOM];
    assert!(write_publickey(&request(), &key, &SESSION_ID, &mut exact, &mut room).is_ok());

    let mut short = vec![0u8; signed - 1];
    assert!(matches!(
        write_publickey(&request(), &key, &SESSION_ID, &mut short, &mut room),
        Err(SshError::OutOfBounds { .. })
    ));
}

#[test]
fn a_buffer_too_small_writes_no_request() {
    let key = ClientKey::new(CLIENT_SECRET);
    let mut scratch = [0u8; ROOM];
    let mut out = [0u8; 8];

    assert!(matches!(
        write_query(&request(), &key, &mut out),
        Err(SshError::OutOfBounds { .. })
    ));
    assert!(matches!(
        write_publickey(&request(), &key, &SESSION_ID, &mut scratch, &mut out),
        Err(SshError::OutOfBounds { .. })
    ));
    assert!(matches!(
        write_service_request(SERVICE_USERAUTH, &mut [0u8; 2]),
        Err(SshError::OutOfBounds { .. })
    ));
}

#[test]
fn the_public_half_is_the_one_the_secret_stands_for() {
    let key = ClientKey::new(CLIENT_SECRET);

    assert_eq!(key.public_key(), &ed25519::public_key(&CLIENT_SECRET));
    assert!(matches!(
        key.write_blob(&mut [0u8; BLOB_LEN - 1]),
        Err(SshError::OutOfBounds { .. })
    ));
}

#[test]
fn a_failure_lists_what_may_continue_and_says_whether_a_step_succeeded() {
    let mut payload = vec![msg::USERAUTH_FAILURE];
    payload.extend_from_slice(&ssh_string(b"publickey,password"));
    payload.push(1);

    let Ok(Response::Failure { methods, partial }) = Response::read(&payload) else {
        panic!("a failure reads as a failure");
    };

    assert!(methods.contains("publickey"));
    assert!(methods.contains("password"));
    assert!(partial);
}

#[test]
fn success_is_the_message_alone_and_a_banner_carries_its_text() {
    let mut banner = vec![msg::USERAUTH_BANNER];
    banner.extend_from_slice(&ssh_string("two lines\r\n".as_bytes()));
    banner.extend_from_slice(&ssh_string(b""));

    assert_eq!(
        Response::read(&[msg::USERAUTH_SUCCESS]),
        Ok(Response::Success)
    );
    assert_eq!(
        Response::read(&banner),
        Ok(Response::Banner {
            message: "two lines\r\n".as_bytes(),
            language: b"",
        })
    );
}

#[test]
fn leave_to_sign_is_the_algorithm_and_the_blob_of_the_query() {
    let key = ClientKey::new(CLIENT_SECRET);
    let blob = key_blob(&key);
    let other = key_blob(&ClientKey::new([0x99; SECRET_LEN]));

    let answer = pk_ok(HOST_KEY_ED25519, &blob);
    let read = Response::read(&answer).expect("the answer is a PK_OK");
    assert!(read.is_leave_to_sign(&blob));

    let for_another_key = pk_ok(HOST_KEY_ED25519, &other);
    let under_another_algorithm = pk_ok("ssh-rsa", &blob);
    let another_key = Response::read(&for_another_key).expect("a PK_OK");
    let another_algorithm = Response::read(&under_another_algorithm).expect("a PK_OK");
    assert!(!another_key.is_leave_to_sign(&blob));
    assert!(!another_algorithm.is_leave_to_sign(&blob));
    assert!(!Response::Success.is_leave_to_sign(&blob));
}

#[test]
fn a_message_of_another_layer_is_no_answer_of_this_one() {
    assert_eq!(
        Response::read(&[msg::KEXINIT]),
        Err(SshError::Message(msg::KEXINIT))
    );
    assert!(matches!(
        Response::read(&[]),
        Err(SshError::OutOfBounds { .. })
    ));
    assert!(matches!(
        Response::read(&[msg::USERAUTH_FAILURE]),
        Err(SshError::OutOfBounds { .. })
    ));
}

/// An `SSH_MSG_EXT_INFO` carrying the named extensions in order.
fn ext_info(extensions: &[(&str, &[u8])]) -> Vec<u8> {
    let mut payload = vec![msg::EXT_INFO];
    payload.extend_from_slice(
        &u32::try_from(extensions.len())
            .unwrap_or_default()
            .to_be_bytes(),
    );
    for (name, value) in extensions {
        payload.extend_from_slice(&ssh_string(name.as_bytes()));
        payload.extend_from_slice(&ssh_string(value));
    }
    payload
}

#[test]
fn server_sig_algs_is_read_and_every_other_extension_is_ignored() {
    let payload = ext_info(&[
        ("delay-compression", &[0x00, 0x01, 0x00]),
        (SERVER_SIG_ALGS, b"ssh-ed25519,rsa-sha2-256"),
        ("no-flow-control", b"p"),
    ]);

    let info = ExtInfo::read(&payload).expect("the message is an EXT_INFO");
    let algorithms = info.server_sig_algs.expect("the extension was sent");

    assert!(algorithms.contains("ssh-ed25519"));
    assert!(algorithms.contains("rsa-sha2-256"));
}

#[test]
fn an_ext_info_without_the_extension_says_nothing_about_algorithms() {
    let payload = ext_info(&[("delay-compression", b"")]);

    let info = ExtInfo::read(&payload).expect("the message is an EXT_INFO");

    assert_eq!(info.server_sig_algs, None);
    assert_eq!(
        ExtInfo::read(&ext_info(&[])),
        Ok(ExtInfo {
            server_sig_algs: None
        })
    );
}

#[test]
fn an_ext_info_that_is_not_one_is_refused() {
    let mut counted_wrong = ext_info(&[(SERVER_SIG_ALGS, b"ssh-ed25519")]);
    counted_wrong.truncate(counted_wrong.len() - 4);

    assert_eq!(
        ExtInfo::read(&[msg::NEWKEYS]),
        Err(SshError::Message(msg::NEWKEYS))
    );
    assert!(matches!(
        ExtInfo::read(&counted_wrong),
        Err(SshError::OutOfBounds { .. })
    ));
    assert_eq!(
        ExtInfo::read(&ext_info(&[(SERVER_SIG_ALGS, b"ssh-ed25519,,rsa")])),
        Err(SshError::NameList)
    );
}
