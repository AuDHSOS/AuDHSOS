// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Names: the two size limits, the syntax, and what a compression pointer
//! is allowed to do.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a message at known offsets"
)]

use core::hash::{Hash as _, Hasher};

use net_wire::{WireError, Writer};

use crate::error::DnsError;
use crate::name::{MAX_LABEL_LEN, MAX_NAME_LEN, Name};

/// The wire form of `www.example.com.`, followed by `mail` and a pointer
/// to the `example.com.` inside it.
fn message() -> [u8; 24] {
    let mut bytes = [0u8; 24];
    bytes[..17].copy_from_slice(b"\x03www\x07example\x03com\x00");
    bytes[17..24].copy_from_slice(b"\x04mail\xc0\x04");
    bytes
}

#[test]
fn a_name_reads_back_as_the_text_it_was_written_from() {
    let name = Name::from_ascii("www.example.com").expect("a name");
    assert_eq!(name.as_bytes(), b"\x03www\x07example\x03com\x00");
    assert_eq!(name.to_string(), "www.example.com.");
    let mut buffer = [0u8; 32];
    let mut writer = Writer::new(&mut buffer);
    name.write(&mut writer).expect("room");
    let written = writer.finish();
    let (again, after) = Name::read(written, 0).expect("a name");
    assert_eq!(again, name);
    assert_eq!(after, written.len());
}

#[test]
fn the_trailing_dot_is_the_root_and_costs_nothing() {
    assert_eq!(
        Name::from_ascii("example.com.").expect("a name"),
        Name::from_ascii("example.com").expect("a name")
    );
    assert_eq!(Name::from_ascii("").expect("the root"), Name::ROOT);
    assert_eq!(Name::from_ascii(".").expect("the root"), Name::ROOT);
    assert!(Name::ROOT.is_root());
    assert_eq!(Name::ROOT.as_bytes(), b"\x00");
    assert_eq!(Name::ROOT.to_string(), ".");
    assert_eq!(Name::ROOT.labels().count(), 0);
}

#[test]
fn a_label_of_63_bytes_is_one_and_a_label_of_64_is_not() {
    let long = "a".repeat(MAX_LABEL_LEN);
    let name = Name::from_ascii(&long).expect("a name");
    assert_eq!(name.labels().next(), Some(long.as_bytes()));
    let longer = "a".repeat(MAX_LABEL_LEN + 1);
    assert_eq!(Name::from_ascii(&longer), Err(DnsError::Label(64)));
}

#[test]
fn a_name_of_255_bytes_is_one_and_a_name_of_256_is_not() {
    // Every label costs its length and the octet in front of it, and the
    // root label costs one more.
    let fits = [
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61),
    ]
    .join(".");
    let name = Name::from_ascii(&fits).expect("a name");
    assert_eq!(name.as_bytes().len(), MAX_NAME_LEN);
    let over = [
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(62),
    ]
    .join(".");
    assert_eq!(Name::from_ascii(&over), Err(DnsError::NameTooLong));
}

#[test]
fn an_empty_label_is_no_label() {
    assert_eq!(Name::from_ascii("example..com"), Err(DnsError::Label(0)));
    assert_eq!(Name::from_ascii(".example.com"), Err(DnsError::Label(0)));
}

#[test]
fn the_syntax_is_letters_digits_and_an_inner_hyphen() {
    assert!(Name::from_ascii("xn--bcher-kva.example").is_ok());
    assert!(Name::from_ascii("h3.example4.com").is_ok());
    assert_eq!(Name::from_ascii("-lead.example"), Err(DnsError::Text));
    assert_eq!(Name::from_ascii("trail-.example"), Err(DnsError::Text));
    assert_eq!(Name::from_ascii("_dmarc.example"), Err(DnsError::Text));
    assert_eq!(Name::from_ascii("a b.example"), Err(DnsError::Text));
    assert_eq!(Name::from_ascii("wörter.example"), Err(DnsError::Text));
}

#[test]
fn case_is_written_as_it_came_and_ignored_when_names_are_compared() {
    let lower = Name::from_ascii("example.com").expect("a name");
    let mixed = Name::from_ascii("ExAmPlE.CoM").expect("a name");
    assert_eq!(lower, mixed);
    assert_eq!(mixed.to_string(), "ExAmPlE.CoM.");
    assert_eq!(hash_of(&lower), hash_of(&mixed));
    assert_ne!(lower, Name::from_ascii("example.org").expect("a name"));
    assert_eq!(format!("{lower:?}"), "Name(example.com.)");
}

/// A hash of `name`, to check that equal names hash alike.
fn hash_of(name: &Name) -> u64 {
    let mut hasher = Counting::default();
    name.hash(&mut hasher);
    hasher.finish()
}

/// A hasher that mixes what it is given, which is all a test needs.
#[derive(Default)]
struct Counting(u64);

impl Hasher for Counting {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = self.0.rotate_left(5) ^ u64::from(*byte);
        }
    }
}

#[test]
fn a_pointer_reaches_a_name_that_stands_earlier() {
    let message = message();
    let (whole, after) = Name::read(&message, 0).expect("a name");
    assert_eq!(whole.to_string(), "www.example.com.");
    assert_eq!(after, 17);
    let (aliased, after) = Name::read(&message, 17).expect("a name");
    assert_eq!(aliased.to_string(), "mail.example.com.");
    // The name ends behind the pointer and not behind what it points at.
    assert_eq!(after, 24);
}

#[test]
fn a_pointer_that_is_the_whole_name_is_read() {
    let message = message();
    let (name, after) = Name::read(message[..].split_at(21).0, 4).expect("a name");
    assert_eq!(name.to_string(), "example.com.");
    assert_eq!(after, 17);
}

#[test]
fn a_pointer_that_does_not_point_backwards_is_refused() {
    // A pointer to itself is the shortest loop there is.
    assert_eq!(
        Name::read(b"\xc0\x00", 0),
        Err(DnsError::PointerForward { at: 0, to: 0 })
    );
    // And a loop of two is a pointer forwards somewhere.
    let looping = [0xC0u8, 0x02, 0xC0, 0x00];
    assert_eq!(
        Name::read(&looping, 0),
        Err(DnsError::PointerForward { at: 0, to: 2 })
    );
    // Entered from the other end the loop is walked one step and then
    // meets the same forward pointer.
    assert_eq!(
        Name::read(&looping, 2),
        Err(DnsError::PointerForward { at: 0, to: 2 })
    );
}

#[test]
fn a_chain_of_more_jumps_than_are_followed_is_refused() {
    // The root at the front, and behind it a ladder of pointers each of
    // which points at the one before. Every jump is backwards, so only the
    // count of them ends this.
    let rungs = 18usize;
    let mut message = [0u8; 1 + 2 * 18];
    for rung in 0..rungs {
        let at = 1 + 2 * rung;
        let to = at.saturating_sub(2);
        message[at] = 0xC0;
        message[at + 1] = u8::try_from(to).expect("a small offset");
    }
    // The first rung points at the root, so a short ladder is a name.
    assert_eq!(Name::read(&message, 1), Ok((Name::ROOT, 3)));
    assert_eq!(Name::read(&message, 1 + 2 * 15), Ok((Name::ROOT, 33)));
    assert_eq!(
        Name::read(&message, 1 + 2 * 17),
        Err(DnsError::PointerChain)
    );
}

#[test]
fn a_length_octet_of_a_reserved_kind_is_refused() {
    assert_eq!(Name::read(b"\x40aa\x00", 0), Err(DnsError::LabelKind(0x40)));
    assert_eq!(Name::read(b"\x80aa\x00", 0), Err(DnsError::LabelKind(0x80)));
}

#[test]
fn a_name_that_reaches_past_the_message_is_refused() {
    assert_eq!(
        Name::read(b"\x03ww", 0),
        Err(DnsError::Wire(WireError::OutOfBounds {
            needed: 4,
            available: 3
        }))
    );
    assert_eq!(
        Name::read(b"\x03www", 0),
        Err(DnsError::Wire(WireError::OutOfBounds {
            needed: 4,
            available: 4
        }))
    );
    assert_eq!(
        Name::read(b"\xc0", 0),
        Err(DnsError::Wire(WireError::OutOfBounds {
            needed: 1,
            available: 1
        }))
    );
    assert_eq!(
        Name::read(b"\x00", 1),
        Err(DnsError::Wire(WireError::OutOfBounds {
            needed: 1,
            available: 1
        }))
    );
}

#[test]
fn labels_assembled_from_pointers_are_bounded_at_255_bytes_as_well() {
    // Sixty-three bytes at the front, and behind them a name that points
    // back into it five times over. Nothing here is longer than a name may
    // be; the assembly of it is.
    let label = [0x3Fu8; 1];
    let mut message = [b'a'; 1 + 63 + 5 * 2];
    message[0] = label[0];
    for jump in 0..5 {
        let at = 64 + 2 * jump;
        message[at] = 0xC0;
        message[at + 1] = 0;
    }
    // Each pointer leads to the same 64 bytes and then to the next
    // pointer, so four of them are already past the limit.
    assert_eq!(Name::read(&message, 64), Err(DnsError::NameTooLong));
}

#[test]
fn a_name_that_does_not_fit_the_buffer_it_is_written_to_says_so() {
    let name = Name::from_ascii("example.com").expect("a name");
    let mut buffer = [0u8; 4];
    let mut writer = Writer::new(&mut buffer);
    assert!(matches!(
        name.write(&mut writer),
        Err(DnsError::Wire(WireError::OutOfBounds { .. }))
    ));
}
