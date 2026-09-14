// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.
//!
//! The fixtures under `fixtures/` were written by the `sqlite3` shell
//! itself, which makes them the format as it is and not as this crate
//! reads it. What produced each is named where it is used.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

mod bytes;
mod db;
mod definition;
mod error;
mod eval;
mod fp;
mod header;
mod image;
mod index;
mod journal;
mod matrix;
mod number;
mod page;
mod parse;
mod record;
mod schema;
mod token;
mod tree;
mod utf8;
mod value;
mod wal;

/// One table, three rows, every storage class, page size 4096, UTF-8.
///
/// `CREATE TABLE t(a INTEGER, b TEXT, c REAL, d BLOB); INSERT INTO t
/// VALUES (1,'one',1.5,x'0102'), (2,'two',-2.5,NULL), (-3,'',0.0,x'ff');`
pub(super) const SMALL: &[u8] = include_bytes!("fixtures/small.db");

/// A table of four hundred rows over pages of five hundred and twelve
/// bytes, which is one interior page over the leaves it split into.
pub(super) const TALL: &[u8] = include_bytes!("fixtures/tall.db");

/// The same rows put in by a key that jumps about, so that every insert
/// lands in the middle of a page.
pub(super) const SHUFFLED: &[u8] = include_bytes!("fixtures/shuffled.db");

/// Four thousand rows put in by a key that jumps about, which fills the
/// root of the tree, grows a third level under it, and then balances the
/// interior pages of that level against each other.
pub(super) const DEEP: &[u8] = include_bytes!("fixtures/deep.db");

/// The same four hundred rows with every third one taken out again,
/// which evens the leaves out and frees no page.
pub(super) const DELETED: &[u8] = include_bytes!("fixtures/deleted.db");

/// The same four hundred rows with three in four taken out again, which
/// joins leaves and puts the pages it frees on the free list.
pub(super) const EMPTIED: &[u8] = include_bytes!("fixtures/emptied.db");

/// The same four hundred rows with every one taken out again, which
/// leaves the root of the table a leaf with no cell on it.
pub(super) const CLEARED: &[u8] = include_bytes!("fixtures/cleared.db");

/// Forty rows whose payloads run onto overflow pages, with two in three
/// taken out again, which frees the chains they ran onto.
pub(super) const UNCHAINED: &[u8] = include_bytes!("fixtures/unchained.db");

use crate::header::Encoding;
use crate::journal::Mode;

/// What a commit of the covering array leaves beside the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keeping {
    /// A rollback journal under the mode given.
    Journal(Mode),
    /// A write-ahead log.
    Log,
}

/// One row of the covering array: a configuration and the files the
/// shell wrote under it.
pub(crate) struct Configuration {
    /// What the database file is called.
    pub(crate) name: &'static str,
    /// The encoding its text is in.
    pub(crate) encoding: Encoding,
    /// The page size it was written with.
    pub(crate) page_size: u32,
    /// How many bytes of every page the b-tree layer may not use.
    pub(crate) reserved: u8,
    /// What its commits leave beside the file.
    pub(crate) keeping: Keeping,
    /// Whether the file vacuums itself, and a step at a time where it
    /// does.
    pub(crate) vacuum: Option<bool>,
    /// The database file the shell wrote.
    pub(crate) bytes: &'static [u8],
    /// The journal or the log beside it, where the mode leaves one.
    pub(crate) beside: Option<&'static [u8]>,
}

/// The schema-format dimension of document 16, section 16.11. No pragma
/// the shell takes asks for a format below four, so the oracle asks for
/// it through `SQLITE_DBCONFIG_LEGACY_FILE_FORMAT`.
///
/// Format 1 stores the whole numbers 0 and 1 as a byte each and ignores
/// the `DESC` of an index.
pub(super) const FORMAT1: &[u8] = include_bytes!("fixtures/format1.db");

/// Format 3, which `ALTER TABLE ADD COLUMN` raises the file to: the rows
/// were written before the fourth and fifth columns, so each is short by
/// two, and the two answer what they fall back to, which is seven for
/// one and nothing for the other. Format 2 has no fixture, because this
/// library writes 3 wherever the file format document allows 2.
pub(super) const FORMAT3: &[u8] = include_bytes!("fixtures/format3.db");

/// Format 4, which the shell writes by default: the same rows and the
/// same added columns, with 0 and 1 stored as serial types 8 and 9.
pub(super) const FORMAT4: &[u8] = include_bytes!("fixtures/format4.db");

/// The five affinities in one table, each column holding the same three
/// values, which is what the comparison rules are read against.
pub(super) const AFFINITY: &[u8] = include_bytes!("fixtures/affinity.db");

/// Columns that fall back, with a row that names one of the three, so
/// the other two hold what they fall back to rather than nothing.
pub(super) const DEFAULTS: &[u8] = include_bytes!("fixtures/defaults.db");

/// The matrix of document 16, section 16.11, over the write path as a
/// covering array: thirty configurations in which every value of every
/// dimension appears and every pair of values from two dimensions
/// appears together at least once.
///
/// The full cross of the five dimensions is 3 x 5 x 3 x 6 x 3 = 810.
/// Thirty is the fewest rows a pair-covering array of them can have,
/// because the two widest dimensions are five and six values wide, and
/// `every_pair_of_the_matrix_is_written_under_some_configuration` is
/// what says the thirty cover the pairs.
pub(crate) const ARRAY: [Configuration; 30] = [
    Configuration {
        name: "x-01.db",
        encoding: Encoding::Utf16Le,
        page_size: 512,
        reserved: 32,
        keeping: Keeping::Journal(Mode::Delete),
        vacuum: Some(true),
        bytes: include_bytes!("fixtures/x-01.db"),
        beside: None,
    },
    Configuration {
        name: "x-02.db",
        encoding: Encoding::Utf16Le,
        page_size: 512,
        reserved: 4,
        keeping: Keeping::Journal(Mode::Memory),
        vacuum: None,
        bytes: include_bytes!("fixtures/x-02.db"),
        beside: None,
    },
    Configuration {
        name: "x-03.db",
        encoding: Encoding::Utf16Be,
        page_size: 512,
        reserved: 32,
        keeping: Keeping::Journal(Mode::Off),
        vacuum: Some(true),
        bytes: include_bytes!("fixtures/x-03.db"),
        beside: None,
    },
    Configuration {
        name: "x-04.db",
        encoding: Encoding::Utf8,
        page_size: 512,
        reserved: 0,
        keeping: Keeping::Journal(Mode::Persist),
        vacuum: None,
        bytes: include_bytes!("fixtures/x-04.db"),
        beside: Some(include_bytes!("fixtures/x-04.db-journal")),
    },
    Configuration {
        name: "x-05.db",
        encoding: Encoding::Utf16Le,
        page_size: 512,
        reserved: 32,
        keeping: Keeping::Journal(Mode::Truncate),
        vacuum: None,
        bytes: include_bytes!("fixtures/x-05.db"),
        beside: Some(include_bytes!("fixtures/x-05.db-journal")),
    },
    Configuration {
        name: "x-06.db",
        encoding: Encoding::Utf16Le,
        page_size: 512,
        reserved: 4,
        keeping: Keeping::Log,
        vacuum: Some(false),
        bytes: include_bytes!("fixtures/x-06.db"),
        beside: Some(include_bytes!("fixtures/x-06.db-wal")),
    },
    Configuration {
        name: "x-07.db",
        encoding: Encoding::Utf16Le,
        page_size: 1024,
        reserved: 32,
        keeping: Keeping::Journal(Mode::Delete),
        vacuum: None,
        bytes: include_bytes!("fixtures/x-07.db"),
        beside: None,
    },
    Configuration {
        name: "x-08.db",
        encoding: Encoding::Utf16Be,
        page_size: 1024,
        reserved: 0,
        keeping: Keeping::Journal(Mode::Memory),
        vacuum: Some(true),
        bytes: include_bytes!("fixtures/x-08.db"),
        beside: None,
    },
    Configuration {
        name: "x-09.db",
        encoding: Encoding::Utf8,
        page_size: 1024,
        reserved: 32,
        keeping: Keeping::Journal(Mode::Off),
        vacuum: None,
        bytes: include_bytes!("fixtures/x-09.db"),
        beside: None,
    },
    Configuration {
        name: "x-10.db",
        encoding: Encoding::Utf16Be,
        page_size: 1024,
        reserved: 32,
        keeping: Keeping::Journal(Mode::Persist),
        vacuum: None,
        bytes: include_bytes!("fixtures/x-10.db"),
        beside: Some(include_bytes!("fixtures/x-10.db-journal")),
    },
    Configuration {
        name: "x-11.db",
        encoding: Encoding::Utf8,
        page_size: 1024,
        reserved: 4,
        keeping: Keeping::Journal(Mode::Truncate),
        vacuum: Some(true),
        bytes: include_bytes!("fixtures/x-11.db"),
        beside: Some(include_bytes!("fixtures/x-11.db-journal")),
    },
    Configuration {
        name: "x-12.db",
        encoding: Encoding::Utf16Be,
        page_size: 1024,
        reserved: 0,
        keeping: Keeping::Log,
        vacuum: Some(false),
        bytes: include_bytes!("fixtures/x-12.db"),
        beside: Some(include_bytes!("fixtures/x-12.db-wal")),
    },
    Configuration {
        name: "x-13.db",
        encoding: Encoding::Utf16Be,
        page_size: 4096,
        reserved: 0,
        keeping: Keeping::Journal(Mode::Delete),
        vacuum: None,
        bytes: include_bytes!("fixtures/x-13.db"),
        beside: None,
    },
    Configuration {
        name: "x-14.db",
        encoding: Encoding::Utf16Be,
        page_size: 4096,
        reserved: 0,
        keeping: Keeping::Journal(Mode::Memory),
        vacuum: Some(true),
        bytes: include_bytes!("fixtures/x-14.db"),
        beside: None,
    },
    Configuration {
        name: "x-15.db",
        encoding: Encoding::Utf8,
        page_size: 4096,
        reserved: 4,
        keeping: Keeping::Journal(Mode::Off),
        vacuum: Some(false),
        bytes: include_bytes!("fixtures/x-15.db"),
        beside: None,
    },
    Configuration {
        name: "x-16.db",
        encoding: Encoding::Utf16Le,
        page_size: 4096,
        reserved: 32,
        keeping: Keeping::Journal(Mode::Persist),
        vacuum: Some(true),
        bytes: include_bytes!("fixtures/x-16.db"),
        beside: Some(include_bytes!("fixtures/x-16.db-journal")),
    },
    Configuration {
        name: "x-17.db",
        encoding: Encoding::Utf16Le,
        page_size: 4096,
        reserved: 32,
        keeping: Keeping::Journal(Mode::Truncate),
        vacuum: Some(false),
        bytes: include_bytes!("fixtures/x-17.db"),
        beside: Some(include_bytes!("fixtures/x-17.db-journal")),
    },
    Configuration {
        name: "x-18.db",
        encoding: Encoding::Utf8,
        page_size: 4096,
        reserved: 32,
        keeping: Keeping::Log,
        vacuum: None,
        bytes: include_bytes!("fixtures/x-18.db"),
        beside: Some(include_bytes!("fixtures/x-18.db-wal")),
    },
    Configuration {
        name: "x-19.db",
        encoding: Encoding::Utf8,
        page_size: 8192,
        reserved: 0,
        keeping: Keeping::Journal(Mode::Delete),
        vacuum: Some(false),
        bytes: include_bytes!("fixtures/x-19.db"),
        beside: None,
    },
    Configuration {
        name: "x-20.db",
        encoding: Encoding::Utf16Le,
        page_size: 8192,
        reserved: 4,
        keeping: Keeping::Journal(Mode::Memory),
        vacuum: None,
        bytes: include_bytes!("fixtures/x-20.db"),
        beside: None,
    },
    Configuration {
        name: "x-21.db",
        encoding: Encoding::Utf16Le,
        page_size: 8192,
        reserved: 4,
        keeping: Keeping::Journal(Mode::Off),
        vacuum: None,
        bytes: include_bytes!("fixtures/x-21.db"),
        beside: None,
    },
    Configuration {
        name: "x-22.db",
        encoding: Encoding::Utf16Le,
        page_size: 8192,
        reserved: 32,
        keeping: Keeping::Journal(Mode::Persist),
        vacuum: Some(true),
        bytes: include_bytes!("fixtures/x-22.db"),
        beside: Some(include_bytes!("fixtures/x-22.db-journal")),
    },
    Configuration {
        name: "x-23.db",
        encoding: Encoding::Utf16Be,
        page_size: 8192,
        reserved: 32,
        keeping: Keeping::Journal(Mode::Truncate),
        vacuum: None,
        bytes: include_bytes!("fixtures/x-23.db"),
        beside: Some(include_bytes!("fixtures/x-23.db-journal")),
    },
    Configuration {
        name: "x-24.db",
        encoding: Encoding::Utf16Le,
        page_size: 8192,
        reserved: 0,
        keeping: Keeping::Log,
        vacuum: Some(true),
        bytes: include_bytes!("fixtures/x-24.db"),
        beside: Some(include_bytes!("fixtures/x-24.db-wal")),
    },
    Configuration {
        name: "x-25.db",
        encoding: Encoding::Utf16Be,
        page_size: 65536,
        reserved: 4,
        keeping: Keeping::Journal(Mode::Delete),
        vacuum: Some(true),
        bytes: include_bytes!("fixtures/x-25.db"),
        beside: None,
    },
    Configuration {
        name: "x-26.db",
        encoding: Encoding::Utf8,
        page_size: 65536,
        reserved: 32,
        keeping: Keeping::Journal(Mode::Memory),
        vacuum: Some(false),
        bytes: include_bytes!("fixtures/x-26.db"),
        beside: None,
    },
    Configuration {
        name: "x-27.db",
        encoding: Encoding::Utf16Le,
        page_size: 65536,
        reserved: 0,
        keeping: Keeping::Journal(Mode::Off),
        vacuum: None,
        bytes: include_bytes!("fixtures/x-27.db"),
        beside: None,
    },
    Configuration {
        name: "x-28.db",
        encoding: Encoding::Utf16Be,
        page_size: 65536,
        reserved: 4,
        keeping: Keeping::Journal(Mode::Persist),
        vacuum: Some(false),
        bytes: include_bytes!("fixtures/x-28.db"),
        beside: Some(include_bytes!("fixtures/x-28.db-journal")),
    },
    Configuration {
        name: "x-29.db",
        encoding: Encoding::Utf16Le,
        page_size: 65536,
        reserved: 0,
        keeping: Keeping::Journal(Mode::Truncate),
        vacuum: Some(false),
        bytes: include_bytes!("fixtures/x-29.db"),
        beside: Some(include_bytes!("fixtures/x-29.db-journal")),
    },
    Configuration {
        name: "x-30.db",
        encoding: Encoding::Utf16Le,
        page_size: 65536,
        reserved: 32,
        keeping: Keeping::Log,
        vacuum: Some(true),
        bytes: include_bytes!("fixtures/x-30.db"),
        beside: Some(include_bytes!("fixtures/x-30.db-wal")),
    },
];

/// The index trees a `CREATE INDEX` writes, each with the statements
/// that make it: over no row at all, over a few, over terms with a
/// collation and an order of their own, over enough rows to fill a
/// second leaf, and over enough to need a page above the leaves.
///
/// `rows` says which set of rows the table holds: none, the three of
/// `small`, two that share a key and one that does not in `repeated`,
/// one whose key runs onto a chain in `wide`, the sixty of `split`, or
/// the four hundred of `shuffled.db`.
pub(crate) const TREES: [(&str, &str, &str, &[u8]); 7] = [
    (
        "index-empty.db",
        "none",
        "CREATE INDEX ta ON t(a)",
        include_bytes!("fixtures/index-empty.db"),
    ),
    (
        "index-few.db",
        "small",
        "CREATE INDEX ta ON t(a)",
        include_bytes!("fixtures/index-few.db"),
    ),
    (
        "index-collated.db",
        "small",
        "CREATE INDEX tb ON t(b COLLATE nocase, a DESC)",
        include_bytes!("fixtures/index-collated.db"),
    ),
    (
        "index-split.db",
        "split",
        "CREATE INDEX ts ON t(s)",
        include_bytes!("fixtures/index-split.db"),
    ),
    (
        "index-deep.db",
        "shuffled",
        "CREATE INDEX ts ON t(s)",
        include_bytes!("fixtures/index-deep.db"),
    ),
    (
        "index-repeated.db",
        "repeated",
        "CREATE INDEX ta ON t(a)",
        include_bytes!("fixtures/index-repeated.db"),
    ),
    (
        "index-wide.db",
        "wide",
        "CREATE INDEX ta ON t(a)",
        include_bytes!("fixtures/index-wide.db"),
    ),
];

/// The entries an index gains as rows are put in after it: a table with
/// no row when the index is made, and a table of four hundred whose
/// index has a page above its leaves.
pub(super) const KEPT: &[u8] = include_bytes!("fixtures/index-kept.db");

/// An index over a column holding every storage class, with two rows
/// that share a value in three of them, which is what makes the walk
/// compare every kind and fall through to the key of the row.
pub(super) const CLASSES: &[u8] = include_bytes!("fixtures/index-classes.db");

/// The same over four hundred rows, where one of the two rows added
/// lands in a leaf that is not the last.
pub(super) const ADDED: &[u8] = include_bytes!("fixtures/index-added.db");

/// The entries a `DELETE` takes out of an index over sixty rows, which
/// is a tree of two leaves under a root, so the balance joins pages.
pub(super) const GONE: &[u8] = include_bytes!("fixtures/index-gone.db");

/// The same over four hundred rows, whose index has a page above its
/// leaves, so half the entries come off leaves and the rest replace
/// entries of the page above, which is the branch of
/// `sqlite3BtreeDelete` that moves the entry before the lost one up.
pub(super) const HOLLOW: &[u8] = include_bytes!("fixtures/index-hollow.db");

/// Every row of an indexed table taken out, which leaves the index one
/// empty root page.
pub(super) const SWEPT: &[u8] = include_bytes!("fixtures/index-emptied.db");

/// The entries an `UPDATE` writes again, which come out under the term
/// the row held and go in under the term it is given.
pub(super) const SHIFTED: &[u8] = include_bytes!("fixtures/index-moved.db");

/// An `UPDATE` that writes the key of a row, so the entry comes out
/// under the old key and goes in under the new one.
pub(super) const REKEYED: &[u8] = include_bytes!("fixtures/index-rekeyed.db");

/// An index over the column the key is another name for, whose entries
/// hold the key twice, and a `DELETE` that finds one of them.
pub(super) const ALIAS: &[u8] = include_bytes!("fixtures/index-alias.db");

/// Nine hundred rows whose text runs from four bytes to fifty-seven,
/// which makes an index three levels deep whose entries differ in
/// length by more than one of them takes, so a `DELETE` moves an entry
/// up into a page the entry it replaces will not fit on.
pub(super) const TALL_INDEX: &[u8] = include_bytes!("fixtures/index-tall.db");

/// What a `DROP` leaves: one table of two rows taken out beside another
/// that stays, four hundred rows whose tree has a page above its
/// leaves, two rows that run onto chains, a table an index is over, the
/// index alone, and the table whose root is the last of the file.
pub(super) const DROPPED: &[(&str, &str, &[u8])] = &[
    ("drop-one.db", "one", include_bytes!("fixtures/drop-one.db")),
    (
        "drop-deep.db",
        "deep",
        include_bytes!("fixtures/drop-deep.db"),
    ),
    (
        "drop-wide.db",
        "wide",
        include_bytes!("fixtures/drop-wide.db"),
    ),
    (
        "drop-indexed.db",
        "indexed",
        include_bytes!("fixtures/drop-indexed.db"),
    ),
    (
        "drop-index.db",
        "index",
        include_bytes!("fixtures/drop-index.db"),
    ),
    (
        "drop-last.db",
        "last",
        include_bytes!("fixtures/drop-last.db"),
    ),
];

/// A transaction is one unit of work: the statements between a `BEGIN`
/// and a `COMMIT` write the file the same statements write on their
/// own, but the change counter counts the transaction once. `tx-grown`
/// frees pages and takes one back inside the transaction; `tx-back` is
/// what a `ROLLBACK` leaves, which is `tx-kept` byte for byte.
pub(super) const ONE_UNIT: &[u8] = include_bytes!("fixtures/tx-one.db");

/// The same over sixty rows, where the transaction frees pages and
/// takes one of them back.
pub(super) const GROWN: &[u8] = include_bytes!("fixtures/tx-grown.db");

/// The file a `ROLLBACK` leaves, which is the file before the `BEGIN`.
pub(super) const ROLLED_BACK: &[u8] = include_bytes!("fixtures/tx-back.db");

/// A view is a named statement: `view-one` is one over a table,
/// `view-named` writes the names its columns are answered under, and
/// `view-gone` is what a `DROP VIEW` leaves.
pub(super) const VIEW_ONE: &[u8] = include_bytes!("fixtures/view-one.db");

/// The same with the column names the definition wrote.
pub(super) const VIEW_NAMED: &[u8] = include_bytes!("fixtures/view-named.db");

/// What a `DROP VIEW` leaves, which is the file without its row.
pub(super) const VIEW_GONE: &[u8] = include_bytes!("fixtures/view-gone.db");

/// The auto-vacuum dimension of document 16, section 16.11, over the
/// write path: the same four hundred rows under both settings, chains
/// that cross the second pointer-map page, the free pages a file that
/// vacuums a step at a time keeps, and the pages a file that vacuums
/// itself whole moves down and gives up at the commit.
///
/// Each case names whether the file vacuums a step at a time, whether
/// its rows run onto chains, and what is taken out after they are in.
pub(crate) const VACUUMING: [(&str, bool, bool, &str, &[u8]); 6] = [
    (
        "v-full.db",
        false,
        false,
        "",
        include_bytes!("fixtures/v-full.db"),
    ),
    (
        "v-incremental.db",
        true,
        false,
        "",
        include_bytes!("fixtures/v-incremental.db"),
    ),
    (
        "v-chained.db",
        false,
        true,
        "",
        include_bytes!("fixtures/v-chained.db"),
    ),
    (
        "v-freed.db",
        true,
        false,
        "DELETE FROM t WHERE rowid%4!=0",
        include_bytes!("fixtures/v-freed.db"),
    ),
    (
        "v-moved.db",
        false,
        false,
        "DELETE FROM t WHERE rowid%4!=0",
        include_bytes!("fixtures/v-moved.db"),
    ),
    (
        "v-moved-chained.db",
        false,
        true,
        "DELETE FROM t WHERE rowid%3!=0",
        include_bytes!("fixtures/v-moved-chained.db"),
    ),
];

/// The four hundred rows of `shuffled.db` written over three times: a
/// row that keeps its length, a row that shrinks, and a row that grows.
pub(super) const UPDATED: &[u8] = include_bytes!("fixtures/updated.db");

/// Three rows written over by a statement that names no rows to leave
/// out, with one key set under the name `rowid` on a table that holds no
/// column the key is another name for.
pub(super) const KEYED: &[u8] = include_bytes!("fixtures/keyed.db");

/// Two rows written over where the new cell is the length the old one
/// was: one whose payload runs onto three overflow pages, and one whose
/// payload grows past the leaf without the cell changing length.
pub(super) const OVERWRITTEN: &[u8] = include_bytes!("fixtures/overwritten.db");

/// Five rows written over where the key moves: a payload that doubles, a
/// key column that is set, an overflow chain that is dropped, and a key
/// set under the name `rowid`.
pub(super) const MOVED: &[u8] = include_bytes!("fixtures/moved.db");

/// Four thousand rows with seven in eight taken out again and a thousand
/// put in after that, so that the pages the delete freed are the ones
/// the insert takes.
pub(super) const REUSED: &[u8] = include_bytes!("fixtures/reused.db");

/// The journal the commit of `emptied.db` left behind, under a journal
/// mode that keeps the file: the header written over with noughts and
/// the records of every page the delete changed.
pub(super) const JOURNALLED: &[u8] = include_bytes!("fixtures/journalled.db-journal");

/// The journal the commit of `shuffled.db` left behind: one record for
/// the leaf the rows went on and one for page one, which the commit
/// writes the change counter into and so puts in last.
pub(super) const APPENDED: &[u8] = include_bytes!("fixtures/appended.db-journal");

/// The database `PRAGMA journal_mode=wal` left: one page, with the write
/// and the read version two.
pub(super) const LOGGING: &[u8] = include_bytes!("fixtures/logging.db");

/// The log beside it, which holds the two statements of `shuffled.db`
/// as two transactions and was never checkpointed.
pub(super) const LOGGING_WAL: &[u8] = include_bytes!("fixtures/logging.db-wal");

/// The matrix of document 16, section 16.11, over the write path: the
/// same four hundred rows under every page size, every encoding and
/// every reserved tail the shell writes, each with the page size, the
/// reserved tail and the encoding the file was written under.
pub(crate) const CONFIGURED: [(&str, u32, u8, crate::header::Encoding, &[u8]); 9] = [
    (
        "w-utf8-512.db",
        512,
        0,
        crate::header::Encoding::Utf8,
        include_bytes!("fixtures/w-utf8-512.db"),
    ),
    (
        "w-utf8-1024.db",
        1024,
        0,
        crate::header::Encoding::Utf8,
        include_bytes!("fixtures/w-utf8-1024.db"),
    ),
    (
        "w-utf8-4096.db",
        4096,
        0,
        crate::header::Encoding::Utf8,
        include_bytes!("fixtures/w-utf8-4096.db"),
    ),
    (
        "w-utf8-8192.db",
        8192,
        0,
        crate::header::Encoding::Utf8,
        include_bytes!("fixtures/w-utf8-8192.db"),
    ),
    (
        "w-utf8-65536.db",
        65_536,
        0,
        crate::header::Encoding::Utf8,
        include_bytes!("fixtures/w-utf8-65536.db"),
    ),
    (
        "w-utf16le-512.db",
        512,
        0,
        crate::header::Encoding::Utf16Le,
        include_bytes!("fixtures/w-utf16le-512.db"),
    ),
    (
        "w-utf16be-512.db",
        512,
        0,
        crate::header::Encoding::Utf16Be,
        include_bytes!("fixtures/w-utf16be-512.db"),
    ),
    (
        "w-reserved4.db",
        1024,
        4,
        crate::header::Encoding::Utf8,
        include_bytes!("fixtures/w-reserved4.db"),
    ),
    (
        "w-reserved32.db",
        512,
        32,
        crate::header::Encoding::Utf8,
        include_bytes!("fixtures/w-reserved32.db"),
    ),
];

/// Four hundred rows over 512-byte pages, which is small enough that the
/// tree has an interior page and the walk has to descend.
///
/// `PRAGMA page_size=512; CREATE TABLE wide(n INTEGER, s TEXT);` filled
/// with `'row ' || i` for `i` in 1..=400, and `CREATE TABLE deep(k TEXT
/// PRIMARY KEY, v) WITHOUT ROWID;` filled the same way, whose key tree
/// has an interior page of its own.
pub(super) const PAGE512: &[u8] = include_bytes!("fixtures/page512.db");

/// Text in UTF-16, little-endian.
///
/// `PRAGMA encoding='UTF-16le'; CREATE TABLE u(t TEXT); INSERT INTO u
/// VALUES ('abc'), ('äöü');`
pub(super) const UTF16: &[u8] = include_bytes!("fixtures/utf16.db");

/// One row whose text is eighteen thousand bytes, which no page holds.
///
/// `CREATE TABLE big(t TEXT); INSERT INTO big VALUES
/// (replace(hex(zeroblob(9000)),'0','x'));`
pub(super) const OVERFLOW: &[u8] = include_bytes!("fixtures/overflow.db");

/// A table with two indexes over it, so that index pages exist to read.
///
/// `CREATE TABLE k(a INTEGER, b TEXT); CREATE INDEX ka ON k(a); CREATE
/// UNIQUE INDEX kb ON k(b);` filled with a hundred rows.
pub(super) const INDEXED: &[u8] = include_bytes!("fixtures/indexed.db");

/// A table whose key is the rowid, one whose rows live in the key's own
/// tree, and one whose key is written backwards.
///
/// `CREATE TABLE r(id INTEGER PRIMARY KEY, v TEXT); CREATE TABLE w(a
/// TEXT, b INT, PRIMARY KEY(a)) WITHOUT ROWID; CREATE TABLE d(k INTEGER
/// PRIMARY KEY DESC, v); CREATE TABLE u(a, b, c, d, PRIMARY KEY(c,a))
/// WITHOUT ROWID; CREATE TABLE p(a, b, PRIMARY KEY(a,a,b)) WITHOUT
/// ROWID; CREATE TABLE q(a REAL, b, c AS (a+1), PRIMARY KEY(b)) WITHOUT
/// ROWID;` — `u` keeps its key columns out of the order they were
/// declared in, `p` names one of them twice, and `q` has a column the
/// record does not hold.
pub(super) const KEYS: &[u8] = include_bytes!("fixtures/keys.db");

/// Two tables with computed columns: one with a column that is not
/// stored, and one where every computed column is.
///
/// `CREATE TABLE g(a, b AS (a+1), c AS (a*2) STORED); CREATE TABLE h(a,
/// c AS (a*2) STORED); CREATE TABLE f(a, b AS (c+1), c AS (a*2));
/// CREATE TABLE i(a, b TEXT AS (a), c INT AS (a)); CREATE TABLE j(a, b
/// AS (hex(a)), c AS (nullif(a,1)));` — `f` names a column computed
/// after it, `i` gives each a declared type, and `j` computes one from
/// the encoding and one from the collation.
pub(super) const GENERATED: &[u8] = include_bytes!("fixtures/generated.db");

/// Text in UTF-16 that reaches past the sixteen-bit range, which is
/// where the order of UTF-16 and the order of its characters part.
///
/// `PRAGMA encoding='UTF-16le'; CREATE TABLE s(t TEXT);` filled with
/// `char(57344)`, `char(65536)`, `'z'`, `char(65533)`, `char(55296)` and
/// `char(65536)||'a'`.
pub(super) const WIDE16: &[u8] = include_bytes!("fixtures/wide16.db");

/// Three tables to join: two that share a column named `x`, and a third
/// that shares one named `y` and collates it without case.
///
/// `CREATE TABLE a(x INTEGER, y TEXT); CREATE TABLE b(x INTEGER, z
/// TEXT); CREATE TABLE c(y TEXT COLLATE NOCASE, w INTEGER);` `b` holds
/// the smallest integer, which `abs` has no positive for.
pub(super) const JOINS: &[u8] = include_bytes!("fixtures/joins.db");

/// What six statements this crate runs from their text leave: a table
/// whose key is one of its columns, rows with the key given and rows
/// without, rows named in another order, and rows read out of one table
/// into another.
pub(super) const STATED: &[u8] = include_bytes!("fixtures/stated.db");

/// A database whose content is in its write-ahead log and not in its
/// file: the file holds one page and names no table, and the log holds
/// the schema and the rows.
///
/// It was written with the same three rows as the matrix, then changed
/// three ways — a text updated, a row deleted, a row inserted — so that
/// a reader that takes the newest committed frame of a page reads
/// something the file never held.
pub(super) const LOGGED: &[u8] = include_bytes!("fixtures/logged.db");

/// The write-ahead log of [`LOGGED`].
pub(super) const LOG: &[u8] = include_bytes!("fixtures/logged.db-wal");

/// A database caught between the sync of its rollback journal and the
/// sync of its own pages, which is the one state such a journal is hot
/// in.
///
/// The file holds a transaction half written: `UPDATE t SET b='changed'`
/// and a fourth row. The journal holds what its two pages began with, so
/// playing it back answers the three rows the transaction started from.
pub(super) const ROLLBACK: &[u8] = include_bytes!("fixtures/rollback.db");

/// The rollback journal of [`ROLLBACK`].
pub(super) const JOURNAL: &[u8] = include_bytes!("fixtures/rollback.db-journal");

/// [`ROLLBACK`] as the transaction that wrote [`JOURNAL`] found it,
/// which is what playing that journal back over the pair gives.
pub(super) const ROLLED: &[u8] = include_bytes!("fixtures/rolled.db");

/// A database committed under a journal mode of `persist`, which leaves
/// the journal file behind with its header zeroed.
pub(super) const PERSIST: &[u8] = include_bytes!("fixtures/m-persist.db");

/// The journal it left, which is not hot.
pub(super) const PERSIST_JOURNAL: &[u8] = include_bytes!("fixtures/m-persist.db-journal");

/// A database committed under a journal mode of `truncate`, which
/// leaves the journal file behind at no bytes at all.
pub(super) const TRUNCATE: &[u8] = include_bytes!("fixtures/m-truncate.db");

/// The journal it left, which holds nothing.
pub(super) const TRUNCATE_JOURNAL: &[u8] = include_bytes!("fixtures/m-truncate.db-journal");

/// A varint, written the way section 1.6 describes, so that the reader is
/// tested against something other than itself.
pub(super) fn varint(value: u64) -> ([u8; 9], usize) {
    let mut out = [0u8; 9];
    let byte = |bits: u64| u8::try_from(bits & 0xff).unwrap_or(0);
    if value > 0x00ff_ffff_ffff_ffff {
        // Nine bytes: fifty-six bits in sevens, then a whole byte.
        let high = value >> 8;
        for (at, slot) in out.iter_mut().take(8).enumerate() {
            *slot = byte((high >> (7 * (7 - at))) & 0x7f) | 0x80;
        }
        out[8] = byte(value);
        return (out, 9);
    }
    let mut groups = [0u8; 8];
    let mut count = 0;
    let mut rest = value;
    loop {
        groups[count] = byte(rest & 0x7f);
        rest >>= 7;
        count += 1;
        if rest == 0 {
            break;
        }
    }
    for (at, slot) in out.iter_mut().take(count).enumerate() {
        let group = groups[count - 1 - at];
        *slot = if at + 1 == count { group } else { group | 0x80 };
    }
    (out, count)
}

/// A database laid out by hand: a header, a page 1 that is an empty leaf,
/// and whatever pages a test appends. It is how a file the shell cannot
/// write — a broken overflow chain, a tree that points into itself — is
/// put in front of the reader.
pub(super) struct Builder {
    /// Bytes per page.
    page_size: usize,
    /// The file so far.
    bytes: Vec<u8>,
}

impl Builder {
    /// A database of one page: the header and an empty leaf table.
    pub(super) fn new(page_size: usize) -> Self {
        let mut bytes = SMALL[..100].to_vec();
        let field = if page_size == 65536 {
            1
        } else {
            u16::try_from(page_size).unwrap()
        };
        bytes[16..18].copy_from_slice(&field.to_be_bytes());
        bytes.resize(page_size, 0);
        // Page 1 after the header: a leaf table page with no cells.
        bytes[100] = 13;
        bytes[105..107].copy_from_slice(&u16::try_from(page_size).unwrap_or(0).to_be_bytes());
        Builder { page_size, bytes }
    }

    /// Appends one page, padded to the page size.
    pub(super) fn page(mut self, page: &[u8]) -> Self {
        let start = self.bytes.len();
        self.bytes.resize(start + self.page_size, 0);
        self.bytes[start..start + page.len()].copy_from_slice(page);
        self
    }

    /// The file, with the header's page count matching its length.
    pub(super) fn finish(mut self) -> Vec<u8> {
        let pages = u32::try_from(self.bytes.len() / self.page_size).unwrap();
        self.bytes[28..32].copy_from_slice(&pages.to_be_bytes());
        self.bytes
    }
}

/// A leaf table page holding one cell: a payload of `total` bytes of which
/// `local` are on the page, and the overflow page that follows.
pub(super) fn leaf_with_overflow(page_size: usize, total: u64, local: usize, next: u32) -> Vec<u8> {
    let (size_bytes, size_len) = varint(total);
    let mut cell = Vec::new();
    cell.extend_from_slice(&size_bytes[..size_len]);
    cell.push(1); // the rowid, as a one-byte varint
    cell.extend(core::iter::repeat_n(b'a', local));
    cell.extend_from_slice(&next.to_be_bytes());
    let offset = page_size - cell.len();
    let mut page = vec![0u8; page_size];
    page[0] = 13;
    page[3..5].copy_from_slice(&1u16.to_be_bytes());
    page[5..7].copy_from_slice(&u16::try_from(offset).unwrap().to_be_bytes());
    page[8..10].copy_from_slice(&u16::try_from(offset).unwrap().to_be_bytes());
    page[offset..].copy_from_slice(&cell);
    page
}

/// An interior table page with no cells whose right-most pointer is
/// `child`, which is how a tree of any depth is built.
pub(super) fn interior_to(page_size: usize, child: u32) -> Vec<u8> {
    let mut page = vec![0u8; page_size];
    page[0] = 5;
    page[5..7].copy_from_slice(&u16::try_from(page_size).unwrap_or(0).to_be_bytes());
    page[8..12].copy_from_slice(&child.to_be_bytes());
    page
}

/// An interior index page with no cells whose right-most pointer is
/// `child`.
pub(super) fn index_interior_to(page_size: usize, child: u32) -> Vec<u8> {
    let mut page = interior_to(page_size, child);
    page[0] = 2;
    page
}

/// An index leaf page of one cell whose pointer reaches past the page.
pub(super) fn index_leaf_past_the_page(page_size: usize) -> Vec<u8> {
    let mut page = vec![0u8; page_size];
    page[0] = 10;
    page[3..5].copy_from_slice(&1u16.to_be_bytes());
    page[5..7].copy_from_slice(&u16::try_from(page_size).unwrap_or(0).to_be_bytes());
    page[8..10].copy_from_slice(&u16::try_from(page_size).unwrap_or(0).to_be_bytes());
    page
}

/// The fixtures a test reads whole: every one the shell wrote that holds
/// a table, and what each was written to hold.
pub(crate) const WRITTEN: [(&str, &[u8]); 10] = [
    // Every serial type, every affinity, a header either side of the
    // size varint counting itself, a key's own tree and a rowid alias.
    ("records.db", include_bytes!("fixtures/records.db")),
    ("small.db", include_bytes!("fixtures/small.db")),
    ("tall.db", include_bytes!("fixtures/tall.db")),
    ("page512.db", include_bytes!("fixtures/page512.db")),
    ("indexed.db", include_bytes!("fixtures/indexed.db")),
    ("joins.db", include_bytes!("fixtures/joins.db")),
    ("keys.db", include_bytes!("fixtures/keys.db")),
    ("overflow.db", include_bytes!("fixtures/overflow.db")),
    ("utf16.db", include_bytes!("fixtures/utf16.db")),
    ("deep.db", include_bytes!("fixtures/deep.db")),
];
