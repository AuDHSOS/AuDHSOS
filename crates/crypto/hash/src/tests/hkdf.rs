// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! HKDF against the vectors of RFC 5869, appendix A. The appendix also
//! carries three SHA-1 cases; this crate has no SHA-1, so the three
//! SHA-256 cases are the ones that apply.

use crate::error::HashError;
use crate::hash::Hash;
use crate::hkdf::{Prk, expand, extract};
use crate::hmac::Hmac;
use crate::sha256::Sha256;
use crate::sha512::Sha384;
use crate::tests::hex;

#[test]
fn the_first_rfc_case_derives_as_documented() {
    let ikm = [0x0bu8; 22];
    let salt: Vec<u8> = (0x00u8..=0x0c).collect();
    let info: Vec<u8> = (0xf0u8..=0xf9).collect();
    let prk = extract::<Sha256>(&salt, &ikm);
    assert_eq!(
        hex(prk.as_bytes()),
        "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"
    );
    let mut okm = [0u8; 42];
    expand::<Sha256>(&prk, &info, &mut okm).expect("42 bytes are within the bound");
    assert_eq!(
        hex(&okm),
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
    );
}

#[test]
fn the_second_rfc_case_derives_as_documented() {
    let ikm: Vec<u8> = (0x00u8..=0x4f).collect();
    let salt: Vec<u8> = (0x60u8..=0xaf).collect();
    let info: Vec<u8> = (0xb0u8..=0xff).collect();
    let prk = extract::<Sha256>(&salt, &ikm);
    assert_eq!(
        hex(prk.as_bytes()),
        "06a6b88c5853361a06104c9ceb35b45cef760014904671014a193f40c15fc244"
    );
    let mut okm = [0u8; 82];
    expand::<Sha256>(&prk, &info, &mut okm).expect("82 bytes are within the bound");
    assert_eq!(
        hex(&okm),
        "b11e398dc80327a1c8e7f78c596a49344f012eda2d4efad8a050cc4c19afa97c59045a99cac7827271cb41c65e590e09da3275600c2f09b8367793a9aca3db71cc30c58179ec3e87c14c01d5c1f3434f1d87"
    );
}

#[test]
fn the_third_rfc_case_derives_with_an_empty_salt_and_no_info() {
    let ikm = [0x0bu8; 22];
    let prk = extract::<Sha256>(b"", &ikm);
    assert_eq!(
        hex(prk.as_bytes()),
        "19ef24a32c717b167f33a91d6f648bdf96596776afdb6377ac434c1c293ccb04"
    );
    let mut okm = [0u8; 42];
    expand::<Sha256>(&prk, b"", &mut okm).expect("42 bytes are within the bound");
    assert_eq!(
        hex(&okm),
        "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8"
    );
}

#[test]
fn an_empty_salt_derives_what_a_zero_salt_derives() {
    let ikm = [0x0bu8; 22];
    let zeros = [0u8; 32];
    assert_eq!(
        hex(extract::<Sha256>(b"", &ikm).as_bytes()),
        hex(extract::<Sha256>(&zeros, &ikm).as_bytes())
    );
}

#[test]
fn a_zero_length_output_is_derived_without_work() {
    let prk = extract::<Sha256>(b"salt", b"key material");
    let mut okm: [u8; 0] = [];
    assert_eq!(expand::<Sha256>(&prk, b"info", &mut okm), Ok(()));
}

#[test]
fn the_output_bound_is_the_last_length_that_is_accepted() {
    let prk = extract::<Sha256>(b"salt", b"key material");
    let mut inside = vec![0u8; 255 * 32];
    assert_eq!(expand::<Sha256>(&prk, b"info", &mut inside), Ok(()));
    assert!(inside.iter().any(|byte| *byte != 0));

    let mut outside = vec![0u8; 255 * 32 + 1];
    assert_eq!(
        expand::<Sha256>(&prk, b"info", &mut outside),
        Err(HashError::OutputTooLong)
    );
    assert!(
        outside.iter().all(|byte| *byte == 0),
        "the buffer stays untouched"
    );
}

#[test]
fn the_expansion_is_the_documented_chain_of_codes() {
    let prk = extract::<Sha384>(b"salt", b"key material");
    let mut okm = [0u8; 100];
    expand::<Sha384>(&prk, b"info", &mut okm).expect("100 bytes are within the bound");

    let mut expected = Vec::new();
    let mut previous: Vec<u8> = Vec::new();
    for counter in 1u8..=3 {
        let mut mac = Hmac::<Sha384>::new(prk.as_bytes());
        mac.update(&previous);
        mac.update(b"info");
        mac.update(&[counter]);
        previous = mac.finish().to_vec();
        expected.extend_from_slice(&previous);
    }
    assert_eq!(hex(&okm), hex(&expected[..100]));
}

#[test]
fn a_pseudorandom_key_can_be_taken_from_a_previous_step() {
    let derived = <Sha256 as Hash>::digest(b"a secret from the schedule");
    let prk = Prk::<Sha256>::from_output(derived);
    assert_eq!(prk.as_bytes(), derived.as_ref());
    let copy = prk;
    assert_eq!(copy.as_bytes(), prk.as_bytes());
}

#[test]
fn the_error_renders_a_message() {
    assert!(!format!("{}", HashError::OutputTooLong).is_empty());
    assert!(!format!("{:?}", HashError::OutputTooLong).is_empty());
}
