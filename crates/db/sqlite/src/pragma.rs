// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The pragmas that say how a database is configured, `src/pragma.c`.
//!
//! A pragma either says what a connection is configured with or asks
//! what it is. The ones here are the ones the file itself holds, so a
//! reader answers them out of the header and a writer applies them
//! before the first table is written.

use alloc::vec::Vec;

use crate::header::{Encoding, Header};
use crate::value::Value;

/// A pragma this crate answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// How many bytes one page takes.
    PageSize,
    /// How many bytes of every page the b-tree layer may not use.
    Reserved,
    /// The encoding the file keeps its text in.
    Encoding,
    /// Whether the file vacuums itself, and how.
    AutoVacuum,
    /// What a commit writes beside the file.
    JournalMode,
    /// How many pages the file holds.
    PageCount,
    /// How many pages the free list holds.
    FreelistCount,
    /// The number the schema is at, which every change to it raises.
    SchemaVersion,
    /// The number `PRAGMA user_version` keeps.
    UserVersion,
    /// The number `PRAGMA application_id` keeps.
    ApplicationId,
    /// The schema format the file was written under.
    SchemaFormat,
    /// Whether a statement that changes rows answers how many it
    /// changed, which the connection holds and the file does not.
    CountChanges,
    /// A pragma the file does not hold, which is answered and changes
    /// nothing: the cache size, how a write is synced, where a
    /// temporary table lives, and the two that ask for an older format.
    Ignored,
}

/// What the pragma of `name` says, or nothing where this crate does not
/// answer that pragma.
#[must_use]
pub fn of_name(name: &[u8]) -> Option<Setting> {
    let name: Vec<u8> = name.to_ascii_lowercase();
    Some(match name.as_slice() {
        b"page_size" => Setting::PageSize,
        b"reserved_bytes" => Setting::Reserved,
        b"encoding" => Setting::Encoding,
        b"auto_vacuum" => Setting::AutoVacuum,
        b"journal_mode" => Setting::JournalMode,
        b"page_count" => Setting::PageCount,
        b"freelist_count" => Setting::FreelistCount,
        b"schema_version" => Setting::SchemaVersion,
        b"user_version" => Setting::UserVersion,
        b"application_id" => Setting::ApplicationId,
        b"schema_format" => Setting::SchemaFormat,
        b"count_changes" => Setting::CountChanges,
        b"cache_size"
        | b"synchronous"
        | b"temp_store"
        | b"legacy_file_format"
        | b"legacy_alter_table"
        | b"short_column_names"
        | b"full_column_names"
        | b"empty_result_callbacks"
        | b"cache_spill"
        | b"defer_foreign_keys"
        | b"foreign_keys"
        | b"recursive_triggers"
        | b"ignore_check_constraints"
        | b"trusted_schema"
        | b"query_only" => Setting::Ignored,
        _ => return None,
    })
}

impl Setting {
    /// What a file whose header is `header` answers for this pragma, or
    /// nothing where the pragma answers no row.
    ///
    /// `PRAGMA journal_mode` answers `wal` for a file whose write
    /// version says so and `delete` for every other, because the file
    /// holds no other mode: the four that write the file itself leave
    /// the same write version.
    #[must_use]
    pub fn read(self, header: &Header) -> Option<Value> {
        let number = |value: u32| Value::Int(i64::from(value));
        Some(match self {
            Setting::PageSize => number(header.page_size),
            Setting::Reserved => Value::Int(i64::from(header.reserved)),
            Setting::Encoding => Value::Text(
                match header.encoding {
                    Encoding::Utf8 => b"UTF-8".as_slice(),
                    Encoding::Utf16Le => b"UTF-16le".as_slice(),
                    Encoding::Utf16Be => b"UTF-16be".as_slice(),
                }
                .to_vec(),
            ),
            Setting::AutoVacuum => {
                let full = u32::from(header.largest_root != 0);
                number(full.saturating_add(header.incremental_vacuum.min(1)))
            }
            Setting::JournalMode => Value::Text(
                if header.write_version == 2 {
                    b"wal".as_slice()
                } else {
                    b"delete".as_slice()
                }
                .to_vec(),
            ),
            Setting::PageCount => number(header.pages),
            Setting::FreelistCount => number(header.freelist_pages),
            Setting::SchemaVersion => number(header.schema_cookie),
            Setting::UserVersion => number(header.user_version),
            Setting::ApplicationId => number(header.application_id),
            Setting::SchemaFormat => number(header.schema_format),
            // A pragma the file does not hold, and the one the
            // connection holds, have no answer out of a header.
            Setting::CountChanges | Setting::Ignored => return None,
        })
    }
}

/// The encoding a `PRAGMA encoding` names, with the quotes and the
/// dashes it may be written with taken off.
#[must_use]
pub fn encoding_of(text: &[u8]) -> Option<Encoding> {
    let text: Vec<u8> = crate::schema::dequote(text)
        .iter()
        .filter(|byte| **byte != b'-')
        .map(u8::to_ascii_lowercase)
        .collect();
    match text.as_slice() {
        b"utf8" => Some(Encoding::Utf8),
        b"utf16le" => Some(Encoding::Utf16Le),
        b"utf16be" => Some(Encoding::Utf16Be),
        _ => None,
    }
}

/// What a `PRAGMA auto_vacuum` names: nought for none, one for a file
/// that vacuums itself whole, two for one that vacuums a step at a
/// time.
#[must_use]
pub fn vacuum_of(text: &[u8]) -> Option<u32> {
    let text: Vec<u8> = crate::schema::dequote(text).to_ascii_lowercase();
    match text.as_slice() {
        b"none" | b"0" | b"off" | b"false" => Some(0),
        b"full" | b"1" => Some(1),
        b"incremental" | b"2" => Some(2),
        _ => None,
    }
}

/// The mode a `PRAGMA journal_mode` names, or nothing where it names
/// the write-ahead log or no mode at all.
#[must_use]
pub fn mode_of(text: &[u8]) -> Option<crate::journal::Mode> {
    let text: Vec<u8> = crate::schema::dequote(text).to_ascii_lowercase();
    match text.as_slice() {
        b"delete" => Some(crate::journal::Mode::Delete),
        b"truncate" => Some(crate::journal::Mode::Truncate),
        b"persist" => Some(crate::journal::Mode::Persist),
        b"memory" => Some(crate::journal::Mode::Memory),
        b"off" => Some(crate::journal::Mode::Off),
        _ => None,
    }
}

/// What a pragma set to a truth value is set to, which is
/// `sqlite3GetBoolean`: a number is true where it is not nought, and
/// four words say so in letters.
#[must_use]
pub fn truth(text: &[u8]) -> Option<bool> {
    let text: Vec<u8> = crate::schema::dequote(text).to_ascii_lowercase();
    match text.as_slice() {
        b"on" | b"yes" | b"true" => Some(true),
        b"off" | b"no" | b"false" => Some(false),
        _ => whole_number(&text).map(|number| number != 0),
    }
}

/// Whether a `PRAGMA journal_mode` names write-ahead logging, which is
/// the sixth mode and the one that writes no rollback journal.
#[must_use]
pub fn is_log(text: &[u8]) -> bool {
    crate::schema::dequote(text).eq_ignore_ascii_case(b"wal")
}

/// The whole number a pragma is set to, or nothing where what is
/// written is not one.
#[must_use]
pub fn whole_number(text: &[u8]) -> Option<u32> {
    let mut out: u32 = 0;
    for byte in text {
        let digit = byte.checked_sub(b'0').filter(|digit| *digit < 10)?;
        out = out
            .checked_mul(10)
            .and_then(|shifted| shifted.checked_add(u32::from(digit)))?;
    }
    Some(out)
}
