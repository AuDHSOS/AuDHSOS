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
