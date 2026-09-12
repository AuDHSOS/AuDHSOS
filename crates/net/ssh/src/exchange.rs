// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two key exchange methods of section 14.5, their two messages, and
//! the exchange hash of RFC 4253, section 8.
//!
//! What a method is here: an ephemeral secret this side keeps, a public
//! value it sends, a shared secret it computes from the peer's, and the
//! encoding of both in the hash. The signature over that hash is step S4
//! and the keys derived from it are [`crate::keys`].
//!
//! Secrets: [`Ephemeral`] holds the private exponent or scalar and clears
//! it when it is dropped. The shared secret leaves this module in a
//! buffer the caller owns, because the key derivation needs it; that
//! buffer is the caller's to clear.

use crypto_ct::Secret;
use crypto_dh::group14;
use crypto_ec::x25519::{self, PUBLIC_LEN, SCALAR_LEN};
use crypto_hash::Sha256;
use crypto_rng::Rng;

use crate::error::SshError;
use crate::kex::{KEX_CURVE25519, KEX_GROUP14};
use crate::wire::{Reader, Writer};

/// `SSH_MSG_KEX_ECDH_INIT` (RFC 5656, section 7.1) and the
/// `SSH_MSG_KEXDH_INIT` of RFC 4253, section 8, which share this number.
///
/// RFC 4253 names its two messages and does not print their values; RFC
/// 4250, section 4.1.1, puts them in the method-specific range 30 to 49,
/// and RFC 5656, section 7.1, prints 30 and 31 for the curve method.
/// Every implementation uses the same two for the finite-field method,
/// and the interop run of step S8 is what says so.
pub const INIT: u8 = 30;

/// `SSH_MSG_KEX_ECDH_REPLY`, and the `SSH_MSG_KEXDH_REPLY` beside it.
pub const REPLY: u8 = 31;

/// The longest public value either method has, which is the 2048-bit
/// group's; the curve's is 32 bytes.
pub const MAX_PUBLIC: usize = 256;

/// The hash both methods use.
pub const HASH_LEN: usize = 32;

/// One of the two methods of section 14.5.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// `curve25519-sha256` (RFC 8731): X25519, and public values that are
    /// strings of 32 bytes.
    Curve25519,
    /// `diffie-hellman-group14-sha256` (RFC 8268 over RFC 3526, section
    /// 3): the 2048-bit MODP group, and public values that are `mpint`s.
    Group14,
}

impl Method {
    /// The method that name stands for, or nothing when this client does
    /// not have it.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Method> {
        match name {
            KEX_CURVE25519 => Some(Method::Curve25519),
            KEX_GROUP14 => Some(Method::Group14),
            _ => None,
        }
    }

    /// The name this method was negotiated under.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Method::Curve25519 => KEX_CURVE25519,
            Method::Group14 => KEX_GROUP14,
        }
    }

    /// Whether the public values go into the hash as strings. The curve
    /// method sends octet strings (RFC 5656, section 4); the finite-field
    /// method sends integers (RFC 4253, section 8).
    const fn public_is_string(self) -> bool {
        matches!(self, Method::Curve25519)
    }
}

/// This side's ephemeral key pair for one exchange.
pub struct Ephemeral {
    /// The method it belongs to.
    method: Method,
    /// The scalar or the exponent, cleared when this is dropped.
    secret: Secret<SCALAR_LEN>,
    /// The public value, as it goes on the wire and into the hash.
    public: [u8; MAX_PUBLIC],
    /// How much of `public` is the value.
    public_len: usize,
}

impl Ephemeral {
    /// A fresh key pair for `method`, with the secret drawn in one call
    /// on `rng`.
    ///
    /// # Errors
    ///
    /// [`SshError::Rng`] when the generator has nothing, and
    /// [`SshError::KeyExchangeFailed`] when the group refuses the public
    /// value this side computed — which RFC 8268, section 4, requires to
    /// be checked on the way out as well as on the way in.
    pub fn new(method: Method, rng: &mut impl Rng) -> Result<Ephemeral, SshError> {
        let mut secret = Secret::<SCALAR_LEN>::zero();
        rng.fill(secret.as_bytes_mut()).map_err(SshError::Rng)?;
        let mut public = [0u8; MAX_PUBLIC];
        let public_len = match method {
            Method::Curve25519 => {
                let value = x25519::base_point(secret.as_bytes());
                let slot = public.get_mut(..PUBLIC_LEN).unwrap_or(&mut []);
                slot.copy_from_slice(&value);
                PUBLIC_LEN
            }
            Method::Group14 => {
                let group = group14().map_err(|_| SshError::KeyExchangeFailed)?;
                let len = group.public_len();
                let slot = public.get_mut(..len).unwrap_or(&mut []);
                group
                    .public_value(secret.as_bytes(), slot)
                    .map_err(|_| SshError::KeyExchangeFailed)?;
                len
            }
        };
        Ok(Ephemeral {
            method,
            secret,
            public,
            public_len,
        })
    }

    /// The method this pair is for.
    #[must_use]
    pub const fn method(&self) -> Method {
        self.method
    }

    /// The public value to send, which is also what goes into the hash.
    #[must_use]
    pub fn public(&self) -> &[u8] {
        self.public.get(..self.public_len).unwrap_or(&[])
    }

    /// Writes the first message of the method: `SSH_MSG_KEX_ECDH_INIT`
    /// with `Q_C` as a string, or `SSH_MSG_KEXDH_INIT` with `e` as an
    /// `mpint`.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when `out` is too small.
    pub fn write_init(&self, out: &mut [u8]) -> Result<usize, SshError> {
        let mut writer = Writer::new(out);
        writer.write_byte(INIT)?;
        if self.method.public_is_string() {
            writer.write_string(self.public())?;
        } else {
            writer.write_unsigned(self.public())?;
        }
        Ok(writer.position())
    }

    /// The shared secret from the peer's public value, as unsigned
    /// big-endian bytes — which is what the exchange hash takes as an
    /// `mpint` and what the key derivation hashes.
    ///
    /// # Errors
    ///
    /// [`SshError::KeyExchangeFailed`] for a public value of the wrong
    /// length, one the group refuses, or a shared secret of all zeros:
    /// RFC 8731, section 3, and RFC 8268, section 4, each answer these
    /// with a disconnect and not with a value. [`SshError::OutOfBounds`]
    /// when `out` is shorter than the method's shared secret, which is
    /// this side's mistake and not the peer's, and so is not a key
    /// exchange that failed.
    pub fn shared_secret(&self, peer: &[u8], out: &mut [u8]) -> Result<usize, SshError> {
        match self.method {
            Method::Curve25519 => {
                let point: &[u8; PUBLIC_LEN] =
                    peer.try_into().map_err(|_| SshError::KeyExchangeFailed)?;
                let shared = x25519::x25519(self.secret.as_bytes(), point)
                    .map_err(|_| SshError::KeyExchangeFailed)?;
                let available = out.len();
                let slot = out.get_mut(..PUBLIC_LEN).ok_or(SshError::OutOfBounds {
                    needed: PUBLIC_LEN,
                    available,
                })?;
                slot.copy_from_slice(&shared);
                Ok(PUBLIC_LEN)
            }
            Method::Group14 => {
                let group = group14().map_err(|_| SshError::KeyExchangeFailed)?;
                let len = group.public_len();
                let available = out.len();
                let slot = out.get_mut(..len).ok_or(SshError::OutOfBounds {
                    needed: len,
                    available,
                })?;
                group
                    .shared_secret(self.secret.as_bytes(), peer, slot)
                    .map_err(|_| SshError::KeyExchangeFailed)?;
                Ok(len)
            }
        }
    }
}

/// The server's answer: its host key, its public value, and its signature
/// over the exchange hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Reply<'a> {
    /// `K_S`, the host key blob, which goes into the hash as it stands.
    pub host_key: &'a [u8],
    /// The server's public value, as bytes without their encoding.
    pub public: &'a [u8],
    /// The signature over the exchange hash, which step S4 checks.
    pub signature: &'a [u8],
}

impl<'a> Reply<'a> {
    /// Reads the second message of `method` from a packet payload.
    ///
    /// # Errors
    ///
    /// [`SshError::Message`] when the payload is another message,
    /// [`SshError::OutOfBounds`] when it ends early, [`SshError::Mpint`]
    /// when the finite-field value is not canonical, and
    /// [`SshError::Negative`] when it is negative, which no group element
    /// is.
    pub fn read(payload: &'a [u8], method: Method) -> Result<Reply<'a>, SshError> {
        let mut reader = Reader::new(payload);
        let number = reader.read_byte()?;
        if number != REPLY {
            return Err(SshError::Message(number));
        }
        let host_key = reader.read_string()?;
        let public = if method.public_is_string() {
            reader.read_string()?
        } else {
            reader.read_mpint()?.magnitude()?
        };
        Ok(Reply {
            host_key,
            public,
            signature: reader.read_string()?,
        })
    }
}

/// Everything the exchange hash of RFC 4253, section 8, is taken over.
#[derive(Clone, Copy, Debug)]
pub struct HashInput<'a> {
    /// `V_C`, the client's identification string without its CR LF.
    pub client_id: &'a str,
    /// `V_S`, the server's.
    pub server_id: &'a str,
    /// `I_C`, the payload of the client's `SSH_MSG_KEXINIT`.
    pub client_kexinit: &'a [u8],
    /// `I_S`, the server's.
    pub server_kexinit: &'a [u8],
    /// `K_S`, the server's host key blob.
    pub host_key: &'a [u8],
    /// The client's public value.
    pub client_public: &'a [u8],
    /// The server's public value.
    pub server_public: &'a [u8],
    /// `K`, the shared secret as unsigned big-endian bytes.
    pub shared: &'a [u8],
}

/// The exchange hash `H` of RFC 4253, section 8.
///
/// The public values are strings for the curve method (RFC 5656, section
/// 4) and `mpint`s for the finite-field one; `K` is an `mpint` for both,
/// which for the curve method is what RFC 8731, section 3.1, spells out
/// and what an implementation that hashes 32 fixed bytes instead gets
/// wrong on half of its connections.
#[must_use]
pub fn exchange_hash(method: Method, input: &HashInput<'_>) -> [u8; HASH_LEN] {
    let mut hash = Sha256::new();
    hash_string(&mut hash, input.client_id.as_bytes());
    hash_string(&mut hash, input.server_id.as_bytes());
    hash_string(&mut hash, input.client_kexinit);
    hash_string(&mut hash, input.server_kexinit);
    hash_string(&mut hash, input.host_key);
    if method.public_is_string() {
        hash_string(&mut hash, input.client_public);
        hash_string(&mut hash, input.server_public);
    } else {
        hash_mpint(&mut hash, input.client_public);
        hash_mpint(&mut hash, input.server_public);
    }
    hash_mpint(&mut hash, input.shared);
    hash.finish()
}

/// Hashes one `string` of RFC 4251, section 5: its length and its bytes.
fn hash_string(hash: &mut Sha256, value: &[u8]) {
    let len = u32::try_from(value.len()).unwrap_or(u32::MAX);
    hash.update(&len.to_be_bytes());
    hash.update(value);
}

/// Hashes one unsigned value as the `mpint` of section 5: leading zeros
/// off, and one zero byte back on when the top bit is set.
///
/// The key derivation of RFC 4253, section 7.2, hashes `K` the same way,
/// and uses this rather than a second copy of the rule: the two have to
/// agree about what `K` is, and one rule cannot disagree with itself.
pub(crate) fn hash_mpint(hash: &mut Sha256, magnitude: &[u8]) {
    let start = magnitude
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(magnitude.len());
    let value = magnitude.get(start..).unwrap_or(&[]);
    let pad = usize::from(value.first().is_some_and(|byte| byte & 0x80 != 0));
    let len = u32::try_from(value.len().saturating_add(pad)).unwrap_or(u32::MAX);
    hash.update(&len.to_be_bytes());
    if pad == 1 {
        hash.update(&[0x00]);
    }
    hash.update(value);
}
