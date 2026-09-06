// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A domain name: the sequence of length-prefixed labels of RFC 1035,
//! section 3.1, read with the compression of section 4.1.4 and written
//! without it.
//!
//! A name is a value and not a borrow into the message it came from. It
//! cannot be a borrow, because a compressed name is not contiguous in
//! those bytes; and it must not be one, because the resolver holds the
//! name it is asking across the datagrams it asks in, by which time the
//! message is gone.
//!
//! Three bounds make the walk finite, and only the last of them is about
//! the bytes that arrived. A compression pointer must point strictly
//! backwards, so the walk provably moves towards the front of the message
//! and can never return to where it has been. The number of jumps is
//! bounded besides, because a chain of pointers that yields no label is
//! work a correct encoder never asks for. And the name being assembled is
//! bounded at the 255 bytes of RFC 1035, section 2.3.4, which is reached
//! long before either of the others on a message that is trying to be
//! expensive.
//!
//! Comparison ignores ASCII case, as RFC 1035, section 2.3.3 requires. The
//! length octets a name carries are at most 63 and are therefore never
//! letters, so folding the whole wire form folds exactly the labels.

use core::fmt::{self, Write as _};
use core::hash::{Hash, Hasher};

use net_wire::{WireError, Writer};

use crate::error::DnsError;

/// The longest name, in the form it takes on the wire: the length octets,
/// the labels, and the root label that closes it (RFC 1035, section 2.3.4).
pub const MAX_NAME_LEN: usize = 255;

/// The longest label (RFC 1035, section 2.3.4).
pub const MAX_LABEL_LEN: usize = 63;

/// How many compression jumps one name may take.
///
/// A pointer that points strictly backwards already bounds the walk, so
/// this is not what makes it finite; it is what keeps a name that is
/// nothing but pointers from costing a walk towards the front of the
/// message for every byte it yields. Two jumps is what an encoder that
/// compresses well produces, and sixteen is far past anything one writes
/// on purpose.
const MAX_JUMPS: usize = 16;

/// The two top bits of a length octet say what follows.
const KIND_MASK: u8 = 0xC0;

/// `00`: the octet is the length of a label.
const KIND_LABEL: u8 = 0x00;

/// `11`: the octet and the one behind it are a pointer.
const KIND_POINTER: u8 = 0xC0;

/// How many bytes a pointer takes.
const POINTER_LEN: usize = 2;

/// One domain name.
#[derive(Clone, Copy)]
pub struct Name {
    /// The wire form, root label included.
    bytes: [u8; MAX_NAME_LEN],
    /// How much of it is the name.
    len: usize,
}

impl Name {
    /// The root, which is the name of no labels.
    pub const ROOT: Name = Name {
        bytes: [0u8; MAX_NAME_LEN],
        len: 1,
    };

    /// The name in `text`, which is labels separated by dots, with or
    /// without the dot that spells the root at the end. An empty text and
    /// a lone dot are both the root.
    ///
    /// The syntax is the preferred one of RFC 1035, section 2.3.1 as
    /// RFC 1123, section 2.1 relaxed it: letters, digits and hyphens, with
    /// a hyphen neither first nor last in its label. This resolver asks
    /// for `A` and `AAAA` and for nothing else, so the underscore labels
    /// of the service names lie outside anything it can be asked.
    ///
    /// # Errors
    ///
    /// [`DnsError::Label`] for a label that is empty or longer than 63
    /// bytes, [`DnsError::Text`] for one that is not in that syntax, and
    /// [`DnsError::NameTooLong`] when the whole comes to more than 255
    /// bytes.
    pub fn from_ascii(text: &str) -> Result<Name, DnsError> {
        let trimmed = text.strip_suffix('.').unwrap_or(text);
        if trimmed.is_empty() {
            return Ok(Name::ROOT);
        }
        let mut bytes = [0u8; MAX_NAME_LEN];
        let mut writer = Writer::new(&mut bytes);
        for label in trimmed.split('.') {
            let octet = check_label(label.as_bytes())?;
            writer.write_u8(octet).map_err(too_long)?;
            writer.write_bytes(label.as_bytes()).map_err(too_long)?;
        }
        writer.write_u8(0).map_err(too_long)?;
        let len = writer.position();
        Ok(Name { bytes, len })
    }

    /// The name that begins at `at` in `message`, and the offset just past
    /// it — past the pointer, where the name ended in one.
    ///
    /// # Errors
    ///
    /// [`DnsError::Wire`] when the name reaches past the message,
    /// [`DnsError::LabelKind`] for a length octet of a reserved kind,
    /// [`DnsError::PointerForward`] for a pointer that does not point
    /// backwards, [`DnsError::PointerChain`] for more jumps than are
    /// followed, and [`DnsError::NameTooLong`] when the labels come to
    /// more than 255 bytes.
    pub fn read(message: &[u8], at: usize) -> Result<(Name, usize), DnsError> {
        let mut bytes = [0u8; MAX_NAME_LEN];
        let mut writer = Writer::new(&mut bytes);
        let mut cursor = at;
        let mut jumps = 0usize;
        // Where the name ends in the message. The first pointer fixes it:
        // everything behind that pointer is read somewhere else.
        let mut after: Option<usize> = None;
        loop {
            let octet = *message.get(cursor).ok_or_else(|| short(message, cursor))?;
            match octet & KIND_MASK {
                KIND_LABEL => {
                    let from = cursor.saturating_add(1);
                    let to = from.saturating_add(usize::from(octet));
                    writer.write_u8(octet).map_err(too_long)?;
                    if octet == 0 {
                        let len = writer.position();
                        return Ok((Name { bytes, len }, after.unwrap_or(from)));
                    }
                    let label = message.get(from..to).ok_or_else(|| short(message, to))?;
                    writer.write_bytes(label).map_err(too_long)?;
                    cursor = to;
                }
                KIND_POINTER => {
                    let behind = cursor.saturating_add(1);
                    let low = *message.get(behind).ok_or_else(|| short(message, behind))?;
                    let to = usize::from(u16::from_be_bytes([octet & !KIND_MASK, low]));
                    if to >= cursor {
                        return Err(DnsError::PointerForward { at: cursor, to });
                    }
                    jumps = jumps.saturating_add(1);
                    if jumps > MAX_JUMPS {
                        return Err(DnsError::PointerChain);
                    }
                    after = Some(after.unwrap_or(cursor.saturating_add(POINTER_LEN)));
                    cursor = to;
                }
                _ => return Err(DnsError::LabelKind(octet)),
            }
        }
    }

    /// The wire form, root label included.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or(&[])
    }

    /// Whether this is the root, which is the only name of no labels.
    #[must_use]
    pub const fn is_root(&self) -> bool {
        self.len == 1
    }

    /// The labels, from the leftmost to the one before the root.
    #[must_use]
    pub fn labels(&self) -> Labels<'_> {
        Labels {
            bytes: self.as_bytes(),
            at: 0,
        }
    }

    /// Writes the name, uncompressed.
    ///
    /// # Errors
    ///
    /// [`DnsError::Wire`] when the buffer has no room.
    pub fn write(&self, writer: &mut Writer<'_>) -> Result<(), DnsError> {
        writer.write_bytes(self.as_bytes())?;
        Ok(())
    }
}

/// The labels of a name, left to right.
#[derive(Clone, Debug)]
pub struct Labels<'a> {
    /// The wire form.
    bytes: &'a [u8],
    /// How much of it has been walked.
    at: usize,
}

impl<'a> Iterator for Labels<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        let len = usize::from(*self.bytes.get(self.at)?);
        let from = self.at.saturating_add(1);
        let label = self.bytes.get(from..from.saturating_add(len))?;
        if label.is_empty() {
            return None;
        }
        self.at = from.saturating_add(len);
        Some(label)
    }
}

impl PartialEq for Name {
    fn eq(&self, other: &Name) -> bool {
        self.as_bytes().eq_ignore_ascii_case(other.as_bytes())
    }
}

impl Eq for Name {}

impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for byte in self.as_bytes() {
            state.write_u8(byte.to_ascii_lowercase());
        }
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for label in self.labels() {
            for byte in label {
                f.write_char(char::from(*byte))?;
            }
            f.write_char('.')?;
        }
        if self.is_root() {
            f.write_char('.')?;
        }
        Ok(())
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Name({self})")
    }
}

/// Whether `label` is a label in the preferred syntax, and its length as
/// the octet in front of it.
fn check_label(label: &[u8]) -> Result<u8, DnsError> {
    let Ok(octet) = u8::try_from(label.len()) else {
        return Err(DnsError::Label(label.len()));
    };
    if label.is_empty() || usize::from(octet) > MAX_LABEL_LEN {
        return Err(DnsError::Label(label.len()));
    }
    let plain = label
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-');
    match label {
        _ if !plain => Err(DnsError::Text),
        [b'-', ..] | [.., b'-'] => Err(DnsError::Text),
        _ => Ok(octet),
    }
}

/// The error for a name that comes to more than 255 bytes.
const fn too_long(_: WireError) -> DnsError {
    DnsError::NameTooLong
}

/// The error for a name that reaches past the message.
const fn short(message: &[u8], needed: usize) -> DnsError {
    DnsError::Wire(WireError::OutOfBounds {
        needed,
        available: message.len(),
    })
}
