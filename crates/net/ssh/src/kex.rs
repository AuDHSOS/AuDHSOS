// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Algorithm negotiation: `SSH_MSG_KEXINIT` of RFC 4253, section 7.1, and
//! the rule that chooses from two of them.
//!
//! The key exchange methods themselves are not here yet. What is here is
//! what both of them start with, and the one rule of the section that is
//! not "the first name both sides have": the key exchange method and the
//! host key algorithm are chosen together, because a method that needs a
//! signature from the host key cannot be run with a key that cannot sign.

use crypto_rng::Rng;

use crate::error::SshError;
use crate::msg;
use crate::wire::{NameList, Reader, Writer};

/// The random bytes each side puts at the front of its `SSH_MSG_KEXINIT`,
/// so that neither alone decides the session identifier.
pub const COOKIE_BYTES: usize = 16;

/// `curve25519-sha256` (RFC 8731).
pub const KEX_CURVE25519: &str = "curve25519-sha256";

/// `diffie-hellman-group14-sha256` (RFC 8268 over RFC 3526, section 3).
pub const KEX_GROUP14: &str = "diffie-hellman-group14-sha256";

/// What a client puts in its key exchange list to say it will read an
/// `SSH_MSG_EXT_INFO` (RFC 8308, section 2.1).
pub const EXT_INFO_C: &str = "ext-info-c";

/// The same indicator for a server, which a client never sends and never
/// negotiates.
pub const EXT_INFO_S: &str = "ext-info-s";

/// `ssh-ed25519` (RFC 8709).
pub const HOST_KEY_ED25519: &str = "ssh-ed25519";

/// `chacha20-poly1305@openssh.com` (D-134).
pub const CIPHER_CHACHA20_POLY1305: &str = "chacha20-poly1305@openssh.com";

/// No compression, which is the only one of RFC 4253, section 6.2, this
/// client has.
pub const COMPRESSION_NONE: &str = "none";

/// The lists this client offers, in the order of section 14.5 of
/// [document 14](../../../docs/14-secure-shell-as-a-client.md).
///
/// The MAC lists are empty because the one cipher offered is an AEAD that
/// carries its own integrity, so there is no MAC to name. Section 7.1
/// asks every algorithm list to hold at least one name; a list of a MAC
/// this client does not have would be the alternative, and naming
/// nothing is the truthful one.
pub const CLIENT: Proposal<'static> = Proposal {
    kex: &[KEX_CURVE25519, KEX_GROUP14, EXT_INFO_C],
    host_key: &[HOST_KEY_ED25519],
    encryption: &[CIPHER_CHACHA20_POLY1305],
    mac: &[],
    compression: &[COMPRESSION_NONE],
};

/// What one side offers. The client of this crate offers the same names
/// in both directions, which section 7.1 allows and every implementation
/// does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Proposal<'a> {
    /// Key exchange methods, most preferred first.
    pub kex: &'a [&'a str],
    /// Host key algorithms this side will accept.
    pub host_key: &'a [&'a str],
    /// Ciphers.
    pub encryption: &'a [&'a str],
    /// MAC algorithms, empty when every cipher offered is an AEAD.
    pub mac: &'a [&'a str],
    /// Compression methods.
    pub compression: &'a [&'a str],
}

impl Proposal<'_> {
    /// Writes the `SSH_MSG_KEXINIT` payload into `out` and answers with
    /// its length. The cookie is one call on `rng`.
    ///
    /// The language lists are empty, which section 7.1 asks of a side
    /// with no preference, and no guessed packet follows, which is what
    /// this client always says.
    ///
    /// # Errors
    ///
    /// [`SshError::NameList`] for a name that may not be in a list,
    /// [`SshError::OutOfBounds`] when `out` is too small, and
    /// [`SshError::Rng`] when the generator has nothing.
    pub fn write(&self, rng: &mut impl Rng, out: &mut [u8]) -> Result<usize, SshError> {
        let mut writer = Writer::new(out);
        writer.write_byte(msg::KEXINIT)?;
        rng.fill(writer.take(COOKIE_BYTES)?)
            .map_err(SshError::Rng)?;
        writer.write_name_list(self.kex)?;
        writer.write_name_list(self.host_key)?;
        writer.write_name_list(self.encryption)?;
        writer.write_name_list(self.encryption)?;
        writer.write_name_list(self.mac)?;
        writer.write_name_list(self.mac)?;
        writer.write_name_list(self.compression)?;
        writer.write_name_list(self.compression)?;
        writer.write_name_list(&[])?;
        writer.write_name_list(&[])?;
        writer.write_boolean(false)?;
        writer.write_u32(0)?;
        Ok(writer.position())
    }
}

/// A `SSH_MSG_KEXINIT` as it arrived, borrowing the payload it was read
/// from — which the caller keeps, because the whole payload goes into the
/// exchange hash of section 8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KexInit<'a> {
    /// The peer's sixteen random bytes.
    pub cookie: &'a [u8],
    /// Key exchange methods, most preferred first.
    pub kex: NameList<'a>,
    /// Host key algorithms the peer has keys for.
    pub host_key: NameList<'a>,
    /// Ciphers for what the client sends.
    pub encryption_c2s: NameList<'a>,
    /// Ciphers for what the server sends.
    pub encryption_s2c: NameList<'a>,
    /// MAC algorithms for what the client sends.
    pub mac_c2s: NameList<'a>,
    /// MAC algorithms for what the server sends.
    pub mac_s2c: NameList<'a>,
    /// Compression for what the client sends.
    pub compression_c2s: NameList<'a>,
    /// Compression for what the server sends.
    pub compression_s2c: NameList<'a>,
    /// Language tags for what the client sends.
    pub languages_c2s: NameList<'a>,
    /// Language tags for what the server sends.
    pub languages_s2c: NameList<'a>,
    /// Whether a guessed key exchange packet follows this one.
    pub first_kex_packet_follows: bool,
}

impl<'a> KexInit<'a> {
    /// Reads one from a packet payload.
    ///
    /// The reserved `uint32` at the end is read and dropped: section 7.1
    /// reserves it for a future extension, so a value other than zero is
    /// that future and not an error.
    ///
    /// # Errors
    ///
    /// [`SshError::Message`] when the payload is another message,
    /// [`SshError::OutOfBounds`] when it ends early, and
    /// [`SshError::NameList`] for a list that is not one.
    pub fn read(payload: &'a [u8]) -> Result<KexInit<'a>, SshError> {
        let mut reader = Reader::new(payload);
        let number = reader.read_byte()?;
        if number != msg::KEXINIT {
            return Err(SshError::Message(number));
        }
        let kexinit = KexInit {
            cookie: reader.read_bytes(COOKIE_BYTES)?,
            kex: reader.read_name_list()?,
            host_key: reader.read_name_list()?,
            encryption_c2s: reader.read_name_list()?,
            encryption_s2c: reader.read_name_list()?,
            mac_c2s: reader.read_name_list()?,
            mac_s2c: reader.read_name_list()?,
            compression_c2s: reader.read_name_list()?,
            compression_s2c: reader.read_name_list()?,
            languages_c2s: reader.read_name_list()?,
            languages_s2c: reader.read_name_list()?,
            first_kex_packet_follows: reader.read_boolean()?,
        };
        reader.read_u32()?;
        Ok(kexinit)
    }
}

/// What the two lists agreed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Choice<'a> {
    /// The key exchange method to run.
    pub kex: &'a str,
    /// The host key algorithm its signature is under.
    pub host_key: &'a str,
    /// The cipher for what the client sends.
    pub encryption_c2s: &'a str,
    /// The cipher for what the server sends.
    pub encryption_s2c: &'a str,
    /// The MAC for what the client sends, or nothing when the cipher
    /// carries its own integrity.
    pub mac_c2s: Option<&'a str>,
    /// The MAC for what the server sends, under the same rule.
    pub mac_s2c: Option<&'a str>,
    /// The compression for what the client sends.
    pub compression_c2s: &'a str,
    /// The compression for what the server sends.
    pub compression_s2c: &'a str,
}

/// The list a negotiation failed on, which is what the disconnect says.
const KEY_EXCHANGE: &str = "key exchange";
/// The host key list.
const HOST_KEY: &str = "host key";
/// A cipher list.
const CIPHER: &str = "cipher";
/// A MAC list.
const MAC: &str = "MAC";
/// A compression list.
const COMPRESSION: &str = "compression";

/// Chooses the algorithms of one connection from what each side offered,
/// by the rule of RFC 4253, section 7.1.
///
/// For the ciphers, the MACs and the compression the rule is the first
/// name on the client's list that the server also has. For the key
/// exchange it is the first method the server also has *and* for which a
/// host key algorithm both sides have satisfies what that method needs,
/// so those two are decided together and not one after the other.
///
/// # Errors
///
/// [`SshError::Negotiation`] naming the list that had nothing in common.
/// An `ext-info-c` or `ext-info-s` that ends up chosen is that error as
/// well: RFC 8308, section 2.2, says the parties disconnect, and the
/// indicator is not a method anybody could run.
pub fn negotiate<'a>(client: &Proposal<'a>, server: &KexInit<'_>) -> Result<Choice<'a>, SshError> {
    if !client.kex.iter().any(|name| server.kex.contains(name)) {
        return Err(SshError::Negotiation(KEY_EXCHANGE));
    }
    // A method both sides have but no host key to run it with is the
    // other half of the same rule, and says so.
    let (kex, host_key) = client
        .kex
        .iter()
        .copied()
        .filter(|name| server.kex.contains(name))
        .find_map(|name| host_key_for(client, server, name).map(|key| (name, key)))
        .ok_or(SshError::Negotiation(HOST_KEY))?;
    if kex == EXT_INFO_C || kex == EXT_INFO_S {
        return Err(SshError::Negotiation(KEY_EXCHANGE));
    }
    let encryption_c2s = first_common(client.encryption, server.encryption_c2s)
        .ok_or(SshError::Negotiation(CIPHER))?;
    let encryption_s2c = first_common(client.encryption, server.encryption_s2c)
        .ok_or(SshError::Negotiation(CIPHER))?;
    Ok(Choice {
        kex,
        host_key,
        encryption_c2s,
        encryption_s2c,
        mac_c2s: mac(client, server.mac_c2s, encryption_c2s)?,
        mac_s2c: mac(client, server.mac_s2c, encryption_s2c)?,
        compression_c2s: first_common(client.compression, server.compression_c2s)
            .ok_or(SshError::Negotiation(COMPRESSION))?,
        compression_s2c: first_common(client.compression, server.compression_s2c)
            .ok_or(SshError::Negotiation(COMPRESSION))?,
    })
}

/// Whether a guessed packet the peer announced has to be ignored: it does
/// when the peer guessed and the guess is not what was chosen (RFC 4253,
/// section 7.1). The guess is the first name of each of its two lists.
#[must_use]
pub fn guess_is_wrong(server: &KexInit<'_>, choice: &Choice<'_>) -> bool {
    server.first_kex_packet_follows
        && (server.kex.iter().next() != Some(choice.kex)
            || server.host_key.iter().next() != Some(choice.host_key))
}

/// The first name of `client` that `server` also has.
fn first_common<'a>(client: &[&'a str], server: NameList<'_>) -> Option<&'a str> {
    client.iter().copied().find(|name| server.contains(name))
}

/// The host key algorithm for `kex`: the first one the client offers that
/// the server has a key for and that the method can be run with.
fn host_key_for<'a>(client: &Proposal<'a>, server: &KexInit<'_>, kex: &str) -> Option<&'a str> {
    client
        .host_key
        .iter()
        .copied()
        .find(|name| server.host_key.contains(name) && satisfies(kex, name))
}

/// Whether `host_key` is what `kex` needs of a host key. Both methods of
/// 14.5 need a signature over the exchange hash, and no method this
/// client offers needs an encryption-capable key.
fn satisfies(kex: &str, host_key: &str) -> bool {
    let _ = kex;
    signature_capable(host_key)
}

/// Whether a signature can be made and checked under this algorithm.
/// A name this client does not know is not one it can check a signature
/// with, whatever the server has a key for.
fn signature_capable(host_key: &str) -> bool {
    host_key == HOST_KEY_ED25519
}

/// The MAC for one direction: none at all when the cipher is an AEAD, and
/// otherwise the first name both sides have.
fn mac<'a>(
    client: &Proposal<'a>,
    server: NameList<'_>,
    cipher: &str,
) -> Result<Option<&'a str>, SshError> {
    if !needs_mac(cipher) {
        return Ok(None);
    }
    first_common(client.mac, server)
        .map(Some)
        .ok_or(SshError::Negotiation(MAC))
}

/// Whether a cipher needs a MAC beside it. The AEAD of D-134 does not,
/// and its own specification says the MAC lists are then ignored.
fn needs_mac(cipher: &str) -> bool {
    cipher != CIPHER_CHACHA20_POLY1305
}
