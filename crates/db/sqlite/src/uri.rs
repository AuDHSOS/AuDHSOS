// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A file name written as a URI, read apart into the path of the
//! database and the parameters after it.
//!
//! `sqlite3ParseUri` of `research/sqlite/src/main.c:3105` is the
//! document. A name that begins `file:` is read apart where the client
//! asks for it; every other name is the path itself. Reading a name of
//! n bytes costs O(n).

use alloc::vec::Vec;

use crate::db::{Error, Moded};

/// What a file name says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    /// The path of the database, with every `%HH` escape read back.
    pub path: Vec<u8>,
    /// The parameters after the path, in the order they were written.
    pub parameters: Vec<(Vec<u8>, Vec<u8>)>,
    /// The access a `mode=` parameter asks for, and nothing where the
    /// name carries none.
    pub mode: Option<Mode>,
}

impl Named {
    /// The value written for a parameter, and nothing where the name
    /// carries no parameter of that name.
    ///
    /// Reading it costs O(n) in the number of parameters.
    #[must_use]
    pub fn parameter(&self, name: &[u8]) -> Option<&[u8]> {
        self.parameters
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_slice())
    }
}

/// The access a `mode=` parameter asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `mode=ro`: the database is read and not written.
    ReadOnly,
    /// `mode=rw`: the database is read and written, and a file that is
    /// not there is not made.
    ReadWrite,
    /// `mode=rwc`: the database is read and written, and a file that is
    /// not there is made.
    Create,
    /// `mode=memory`: the database is the connection's own and no file
    /// holds it.
    Memory,
}

/// What a file name says: the path itself where the client reads no
/// URI or the name does not begin `file:`, and the parts of the URI
/// otherwise.
///
/// # Errors
///
/// [`Error::UriAuthority`] for an authority that is neither empty nor
/// `localhost`, [`Error::UriEscape`] for a `%00` escape, which this
/// library is built to refuse, and [`Error::NoUriMode`] for a `mode=`
/// or `cache=` value the library holds no mode under.
pub fn named(name: &[u8], uri: bool) -> Result<Named, Error> {
    if !uri || !name.starts_with(b"file:") {
        return Ok(Named {
            path: name.to_vec(),
            parameters: Vec::new(),
            mode: None,
        });
    }
    let mut at = authority(name)?;
    let mut path = Vec::new();
    let mut key = Vec::new();
    let mut value = Vec::new();
    let mut parameters: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    // Where the walk stands: 0 in the path, 1 in the name of a
    // parameter, 2 in its value.
    let mut state = 0_u8;
    while let Some(held) = name.get(at).copied() {
        if held == b'#' {
            break;
        }
        at = at.saturating_add(1);
        if held == b'%'
            && let Some(octet) = escaped(name, at)
        {
            at = at.saturating_add(2);
            if octet == 0 {
                return Err(Error::UriEscape);
            }
            pushed(state, octet, (&mut path, &mut key, &mut value));
            continue;
        }
        if state == 1 && (held == b'&' || held == b'=') {
            if key.is_empty() {
                // A parameter of no name is left out, and the value
                // written after it with it.
                at = past(name, at);
                continue;
            }
            if held == b'&' {
                parameters.push((core::mem::take(&mut key), Vec::new()));
            } else {
                state = 2;
            }
            continue;
        }
        if (state == 0 && held == b'?') || (state == 2 && held == b'&') {
            if state == 2 {
                parameters.push((core::mem::take(&mut key), core::mem::take(&mut value)));
            }
            state = 1;
            continue;
        }
        pushed(state, held, (&mut path, &mut key, &mut value));
    }
    if state == 1 && !key.is_empty() {
        parameters.push((key, Vec::new()));
    } else if state == 2 {
        parameters.push((key, value));
    }
    let mode = asked(&parameters)?;
    Ok(Named {
        path,
        parameters,
        mode,
    })
}

/// Where the path of a URI begins: past `file:`, and past the authority
/// where the name carries one.
///
/// # Errors
///
/// [`Error::UriAuthority`] names an authority that is neither empty nor
/// `localhost`, which the library reads no file of another machine for.
fn authority(name: &[u8]) -> Result<usize, Error> {
    if name.get(5) != Some(&b'/') || name.get(6) != Some(&b'/') {
        return Ok(5);
    }
    let mut at = 7;
    while name.get(at).is_some_and(|held| *held != b'/') {
        at = at.saturating_add(1);
    }
    let held = name.get(7..at).unwrap_or_default();
    if at != 7 && held != b"localhost" {
        return Err(Error::UriAuthority(held.to_vec()));
    }
    Ok(at)
}

/// The byte the two hexadecimal digits at `at` stand for, which the `%`
/// before them opened.
fn escaped(name: &[u8], at: usize) -> Option<u8> {
    let high = digit(*name.get(at)?)?;
    let low = digit(*name.get(at.saturating_add(1))?)?;
    Some(high.saturating_mul(16).saturating_add(low))
}

/// What one hexadecimal digit counts, and nothing for a byte that is no
/// digit.
const fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte.saturating_sub(b'0')),
        b'a'..=b'f' => Some(byte.saturating_sub(b'a').saturating_add(10)),
        b'A'..=b'F' => Some(byte.saturating_sub(b'A').saturating_add(10)),
        _ => None,
    }
}

/// Where the parameter after the one being read begins: past the next
/// `&`, or at the end of the name.
fn past(name: &[u8], at: usize) -> usize {
    let mut at = at;
    while name.get(at).is_some_and(|held| *held != b'#')
        && name.get(at.saturating_sub(1)) != Some(&b'&')
    {
        at = at.saturating_add(1);
    }
    at
}

/// Writes one byte where the walk stands.
fn pushed(state: u8, byte: u8, held: (&mut Vec<u8>, &mut Vec<u8>, &mut Vec<u8>)) {
    let (path, key, value) = held;
    match state {
        0 => path.push(byte),
        1 => key.push(byte),
        _ => value.push(byte),
    }
}

/// The access the parameters ask for, and nothing where they ask for
/// none.
///
/// # Errors
///
/// [`Error::NoUriMode`] names the parameter and the value, where a
/// `mode=` or `cache=` parameter carries a value the library holds no
/// mode under.
fn asked(parameters: &[(Vec<u8>, Vec<u8>)]) -> Result<Option<Mode>, Error> {
    let mut mode = None;
    for (name, value) in parameters {
        if name == b"cache" && value.as_slice() != b"shared" && value.as_slice() != b"private" {
            return Err(Error::NoUriMode(Moded::Cache, value.clone()));
        }
        if name == b"mode" {
            mode = Some(match value.as_slice() {
                b"ro" => Mode::ReadOnly,
                b"rw" => Mode::ReadWrite,
                b"rwc" => Mode::Create,
                b"memory" => Mode::Memory,
                held => return Err(Error::NoUriMode(Moded::Access, held.to_vec())),
            });
        }
    }
    Ok(mode)
}
