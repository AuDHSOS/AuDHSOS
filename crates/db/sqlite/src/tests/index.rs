// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! An index used to answer a statement, and what happens where the
//! index is damaged.
//!
//! A file is not obliged to hold an index the shell would write. What a
//! statement answers is what the table holds, so an index the walk
//! cannot descend is one the walk gives up on, and the table is scanned.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use alloc::vec::Vec;

use crate::db::Database;
use crate::image::Image;
use crate::record::Value as Stored;
use crate::value::Value;

/// The root page of the index named `name`.
fn index_root(bytes: &[u8], name: &[u8]) -> u32 {
    let image = Image::open(bytes).unwrap();
    for row in image.schema() {
        let record = row.unwrap().record().unwrap();
        if record.value(0).unwrap() == Some(Stored::Text(b"index"))
            && record.value(1).unwrap() == Some(Stored::Text(name))
            && let Some(Stored::Int(root)) = record.value(3).unwrap()
        {
            return u32::try_from(root).unwrap();
        }
    }
    panic!("no index named {name:?}");
}

/// The fixture with the cell at `cell` of the index `name` claiming a
/// payload no file is long enough to hold.
///
/// A leaf of an index tree keeps an eight-byte header and then two bytes
/// per cell naming where it begins, and a cell begins with the length of
/// its payload as a varint. The cells of this index are five bytes and
/// lie at the end of the page, so one byte of `0x7f` claims a payload of
/// a hundred and twenty-seven bytes, which reaches past the page. One
/// byte is all that is written, because a longer varint would reach into
/// the cell beside it and damage that one as well.
fn damaged(name: &[u8], cell: usize) -> Vec<u8> {
    let mut bytes = super::INDEXED.to_vec();
    let root = usize::try_from(index_root(&bytes, name)).unwrap();
    let page_size = 4096usize;
    let at = (root - 1) * page_size;
    let pointer = at + 8 + cell * 2;
    let offset = usize::from(u16::from_be_bytes([bytes[pointer], bytes[pointer + 1]]));
    bytes[at + offset] = 0x7f;
    bytes
}

/// Where the cell at `cell` of the page at `page` begins.
fn cell_at(bytes: &[u8], page: usize, cell: usize) -> usize {
    let at = (page - 1) * 4096;
    let head = if page == 1 { 100 } else { 0 };
    let header = if bytes[at + head] == 2 || bytes[at + head] == 5 {
        12
    } else {
        8
    };
    let pointer = at + head + header + cell * 2;
    at + usize::from(u16::from_be_bytes([bytes[pointer], bytes[pointer + 1]]))
}

/// What the fixture answers for `sql`.
fn answers(bytes: &[u8], sql: &[u8]) -> Result<Vec<Vec<Value>>, crate::db::Error> {
    Database::open(bytes)?.query(sql).map(|answer| answer.rows)
}

#[test]
fn an_index_answers_the_rows_a_scan_of_the_table_answers() {
    let rows = answers(super::INDEXED, b"SELECT p, r FROM m WHERE q='A' ORDER BY 1").unwrap();
    assert_eq!(
        rows,
        [
            alloc::vec![Value::Int(1), Value::Int(10)],
            alloc::vec![Value::Int(2), Value::Int(30)],
        ]
    );
}

#[test]
fn an_index_the_descent_cannot_read_is_given_up_and_the_table_is_scanned() {
    // The descent of `mq` reads cells two, one and nought looking for
    // `A`, so a cell it reads that cannot be read gives the index up.
    let bytes = damaged(b"mq", 1);
    let rows = answers(&bytes, b"SELECT p, r FROM m WHERE q='A' ORDER BY 1").unwrap();
    assert_eq!(
        rows,
        [
            alloc::vec![Value::Int(1), Value::Int(10)],
            alloc::vec![Value::Int(2), Value::Int(30)],
        ]
    );
}

#[test]
fn an_index_the_walk_cannot_read_refuses_rather_than_answers_fewer_rows() {
    // Cell three is the one after the entries the key reaches, so the
    // descent never reads it and the walk does. Answering the rows found
    // before it would be answering fewer rows than the table holds.
    let bytes = damaged(b"mq", 3);
    assert_eq!(
        answers(&bytes, b"SELECT p FROM m WHERE q='A' ORDER BY 1"),
        Err(crate::db::Error::Image(crate::error::Error::Overrun))
    );
}

#[test]
fn a_statement_that_names_no_index_column_reads_the_table_whole() {
    // Every index of the fixture is damaged, and a statement that asks
    // about none of them is answered all the same.
    let mut bytes = super::INDEXED.to_vec();
    for name in [b"mq".as_slice(), b"mpq".as_slice(), b"mr".as_slice()] {
        let root = usize::try_from(index_root(&bytes, name)).unwrap();
        let at = (root - 1) * 4096;
        for byte in 0..4096 {
            bytes[at + byte] = 0xff;
        }
    }
    let rows = answers(&bytes, b"SELECT count(*) FROM m WHERE r>15").unwrap();
    assert_eq!(rows, [alloc::vec![Value::Int(3)]]);
}

#[test]
fn an_entry_whose_record_is_broken_refuses_where_the_walk_reads_it() {
    // The header of a record is as long as its first varint says, and a
    // record five bytes long that claims a hundred and twenty-seven is
    // one no reader can take values out of. Cell three is the one the
    // walk reads and the descent does not.
    let mut bytes = super::INDEXED.to_vec();
    let root = usize::try_from(index_root(&bytes, b"mq")).unwrap();
    let at = cell_at(&bytes, root, 3);
    bytes[at + 1] = 0x7f;
    assert_eq!(
        answers(&bytes, b"SELECT p FROM m WHERE q='A' ORDER BY 1"),
        Err(crate::db::Error::Image(crate::error::Error::Overrun))
    );
}

#[test]
fn a_row_an_entry_names_that_is_broken_refuses_rather_than_answers_nothing() {
    // The index is whole and the table it names is not, so the walk
    // reaches the row and cannot read it.
    let mut bytes = super::INDEXED.to_vec();
    let image = Image::open(&bytes).unwrap();
    let mut table = 0usize;
    for row in image.schema() {
        let record = row.unwrap().record().unwrap();
        if record.value(0).unwrap() == Some(Stored::Text(b"table"))
            && record.value(1).unwrap() == Some(Stored::Text(b"m"))
            && let Some(Stored::Int(root)) = record.value(3).unwrap()
        {
            table = usize::try_from(root).unwrap();
        }
    }
    for cell in 0..5 {
        let at = cell_at(&bytes, table, cell);
        // A table leaf cell is the payload's length, then the rowid,
        // then the record, so the record's own header is the third byte
        // while both varints are one byte long.
        bytes[at + 2] = 0x7f;
    }
    assert_eq!(
        answers(&bytes, b"SELECT p FROM m WHERE q='A' ORDER BY 1"),
        Err(crate::db::Error::Image(crate::error::Error::Overrun))
    );
}

#[test]
fn an_entry_that_runs_onto_an_overflow_page_is_read_whole_to_be_taken() {
    // The index `ot` holds one entry of eight thousand letters, which no
    // page holds, so both the comparison and the rowid at the end of it
    // are read off the chain that follows.
    let mut sql = b"SELECT a FROM o WHERE t='".to_vec();
    sql.extend(core::iter::repeat_n(b'y', 8000));
    sql.extend_from_slice(b"'");
    assert_eq!(
        answers(super::INDEXED, &sql).unwrap(),
        [alloc::vec![Value::Int(1)]]
    );
}

#[test]
fn an_entry_that_ends_with_no_rowid_is_refused() {
    // Every entry of an index over a table with a rowid ends with that
    // rowid, so one that ends with anything else is a file whose index
    // is not the index its table has. The second value of the record is
    // made text, which leaves the first readable and the descent right.
    let mut bytes = super::INDEXED.to_vec();
    let root = usize::try_from(index_root(&bytes, b"mq")).unwrap();
    let at = cell_at(&bytes, root, 2);
    // The payload is the record's header length, then one serial type
    // per value; the second of them is the rowid's.
    bytes[at + 3] = 0x0d;
    assert_eq!(
        answers(&bytes, b"SELECT p FROM m WHERE q='A' ORDER BY 1"),
        Err(crate::db::Error::Image(crate::error::Error::Overrun))
    );
}
