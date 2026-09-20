// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::ssh`, covering the catalog item 6.6.80: the key
//! material the Secure Shell interop run is given.
//!
//! The fingerprint is a known answer taken from OpenSSH itself —
//! `ssh-keygen -lf` over the line below printed it — so this side is held
//! to what the other side computes and not to itself. The private key is
//! built here out of `docs/openssh/PROTOCOL.key` rather than embedded,
//! because no key of this repository is tracked (D-146); that the reader
//! also reads what `ssh-keygen` writes is what the interop run says.

#![allow(clippy::arithmetic_side_effects)]

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use audhsos_encoding::pem;

use crate::ssh::{Material, configuration, fingerprint_of, seed_of};

/// A public key line whose key is thirty-two `0x42` octets.
const FIXTURE: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJC fixture";

/// What `ssh-keygen -lf` prints for that line.
const FIXTURE_FINGERPRINT: &str = "SHA256:LgZpRzWEAbVvBimxTv69/UB88PD8jyOSDqfozr+iNX0";

/// The seed the built private keys carry.
const SEED: [u8; 32] = [0x11; 32];

/// The public half the built private keys name. Its value does not
/// matter to the reader, which takes the seed.
const PUBLIC: [u8; 32] = [0x22; 32];

/// A `string` of RFC 4251, section 5.
fn string(bytes: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(bytes.len()).unwrap().to_be_bytes().to_vec();
    out.extend_from_slice(bytes);
    out
}

/// A private key file as `PROTOCOL.key` describes it, under the cipher
/// and the key derivation each named by `cipher` and `kdf`.
fn private_key(cipher: &[u8], kdf: &[u8], keys: u32, algorithm: &[u8], secret: &[u8]) -> Vec<u8> {
    let mut inner = 0x0102_0304u32.to_be_bytes().to_vec();
    inner.extend_from_slice(&0x0102_0304u32.to_be_bytes());
    inner.extend_from_slice(&string(algorithm));
    inner.extend_from_slice(&string(&PUBLIC));
    inner.extend_from_slice(&string(secret));
    inner.extend_from_slice(&string(b"a comment"));
    while !inner.len().is_multiple_of(8) {
        inner.push(u8::try_from(inner.len() % 8).unwrap());
    }

    let mut body = b"openssh-key-v1\0".to_vec();
    body.extend_from_slice(&string(cipher));
    body.extend_from_slice(&string(kdf));
    body.extend_from_slice(&string(b""));
    body.extend_from_slice(&keys.to_be_bytes());
    let public_blob = [string(b"ssh-ed25519"), string(&PUBLIC)].concat();
    body.extend_from_slice(&string(&public_blob));
    body.extend_from_slice(&string(&inner));
    body
}

/// That file, wrapped the way OpenSSH wraps one.
fn wrapped(body: &[u8]) -> Vec<u8> {
    let wrap = NonZeroUsize::new(70).unwrap();
    let mut out = vec![0u8; body.len() * 2 + 128];
    let len = pem::encode_wrapped("OPENSSH PRIVATE KEY", body, wrap, &mut out).unwrap();
    out.truncate(len);
    out
}

/// The unencrypted Ed25519 key this project writes and reads.
fn plain_key() -> Vec<u8> {
    let secret = [SEED.as_slice(), PUBLIC.as_slice()].concat();
    wrapped(&private_key(b"none", b"none", 1, b"ssh-ed25519", &secret))
}

#[test]
fn a_public_key_line_answers_the_fingerprint_openssh_prints() {
    assert_eq!(
        fingerprint_of(FIXTURE.as_bytes()).unwrap(),
        FIXTURE_FINGERPRINT
    );
}

#[test]
fn the_comment_and_the_line_terminator_are_not_part_of_the_fingerprint() {
    let with_terminator = format!("{FIXTURE}\n");
    let without_comment = FIXTURE.trim_end_matches(" fixture").to_owned();

    assert_eq!(
        fingerprint_of(with_terminator.as_bytes()).unwrap(),
        FIXTURE_FINGERPRINT
    );
    assert_eq!(
        fingerprint_of(without_comment.as_bytes()).unwrap(),
        FIXTURE_FINGERPRINT
    );
}

#[test]
fn a_public_key_of_another_algorithm_is_refused() {
    let line = FIXTURE.replace("ssh-ed25519", "ssh-rsa");

    assert!(fingerprint_of(line.as_bytes()).is_err());
}

#[test]
fn a_public_key_with_no_blob_is_refused() {
    assert!(fingerprint_of(b"ssh-ed25519").is_err());
    assert!(fingerprint_of(b"").is_err());
    assert!(fingerprint_of(b"ssh-ed25519 not-base64!! fixture").is_err());
}

#[test]
fn the_seed_is_the_first_thirty_two_octets_of_the_private_half() {
    assert_eq!(seed_of(&plain_key()).unwrap(), SEED);
}

#[test]
fn a_key_under_a_passphrase_is_refused() {
    let secret = [SEED.as_slice(), PUBLIC.as_slice()].concat();
    let encrypted = wrapped(&private_key(
        b"aes256-ctr",
        b"bcrypt",
        1,
        b"ssh-ed25519",
        &secret,
    ));

    assert!(seed_of(&encrypted).is_err());
}

#[test]
fn a_file_holding_more_than_one_key_is_refused() {
    let secret = [SEED.as_slice(), PUBLIC.as_slice()].concat();
    let two = wrapped(&private_key(b"none", b"none", 2, b"ssh-ed25519", &secret));

    assert!(seed_of(&two).is_err());
}

#[test]
fn a_key_of_another_algorithm_is_refused() {
    let secret = [SEED.as_slice(), PUBLIC.as_slice()].concat();
    let other = wrapped(&private_key(b"none", b"none", 1, b"ssh-rsa", &secret));

    assert!(seed_of(&other).is_err());
}

#[test]
fn a_private_half_of_another_length_is_refused() {
    let short = wrapped(&private_key(b"none", b"none", 1, b"ssh-ed25519", &SEED));

    assert!(seed_of(&short).is_err());
}

#[test]
fn a_file_that_is_not_that_envelope_is_refused() {
    assert!(seed_of(b"not a key at all").is_err());
    assert!(seed_of(&wrapped(b"openssh-key-v2\0and then nothing")).is_err());

    let mut body = private_key(b"none", b"none", 1, b"ssh-ed25519", &SEED);
    body.truncate(20);
    assert!(seed_of(&wrapped(&body)).is_err());
}

#[test]
fn a_key_under_another_label_is_refused() {
    let wrap = NonZeroUsize::new(70).unwrap();
    let body = private_key(b"none", b"none", 1, b"ssh-ed25519", &SEED);
    let mut out = vec![0u8; body.len() * 2 + 128];
    let len = pem::encode_wrapped("PRIVATE KEY", &body, wrap, &mut out).unwrap();
    out.truncate(len);

    assert!(seed_of(&out).is_err());
}

/// Key material naming two paths, for the configuration tests.
fn material() -> Material {
    Material {
        host_key: PathBuf::from("/keys/host_ed25519"),
        authorized: PathBuf::from("/keys/client_ed25519.pub"),
        seed: SEED,
        fingerprints: vec![FIXTURE_FINGERPRINT.to_owned()],
    }
}

#[test]
fn the_server_admits_a_public_key_of_root_where_the_check_runs_as_root() {
    let text = configuration(22, &material(), Path::new("/run"), "root");

    assert!(text.contains("PermitRootLogin prohibit-password\n"));
}

#[test]
fn the_server_refuses_root_where_the_check_runs_as_another_account() {
    let text = configuration(22, &material(), Path::new("/run"), "runner");

    assert!(text.contains("PermitRootLogin no\n"));
}
