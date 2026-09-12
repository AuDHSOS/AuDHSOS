// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `SSH_MSG_KEXINIT` and the rule of RFC 4253, section 7.1, that chooses
//! from two of them.

use crypto_rng::doubles::ScriptedRng;

use crate::error::SshError;
use crate::kex::{
    self, CIPHER_CHACHA20_POLY1305, COMPRESSION_NONE, COOKIE_BYTES, Choice, EXT_INFO_C, EXT_INFO_S,
    HOST_KEY_ED25519, KEX_CURVE25519, KEX_GROUP14, KexInit, Proposal,
};
use crate::msg;
use crate::wire::Reader;

/// Bytes for the cookie, which is all the generator is asked for here.
const COOKIE: [u8; 32] = [0x5a; 32];

/// A server's message, built from the lists it would offer.
fn server(
    kex: &[&str],
    host_key: &[&str],
    encryption: &[&str],
    mac: &[&str],
    compression: &[&str],
    guess: bool,
) -> Vec<u8> {
    let proposal = Proposal {
        kex,
        host_key,
        encryption,
        mac,
        compression,
    };
    let mut out = vec![0u8; 1024];
    let mut rng = ScriptedRng::new(&COOKIE);
    let len = proposal.write(&mut rng, &mut out).unwrap_or_default();
    assert!(len > 0, "the server message is written");
    out.truncate(len);
    if guess {
        // The guess flag is the second-to-last byte, before the reserved
        // uint32 that ends the message.
        let flag = out.len().saturating_sub(5);
        if let Some(slot) = out.get_mut(flag) {
            *slot = 1;
        }
    }
    out
}

#[test]
fn the_message_is_the_fields_section_7_1_lists_in_that_order() {
    let mut out = [0u8; 512];
    let mut rng = ScriptedRng::new(&COOKIE);
    let len = kex::CLIENT.write(&mut rng, &mut out).unwrap_or_default();
    let payload = out.get(..len).unwrap_or(&[]);

    let mut reader = Reader::new(payload);
    assert_eq!(reader.read_byte(), Ok(msg::KEXINIT));
    assert_eq!(
        reader.read_bytes(COOKIE_BYTES),
        Ok(COOKIE.get(..COOKIE_BYTES).unwrap_or(&[]))
    );
    for expected in [
        "curve25519-sha256,diffie-hellman-group14-sha256,ext-info-c",
        "ssh-ed25519",
        "chacha20-poly1305@openssh.com",
        "chacha20-poly1305@openssh.com",
        "",
        "",
        "none",
        "none",
        "",
        "",
    ] {
        assert_eq!(
            reader.read_name_list().map(|list| list.as_str()),
            Ok(expected)
        );
    }
    // No guessed packet follows, and the reserved word is zero.
    assert_eq!(reader.read_boolean(), Ok(false));
    assert_eq!(reader.read_u32(), Ok(0));
    assert!(reader.is_empty());
}

#[test]
fn the_cookie_is_one_call_on_the_generator() {
    let mut out = [0u8; 512];
    let mut rng = ScriptedRng::new(&COOKIE);
    assert!(kex::CLIENT.write(&mut rng, &mut out).is_ok());
    assert_eq!(COOKIE.len().saturating_sub(rng.left()), COOKIE_BYTES);
}

#[test]
fn what_was_written_reads_back_as_the_lists_that_were_offered() {
    let mut out = [0u8; 512];
    let mut rng = ScriptedRng::new(&COOKIE);
    let len = kex::CLIENT.write(&mut rng, &mut out).unwrap_or_default();
    let read = KexInit::read(out.get(..len).unwrap_or(&[]));
    assert_eq!(
        read.map(|kexinit| (
            kexinit.cookie,
            kexinit.kex.contains(KEX_CURVE25519),
            kexinit.kex.contains(EXT_INFO_C),
            kexinit.host_key.contains(HOST_KEY_ED25519),
            kexinit.encryption_c2s.contains(CIPHER_CHACHA20_POLY1305),
            kexinit.mac_c2s.is_empty(),
            kexinit.compression_s2c.contains(COMPRESSION_NONE),
            kexinit.languages_c2s.is_empty(),
            kexinit.first_kex_packet_follows,
        )),
        Ok((
            COOKIE.get(..COOKIE_BYTES).unwrap_or(&[]),
            true,
            true,
            true,
            true,
            true,
            true,
            true,
            false
        ))
    );
}

#[test]
fn another_message_and_a_message_that_ends_early_are_refused() {
    assert_eq!(KexInit::read(&[msg::NEWKEYS]), Err(SshError::Message(21)));
    assert_eq!(
        KexInit::read(&[]),
        Err(SshError::OutOfBounds {
            needed: 1,
            available: 0
        })
    );
    let mut out = [0u8; 512];
    let mut rng = ScriptedRng::new(&COOKIE);
    let len = kex::CLIENT.write(&mut rng, &mut out).unwrap_or_default();
    assert!(len > COOKIE_BYTES);
    for short in [1, COOKIE_BYTES, len.saturating_sub(1)] {
        assert!(matches!(
            KexInit::read(out.get(..short).unwrap_or(&[])),
            Err(SshError::OutOfBounds { .. })
        ));
    }
}

#[test]
fn a_server_that_offers_what_this_client_does_agrees_with_it() {
    let message = server(
        &["curve25519-sha256", "diffie-hellman-group14-sha256"],
        &["rsa-sha2-512", "ssh-ed25519"],
        &["aes128-ctr", "chacha20-poly1305@openssh.com"],
        &["hmac-sha2-256"],
        &["none", "zlib@openssh.com"],
        false,
    );
    assert_eq!(
        KexInit::read(&message).and_then(|kexinit| kex::negotiate(&kex::CLIENT, &kexinit)),
        Ok(Choice {
            kex: KEX_CURVE25519,
            host_key: HOST_KEY_ED25519,
            encryption_c2s: CIPHER_CHACHA20_POLY1305,
            encryption_s2c: CIPHER_CHACHA20_POLY1305,
            mac_c2s: None,
            mac_s2c: None,
            compression_c2s: COMPRESSION_NONE,
            compression_s2c: COMPRESSION_NONE,
        })
    );
}

#[test]
fn the_clients_order_decides_and_not_the_servers() {
    // Section 7.1: the first name on the client's list that the server
    // also has, whatever the server prefers.
    let message = server(
        &["diffie-hellman-group14-sha256", "curve25519-sha256"],
        &["ssh-ed25519"],
        &["chacha20-poly1305@openssh.com"],
        &[],
        &["none"],
        false,
    );
    assert_eq!(
        KexInit::read(&message)
            .and_then(|kexinit| kex::negotiate(&kex::CLIENT, &kexinit))
            .map(|choice| choice.kex),
        Ok(KEX_CURVE25519)
    );

    // The one method both sides have is the one that is run.
    let message = server(
        &["diffie-hellman-group14-sha256"],
        &["ssh-ed25519"],
        &["chacha20-poly1305@openssh.com"],
        &[],
        &["none"],
        false,
    );
    assert_eq!(
        KexInit::read(&message)
            .and_then(|kexinit| kex::negotiate(&kex::CLIENT, &kexinit))
            .map(|choice| choice.kex),
        Ok(KEX_GROUP14)
    );
}

#[test]
fn a_method_with_no_host_key_to_run_it_with_is_not_chosen() {
    // The server has the method and a key, but not a key this client can
    // check a signature with: the two are chosen together, so this is a
    // failed negotiation and not a chosen method with no key.
    let message = server(
        &["curve25519-sha256"],
        &["ssh-rsa", "ssh-dss"],
        &["chacha20-poly1305@openssh.com"],
        &[],
        &["none"],
        false,
    );
    assert_eq!(
        KexInit::read(&message).and_then(|kexinit| kex::negotiate(&kex::CLIENT, &kexinit)),
        Err(SshError::Negotiation("host key"))
    );
}

#[test]
fn nothing_in_common_in_any_list_ends_the_connection() {
    /// The lists a server offers, and the one the negotiation stops at.
    type Case = (
        &'static [&'static str],
        &'static [&'static str],
        &'static [&'static str],
        &'static str,
    );

    let cases: [Case; 3] = [
        (
            &["diffie-hellman-group1-sha1"],
            &["chacha20-poly1305@openssh.com"],
            &["none"],
            "key exchange",
        ),
        (&["curve25519-sha256"], &["aes128-ctr"], &["none"], "cipher"),
        (
            &["curve25519-sha256"],
            &["chacha20-poly1305@openssh.com"],
            &["zlib"],
            "compression",
        ),
    ];
    for (kex_names, ciphers, compression, list) in cases {
        let message = server(
            kex_names,
            &["ssh-ed25519"],
            ciphers,
            &[],
            compression,
            false,
        );
        assert_eq!(
            KexInit::read(&message).and_then(|kexinit| kex::negotiate(&kex::CLIENT, &kexinit)),
            Err(SshError::Negotiation(list))
        );
    }
}

#[test]
fn a_cipher_that_carries_no_integrity_needs_a_mac_and_says_so() {
    // This client offers one cipher and it is an AEAD, so the MAC lists
    // are never read. A proposal that offers another cipher is what the
    // rule is for, and it is the same rule for both directions.
    let client = Proposal {
        kex: &[KEX_CURVE25519],
        host_key: &[HOST_KEY_ED25519],
        encryption: &["aes128-ctr"],
        mac: &["hmac-sha2-256"],
        compression: &[COMPRESSION_NONE],
    };
    let agreed = server(
        &["curve25519-sha256"],
        &["ssh-ed25519"],
        &["aes128-ctr"],
        &["hmac-sha2-256"],
        &["none"],
        false,
    );
    assert_eq!(
        KexInit::read(&agreed)
            .and_then(|kexinit| kex::negotiate(&client, &kexinit))
            .map(|choice| (choice.mac_c2s, choice.mac_s2c)),
        Ok((Some("hmac-sha2-256"), Some("hmac-sha2-256")))
    );

    let without = server(
        &["curve25519-sha256"],
        &["ssh-ed25519"],
        &["aes128-ctr"],
        &["hmac-sha1"],
        &["none"],
        false,
    );
    assert_eq!(
        KexInit::read(&without).and_then(|kexinit| kex::negotiate(&client, &kexinit)),
        Err(SshError::Negotiation("MAC"))
    );
}

#[test]
fn an_extension_indicator_is_never_the_method() {
    // RFC 8308, section 2.2: if an indicator ends up negotiated as a key
    // exchange method, the parties disconnect. A server that offers the
    // client's own indicator is the only way to get there.
    let message = server(
        &["ext-info-c"],
        &["ssh-ed25519"],
        &["chacha20-poly1305@openssh.com"],
        &[],
        &["none"],
        false,
    );
    assert_eq!(
        KexInit::read(&message).and_then(|kexinit| kex::negotiate(&kex::CLIENT, &kexinit)),
        Err(SshError::Negotiation("key exchange"))
    );

    let client = Proposal {
        kex: &[EXT_INFO_S],
        ..kex::CLIENT
    };
    let message = server(
        &["ext-info-s"],
        &["ssh-ed25519"],
        &["chacha20-poly1305@openssh.com"],
        &[],
        &["none"],
        false,
    );
    assert_eq!(
        KexInit::read(&message).and_then(|kexinit| kex::negotiate(&client, &kexinit)),
        Err(SshError::Negotiation("key exchange"))
    );
}

/// Whether a message announces a guess, and whether that guess is wrong.
fn judge(message: &[u8]) -> Result<(bool, bool), SshError> {
    let kexinit = KexInit::read(message)?;
    let choice = kex::negotiate(&kex::CLIENT, &kexinit)?;
    Ok((
        kexinit.first_kex_packet_follows,
        kex::guess_is_wrong(&kexinit, &choice),
    ))
}

#[test]
fn a_guess_is_wrong_unless_it_is_what_was_chosen() {
    // Section 7.1: a guessed packet follows only when the flag is set,
    // and it is right only when the peer's first names are the chosen
    // ones. A wrong guess is ignored, not an error.
    let right = server(
        &["curve25519-sha256", "diffie-hellman-group14-sha256"],
        &["ssh-ed25519"],
        &["chacha20-poly1305@openssh.com"],
        &[],
        &["none"],
        true,
    );
    assert_eq!(judge(&right), Ok((true, false)));

    let wrong = server(
        &["diffie-hellman-group14-sha256", "curve25519-sha256"],
        &["ssh-ed25519"],
        &["chacha20-poly1305@openssh.com"],
        &[],
        &["none"],
        true,
    );
    assert_eq!(judge(&wrong), Ok((true, true)));

    // The guess is both names, so the right method under the wrong host
    // key is still a guess to ignore.
    let host_key = server(
        &["curve25519-sha256"],
        &["rsa-sha2-512", "ssh-ed25519"],
        &["chacha20-poly1305@openssh.com"],
        &[],
        &["none"],
        true,
    );
    assert_eq!(judge(&host_key), Ok((true, true)));

    // No guess at all is nothing to ignore, whatever the first names are.
    let none = server(
        &["diffie-hellman-group14-sha256", "curve25519-sha256"],
        &["ssh-ed25519"],
        &["chacha20-poly1305@openssh.com"],
        &[],
        &["none"],
        false,
    );
    assert_eq!(judge(&none), Ok((false, false)));
}

#[test]
fn a_buffer_too_small_for_the_message_writes_no_message() {
    let mut rng = ScriptedRng::new(&COOKIE);
    let mut out = [0u8; 32];
    assert!(matches!(
        kex::CLIENT.write(&mut rng, &mut out),
        Err(SshError::OutOfBounds { .. })
    ));

    // A generator with nothing left is the other way it fails.
    let mut empty = ScriptedRng::new(&[]);
    let mut out = [0u8; 512];
    assert_eq!(
        kex::CLIENT.write(&mut empty, &mut out),
        Err(SshError::Rng(crypto_rng::RngError::Exhausted))
    );
}

#[test]
fn a_name_that_may_not_be_in_a_list_is_refused_before_anything_is_written() {
    let client = Proposal {
        kex: &["curve25519-sha256,ext-info-c"],
        ..kex::CLIENT
    };
    let mut rng = ScriptedRng::new(&COOKIE);
    let mut out = [0u8; 512];
    assert_eq!(client.write(&mut rng, &mut out), Err(SshError::NameList));
}
