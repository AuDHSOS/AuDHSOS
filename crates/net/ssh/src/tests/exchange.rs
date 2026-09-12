// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two key exchange methods, their messages, and the exchange hash.
//!
//! No document publishes a complete SSH key exchange with the values that
//! made it, so the hash is checked against the same concatenation written
//! a second time, and the methods against each other.

use crypto_hash::Sha256;
use crypto_rng::doubles::ScriptedRng;

use crate::error::SshError;
use crate::exchange::{
    Ephemeral, HashInput, INIT, MAX_PUBLIC, Method, REPLY, Reply, exchange_hash,
};
use crate::kex::{KEX_CURVE25519, KEX_GROUP14};
use crate::tests::{ssh_mpint, ssh_string, unhex};
use crate::wire::{Reader, Writer};

/// A scalar for one side, and another for the other.
const SECRET_A: [u8; 32] = [0x11; 32];
/// The second side's.
const SECRET_B: [u8; 32] = [0x22; 32];

/// The hash the test builds for itself, from the same pieces.
fn hash_by_hand(method: Method, input: &HashInput<'_>) -> [u8; 32] {
    let mut preimage = Vec::new();
    preimage.extend_from_slice(&ssh_string(input.client_id.as_bytes()));
    preimage.extend_from_slice(&ssh_string(input.server_id.as_bytes()));
    preimage.extend_from_slice(&ssh_string(input.client_kexinit));
    preimage.extend_from_slice(&ssh_string(input.server_kexinit));
    preimage.extend_from_slice(&ssh_string(input.host_key));
    if method == Method::Curve25519 {
        preimage.extend_from_slice(&ssh_string(input.client_public));
        preimage.extend_from_slice(&ssh_string(input.server_public));
    } else {
        preimage.extend_from_slice(&ssh_mpint(input.client_public));
        preimage.extend_from_slice(&ssh_mpint(input.server_public));
    }
    preimage.extend_from_slice(&ssh_mpint(input.shared));
    Sha256::digest(&preimage)
}

/// An input with every field different, so that no two are swapped
/// without the hash saying so.
fn input<'a>(client: &'a [u8], server: &'a [u8], shared: &'a [u8]) -> HashInput<'a> {
    HashInput {
        client_id: "SSH-2.0-AuDHSOS_0.1.0",
        server_id: "SSH-2.0-OpenSSH_9.6",
        client_kexinit: b"I_C",
        server_kexinit: b"I_S",
        host_key: b"K_S",
        client_public: client,
        server_public: server,
        shared,
    }
}

#[test]
fn a_method_is_the_name_it_was_negotiated_under() {
    assert_eq!(Method::from_name(KEX_CURVE25519), Some(Method::Curve25519));
    assert_eq!(Method::from_name(KEX_GROUP14), Some(Method::Group14));
    assert_eq!(Method::from_name("diffie-hellman-group1-sha1"), None);
    assert_eq!(Method::Curve25519.name(), KEX_CURVE25519);
    assert_eq!(Method::Group14.name(), KEX_GROUP14);
}

#[test]
fn both_sides_of_the_curve_method_reach_one_secret() {
    let mut first = ScriptedRng::new(&SECRET_A);
    let mut second = ScriptedRng::new(&SECRET_B);
    let client = Ephemeral::new(Method::Curve25519, &mut first).expect("the generator has bytes");
    let server = Ephemeral::new(Method::Curve25519, &mut second).expect("the generator has bytes");
    assert_eq!(client.public().len(), 32);
    assert_eq!(client.method(), Method::Curve25519);

    let mut ours = [0u8; MAX_PUBLIC];
    let mut theirs = [0u8; MAX_PUBLIC];
    let a = client.shared_secret(server.public(), &mut ours);
    let b = server.shared_secret(client.public(), &mut theirs);
    assert_eq!(a, Ok(32));
    assert_eq!(b, Ok(32));
    assert_eq!(ours, theirs);
    assert_ne!(ours.get(..32), Some([0u8; 32].as_slice()));
}

#[test]
fn both_sides_of_the_finite_field_method_reach_one_secret() {
    let mut first = ScriptedRng::new(&SECRET_A);
    let mut second = ScriptedRng::new(&SECRET_B);
    let client = Ephemeral::new(Method::Group14, &mut first).expect("the generator has bytes");
    let server = Ephemeral::new(Method::Group14, &mut second).expect("the generator has bytes");
    assert_eq!(client.public().len(), 256);

    let mut ours = [0u8; MAX_PUBLIC];
    let mut theirs = [0u8; MAX_PUBLIC];
    assert_eq!(client.shared_secret(server.public(), &mut ours), Ok(256));
    assert_eq!(server.shared_secret(client.public(), &mut theirs), Ok(256));
    assert_eq!(ours, theirs);
}

#[test]
fn a_generator_with_nothing_left_makes_no_key_pair() {
    let mut rng = ScriptedRng::new(&[]);
    assert!(matches!(
        Ephemeral::new(Method::Curve25519, &mut rng),
        Err(SshError::Rng(_))
    ));
}

#[test]
fn the_first_message_carries_the_public_value_as_its_method_encodes_it() {
    let mut rng = ScriptedRng::new(&SECRET_A);
    let client = Ephemeral::new(Method::Curve25519, &mut rng).expect("the generator has bytes");
    let mut out = [0u8; 512];
    let len = client.write_init(&mut out).unwrap_or_default();
    let mut reader = Reader::new(out.get(..len).unwrap_or(&[]));
    assert_eq!(reader.read_byte(), Ok(INIT));
    assert_eq!(reader.read_string(), Ok(client.public()));
    assert!(reader.is_empty());

    // The finite-field method sends an integer, so its encoding is the
    // mpint of section 5 and not a fixed-length string.
    let mut rng = ScriptedRng::new(&SECRET_A);
    let client = Ephemeral::new(Method::Group14, &mut rng).expect("the generator has bytes");
    let len = client.write_init(&mut out).unwrap_or_default();
    let mut reader = Reader::new(out.get(..len).unwrap_or(&[]));
    assert_eq!(reader.read_byte(), Ok(INIT));
    assert_eq!(
        reader.read_mpint().and_then(|value| value.magnitude()),
        Ok(client.public())
    );
    assert!(reader.is_empty());

    let mut small = [0u8; 8];
    assert!(matches!(
        client.write_init(&mut small),
        Err(SshError::OutOfBounds { .. })
    ));
}

#[test]
fn the_reply_is_the_host_key_the_public_value_and_the_signature() {
    let mut out = [0u8; 512];
    let mut writer = Writer::new(&mut out);
    assert_eq!(writer.write_byte(REPLY), Ok(()));
    assert_eq!(writer.write_string(b"K_S"), Ok(()));
    assert_eq!(writer.write_string(&[0x42u8; 32]), Ok(()));
    assert_eq!(writer.write_string(b"signature"), Ok(()));
    let len = writer.position();

    let payload = out.get(..len).unwrap_or(&[]);
    assert_eq!(
        Reply::read(payload, Method::Curve25519),
        Ok(Reply {
            host_key: b"K_S",
            public: [0x42u8; 32].as_slice(),
            signature: b"signature",
        })
    );

    // Another message, and one that ends early.
    assert_eq!(
        Reply::read(&[INIT], Method::Curve25519),
        Err(SshError::Message(INIT))
    );
    for short in [1usize, 5, len.saturating_sub(1)] {
        assert!(matches!(
            Reply::read(payload.get(..short).unwrap_or(&[]), Method::Curve25519),
            Err(SshError::OutOfBounds { .. })
        ));
    }
}

#[test]
fn the_finite_field_reply_carries_an_mpint_and_refuses_what_is_not_one() {
    let mut out = [0u8; 64];
    let mut writer = Writer::new(&mut out);
    assert_eq!(writer.write_byte(REPLY), Ok(()));
    assert_eq!(writer.write_string(b"K_S"), Ok(()));
    assert_eq!(writer.write_unsigned(&[0x80, 0x01]), Ok(()));
    assert_eq!(writer.write_string(b"s"), Ok(()));
    let len = writer.position();
    assert_eq!(
        Reply::read(out.get(..len).unwrap_or(&[]), Method::Group14).map(|reply| reply.public),
        Ok([0x80, 0x01].as_slice())
    );

    // A negative f is no group element, and a non-canonical one is no
    // mpint.
    let negative = unhex("1f000000034b5f5300000002edcc00000001 73");
    assert_eq!(
        Reply::read(negative.as_slice(), Method::Group14),
        Err(SshError::Negative)
    );
    let sloppy = unhex("1f000000034b5f53000000020 07f0000000173");
    assert_eq!(
        Reply::read(sloppy.as_slice(), Method::Group14),
        Err(SshError::Mpint)
    );
}

#[test]
fn the_aborts_of_the_two_methods_are_refusals_and_not_values() {
    let mut rng = ScriptedRng::new(&SECRET_A);
    let client = Ephemeral::new(Method::Curve25519, &mut rng).expect("the generator has bytes");
    let mut out = [0u8; MAX_PUBLIC];

    // RFC 8731, section 3: a public value that is not 32 bytes.
    let wrong = [0x42u8; 64];
    for len in [0usize, 31, 33] {
        assert_eq!(
            client.shared_secret(wrong.get(..len).unwrap_or(&[]), &mut out),
            Err(SshError::KeyExchangeFailed)
        );
    }
    // And a point of small order, whose shared secret is all zeros.
    assert_eq!(
        client.shared_secret(&[0u8; 32], &mut out),
        Err(SshError::KeyExchangeFailed)
    );

    // RFC 8268, section 4: the open interval, which crypto-dh holds the
    // peer to. One and p-1 are the two ends that RFC 4253's closed form
    // would have let through.
    let mut rng = ScriptedRng::new(&SECRET_B);
    let client = Ephemeral::new(Method::Group14, &mut rng).expect("the generator has bytes");
    let mut one = [0u8; 256];
    if let Some(byte) = one.last_mut() {
        *byte = 1;
    }
    assert_eq!(
        client.shared_secret(&one, &mut out),
        Err(SshError::KeyExchangeFailed)
    );
    assert_eq!(
        client.shared_secret(&[0u8; 256], &mut out),
        Err(SshError::KeyExchangeFailed)
    );
}

#[test]
fn a_buffer_of_this_sides_own_is_not_a_key_exchange_that_failed() {
    // A shared secret that does not fit is this side's mistake, and the
    // peer is not disconnected for it.
    let mut rng = ScriptedRng::new(&SECRET_A);
    let client = Ephemeral::new(Method::Curve25519, &mut rng).expect("the generator has bytes");
    let peer = [0x42u8; 32];
    let mut small = [0u8; 31];
    assert_eq!(
        client.shared_secret(&peer, &mut small),
        Err(SshError::OutOfBounds {
            needed: 32,
            available: 31
        })
    );

    let mut rng = ScriptedRng::new(&SECRET_B);
    let client = Ephemeral::new(Method::Group14, &mut rng).expect("the generator has bytes");
    let mut peer = [0u8; 256];
    if let Some(byte) = peer.last_mut() {
        *byte = 3;
    }
    let mut small = [0u8; 32];
    assert_eq!(
        client.shared_secret(&peer, &mut small),
        Err(SshError::OutOfBounds {
            needed: 256,
            available: 32
        })
    );
}

#[test]
fn the_exchange_hash_is_the_concatenation_the_document_names() {
    // Top bits set, so that the string encoding of a public value and
    // its mpint encoding are not the same bytes.
    let client = [0x91u8; 32];
    let server = [0xa2u8; 32];
    let shared = [0x33u8; 32];
    let input = input(&client, &server, &shared);
    assert_eq!(
        exchange_hash(Method::Curve25519, &input),
        hash_by_hand(Method::Curve25519, &input)
    );
    // The same eight values under the other method hash differently,
    // because the two public values are integers there and strings here.
    assert_eq!(
        exchange_hash(Method::Group14, &input),
        hash_by_hand(Method::Group14, &input)
    );
    assert_ne!(
        exchange_hash(Method::Curve25519, &input),
        exchange_hash(Method::Group14, &input)
    );
}

#[test]
fn the_shared_secret_enters_the_hash_as_an_mpint() {
    // The trap of 14.15: X25519 gives 32 bytes that SSH reads as an
    // unsigned integer and encodes as an mpint, so half of all shared
    // secrets carry a leading zero byte and the other half do not. An
    // implementation that hashes 32 fixed bytes agrees with its peer on
    // about half of its connections.
    let client = [0x11u8; 32];
    let server = [0x22u8; 32];
    // The top bit set: the mpint carries a zero byte the fixed-length
    // string does not, and the two hashes differ.
    let set = input(&client, &server, &[0x80u8; 32]);
    assert_eq!(
        exchange_hash(Method::Curve25519, &set),
        hash_by_hand(Method::Curve25519, &set)
    );
    assert_ne!(
        exchange_hash(Method::Curve25519, &set),
        Sha256::digest(&fixed_length_preimage(&set)),
        "a fixed-length K is what this test exists to refuse"
    );

    // The top bit clear: the two encodings are the same bytes, which is
    // why the mistake succeeds on about half of all connections and is
    // found on the other half.
    let clear = input(&client, &server, &[0x7fu8; 32]);
    assert_eq!(
        exchange_hash(Method::Curve25519, &clear),
        hash_by_hand(Method::Curve25519, &clear)
    );
    assert_eq!(
        exchange_hash(Method::Curve25519, &clear),
        Sha256::digest(&fixed_length_preimage(&clear))
    );

    // A leading zero in the shared secret is not part of the value.
    let mut zeroed = [0x11u8; 32];
    if let Some(byte) = zeroed.first_mut() {
        *byte = 0;
    }
    let with = input(&client, &server, &zeroed);
    let without = input(&client, &server, zeroed.get(1..).unwrap_or(&[]));
    assert_eq!(
        exchange_hash(Method::Curve25519, &with),
        exchange_hash(Method::Curve25519, &without)
    );
}

/// The preimage an implementation builds when it hashes the shared secret
/// as a fixed-length string instead of as an `mpint`.
fn fixed_length_preimage(input: &HashInput<'_>) -> Vec<u8> {
    let mut preimage = Vec::new();
    preimage.extend_from_slice(&ssh_string(input.client_id.as_bytes()));
    preimage.extend_from_slice(&ssh_string(input.server_id.as_bytes()));
    preimage.extend_from_slice(&ssh_string(input.client_kexinit));
    preimage.extend_from_slice(&ssh_string(input.server_kexinit));
    preimage.extend_from_slice(&ssh_string(input.host_key));
    preimage.extend_from_slice(&ssh_string(input.client_public));
    preimage.extend_from_slice(&ssh_string(input.server_public));
    preimage.extend_from_slice(&ssh_string(input.shared));
    preimage
}

#[test]
fn every_field_of_the_hash_changes_it() {
    let client = [0x11u8; 32];
    let server = [0x22u8; 32];
    let shared = [0x33u8; 32];
    let base = exchange_hash(Method::Curve25519, &input(&client, &server, &shared));
    let mut changed = Vec::new();
    for altered in [
        HashInput {
            client_id: "SSH-2.0-Other",
            ..input(&client, &server, &shared)
        },
        HashInput {
            server_id: "SSH-2.0-Other",
            ..input(&client, &server, &shared)
        },
        HashInput {
            client_kexinit: b"other",
            ..input(&client, &server, &shared)
        },
        HashInput {
            server_kexinit: b"other",
            ..input(&client, &server, &shared)
        },
        HashInput {
            host_key: b"other",
            ..input(&client, &server, &shared)
        },
        input(&server, &server, &shared),
        input(&client, &client, &shared),
        input(&client, &server, &client),
    ] {
        let hash = exchange_hash(Method::Curve25519, &altered);
        assert_ne!(hash, base);
        assert!(!changed.contains(&hash));
        changed.push(hash);
    }
}
