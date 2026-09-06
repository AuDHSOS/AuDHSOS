// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The handshake messages a client sends and reads.
//!
//! Only the ones this client needs exist. A message it never sends is not
//! written, and a message it may receive but has no use for is skipped by
//! its length rather than parsed into fields nobody reads.
//!
//! Invariants: every message borrows from the bytes it was read out of;
//! reading a message consumes exactly the length its header declares; an
//! extension that appears twice is refused, because a peer that says a
//! thing twice may be saying two different things.

use crate::codec::{Reader, Writer};
use crate::error::TlsError;
use crate::suite::CipherSuite;

/// Bytes of handshake header: one of type, three of length.
pub const HEADER_LEN: usize = 4;

/// The version every `ClientHello` carries in its legacy field.
const LEGACY_VERSION: u16 = 0x0303;
/// The version this client speaks, as the extension names it.
pub const VERSION_TLS13: u16 = 0x0304;

/// The group this client offers.
pub const GROUP_X25519: u16 = 0x001D;

/// `ecdsa_secp256r1_sha256`: P-256 and SHA-256, and no other pair.
pub const ECDSA_SECP256R1_SHA256: u16 = 0x0403;
/// `ecdsa_secp384r1_sha384`: P-384 and SHA-384, and no other pair.
///
/// RFC 8446 section 4.2.3 names the curve in an ECDSA scheme as firmly as
/// it names the hash. There is no `ecdsa_secp256r1_sha384`; a P-256 key
/// signing with SHA-384 has no code point here, whatever X.509 allows a
/// certificate to do.
pub const ECDSA_SECP384R1_SHA384: u16 = 0x0503;
/// `ed25519`.
pub const ED25519: u16 = 0x0807;
/// `rsa_pss_rsae_sha256`: PSS over a key an `rsaEncryption` certificate
/// carries, with MGF1 over SHA-256 and a salt of thirty-two bytes.
pub const RSA_PSS_RSAE_SHA256: u16 = 0x0804;
/// `rsa_pss_rsae_sha384`.
pub const RSA_PSS_RSAE_SHA384: u16 = 0x0805;
/// `rsa_pss_rsae_sha512`.
pub const RSA_PSS_RSAE_SHA512: u16 = 0x0806;
/// `rsa_pkcs1_sha256`.
///
/// RFC 8446 section 4.2.3 gives this code point and its two siblings
/// exactly one meaning: the client can verify a *certificate* signed that
/// way. The same section forbids them in a `CertificateVerify`. So the
/// three are offered in the `ClientHello` and refused in the handshake
/// signature, and that is not a contradiction — the offer is about the
/// chain, the refusal is about the signature the server makes itself
/// (D-82). A client that stayed silent about them would be telling a
/// server its chain may not be signed the way nearly every chain is
/// signed.
pub const RSA_PKCS1_SHA256: u16 = 0x0401;
/// `rsa_pkcs1_sha384`. Offered for a chain, refused in a
/// `CertificateVerify`; see [`RSA_PKCS1_SHA256`].
pub const RSA_PKCS1_SHA384: u16 = 0x0501;
/// `rsa_pkcs1_sha512`. Offered for a chain, refused in a
/// `CertificateVerify`; see [`RSA_PKCS1_SHA256`].
pub const RSA_PKCS1_SHA512: u16 = 0x0601;

/// The random a server sends when it means to retry rather than agree.
pub const RETRY_RANDOM: [u8; 32] = [
    0xCF, 0x21, 0xAD, 0x74, 0xE5, 0x9A, 0x61, 0x11, 0xBE, 0x1D, 0x8C, 0x02, 0x1E, 0x65, 0xB8, 0x91,
    0xC2, 0xA2, 0x11, 0x16, 0x7A, 0xBB, 0x8C, 0x5E, 0x07, 0x9E, 0x09, 0xE2, 0xC8, 0xA8, 0x33, 0x9C,
];

/// The last eight bytes of the random a TLS 1.2 server sends to say it
/// would have spoken 1.3 if it could, which a 1.3 client must treat as an
/// attack.
pub const DOWNGRADE_TLS12: [u8; 8] = [0x44, 0x4F, 0x57, 0x4E, 0x47, 0x52, 0x44, 0x01];
/// The same for a server that would have spoken 1.1 or below.
pub const DOWNGRADE_TLS11: [u8; 8] = [0x44, 0x4F, 0x57, 0x4E, 0x47, 0x52, 0x44, 0x00];

/// The extensions this client reads or writes.
mod extension {
    /// `server_name`.
    pub(super) const SERVER_NAME: u16 = 0;
    /// `supported_groups`.
    pub(super) const SUPPORTED_GROUPS: u16 = 10;
    /// `signature_algorithms`.
    pub(super) const SIGNATURE_ALGORITHMS: u16 = 13;
    /// `application_layer_protocol_negotiation`.
    pub(super) const ALPN: u16 = 16;
    /// `supported_versions`.
    pub(super) const SUPPORTED_VERSIONS: u16 = 43;
    /// `key_share`.
    pub(super) const KEY_SHARE: u16 = 51;
}

/// What kind of handshake message this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HandshakeType {
    /// The client's first message.
    ClientHello,
    /// The server's answer, or its retry.
    ServerHello,
    /// A ticket for a session this client will not resume.
    NewSessionTicket,
    /// The extensions that did not fit in the `ServerHello`.
    EncryptedExtensions,
    /// The certificate chain.
    Certificate,
    /// The signature over the handshake so far.
    CertificateVerify,
    /// The code that closes one side of the handshake.
    Finished,
    /// A request to move to the next traffic key.
    KeyUpdate,
}

impl HandshakeType {
    /// The type a byte names.
    ///
    /// # Errors
    ///
    /// [`TlsError::UnexpectedMessage`] for a type this client never
    /// handles, which includes every message of a server or of a version
    /// this is not.
    pub const fn from_byte(byte: u8) -> Result<HandshakeType, TlsError> {
        match byte {
            1 => Ok(HandshakeType::ClientHello),
            2 => Ok(HandshakeType::ServerHello),
            4 => Ok(HandshakeType::NewSessionTicket),
            8 => Ok(HandshakeType::EncryptedExtensions),
            11 => Ok(HandshakeType::Certificate),
            15 => Ok(HandshakeType::CertificateVerify),
            20 => Ok(HandshakeType::Finished),
            24 => Ok(HandshakeType::KeyUpdate),
            _ => Err(TlsError::UnexpectedMessage),
        }
    }

    /// The byte that names the type.
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            HandshakeType::ClientHello => 1,
            HandshakeType::ServerHello => 2,
            HandshakeType::NewSessionTicket => 4,
            HandshakeType::EncryptedExtensions => 8,
            HandshakeType::Certificate => 11,
            HandshakeType::CertificateVerify => 15,
            HandshakeType::Finished => 20,
            HandshakeType::KeyUpdate => 24,
        }
    }
}

/// A message at the front of a buffer: its type, its body, and how many
/// bytes it occupied, header included.
pub type Framed<'a> = (HandshakeType, &'a [u8], usize);

/// One message at the front of `bytes`. Nothing, when it has not arrived
/// in full.
///
/// # Errors
///
/// [`TlsError::UnexpectedMessage`] for a type this client never handles.
pub fn read_message(bytes: &[u8]) -> Result<Option<Framed<'_>>, TlsError> {
    let mut reader = Reader::new(bytes);
    if reader.left() < HEADER_LEN {
        return Ok(None);
    }
    let kind = HandshakeType::from_byte(reader.u8()?)?;
    let length = reader.u24()?;
    if reader.left() < length {
        return Ok(None);
    }
    let body = reader.take(length)?;
    Ok(Some((kind, body, HEADER_LEN.wrapping_add(length))))
}

/// What a client puts in its first message.
#[derive(Clone, Copy, Debug)]
pub struct ClientHelloParams<'a> {
    /// The thirty-two random bytes.
    pub random: &'a [u8; 32],
    /// The session identifier, which carries no session and exists so that
    /// a middlebox sees what it expects.
    pub session_id: &'a [u8],
    /// The suites, in the order they are offered.
    pub suites: &'a [CipherSuite],
    /// The public value of the key exchange.
    pub key_share: &'a [u8; 32],
    /// The name of the server, if it has one.
    pub server_name: Option<&'a str>,
    /// The protocols to offer, if any.
    pub alpn: &'a [&'a [u8]],
}

/// Writes a `ClientHello`, header included, and returns its length.
///
/// # Errors
///
/// [`TlsError::BufferTooSmall`] when it does not fit, and
/// [`TlsError::Encode`] when a field is longer than its length can say.
pub fn write_client_hello(
    params: &ClientHelloParams<'_>,
    out: &mut [u8],
) -> Result<usize, TlsError> {
    let mut writer = Writer::new(out);
    writer.u8(HandshakeType::ClientHello.to_byte())?;
    writer.vector24(|body| {
        body.u16(LEGACY_VERSION)?;
        body.bytes(params.random)?;
        body.vector8(|id| id.bytes(params.session_id))?;
        body.vector16(|suites| {
            for suite in params.suites {
                suites.u16(suite.code())?;
            }
            Ok(())
        })?;
        // legacy_compression_methods: the null method and nothing else.
        body.vector8(|methods| methods.u8(0))?;
        body.vector16(|extensions| write_client_extensions(params, extensions))
    })?;
    Ok(writer.len())
}

/// The extensions of a `ClientHello`.
fn write_client_extensions(
    params: &ClientHelloParams<'_>,
    out: &mut Writer<'_>,
) -> Result<(), TlsError> {
    if let Some(name) = params.server_name {
        out.u16(extension::SERVER_NAME)?;
        out.vector16(|body| {
            body.vector16(|list| {
                list.u8(0)?; // host_name
                list.vector16(|host| host.bytes(name.as_bytes()))
            })
        })?;
    }

    out.u16(extension::SUPPORTED_GROUPS)?;
    out.vector16(|body| body.vector16(|groups| groups.u16(GROUP_X25519)))?;

    out.u16(extension::SIGNATURE_ALGORITHMS)?;
    out.vector16(|body| {
        body.vector16(|schemes| {
            schemes.u16(ECDSA_SECP256R1_SHA256)?;
            schemes.u16(ECDSA_SECP384R1_SHA384)?;
            schemes.u16(ED25519)?;
            schemes.u16(RSA_PSS_RSAE_SHA256)?;
            schemes.u16(RSA_PSS_RSAE_SHA384)?;
            schemes.u16(RSA_PSS_RSAE_SHA512)?;
            schemes.u16(RSA_PKCS1_SHA256)?;
            schemes.u16(RSA_PKCS1_SHA384)?;
            schemes.u16(RSA_PKCS1_SHA512)
        })
    })?;

    if !params.alpn.is_empty() {
        out.u16(extension::ALPN)?;
        out.vector16(|body| {
            body.vector16(|list| {
                for protocol in params.alpn {
                    list.vector8(|name| name.bytes(protocol))?;
                }
                Ok(())
            })
        })?;
    }

    out.u16(extension::SUPPORTED_VERSIONS)?;
    out.vector16(|body| body.vector8(|versions| versions.u16(VERSION_TLS13)))?;

    out.u16(extension::KEY_SHARE)?;
    out.vector16(|body| {
        body.vector16(|shares| {
            shares.u16(GROUP_X25519)?;
            shares.vector16(|value| value.bytes(params.key_share))
        })
    })
}

/// What a server said in its answer.
#[derive(Clone, Copy, Debug)]
pub struct ServerHello<'a> {
    /// The thirty-two bytes of random, which say whether this is a retry.
    pub random: &'a [u8],
    /// The session identifier, which must be the one that was sent.
    pub session_id: &'a [u8],
    /// The suite the server chose.
    pub suite: CipherSuite,
    /// The public value, when the server agreed.
    pub key_share: Option<&'a [u8]>,
    /// The group, when the server asked for a retry.
    pub retry_group: Option<u16>,
    /// Whether this is a retry rather than an agreement.
    pub is_retry: bool,
}

impl<'a> ServerHello<'a> {
    /// Reads a `ServerHello` from the body of the message.
    ///
    /// # Errors
    ///
    /// [`TlsError::Decode`] for a malformed message,
    /// [`TlsError::UnsupportedSuite`] for a suite that was not offered,
    /// [`TlsError::UnsupportedVersion`] when the version extension is
    /// absent or names another version, [`TlsError::MissingExtension`]
    /// when the key share is absent, and
    /// [`TlsError::UnexpectedExtension`] when one repeats.
    pub fn parse(body: &'a [u8]) -> Result<ServerHello<'a>, TlsError> {
        let mut reader = Reader::new(body);
        let _legacy = reader.u16()?;
        let random = reader.take(32)?;
        let session_id = reader.vector8()?;
        let suite = CipherSuite::from_code(reader.u16()?).ok_or(TlsError::UnsupportedSuite)?;
        if reader.u8()? != 0 {
            return Err(TlsError::IllegalParameter);
        }

        let is_retry = random == RETRY_RANDOM;
        let mut version = None;
        let mut key_share = None;
        let mut retry_group = None;

        let mut extensions = Reader::new(reader.vector16()?);
        reader.finish()?;
        while !extensions.is_empty() {
            let kind = extensions.u16()?;
            let body = extensions.vector16()?;
            let mut value = Reader::new(body);
            match kind {
                extension::SUPPORTED_VERSIONS => {
                    if version.is_some() {
                        return Err(TlsError::UnexpectedExtension);
                    }
                    version = Some(value.u16()?);
                    value.finish()?;
                }
                extension::KEY_SHARE if is_retry => {
                    if retry_group.is_some() {
                        return Err(TlsError::UnexpectedExtension);
                    }
                    retry_group = Some(value.u16()?);
                    value.finish()?;
                }
                extension::KEY_SHARE => {
                    if key_share.is_some() {
                        return Err(TlsError::UnexpectedExtension);
                    }
                    if value.u16()? != GROUP_X25519 {
                        return Err(TlsError::IllegalParameter);
                    }
                    key_share = Some(value.vector16()?);
                    value.finish()?;
                }
                _ => return Err(TlsError::UnexpectedExtension),
            }
        }

        if version != Some(VERSION_TLS13) {
            return Err(TlsError::UnsupportedVersion);
        }
        if !is_retry && key_share.is_none() {
            return Err(TlsError::MissingExtension);
        }
        if is_retry && retry_group.is_none() {
            return Err(TlsError::MissingExtension);
        }

        Ok(ServerHello {
            random,
            session_id,
            suite,
            key_share,
            retry_group,
            is_retry,
        })
    }

    /// Whether the random ends in one of the two values that say the
    /// server would have negotiated an older version.
    #[must_use]
    pub fn is_downgrade(&self) -> bool {
        self.random
            .last_chunk::<8>()
            .is_some_and(|tail| *tail == DOWNGRADE_TLS12 || *tail == DOWNGRADE_TLS11)
    }
}

/// What the server said once the handshake was encrypted.
#[derive(Clone, Copy, Debug)]
pub struct EncryptedExtensions<'a> {
    /// The protocol the server chose, when it chose one.
    pub alpn: Option<&'a [u8]>,
}

impl<'a> EncryptedExtensions<'a> {
    /// Reads the message.
    ///
    /// Extensions this client did not offer are refused; extensions it
    /// offered and does not need to read are skipped.
    ///
    /// # Errors
    ///
    /// [`TlsError::Decode`] for a malformed message and
    /// [`TlsError::UnexpectedExtension`] for one that repeats.
    pub fn parse(body: &'a [u8]) -> Result<EncryptedExtensions<'a>, TlsError> {
        let mut reader = Reader::new(body);
        let mut extensions = Reader::new(reader.vector16()?);
        reader.finish()?;

        let mut alpn = None;
        while !extensions.is_empty() {
            let kind = extensions.u16()?;
            let body = extensions.vector16()?;
            if kind == extension::ALPN {
                if alpn.is_some() {
                    return Err(TlsError::UnexpectedExtension);
                }
                let mut value = Reader::new(body);
                let mut list = Reader::new(value.vector16()?);
                value.finish()?;
                alpn = Some(list.vector8()?);
                list.finish()?;
            }
        }
        Ok(EncryptedExtensions { alpn })
    }
}

/// The certificates a server sent.
#[derive(Clone, Copy, Debug)]
pub struct CertificateChain<'a> {
    /// The entries, still encoded.
    entries: &'a [u8],
}

impl<'a> CertificateChain<'a> {
    /// Reads the message.
    ///
    /// # Errors
    ///
    /// [`TlsError::Decode`] for a malformed message, and
    /// [`TlsError::IllegalParameter`] when the request context is not
    /// empty, which it must be for a certificate a server sends
    /// unprompted.
    pub fn parse(body: &'a [u8]) -> Result<CertificateChain<'a>, TlsError> {
        let mut reader = Reader::new(body);
        if !reader.vector8()?.is_empty() {
            return Err(TlsError::IllegalParameter);
        }
        let entries = reader.vector24()?;
        reader.finish()?;
        Ok(CertificateChain { entries })
    }

    /// The certificates, in the order the server sent them.
    #[must_use]
    pub const fn certificates(&self) -> Certificates<'a> {
        Certificates {
            reader: Reader::new(self.entries),
        }
    }
}

/// The certificates of a chain.
pub struct Certificates<'a> {
    /// What has not been read.
    reader: Reader<'a>,
}

impl<'a> Iterator for Certificates<'a> {
    type Item = Result<&'a [u8], TlsError>;

    fn next(&mut self) -> Option<Result<&'a [u8], TlsError>> {
        if self.reader.is_empty() {
            return None;
        }
        let certificate = match self.reader.vector24() {
            Ok(bytes) => bytes,
            Err(error) => return Some(Err(error)),
        };
        // Each entry carries extensions this client has no use for.
        match self.reader.vector16() {
            Ok(_) => Some(Ok(certificate)),
            Err(error) => Some(Err(error)),
        }
    }
}

/// The signature over the handshake.
#[derive(Clone, Copy, Debug)]
pub struct CertificateVerify<'a> {
    /// The scheme the server signed with.
    pub scheme: u16,
    /// The signature.
    pub signature: &'a [u8],
}

impl<'a> CertificateVerify<'a> {
    /// Reads the message.
    ///
    /// # Errors
    ///
    /// [`TlsError::Decode`] for a malformed message.
    pub fn parse(body: &'a [u8]) -> Result<CertificateVerify<'a>, TlsError> {
        let mut reader = Reader::new(body);
        let scheme = reader.u16()?;
        let signature = reader.vector16()?;
        reader.finish()?;
        Ok(CertificateVerify { scheme, signature })
    }
}

/// Writes a `Finished` message, header included, and returns its length.
///
/// # Errors
///
/// [`TlsError::BufferTooSmall`] when it does not fit.
pub fn write_finished(verify_data: &[u8], out: &mut [u8]) -> Result<usize, TlsError> {
    let mut writer = Writer::new(out);
    writer.u8(HandshakeType::Finished.to_byte())?;
    writer.vector24(|body| body.bytes(verify_data))?;
    Ok(writer.len())
}

/// Writes a `KeyUpdate`, header included, and returns its length.
///
/// # Errors
///
/// [`TlsError::BufferTooSmall`] when it does not fit.
pub fn write_key_update(request_update: bool, out: &mut [u8]) -> Result<usize, TlsError> {
    let mut writer = Writer::new(out);
    writer.u8(HandshakeType::KeyUpdate.to_byte())?;
    writer.vector24(|body| body.u8(u8::from(request_update)))?;
    Ok(writer.len())
}

/// Whether a `KeyUpdate` asks for one in return.
///
/// # Errors
///
/// [`TlsError::Decode`] for a body that is not one byte, and
/// [`TlsError::IllegalParameter`] for a byte that is neither value.
pub fn read_key_update(body: &[u8]) -> Result<bool, TlsError> {
    let mut reader = Reader::new(body);
    let value = reader.u8()?;
    reader.finish()?;
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(TlsError::IllegalParameter),
    }
}
