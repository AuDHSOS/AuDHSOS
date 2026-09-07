// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The 8.3 name, as the eleven bytes a directory entry carries.

use core::fmt::{self, Write as _};

use crate::error::Error;

/// Bytes of a name in a directory entry: eight of stem and three of
/// extension, each padded with spaces.
pub const NAME_LEN: usize = 11;

/// Bytes of the stem.
const STEM_LEN: usize = 8;

/// The eleven bytes of a name in the 8.3 form.
///
/// [`Name::new`] takes what a caller writes, in either case, and keeps it
/// upper-cased, which is the only case the form has room for.
/// [`Name::from_entry`] takes what a directory holds and refuses anything
/// this crate would not have written, a lower-case letter included: the
/// byte that would say what a lower-case letter meant belongs to the long
/// file names, which this crate does not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name([u8; NAME_LEN]);

impl Name {
    /// The name of the entry that points at the directory itself.
    pub const DOT: Name = Name(*b".          ");

    /// The name of the entry that points at the parent directory.
    pub const DOT_DOT: Name = Name(*b"..         ");

    /// The name `name` stands for, upper-cased and padded.
    ///
    /// # Errors
    ///
    /// [`Error::Name`] for an empty name, a stem of more than eight bytes
    /// or an extension of more than three, a dot with nothing after it,
    /// more than one dot, or a byte the short form does not allow.
    pub fn new(name: &str) -> Result<Name, Error> {
        let (stem, extension) = match name.split_once('.') {
            Some((stem, extension)) => (stem, extension),
            None => (name, ""),
        };
        if stem.is_empty()
            || stem.len() > STEM_LEN
            || extension.len() > NAME_LEN.saturating_sub(STEM_LEN)
            || extension.contains('.')
            || (name.contains('.') && extension.is_empty())
        {
            return Err(Error::Name);
        }
        let mut bytes = [b' '; NAME_LEN];
        for (slot, byte) in bytes.iter_mut().zip(stem.bytes()) {
            *slot = allowed(byte)?;
        }
        for (slot, byte) in bytes.iter_mut().skip(STEM_LEN).zip(extension.bytes()) {
            *slot = allowed(byte)?;
        }
        Ok(Name(bytes))
    }

    /// The name a directory entry carries.
    ///
    /// # Errors
    ///
    /// [`Error::EntryName`] for eleven bytes that are not a name this
    /// crate would have written: an empty stem, or any byte the short
    /// form does not allow, which a lower-case letter is.
    pub fn from_entry(bytes: [u8; NAME_LEN]) -> Result<Name, Error> {
        if bytes.first() == Some(&b' ') {
            return Err(Error::EntryName);
        }
        for byte in bytes {
            if byte != b' ' && allowed(byte) != Ok(byte) {
                return Err(Error::EntryName);
            }
        }
        Ok(Name(bytes))
    }

    /// The eleven bytes, as a directory entry carries them.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; NAME_LEN] {
        &self.0
    }

    /// Whether the name is one of the two a directory keeps for itself.
    #[must_use]
    pub fn is_dot(&self) -> bool {
        *self == Name::DOT || *self == Name::DOT_DOT
    }
}

impl fmt::Display for Name {
    /// Writes the name as `STEM.EXT`, or as `STEM` where there is no
    /// extension.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0.get(..STEM_LEN).unwrap_or_default() {
            if *byte != b' ' {
                f.write_char(char::from(*byte))?;
            }
        }
        let extension = self.0.get(STEM_LEN..).unwrap_or_default();
        if extension.first().is_some_and(|byte| *byte != b' ') {
            f.write_char('.')?;
            for byte in extension {
                if *byte != b' ' {
                    f.write_char(char::from(*byte))?;
                }
            }
        }
        Ok(())
    }
}

/// The upper case of `byte`, where the short form allows it.
fn allowed(byte: u8) -> Result<u8, Error> {
    let upper = byte.to_ascii_uppercase();
    if upper.is_ascii_uppercase() || upper.is_ascii_digit() || ALLOWED_PUNCTUATION.contains(&upper)
    {
        Ok(upper)
    } else {
        Err(Error::Name)
    }
}

/// The punctuation a short name may hold beside letters and digits.
const ALLOWED_PUNCTUATION: &[u8] = b"$%'-_@~`!(){}^#&";
