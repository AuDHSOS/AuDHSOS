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
    /// A pragma the connection keeps a value for, which the file does
    /// not hold, at this place of [`HELD`].
    Held(usize),
    /// A pragma the file does not hold and the connection answers
    /// nothing for, which is accepted and changes nothing.
    Ignored,
    /// `PRAGMA integrity_check`, which answers one row per problem the
    /// file holds and one row of `ok` where it holds none.
    Integrity,
    /// `PRAGMA quick_check`, which is the same walk without holding an
    /// index to the rows it is over.
    Quick,
    /// `PRAGMA foreign_key_list(table)`, which answers one row per
    /// foreign key of that table.
    ForeignKeyList,
    /// `PRAGMA foreign_key_check`, which answers one row per row that
    /// points at no row.
    ForeignKeyCheck,
}

/// What a pragma the connection keeps a value for is written as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Written {
    /// A whole number.
    Number,
    /// A truth value, which is `sqlite3GetBoolean` with nought for a
    /// word it does not know.
    Truth,
    /// `normal` or `exclusive`, which the pragma answers back as the
    /// word it was written as.
    Locking,
    /// `off`, `on` or `fast`, which the pragma answers back as nought,
    /// one or two.
    Secure,
    /// `off`, `normal`, `full` or `extra`, which the pragma answers
    /// back as nought to three.
    Syncing,
    /// `file` or `memory`, which the pragma answers back as one or two.
    Storing,
}

/// One pragma the connection keeps a value for: its name, what a
/// connection that was told nothing answers, how it is written, and
/// whether setting it answers the value it was set to.
pub struct Keeps {
    /// The name.
    pub name: &'static [u8],
    /// What a connection that was told nothing answers.
    pub fallback: i64,
    /// How the value is written.
    pub written: Written,
    /// Whether setting it answers a row.
    pub answers: bool,
    /// Whether the value stands whatever a statement sets it to, which
    /// `data_version` does because only a write by another connection
    /// raises it and `mmap_size` does because this crate maps no file.
    pub fixed: bool,
}

/// The pragmas the connection keeps a value for, which are the ones
/// `sqlite3Pragma` answers out of the connection and no byte of the
/// file holds.
pub static HELD: &[Keeps] = &[
    Keeps {
        name: b"mmap_size",
        fallback: 0,
        written: Written::Number,
        answers: true,
        fixed: true,
    },
    Keeps {
        name: b"locking_mode",
        fallback: 0,
        written: Written::Locking,
        answers: true,
        fixed: false,
    },
    Keeps {
        name: b"secure_delete",
        fallback: 0,
        written: Written::Secure,
        answers: true,
        fixed: false,
    },
    Keeps {
        name: b"max_page_count",
        fallback: 4_294_967_294,
        written: Written::Number,
        answers: true,
        fixed: false,
    },
    Keeps {
        name: b"journal_size_limit",
        fallback: -1,
        written: Written::Number,
        answers: true,
        fixed: false,
    },
    Keeps {
        name: b"wal_autocheckpoint",
        fallback: 1000,
        written: Written::Number,
        answers: true,
        fixed: false,
    },
    Keeps {
        name: b"threads",
        fallback: 0,
        written: Written::Number,
        answers: true,
        fixed: false,
    },
    Keeps {
        name: b"analysis_limit",
        fallback: 0,
        written: Written::Number,
        answers: true,
        fixed: false,
    },
    Keeps {
        name: b"busy_timeout",
        fallback: 0,
        written: Written::Number,
        answers: true,
        fixed: false,
    },
    Keeps {
        name: b"query_only",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"read_uncommitted",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"ignore_check_constraints",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"automatic_index",
        fallback: 1,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"defer_foreign_keys",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"foreign_keys",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"recursive_triggers",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"reverse_unordered_selects",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"trusted_schema",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"writable_schema",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"synchronous",
        fallback: 2,
        written: Written::Syncing,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"temp_store",
        fallback: 0,
        written: Written::Storing,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"cache_size",
        fallback: -2000,
        written: Written::Number,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"default_cache_size",
        fallback: -2000,
        written: Written::Number,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"data_version",
        fallback: 1,
        written: Written::Number,
        answers: true,
        fixed: true,
    },
];

/// What a pragma the connection keeps answers for `value`.
#[must_use]
pub fn kept(at: usize, value: i64) -> Value {
    match HELD.get(at).map(|keeps| keeps.written) {
        Some(Written::Locking) => Value::Text(
            if value == 0 {
                b"normal".as_slice()
            } else {
                b"exclusive".as_slice()
            }
            .to_vec(),
        ),
        _ => Value::Int(value),
    }
}

/// The value a pragma the connection keeps is set to, or nothing where
/// what is written is not one of its values.
#[must_use]
pub fn keeping(at: usize, text: &[u8]) -> Option<i64> {
    let written: Vec<u8> = crate::schema::dequote(text).to_ascii_lowercase();
    match HELD.get(at).map(|keeps| keeps.written) {
        // `PragTyp_FLAG` reads `sqlite3GetBoolean(zRight, 0)`, so a
        // word that names no truth value turns the flag off.
        Some(Written::Truth) => Some(i64::from(truth(&written).unwrap_or(false))),
        Some(Written::Locking) => match written.as_slice() {
            b"normal" => Some(0),
            b"exclusive" => Some(1),
            _ => None,
        },
        Some(Written::Secure) => match written.as_slice() {
            b"fast" => Some(2),
            _ => truth(&written).map(i64::from),
        },
        Some(Written::Syncing) => match written.as_slice() {
            b"off" => Some(0),
            b"normal" => Some(1),
            b"full" => Some(2),
            b"extra" => Some(3),
            _ => signed_number(&written),
        },
        Some(Written::Storing) => match written.as_slice() {
            b"default" => Some(0),
            b"file" => Some(1),
            b"memory" => Some(2),
            _ => signed_number(&written),
        },
        // A whole number, and a place [`HELD`] does not have.
        _ => signed_number(&written),
    }
}

/// The whole number a pragma is set to, with the minus sign a cache
/// size and a size limit may carry.
fn signed_number(text: &[u8]) -> Option<i64> {
    let (negative, digits) = match text.strip_prefix(b"-") {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let mut out: i64 = 0;
    for byte in digits {
        let digit = byte.checked_sub(b'0').filter(|digit| *digit < 10)?;
        out = out
            .checked_mul(10)
            .and_then(|shifted| shifted.checked_add(i64::from(digit)))?;
    }
    if digits.is_empty() {
        return None;
    }
    Some(if negative { out.wrapping_neg() } else { out })
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
        b"integrity_check" => Setting::Integrity,
        b"quick_check" => Setting::Quick,
        b"foreign_key_list" => Setting::ForeignKeyList,
        b"foreign_key_check" => Setting::ForeignKeyCheck,
        b"legacy_file_format"
        | b"legacy_alter_table"
        | b"short_column_names"
        | b"full_column_names"
        | b"empty_result_callbacks"
        | b"cache_spill"
        | b"case_sensitive_like"
        | b"shrink_memory"
        | b"optimize" => Setting::Ignored,
        _ => {
            let at = HELD
                .iter()
                .position(|keeps| keeps.name == name.as_slice())?;
            Setting::Held(at)
        }
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
            // A pragma the file does not hold, the ones the connection
            // holds, and the two that walk the file rather than read
            // its header have no answer out of a header.
            Setting::CountChanges
            | Setting::Held(_)
            | Setting::Ignored
            | Setting::Integrity
            | Setting::Quick
            | Setting::ForeignKeyList
            | Setting::ForeignKeyCheck => return None,
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
