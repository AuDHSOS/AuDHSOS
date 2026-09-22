// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A connection over a database that is already written: what it reads
//! out of the header, and what it refuses to open.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::{Database, Error};
use crate::error::Error as Broken;
use crate::header::{Encoding, Header};
use crate::tree::Pages;
use crate::value::Value;

/// What `sql` answers over `image`.
fn answered(image: &[u8], sql: &[u8]) -> Vec<Vec<Value>> {
    let database = Database::open(image).unwrap();
    database.query(sql).unwrap().rows
}

/// One row of one text.
fn text(word: &[u8]) -> Vec<Value> {
    alloc::vec![Value::Text(word.to_vec())]
}

/// `count` rows written into the table `name`, one statement each.
fn filled(writer: &mut Writer, name: &[u8], count: i64) {
    for at in 0..count {
        let mut sql = b"INSERT INTO ".to_vec();
        sql.extend_from_slice(name);
        sql.extend_from_slice(b" VALUES(");
        sql.extend_from_slice(&crate::number::integer_text(at));
        sql.push(b')');
        writer.run(&sql).unwrap();
    }
}

/// A table of three rows and an index over it, page size 512.
fn written() -> Vec<u8> {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a INTEGER PRIMARY KEY, b TEXT)".as_slice(),
        b"CREATE INDEX i ON t(b)",
        b"INSERT INTO t VALUES(1,'x'),(2,'y'),(3,'z')",
    ] {
        writer.run(sql).unwrap();
    }
    writer.written()
}

#[test]
fn a_connection_over_a_file_that_was_there_writes_the_rows_it_holds() {
    let image = written();
    let mut writer = Writer::opened(&image).unwrap();
    writer.run(b"INSERT INTO t VALUES(4,'w')").unwrap();
    writer.run(b"DELETE FROM t WHERE a=2").unwrap();
    let after = writer.written();
    assert_eq!(
        answered(&after, b"SELECT b FROM t ORDER BY a"),
        [text(b"x"), text(b"z"), text(b"w")]
    );
    // The index the file held answers the rows written after it was
    // opened again.
    assert_eq!(
        answered(&after, b"SELECT a FROM t WHERE b='w'"),
        [alloc::vec![Value::Int(4)]]
    );
    assert_eq!(answered(&after, b"PRAGMA integrity_check"), [text(b"ok")]);
}

#[test]
fn a_connection_over_a_file_that_was_there_writes_onto_its_free_list() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    filled(&mut writer, b"t", 200);
    writer.run(b"DROP TABLE t").unwrap();
    let image = writer.written();
    let header = Header::parse(&image).unwrap();
    assert!(header.freelist_pages > 0);
    let pages = header.pages;

    let mut writer = Writer::opened(&image).unwrap();
    writer.run(b"CREATE TABLE u(a)").unwrap();
    filled(&mut writer, b"u", 100);
    let after = writer.written();
    // The pages the dropped table gave up carry the new table, so the
    // file grows by no page at all.
    assert!(Header::parse(&after).unwrap().pages <= pages);
    assert_eq!(
        answered(&after, b"SELECT count(*) FROM u"),
        [alloc::vec![Value::Int(100)]]
    );
    assert_eq!(answered(&after, b"PRAGMA integrity_check"), [text(b"ok")]);
}

#[test]
fn a_file_that_vacuums_itself_keeps_its_maps_when_it_is_opened_again() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.vacuuming(false);
    writer.run(b"CREATE TABLE t(a)").unwrap();
    filled(&mut writer, b"t", 200);
    let image = writer.written();
    assert!(Header::parse(&image).unwrap().largest_root != 0);

    let mut writer = Writer::opened(&image).unwrap();
    writer.run(b"CREATE TABLE u(a)").unwrap();
    filled(&mut writer, b"u", 200);
    let after = writer.written();
    assert_eq!(answered(&after, b"PRAGMA integrity_check"), [text(b"ok")]);
}

#[test]
fn a_header_that_counts_no_pages_is_read_as_the_pages_the_image_carries() {
    let mut image = written();
    // Offset 28 is the page count, which a library older than 3.7.0
    // left at nought.
    for byte in image.iter_mut().take(32).skip(28) {
        *byte = 0;
    }
    let mut writer = Writer::opened(&image).unwrap();
    writer.run(b"INSERT INTO t VALUES(4,'w')").unwrap();
    assert_eq!(
        answered(&writer.written(), b"SELECT count(*) FROM t"),
        [alloc::vec![Value::Int(4)]]
    );
}

#[test]
fn a_connection_refuses_to_open_what_is_not_a_database() {
    assert_eq!(
        Writer::opened(b"").err(),
        Some(Error::Image(Broken::Truncated))
    );
    let mut image = written();
    image[0] = b'X';
    assert_eq!(
        Writer::opened(&image).err(),
        Some(Error::Image(Broken::Magic))
    );
}

#[test]
fn a_header_the_pages_do_not_answer_is_refused() {
    let image = written();
    let header = Header::parse(&image).unwrap();

    // The image carries fewer pages than the header counts.
    let cut = image.len() - 512;
    assert_eq!(
        Pages::opened(&image[..cut], &header).err(),
        Some(Broken::Overrun)
    );
    // The image carries no whole page at all.
    assert_eq!(
        Pages::opened(&image[..100], &header).err(),
        Some(Broken::Overrun)
    );

    // A header that counts no pages, over an image that carries none.
    let mut broken = header;
    broken.pages = 0;
    assert_eq!(
        Pages::opened(&image[..100], &broken).err(),
        Some(Broken::Overrun)
    );

    let mut broken = header;
    broken.page_size = 100;
    assert_eq!(
        Pages::opened(&image, &broken).err(),
        Some(Broken::PageSize(100))
    );
    // A page size inside the range the format allows that is not a
    // power of two.
    let mut broken = header;
    broken.page_size = 1000;
    assert_eq!(
        Pages::opened(&image, &broken).err(),
        Some(Broken::PageSize(1000))
    );
    let mut broken = header;
    broken.reserved = 250;
    assert_eq!(
        Pages::opened(&image, &broken).err(),
        Some(Broken::Reserved(250))
    );
}

#[test]
fn a_connection_over_a_file_that_was_there_reads_the_encoding_it_was_written_under() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf16Le).unwrap();
    writer.run(b"CREATE TABLE t(a TEXT)").unwrap();
    writer.run(b"INSERT INTO t VALUES('x')").unwrap();
    let image = writer.written();

    let mut writer = Writer::opened(&image).unwrap();
    writer.run(b"INSERT INTO t VALUES('y')").unwrap();
    let after = writer.written();
    assert_eq!(Header::parse(&after).unwrap().encoding, Encoding::Utf16Le);
    assert_eq!(
        answered(&after, b"SELECT a FROM t ORDER BY a"),
        [text(b"x"), text(b"y")]
    );
}

/// A file whose header names a schema format above four is refused, and
/// one naming four or below is read.
#[test]
fn a_schema_format_the_library_does_not_hold_is_refused() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    let image = writer.written();
    let formatted = |format: u8| {
        let mut bytes = image.clone();
        bytes[47] = format;
        bytes
    };
    for format in 1..=4 {
        assert!(
            Database::open(&formatted(format)).is_ok(),
            "format {format}"
        );
    }
    assert_eq!(
        Database::open(&formatted(5)).unwrap_err().message(),
        "unsupported file format"
    );
}
