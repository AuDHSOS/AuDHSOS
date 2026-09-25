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
    /// `PRAGMA [schema.]locking_mode`, which every database of the
    /// connection holds one of and no byte of the file carries.
    LockingMode,
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
    /// `PRAGMA wal_checkpoint`, which moves the frames of the log into
    /// the database file.
    WalCheckpoint,
    /// `PRAGMA case_sensitive_like`, which sets whether `LIKE` tells
    /// the twenty-six letters apart and answers no row either way,
    /// because `PragFlg_NoColumns` stands against its name in
    /// `research/sqlite/pragma.h:200`.
    CaseSensitiveLike,
    /// `PRAGMA table_info(table)`, which answers one row per column of a
    /// table that is not computed, and is `PragTyp_TABLE_INFO` of
    /// `research/sqlite/src/pragma.c:1211`.
    TableInfo,
    /// The same with every column, computed or not, and a seventh column
    /// saying which of the two each is.
    TableXinfo,
    /// `PRAGMA index_info(index)`, which answers one row per place of an
    /// index.
    IndexInfo,
    /// The same with the order, the collation and whether the place is one
    /// the entries are held in the order of.
    IndexXinfo,
    /// `PRAGMA index_list(table)`, which answers one row per index over a
    /// table.
    IndexList,
    /// `PRAGMA collation_list`, which answers one row per collation the
    /// connection holds.
    CollationList,
    /// `PRAGMA compile_options`, which answers one row per option the
    /// library was built with, and is `PragTyp_COMPILE_OPTIONS` of
    /// `research/sqlite/src/pragma.c:1105`.
    CompileOptions,
    /// `PRAGMA database_list`, which answers one row per database the
    /// connection holds, and is `PragTyp_DATABASE_LIST` of
    /// `research/sqlite/src/pragma.c:1436`.
    DatabaseList,
    /// `PRAGMA default_cache_size`, which writes the word at offset 48
    /// of the header and the connection's own cache size together, and
    /// is `PragTyp_DEFAULT_CACHE_SIZE` of
    /// `research/sqlite/src/pragma.c:553`.
    DefaultCacheSize,
    /// `PRAGMA lock_status`, which answers the lock each database of the
    /// connection is held under, and is `PragTyp_LOCK_STATUS` of
    /// `research/sqlite/src/pragma.c:1877`.
    LockStatus,
    /// `PRAGMA incremental_vacuum(N)`, which gives up as many as N pages
    /// at the end of a file that vacuums itself incrementally, and is
    /// `PragTyp_INCREMENTAL_VACUUM` of
    /// `research/sqlite/src/pragma.c:854`.
    IncrementalVacuum,
}

/// What a pragma the connection keeps a value for is written as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Written {
    /// A whole number.
    Number,
    /// A truth value, which is `sqlite3GetBoolean` with nought for a
    /// word it does not know.
    Truth,
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

/// What a connection was told for the pragmas it keeps a value for.
///
/// A caller that holds one writer per file and several connections over
/// it carries one of these per connection, because the values belong to
/// a connection and the pages belong to a file. A connection that was
/// told nothing carries the default, which every pragma answers its own
/// fallback for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kept(pub(crate) Vec<Option<i64>>);

impl Kept {
    /// Whether `PRAGMA fullfsync` is on, which says a sync holds the file
    /// on the disk of the machine and not only in the cache of its
    /// driver, and which `sqlite3_fullsync_count` of
    /// `research/sqlite/src/os_unix.c` counts the syncs made under.
    #[must_use]
    pub fn fullfsync(&self) -> bool {
        HELD.iter()
            .position(|keeps| keeps.name == b"fullfsync")
            .and_then(|at| self.0.get(at).copied().flatten())
            .is_some_and(|value| value != 0)
    }

    /// The connection told `value` for `name`, whatever a statement may
    /// set it to.
    ///
    /// A caller that holds one writer per file and several connections
    /// over it raises `data_version` this way, because the value belongs
    /// to a connection and the commits belong to the file. Reading the
    /// list costs O(n) in the pragmas it holds.
    pub fn tells(&mut self, name: &[u8], value: i64) {
        let Some(at) = HELD.iter().position(|keeps| keeps.name == name) else {
            return;
        };
        self.0.resize(HELD.len(), None);
        for slot in self.0.iter_mut().skip(at).take(1) {
            *slot = Some(value);
        }
    }
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
        name: b"short_column_names",
        fallback: 1,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"full_column_names",
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
        // `sqlite3_open` writes `SQLITE_TrustedSchema` into the flags of
        // every connection, which `research/sqlite/src/main.c:3472`
        // leaves out only for a build under `SQLITE_TRUSTED_SCHEMA=0`.
        fallback: 1,
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
        fallback: CACHE_SIZE,
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
    // The three flags of the file system that this crate reads nothing
    // of: a connection answers what it was told and the file is written
    // the same way either way.
    Keeps {
        name: b"cell_size_check",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"checkpoint_fullfsync",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"fullfsync",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"empty_result_callbacks",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
    Keeps {
        name: b"legacy_alter_table",
        fallback: 0,
        written: Written::Truth,
        answers: false,
        fixed: false,
    },
];

/// The value a pragma the connection keeps is set to, or nothing where
/// what is written is not one of its values.
#[must_use]
pub fn keeping(at: usize, text: &[u8]) -> Option<i64> {
    let written: Vec<u8> = crate::schema::dequote(text).to_ascii_lowercase();
    match HELD.get(at).map(|keeps| keeps.written) {
        // `PragTyp_FLAG` reads `sqlite3GetBoolean(zRight, 0)`, so a
        // word that names no truth value turns the flag off.
        Some(Written::Truth) => Some(i64::from(truth(&written).unwrap_or(false))),
        Some(Written::Secure) => match written.as_slice() {
            b"fast" => Some(2),
            _ => truth(&written).map(i64::from),
        },
        // `getSafetyLevel` of `research/sqlite/src/pragma.c:72` reads a
        // number where the first byte is a digit, else one of eight
        // words, else the level a connection opens under, which is one.
        Some(Written::Syncing) => Some(match written.as_slice() {
            b"full" => 2,
            b"extra" => 3,
            held => match held.first().filter(|byte| byte.is_ascii_digit()) {
                Some(_) => signed_number(held).unwrap_or(1),
                None => i64::from(truth(held).unwrap_or(true)),
            },
        }),
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
#[must_use]
pub fn signed_number(text: &[u8]) -> Option<i64> {
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
        b"locking_mode" => Setting::LockingMode,
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
        b"database_list" => Setting::DatabaseList,
        b"table_info" => Setting::TableInfo,
        b"table_xinfo" => Setting::TableXinfo,
        b"index_info" => Setting::IndexInfo,
        b"index_xinfo" => Setting::IndexXinfo,
        b"index_list" => Setting::IndexList,
        b"collation_list" => Setting::CollationList,
        b"foreign_key_check" => Setting::ForeignKeyCheck,
        b"wal_checkpoint" => Setting::WalCheckpoint,
        b"case_sensitive_like" => Setting::CaseSensitiveLike,
        b"default_cache_size" => Setting::DefaultCacheSize,
        b"incremental_vacuum" => Setting::IncrementalVacuum,
        b"lock_status" => Setting::LockStatus,
        // `PRAGMA default_synchronous` is a pragma no version of the
        // library still holds, and `sqlite3Pragma` answers no row for a
        // name it does not know.
        b"default_synchronous"
        | b"legacy_file_format"
        | b"cache_spill"
        | b"shrink_memory"
        | b"optimize" => Setting::Ignored,
        b"compile_options" => Setting::CompileOptions,
        _ => {
            if let Some(at) = HELD.iter().position(|keeps| keeps.name == name.as_slice()) {
                Setting::Held(at)
            } else if KNOWN.contains(&name.as_slice()) {
                // A pragma the C library holds and this crate does not
                // write is refused rather than left out, because a
                // statement that named it asked for something.
                return None;
            } else {
                // `sqlite3Pragma` looks the name up in the table
                // `research/sqlite/pragma.h` holds and answers no row for
                // a name that is not in it, so `PRAGMA autovacuum`, which
                // no version of the library ever held, changes nothing.
                Setting::Ignored
            }
        }
    })
}

/// The names the pragma table of `research/sqlite/pragma.h` holds, which
/// `sqlite3Pragma` of `research/sqlite/src/pragma.c` looks a name up in:
/// a name that is in it and that this crate does not write is refused,
/// and a name that is not in it changes nothing.
///
/// `parser_trace`, `sql_trace`, `vdbe_addoptrace`, `vdbe_debug`,
/// `vdbe_eqp`, `vdbe_listing` and `vdbe_trace` stand in that table under
/// `SQLITE_DEBUG` alone, which writes the program of a statement out as
/// it runs; this crate builds no program and holds none of the seven, so
/// each of them changes nothing.
const KNOWN: [&[u8]; 71] = [
    b"activate_extensions",
    b"analysis_limit",
    b"application_id",
    b"auto_vacuum",
    b"automatic_index",
    b"busy_timeout",
    b"cache_size",
    b"cache_spill",
    b"case_sensitive_like",
    b"cell_size_check",
    b"checkpoint_fullfsync",
    b"collation_list",
    b"compile_options",
    b"count_changes",
    b"data_store_directory",
    b"data_version",
    b"database_list",
    b"default_cache_size",
    b"defer_foreign_keys",
    b"empty_result_callbacks",
    b"encoding",
    b"foreign_key_check",
    b"foreign_key_list",
    b"foreign_keys",
    b"freelist_count",
    b"full_column_names",
    b"fullfsync",
    b"function_list",
    b"hard_heap_limit",
    b"ignore_check_constraints",
    b"incremental_vacuum",
    b"index_info",
    b"index_list",
    b"index_xinfo",
    b"integrity_check",
    b"journal_mode",
    b"journal_size_limit",
    b"legacy_alter_table",
    b"lock_proxy_file",
    b"lock_status",
    b"locking_mode",
    b"max_page_count",
    b"mmap_size",
    b"module_list",
    b"optimize",
    b"page_count",
    b"page_size",
    b"pragma_list",
    b"query_only",
    b"quick_check",
    b"read_uncommitted",
    b"recursive_triggers",
    b"reverse_unordered_selects",
    b"schema_version",
    b"secure_delete",
    b"short_column_names",
    b"shrink_memory",
    b"soft_heap_limit",
    b"stats",
    b"synchronous",
    b"table_info",
    b"table_list",
    b"table_xinfo",
    b"temp_store",
    b"temp_store_directory",
    b"threads",
    b"trusted_schema",
    b"user_version",
    b"wal_autocheckpoint",
    b"wal_checkpoint",
    b"writable_schema",
];

impl Setting {
    /// The columns this pragma answers, each by name, and none where it
    /// answers no column. `valued` says whether a value follows the name.
    ///
    /// `setPragmaResultColumnNames` of
    /// `research/sqlite/src/pragma.c:207` sets the names `pragCName`
    /// holds where the pragma names columns of its own, and one column
    /// named after the pragma where it does not. `PragFlg_NoColumns`
    /// answers none at all, and `PragFlg_NoColumns1` none where a value
    /// follows the name, which every pragma written `TYPE: FLAG` in
    /// `research/sqlite/tool/mkpragmatab.tcl` carries.
    #[must_use]
    pub fn columns(self, name: &[u8], valued: bool) -> Vec<Vec<u8>> {
        let named = |words: &[&[u8]]| words.iter().map(|word| word.to_vec()).collect();
        match self {
            Setting::TableInfo => {
                return named(&[b"cid", b"name", b"type", b"notnull", b"dflt_value", b"pk"]);
            }
            Setting::TableXinfo => {
                return named(&[
                    b"cid",
                    b"name",
                    b"type",
                    b"notnull",
                    b"dflt_value",
                    b"pk",
                    b"hidden",
                ]);
            }
            Setting::IndexInfo => return named(&[b"seqno", b"cid", b"name"]),
            Setting::IndexXinfo => {
                return named(&[b"seqno", b"cid", b"name", b"desc", b"coll", b"key"]);
            }
            Setting::IndexList => {
                return named(&[b"seq", b"name", b"unique", b"origin", b"partial"]);
            }
            Setting::CollationList => return named(&[b"seq", b"name"]),
            Setting::CompileOptions => return named(&[b"compile_options"]),
            Setting::DatabaseList => return named(&[b"seq", b"name", b"file"]),
            Setting::ForeignKeyList => {
                return named(&[
                    b"id",
                    b"seq",
                    b"table",
                    b"from",
                    b"to",
                    b"on_update",
                    b"on_delete",
                    b"match",
                ]);
            }
            Setting::ForeignKeyCheck => return named(&[b"table", b"rowid", b"parent", b"fkid"]),
            Setting::WalCheckpoint => return named(&[b"busy", b"log", b"checkpointed"]),
            // `PRAGMA case_sensitive_like` and `PRAGMA
            // incremental_vacuum` answer no column, which the
            // `NoColumns` of `research/sqlite/tool/mkpragmatab.tcl:206`
            // and `:317` states, and the three names no version of the
            // library still holds answer none either.
            Setting::CaseSensitiveLike | Setting::IncrementalVacuum => return Vec::new(),
            // A name the pragma table does not hold answers no column
            // either, because `sqlite3Pragma` answers no row for it.
            Setting::Ignored
                if name.eq_ignore_ascii_case(b"shrink_memory")
                    || !KNOWN.contains(&name.to_ascii_lowercase().as_slice()) =>
            {
                return Vec::new();
            }
            _ => {}
        }
        // `PRAGMA optimize` names its column whatever follows the name,
        // because `PragFlg_NoColumns1` does not stand against it.
        let named = name.eq_ignore_ascii_case(b"optimize");
        if valued && !named && self.none_valued() {
            return Vec::new();
        }
        alloc::vec![name.to_ascii_lowercase()]
    }

    /// Whether this pragma answers no column where a value follows its
    /// name, which is `PragFlg_NoColumns1`.
    const fn none_valued(self) -> bool {
        matches!(
            self,
            Setting::PageSize
                | Setting::Reserved
                | Setting::Encoding
                | Setting::AutoVacuum
                | Setting::SchemaVersion
                | Setting::UserVersion
                | Setting::ApplicationId
                | Setting::SchemaFormat
                | Setting::CountChanges
                | Setting::DefaultCacheSize
                | Setting::Held(_)
                | Setting::Ignored
        )
    }

    /// What a file whose header is `header` answers for this pragma, or
    /// nothing where the pragma answers no row.
    ///
    /// `PRAGMA journal_mode` is not among them: the file says only
    /// whether it is in write-ahead logging, and the four modes that
    /// write the file itself belong to the connection.
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
            Setting::PageCount => number(header.pages),
            Setting::FreelistCount => number(header.freelist_pages),
            Setting::SchemaVersion => number(header.schema_cookie),
            Setting::UserVersion => number(header.user_version),
            Setting::ApplicationId => number(header.application_id),
            Setting::SchemaFormat => number(header.schema_format),
            Setting::DefaultCacheSize => Value::Int(default_cache(header.cache_size)),
            // A pragma the file does not hold, the ones the connection
            // holds, and the two that walk the file rather than read
            // its header have no answer out of a header.
            Setting::JournalMode
            | Setting::LockingMode
            | Setting::CountChanges
            | Setting::Held(_)
            | Setting::Ignored
            | Setting::CaseSensitiveLike
            | Setting::Integrity
            | Setting::Quick
            | Setting::DatabaseList
            | Setting::TableInfo
            | Setting::TableXinfo
            | Setting::IndexInfo
            | Setting::IndexXinfo
            | Setting::IndexList
            | Setting::CollationList
            | Setting::CompileOptions
            | Setting::ForeignKeyList
            | Setting::ForeignKeyCheck
            | Setting::WalCheckpoint
            | Setting::LockStatus
            | Setting::IncrementalVacuum => return None,
        })
    }
}

/// Where `cache_size` stands in [`HELD`], which `PRAGMA
/// default_cache_size` sets along with the word at offset 48 of the
/// header.
pub const CACHED: usize = 22;

/// What a header word of `held` answers for `PRAGMA
/// default_cache_size` and for the cache size a connection was told
/// nothing for.
///
/// `research/sqlite/src/prepare.c:325` reads the word through
/// `sqlite3AbsInt32`: the word where it is above nought, its negation
/// where it is below, and [`CACHE_SIZE`] where it is nought.
#[must_use]
pub fn default_cache(held: u32) -> i64 {
    let signed = held.cast_signed();
    match signed {
        0 => CACHE_SIZE,
        _ => i64::from(signed.unsigned_abs()),
    }
}

/// What a connection over a file whose header word is nought holds,
/// which is `SQLITE_DEFAULT_CACHE_SIZE` of
/// `research/sqlite/src/sqliteLimit.h:161`.
pub const CACHE_SIZE: i64 = -2000;

/// The header word `PRAGMA default_cache_size = value` writes, which is
/// `sqlite3AbsInt32(sqlite3Atoi(zRight))` of
/// `research/sqlite/src/pragma.c:577`, with nought for text that names
/// no number.
#[must_use]
pub fn cache_word(text: &[u8]) -> u32 {
    let held = signed_number(&crate::schema::dequote(text).to_ascii_lowercase()).unwrap_or(0);
    i32::try_from(held).unwrap_or(0).unsigned_abs()
}

/// The header word `PRAGMA name = value` writes for the three pragmas
/// `PragTyp_HEADER_VALUE` writes, which is `sqlite3Atoi(zRight)` of
/// `research/sqlite/src/pragma.c:2340` kept as the bytes of a word of 32
/// bits, with nought for text that names no number.
#[must_use]
pub fn header_word(text: &[u8]) -> u32 {
    let held = signed_number(&crate::schema::dequote(text).to_ascii_lowercase()).unwrap_or(0);
    i32::try_from(held).unwrap_or(0).cast_unsigned()
}

/// The encoding a `PRAGMA encoding` names, read against `encnames` of
/// `research/sqlite/src/pragma.c:2249` with the quotes taken off, where
/// `UTF-16` and `UTF16` name [`Encoding::NATIVE`].
#[must_use]
pub fn encoding_of(text: &[u8]) -> Option<Encoding> {
    let text = crate::schema::dequote(text).to_ascii_lowercase();
    match text.as_slice() {
        b"utf8" | b"utf-8" => Some(Encoding::Utf8),
        b"utf-16le" | b"utf16le" => Some(Encoding::Utf16Le),
        b"utf-16be" | b"utf16be" => Some(Encoding::Utf16Be),
        b"utf-16" | b"utf16" => Some(Encoding::NATIVE),
        _ => None,
    }
}

/// What a `PRAGMA auto_vacuum` names: nought for none, one for a file
/// that vacuums itself whole, two for one that vacuums a step at a
/// time.
///
/// `getAutoVacuum` of `research/sqlite/src/pragma.c:126` reads the three
/// words, then the number the text begins with, and answers none for
/// every other text, so no value is refused.
#[must_use]
pub fn vacuum_of(text: &[u8]) -> u32 {
    let text: Vec<u8> = crate::schema::dequote(text).to_ascii_lowercase();
    match text.as_slice() {
        b"none" => 0,
        b"full" => 1,
        b"incremental" => 2,
        digits => signed_number(digits)
            .and_then(|number| u32::try_from(number).ok())
            .filter(|number| *number <= 2)
            .unwrap_or(0),
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

/// The word one journal mode is written as, which is what
/// `PRAGMA journal_mode` answers.
#[must_use]
pub const fn mode_word(mode: crate::journal::Mode) -> &'static [u8] {
    match mode {
        crate::journal::Mode::Delete => b"delete",
        crate::journal::Mode::Truncate => b"truncate",
        crate::journal::Mode::Persist => b"persist",
        crate::journal::Mode::Memory => b"memory",
        crate::journal::Mode::Off => b"off",
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

/// The options the library was built with, which `PRAGMA
/// compile_options` answers one row for and
/// `sqlite3_compileoption_get` names by place.
///
/// `sqlite3CompileOptions` of the generated `ctime.c` carries the list
/// the C library was built with, in alphabetical order, which a test
/// reads the order of. This crate is built with two: it refuses a `%00`
/// escape in a file name written as a URI, which
/// `SQLITE_ENABLE_URI_00_ERROR` says, and it holds no lock of its own,
/// which `SQLITE_THREADSAFE=0` says.
pub const BUILT: [&[u8]; 2] = [b"ENABLE_URI_00_ERROR", b"THREADSAFE=0"];

/// Whether the library was built with the option a name names, which
/// `sqlite3_compileoption_used` of `research/sqlite/src/main.c:5200`
/// answers.
///
/// The `SQLITE_` in front of the name is left off, the rest is compared
/// without regard to case, and a name matches an option that carries
/// more than it only where the byte after it opens no name, so
/// `THREADSAFE` names `THREADSAFE=0` and `THREADSAFE=` names nothing.
/// Reading the list costs O(n) in its length.
#[must_use]
pub fn built(name: &[u8]) -> bool {
    let held = match name.get(..7) {
        Some(front) if front.eq_ignore_ascii_case(b"SQLITE_") => name.get(7..).unwrap_or_default(),
        _ => name,
    };
    BUILT.iter().any(|option| {
        option
            .get(..held.len())
            .is_some_and(|front| front.eq_ignore_ascii_case(held))
            && option
                .get(held.len())
                .is_none_or(|byte| !crate::change::is_name_byte(*byte))
    })
}

/// The option at a place in the list, and nothing where no option
/// stands there, which `sqlite3_compileoption_get` answers null for.
///
/// Reading it costs O(1).
#[must_use]
pub fn built_at(at: i64) -> Option<&'static [u8]> {
    usize::try_from(at)
        .ok()
        .and_then(|at| BUILT.get(at).copied())
}
