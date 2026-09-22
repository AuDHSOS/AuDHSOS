// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a commit writes, and a connection over the three files a
//! database is kept in, `src/pager.c`.

use alloc::vec::Vec;

use crate::change::{Does, Onto, Writer};
use crate::header::Encoding;
use crate::value::Value;

/// A connection over a fresh database with one table and one row in it.
fn written(mode: &[u8]) -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    let mut sql = b"PRAGMA journal_mode=".to_vec();
    sql.extend_from_slice(mode);
    writer.run(&sql).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    let _ = writer.did();
    writer
}

/// The values one statement answers, read through the pages the
/// connection holds.
fn answered(writer: &Writer, sql: &[u8]) -> Vec<Value> {
    crate::db::Database::open(&writer.inside())
        .unwrap()
        .query(sql)
        .unwrap()
        .rows
        .into_iter()
        .flatten()
        .collect()
}

/// Which file each thing a commit did reaches, and what kind it is.
fn shaped(did: &[Does]) -> Vec<(&'static str, Onto)> {
    did.iter()
        .map(|held| match held {
            Does::Write { onto, .. } => ("write", *onto),
            Does::Sync(onto) => ("sync", *onto),
            Does::Truncate { onto, .. } => ("truncate", *onto),
            Does::Remove(onto) => ("remove", *onto),
        })
        .collect()
}

#[test]
fn a_commit_that_keeps_a_rollback_journal_writes_it_before_the_file() {
    let mut writer = written(b"delete");
    writer.run(b"INSERT INTO t VALUES (1)").unwrap();
    let did = writer.did();
    let shape = shaped(&did);
    // The journal is on the disk before a page of the file is written
    // over, which is what makes the commit recoverable.
    let first = shape
        .iter()
        .position(|held| *held == ("sync", Onto::Journal));
    let page = shape.iter().position(|held| *held == ("write", Onto::Main));
    assert!(first.is_some_and(|first| page.is_some_and(|page| first < page)));
    assert_eq!(shape.first(), Some(&("write", Onto::Journal)));
    assert_eq!(shape.last(), Some(&("remove", Onto::Journal)));
    // The records of the journal begin after the header, which takes the
    // sector, and each holds a page number, the page and a checksum.
    let records = did
        .iter()
        .filter(|held| matches!(held, Does::Write { onto: Onto::Journal, at, .. } if *at > 0))
        .count();
    assert!(records > 0);
    let written = writer.written();
    for held in &did {
        let Does::Write {
            onto: Onto::Main,
            at,
            bytes,
        } = held
        else {
            continue;
        };
        let at = usize::try_from(*at).unwrap();
        assert_eq!(written.get(at..at + bytes.len()), Some(bytes.as_slice()));
    }
}

#[test]
fn what_each_journal_mode_leaves_of_the_journal() {
    for (mode, kind) in [
        (b"delete".as_slice(), "remove"),
        (b"truncate", "truncate"),
        (b"persist", "write"),
    ] {
        let mut writer = written(mode);
        writer.run(b"INSERT INTO t VALUES (1)").unwrap();
        let did = writer.did();
        assert_eq!(shaped(&did).last(), Some(&(kind, Onto::Journal)));
    }
}

#[test]
fn a_commit_in_write_ahead_logging_writes_the_frames_and_holds_them() {
    let mut writer = written(b"wal");
    writer.run(b"INSERT INTO t VALUES (1)").unwrap();
    let did = writer.did();
    let shape = shaped(&did);
    assert!(shape.iter().all(|held| held.1 == Onto::Log));
    assert_eq!(shape.last(), Some(&("sync", Onto::Log)));
    // The frames the commit appended stand where the log holds them.
    let log = writer.log().unwrap().to_vec();
    for held in &did {
        let Does::Write { at, bytes, .. } = held else {
            continue;
        };
        let at = usize::try_from(*at).unwrap();
        assert_eq!(log.get(at..at + bytes.len()), Some(bytes.as_slice()));
    }
}

#[test]
fn a_connection_over_a_file_beside_a_hot_journal_plays_the_journal_back() {
    let mut writer = written(b"persist");
    let was = writer.written();
    writer.run(b"INSERT INTO t VALUES (1)").unwrap();
    let journal = crate::journal::write(
        &[
            (1, was.get(..1024).unwrap()),
            (2, was.get(1024..2048).unwrap()),
        ],
        2,
        0,
        512,
    );
    // The journal holds the pages as they were, so the row the commit
    // wrote is not in the database the connection reads.
    let recovered = Writer::recovered(&writer.written(), &journal, &[]).unwrap();
    assert_eq!(
        answered(&recovered, b"SELECT count(*) FROM t"),
        [Value::Int(0)]
    );
    // A journal that is not hot leaves the file as it stands.
    let stood = Writer::recovered(&writer.written(), &[], &[]).unwrap();
    assert_eq!(stood.written(), writer.written());
}

#[test]
fn a_connection_over_a_file_beside_a_log_reads_the_pages_the_log_holds() {
    let mut writer = written(b"wal");
    writer.run(b"INSERT INTO t VALUES (7)").unwrap();
    let file = writer.written();
    let log = writer.log().unwrap().to_vec();
    let recovered = Writer::recovered(&file, &[], &log).unwrap();
    assert_eq!(answered(&recovered, b"SELECT a FROM t"), [Value::Int(7)]);
    // The connection writes further frames beside the ones the log holds.
    let mut carried = recovered;
    carried.run(b"INSERT INTO t VALUES (8)").unwrap();
    assert!(carried.log().unwrap().len() > log.len());
    assert_eq!(
        answered(&carried, b"SELECT a FROM t"),
        [Value::Int(7), Value::Int(8)]
    );
}

#[test]
fn three_files_of_no_bytes_are_a_database_of_no_page() {
    let writer = Writer::recovered(&[], &[], &[]);
    assert!(writer.is_err());
    // A log whose header does not read is one no commit can carry on.
    assert!(crate::wal::Log::opened(&[]).is_none());
    assert!(crate::wal::Log::opened(&[0u8; 32]).is_none());
    let mut broken = written(b"wal").log().unwrap().to_vec();
    for slot in broken.iter_mut().skip(8).take(4) {
        *slot = 0;
    }
    assert!(crate::wal::Log::opened(&broken).is_none());
}

#[test]
fn a_frame_after_the_last_commit_frame_takes_no_page_out_of_what_a_reader_reads() {
    // The transaction the log ends with wrote a page no frame before it
    // wrote, so a reader that counted the pages the frames named rather
    // than reading up to the commit frame read one page out of the file.
    let mut writer = written(b"wal");
    for sql in [
        b"CREATE TABLE u(b)".as_slice(),
        b"INSERT INTO t VALUES (1)",
        b"INSERT INTO u VALUES (2)",
    ] {
        writer.run(sql).unwrap();
    }
    let file = writer.written();
    let log = writer.log().unwrap().to_vec();
    let recovered = Writer::recovered(&file, &[], &log).unwrap();
    assert_eq!(answered(&recovered, b"SELECT a FROM t"), [Value::Int(1)]);
    assert_eq!(answered(&recovered, b"SELECT b FROM u"), [Value::Int(2)]);
}

/// A log header naming `magic` and `page_size`, which is all a log that
/// holds no frame is.
fn headed(magic: u32, page_size: u32) -> Vec<u8> {
    let mut bytes = alloc::vec![0u8; 32];
    for (slot, byte) in bytes.iter_mut().zip(magic.to_be_bytes()) {
        *slot = byte;
    }
    for (slot, byte) in bytes.iter_mut().skip(8).zip(page_size.to_be_bytes()) {
        *slot = byte;
    }
    bytes
}

#[test]
fn what_log_a_commit_cannot_carry_on() {
    // The magic names the byte order the checksums read their words in,
    // and either order is one a commit carries on.
    assert!(crate::wal::Log::opened(&headed(0x377f_0682, 1024)).is_some());
    assert!(crate::wal::Log::opened(&headed(0x377f_0683, 1024)).is_some());
    assert!(crate::wal::Log::opened(&headed(0x377f_0684, 1024)).is_none());
    // A page size the format does not allow is one no frame reads.
    assert!(crate::wal::Log::opened(&headed(0x377f_0682, 1000)).is_none());
    assert!(crate::wal::Log::opened(&headed(0x377f_0682, 256)).is_none());
}

#[test]
fn a_log_is_carried_on_from_the_last_frame_that_reads() {
    let mut writer = written(b"wal");
    writer.run(b"INSERT INTO t VALUES (1)").unwrap();
    writer.run(b"INSERT INTO t VALUES (2)").unwrap();
    let log = writer.log().unwrap().to_vec();
    let held = crate::wal::Log::opened(&log).unwrap();
    assert_eq!(held.bytes().len(), log.len());
    // A frame whose salts are not the header's is where the log ends,
    // and so is one whose checksum is not the running one.
    for at in [40, 60] {
        let mut broken = log.clone();
        for slot in broken.iter_mut().skip(at).take(1) {
            *slot ^= 0xff;
        }
        let held = crate::wal::Log::opened(&broken).unwrap();
        assert!(held.bytes().len() < log.len());
    }
}

#[test]
fn a_page_no_frame_of_the_log_holds_is_read_out_of_the_file() {
    // The checkpoint begins the log again, so the frames after it hold
    // the pages that one commit wrote and no other page of the file.
    let mut writer = written(b"wal");
    for sql in [
        b"CREATE TABLE u(b)".as_slice(),
        b"INSERT INTO u VALUES (1)",
        b"INSERT INTO t VALUES (2)",
        b"PRAGMA wal_checkpoint(restart)",
        b"INSERT INTO t VALUES (3)",
    ] {
        writer.run(sql).unwrap();
    }
    let recovered = Writer::recovered(&writer.written(), &[], writer.log().unwrap()).unwrap();
    assert_eq!(answered(&recovered, b"SELECT b FROM u"), [Value::Int(1)]);
    assert_eq!(
        answered(&recovered, b"SELECT a FROM t"),
        [Value::Int(2), Value::Int(3)]
    );
}

#[test]
fn a_log_of_no_committed_frame_leaves_the_file_as_it_stands() {
    let mut writer = written(b"wal");
    writer.run(b"INSERT INTO t VALUES (1)").unwrap();
    let file = writer.written();
    let empty = writer.log().unwrap().get(..32).unwrap().to_vec();
    // The file beside such a log is the file itself, and a file whose
    // header does not read is refused.
    assert!(Writer::recovered(&file, &[], &empty).is_ok());
    assert!(Writer::recovered(&[7u8; 2048], &[], &empty).is_err());
    // A file of no bytes beside such a log is a database of no page.
    let made = Writer::recovered(&[], &[], &empty).unwrap();
    assert_eq!(made.written().len(), 0);
}
