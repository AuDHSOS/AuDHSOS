// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The authentication layer of RFC 4252: the service request, the
//! `publickey` method with an Ed25519 key, and what a server answers
//! with.
//!
//! This client authenticates with a key and with nothing else. The
//! signature of section 7 is over the session identifier followed by the
//! request fields, which is what makes a signature captured from one
//! connection worthless on another.
//!
//! Secrets: [`ClientKey`] holds the private key and clears it when it is
//! dropped. The signed data goes into a buffer the caller owns, and that
//! buffer holds no secret — every field of it is on the wire.
//!
//! `server-sig-algs` of RFC 8308 arrives in an [`ExtInfo`], which this
//! module reads and the caller remembers. With one host key algorithm and
//! one authentication algorithm this client has nothing to choose from,
//! so the extension changes no behaviour today.

use crypto_ct::Secret;
use crypto_ec::ed25519::{self, PUBLIC_LEN, SIGNATURE_LEN};

use crate::error::SshError;
use crate::exchange::HASH_LEN;
use crate::hostkey::{BLOB_LEN, SIGNATURE_BLOB_LEN, write_blob};
use crate::kex::HOST_KEY_ED25519;
use crate::msg;
use crate::wire::{NameList, Reader, Writer};

/// The service a client asks for before it authenticates.
pub const SERVICE_USERAUTH: &str = "ssh-userauth";

/// The service it asks to have started once it has (RFC 4254, section
/// 1).
pub const SERVICE_CONNECTION: &str = "ssh-connection";

/// The one method of section 14.5.
pub const METHOD_PUBLICKEY: &str = "publickey";

/// Bytes of an Ed25519 private key (RFC 8032, section 5.1.5).
pub const SECRET_LEN: usize = 32;

/// The extension name of RFC 8308, section 3.1.
pub const SERVER_SIG_ALGS: &str = "server-sig-algs";

/// `SSH_MSG_USERAUTH_PK_OK` (RFC 4252, section 7), which is in the range
/// 60 to 79 that every method reuses and so is named with its method.
pub const PK_OK: u8 = 60;

/// Every field of the signed data but the two names: the session
/// identifier as a string, the message number, the length fields of the
/// names, the boolean, the algorithm name, and the key blob.
const SIGNED_FIXED: usize = 4
    + HASH_LEN
    + 1
    + 4
    + 4
    + 4
    + METHOD_PUBLICKEY.len()
    + 1
    + 4
    + HOST_KEY_ED25519.len()
    + 4
    + BLOB_LEN;

/// The session identifier as it stands in front of the signed data, and
/// nowhere in the request.
const SESSION_PREFIX: usize = 4 + HASH_LEN;

/// The signature blob as one `string`.
const SIGNATURE_FIELD: usize = 4 + SIGNATURE_BLOB_LEN;

/// The signed data of section 7 for a user name and a service name of
/// these lengths.
#[must_use]
pub const fn signed_len(user: usize, service: usize) -> usize {
    SIGNED_FIXED.saturating_add(user).saturating_add(service)
}

/// The request of section 7 for names of these lengths: the signed data
/// without the session identifier, and the signature after it.
#[must_use]
pub const fn request_len(user: usize, service: usize) -> usize {
    signed_len(user, service)
        .saturating_sub(SESSION_PREFIX)
        .saturating_add(SIGNATURE_FIELD)
}

/// The client's key: the secret of RFC 8032, section 5.1.5, and the
/// public half the server is asked to accept.
///
/// Where the secret comes from is the caller's — the scratch disk or a
/// generator — and section 14.13 of
/// [document 14](../../../docs/14-secure-shell-as-a-client.md) holds that
/// question open. This crate opens no file.
pub struct ClientKey {
    /// The private key, cleared when this is dropped.
    secret: Secret<SECRET_LEN>,
    /// Its public half, computed once because a scalar multiplication is
    /// not what a second request should cost.
    public: [u8; PUBLIC_LEN],
}

impl ClientKey {
    /// The pair `secret` stands for.
    #[must_use]
    pub fn new(secret: [u8; SECRET_LEN]) -> ClientKey {
        ClientKey {
            public: ed25519::public_key(&secret),
            secret: Secret::new(secret),
        }
    }

    /// The public half, as RFC 8032, section 5.1.5, encodes it.
    #[must_use]
    pub const fn public_key(&self) -> &[u8; PUBLIC_LEN] {
        &self.public
    }

    /// Writes the key blob of RFC 8709, section 4, which every request
    /// carries.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when `out` is too small.
    pub fn write_blob(&self, out: &mut [u8]) -> Result<usize, SshError> {
        write_blob(&self.public, out)
    }
}

/// Whom the request is for and what it asks to have started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Request<'a> {
    /// The user name, UTF-8 (RFC 4252, section 5).
    pub user: &'a str,
    /// The service to start once authentication succeeds, which for this
    /// client is [`SERVICE_CONNECTION`].
    pub service: &'a str,
}

/// Writes `SSH_MSG_SERVICE_REQUEST` for `service`.
///
/// # Errors
///
/// [`SshError::OutOfBounds`] when `out` is too small.
pub fn write_service_request(service: &str, out: &mut [u8]) -> Result<usize, SshError> {
    let mut writer = Writer::new(out);
    writer.write_byte(msg::SERVICE_REQUEST)?;
    writer.write_string(service.as_bytes())?;
    Ok(writer.position())
}

/// Reads the service name out of an `SSH_MSG_SERVICE_ACCEPT`.
///
/// # Errors
///
/// [`SshError::Message`] when the payload is another message and
/// [`SshError::OutOfBounds`] when it ends early.
pub fn read_service_accept(payload: &[u8]) -> Result<&[u8], SshError> {
    let mut reader = Reader::new(payload);
    let number = reader.read_byte()?;
    if number != msg::SERVICE_ACCEPT {
        return Err(SshError::Message(number));
    }
    reader.read_string()
}

/// Writes the query of section 7: the request with the boolean false and
/// no signature, which asks whether this key would be accepted.
///
/// The query costs one round trip and the signature it saves costs a
/// private key operation, so this client sends it first.
///
/// # Errors
///
/// [`SshError::OutOfBounds`] when `out` is too small.
pub fn write_query(
    request: &Request<'_>,
    key: &ClientKey,
    out: &mut [u8],
) -> Result<usize, SshError> {
    let mut writer = Writer::new(out);
    write_head(&mut writer, request)?;
    writer.write_boolean(false)?;
    writer.write_string(HOST_KEY_ED25519.as_bytes())?;
    write_key_string(&mut writer, key)?;
    Ok(writer.position())
}

/// Writes the request that authenticates: the same fields with the
/// boolean true, and the signature over them.
///
/// `scratch` holds the data that is signed — the session identifier and
/// then those fields — and [`signed_len`] is how long that is. The
/// request itself is that data without the session identifier, with the
/// signature blob after it, so no field is written twice.
///
/// # Errors
///
/// [`SshError::OutOfBounds`] when `out` or `scratch` is too small.
pub fn write_publickey(
    request: &Request<'_>,
    key: &ClientKey,
    session_id: &[u8; HASH_LEN],
    scratch: &mut [u8],
    out: &mut [u8],
) -> Result<usize, SshError> {
    let mut writer = Writer::new(scratch);
    writer.write_string(session_id)?;
    let start = writer.position();
    write_head(&mut writer, request)?;
    writer.write_boolean(true)?;
    writer.write_string(HOST_KEY_ED25519.as_bytes())?;
    write_key_string(&mut writer, key)?;

    let signed = writer.written();
    let signature = ed25519::sign(key.secret.as_bytes(), signed);
    let fields = signed.get(start..).unwrap_or(&[]);

    let mut packet = Writer::new(out);
    packet.write_bytes(fields)?;
    write_signature(&mut packet, &signature)?;
    Ok(packet.position())
}

/// The message number and the three names every request of section 5
/// begins with.
fn write_head(writer: &mut Writer<'_>, request: &Request<'_>) -> Result<(), SshError> {
    writer.write_byte(msg::USERAUTH_REQUEST)?;
    writer.write_string(request.user.as_bytes())?;
    writer.write_string(request.service.as_bytes())?;
    writer.write_string(METHOD_PUBLICKEY.as_bytes())
}

/// The key blob as one `string`, which is how a request carries it.
fn write_key_string(writer: &mut Writer<'_>, key: &ClientKey) -> Result<(), SshError> {
    let mut blob = [0u8; BLOB_LEN];
    let len = key.write_blob(&mut blob)?;
    writer.write_string(blob.get(..len).unwrap_or(&[]))
}

/// The signature blob of RFC 8709, section 6, as one `string`.
fn write_signature(
    writer: &mut Writer<'_>,
    signature: &[u8; SIGNATURE_LEN],
) -> Result<(), SshError> {
    let mut blob = [0u8; SIGNATURE_BLOB_LEN];
    let mut inner = Writer::new(&mut blob);
    inner.write_string(HOST_KEY_ED25519.as_bytes())?;
    inner.write_string(signature)?;
    let len = inner.position();
    writer.write_string(blob.get(..len).unwrap_or(&[]))
}

/// What a server answers an authentication request with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Response<'a> {
    /// `SSH_MSG_USERAUTH_FAILURE` (section 5.1): what may still be tried,
    /// and whether the request that earned this answer did succeed as one
    /// step of several.
    ///
    /// A server that does not know the user name may answer with a list
    /// that cannot succeed, so that the answer does not say which
    /// accounts exist (section 5). A client cannot tell that from a real
    /// list and must not try.
    Failure {
        /// The methods that may continue.
        methods: NameList<'a>,
        /// Whether this step of the dialogue succeeded.
        partial: bool,
    },
    /// `SSH_MSG_USERAUTH_SUCCESS`, which ends authentication for the
    /// connection and is sent once (section 5.3).
    Success,
    /// `SSH_MSG_USERAUTH_BANNER` (section 5.4): text for the user, which
    /// may arrive at any time before success and carries whatever bytes
    /// the server put in it.
    Banner {
        /// The text, UTF-8 as the server wrote it.
        message: &'a [u8],
        /// The language tag, which may be empty.
        language: &'a [u8],
    },
    /// `SSH_MSG_USERAUTH_PK_OK`: the algorithm and the key blob of the
    /// query, sent back, which is leave to sign.
    PkOk {
        /// The algorithm name from the request.
        algorithm: &'a [u8],
        /// The key blob from the request.
        blob: &'a [u8],
    },
}

impl<'a> Response<'a> {
    /// Reads one from a packet payload.
    ///
    /// # Errors
    ///
    /// [`SshError::Message`] for a number no answer of this layer
    /// carries, [`SshError::OutOfBounds`] when the payload ends early,
    /// and [`SshError::NameList`] for a method list that is not one.
    pub fn read(payload: &'a [u8]) -> Result<Response<'a>, SshError> {
        let mut reader = Reader::new(payload);
        let number = reader.read_byte()?;
        match number {
            msg::USERAUTH_FAILURE => Ok(Response::Failure {
                methods: reader.read_name_list()?,
                partial: reader.read_boolean()?,
            }),
            msg::USERAUTH_SUCCESS => Ok(Response::Success),
            msg::USERAUTH_BANNER => Ok(Response::Banner {
                message: reader.read_string()?,
                language: reader.read_string()?,
            }),
            PK_OK => Ok(Response::PkOk {
                algorithm: reader.read_string()?,
                blob: reader.read_string()?,
            }),
            other => Err(SshError::Message(other)),
        }
    }

    /// Whether this answer is leave to sign for the key the query
    /// carried: section 7 has the server send the algorithm and the blob
    /// of the request back, and anything else is another key's answer.
    #[must_use]
    pub fn is_leave_to_sign(&self, key_blob: &[u8]) -> bool {
        matches!(
            self,
            Response::PkOk { algorithm, blob }
                if *algorithm == HOST_KEY_ED25519.as_bytes() && *blob == key_blob
        )
    }
}

/// An `SSH_MSG_EXT_INFO` (RFC 8308, section 2.3), of which this client
/// reads one extension and ignores the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExtInfo<'a> {
    /// The public key algorithms the server will accept in a `publickey`
    /// request, or nothing when the message carried no such extension.
    pub server_sig_algs: Option<NameList<'a>>,
}

impl<'a> ExtInfo<'a> {
    /// Reads one from a packet payload.
    ///
    /// An extension this client does not know is skipped whatever its
    /// value holds, null bytes included, which section 2.5 requires of
    /// every reader.
    ///
    /// # Errors
    ///
    /// [`SshError::Message`] when the payload is another message,
    /// [`SshError::OutOfBounds`] when it ends early, and
    /// [`SshError::NameList`] when `server-sig-algs` carries a list that
    /// is not one.
    pub fn read(payload: &'a [u8]) -> Result<ExtInfo<'a>, SshError> {
        let mut reader = Reader::new(payload);
        let number = reader.read_byte()?;
        if number != msg::EXT_INFO {
            return Err(SshError::Message(number));
        }
        let count = reader.read_u32()?;
        let mut server_sig_algs = None;
        for _ in 0..count {
            let name = reader.read_string()?;
            let value = reader.read_string()?;
            if name == SERVER_SIG_ALGS.as_bytes() {
                server_sig_algs = Some(NameList::new(value)?);
            }
        }
        Ok(ExtInfo { server_sig_algs })
    }
}
