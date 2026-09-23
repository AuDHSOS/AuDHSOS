// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `VACUUM`, which makes the database again from nothing.

use crate::change::Writer;
use crate::db::{Database, Error};
use crate::header::Encoding;
use crate::value::Value;

/// A connection that ran the statements.
fn ran(statements: &[&str]) -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in statements {
        writer.run(sql.as_bytes()).expect(sql);
    }
    writer
}

/// What one statement answers over the file the writer holds.
fn answered(writer: &Writer, sql: &str) -> alloc::vec::Vec<alloc::vec::Vec<Value>> {
    let bytes = writer.written();
    Database::open(&bytes)
        .unwrap()
        .query(sql.as_bytes())
        .unwrap()
        .rows
}

/// `vacuum-1.*` of `test/vacuum.test`: the rows, the indexes and the
/// triggers stand after a vacuum, and the file holds no free page.
#[test]
fn the_rows_the_indexes_and_the_triggers_stand_after_a_vacuum() {
    let mut writer = ran(&[
        "CREATE TABLE t(a INTEGER PRIMARY KEY, b, c)",
        "CREATE INDEX i ON t(b, c)",
        "CREATE VIEW v AS SELECT b FROM t",
        "CREATE TABLE log(x)",
        "CREATE TRIGGER r AFTER DELETE ON t BEGIN INSERT INTO log VALUES(old.a); END",
        "INSERT INTO t VALUES(1,'one',1.5),(2,'two',x'00ff'),(3,NULL,NULL)",
        "DELETE FROM t WHERE a=2",
    ]);
    let before = answered(&writer, "SELECT a, b, c FROM t ORDER BY a");
    writer.run(b"VACUUM").unwrap();
    assert_eq!(
        answered(&writer, "SELECT a, b, c FROM t ORDER BY a"),
        before
    );
    // The trigger fired over the delete before the vacuum, and the row
    // it wrote stands.
    assert_eq!(
        answered(&writer, "SELECT x FROM log"),
        [alloc::vec![Value::Int(2)]]
    );
    // The index answers the row it holds, the view answers its rows, and
    // the trigger fires again.
    assert_eq!(
        answered(&writer, "SELECT a FROM t WHERE b='one'"),
        [alloc::vec![Value::Int(1)]]
    );
    assert_eq!(
        answered(&writer, "SELECT count(*) FROM v"),
        [alloc::vec![Value::Int(2)]]
    );
    writer.run(b"DELETE FROM t WHERE a=3").unwrap();
    assert_eq!(
        answered(&writer, "SELECT count(*) FROM log"),
        [alloc::vec![Value::Int(2)]]
    );
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    assert_eq!(
        database.query(b"PRAGMA freelist_count").unwrap().rows,
        [alloc::vec![Value::Int(0)]]
    );
    assert_eq!(
        database.query(b"PRAGMA integrity_check").unwrap().rows,
        [alloc::vec![Value::Text(b"ok".to_vec())]]
    );
}

/// `vacuum2-1.*`: a `PRAGMA page_size` written after the first table
/// says what the next vacuum writes the file under.
#[test]
fn a_page_size_written_after_the_first_table_is_what_the_vacuum_writes() {
    let mut writer = ran(&[
        "CREATE TABLE t(a)",
        "INSERT INTO t VALUES(1),(2),(3)",
        "PRAGMA page_size=4096",
    ]);
    // The pragma changes nothing until the vacuum runs.
    assert_eq!(
        answered(&writer, "PRAGMA page_size"),
        [alloc::vec![Value::Int(1024)]]
    );
    writer.run(b"VACUUM").unwrap();
    assert_eq!(
        answered(&writer, "PRAGMA page_size"),
        [alloc::vec![Value::Int(4096)]]
    );
    assert_eq!(
        answered(&writer, "SELECT count(*) FROM t"),
        [alloc::vec![Value::Int(3)]]
    );
}

/// A table without a rowid and one whose key counts up are written again
/// as they stood.
#[test]
fn a_table_without_a_rowid_and_one_that_counts_up_stand_after_a_vacuum() {
    let mut writer = ran(&[
        "CREATE TABLE k(a TEXT, b, PRIMARY KEY(a)) WITHOUT ROWID",
        "CREATE TABLE s(a INTEGER PRIMARY KEY AUTOINCREMENT, b)",
        "INSERT INTO k VALUES('x',1),('y',2)",
        "INSERT INTO s(b) VALUES(1),(2)",
        "DELETE FROM s WHERE a=2",
    ]);
    writer.run(b"VACUUM").unwrap();
    assert_eq!(
        answered(&writer, "SELECT a, b FROM k ORDER BY a"),
        [
            alloc::vec![Value::Text(b"x".to_vec()), Value::Int(1)],
            alloc::vec![Value::Text(b"y".to_vec()), Value::Int(2)],
        ]
    );
    // The counter the table keeps stands, so the next key is past the
    // one the delete took away.
    writer.run(b"INSERT INTO s(b) VALUES(3)").unwrap();
    assert_eq!(
        answered(&writer, "SELECT a FROM s ORDER BY a"),
        [alloc::vec![Value::Int(1)], alloc::vec![Value::Int(3)]]
    );
}

/// A file that vacuums itself keeps its maps, and an index a `UNIQUE`
/// made carries no statement of its own and is made again with its
/// table.
#[test]
fn a_file_that_vacuums_itself_and_an_index_a_unique_made_stand() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.vacuuming(true);
    writer.run(b"CREATE TABLE t(a UNIQUE, b)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x'),(2,'y')").unwrap();
    // A pragma the file holds, written after the first table, changes
    // nothing and is not the page size the vacuum reads.
    writer.run(b"PRAGMA auto_vacuum=0").unwrap();
    writer.run(b"VACUUM temp").unwrap();
    assert_eq!(
        answered(&writer, "PRAGMA auto_vacuum"),
        [alloc::vec![Value::Int(2)]]
    );
    assert_eq!(
        answered(&writer, "SELECT b FROM t WHERE a=2"),
        [alloc::vec![Value::Text(b"y".to_vec())]]
    );
    // The index the `UNIQUE` made holds the rows again, so a row that
    // shares a value with one the table holds is refused.
    assert!(writer.run(b"INSERT INTO t VALUES(1,'z')").is_err());
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    assert_eq!(
        database.query(b"PRAGMA integrity_check").unwrap().rows,
        [alloc::vec![Value::Text(b"ok".to_vec())]]
    );
}

/// A `VACUUM` inside a transaction, one that names a schema this
/// connection does not hold, and `VACUUM INTO` are refused.
#[test]
fn what_a_vacuum_is_refused_for() {
    let mut writer = ran(&["CREATE TABLE t(a)"]);
    assert_eq!(
        writer.run(b"VACUUM aux"),
        Err(Error::NoSchema(b"aux".to_vec()))
    );
    writer.run(b"BEGIN").unwrap();
    assert_eq!(writer.run(b"VACUUM"), Err(Error::VacuumInTransaction));
    writer.run(b"COMMIT").unwrap();
    writer.run(b"VACUUM main").unwrap();
    // A semicolon after the word names no schema, and a number is no
    // name at all.
    writer.run(b"VACUUM;").unwrap();
    assert!(writer.run(b"VACUUM 5").is_err());
    // A word that may not be a name is no schema either.
    assert!(writer.run(b"VACUUM SELECT").is_err());
    // The two refusals carry the text the C library writes.
    assert_eq!(
        Error::NoSchema(b"aux".to_vec()).message(),
        "unknown database aux"
    );
    assert_eq!(
        Error::VacuumInTransaction.message(),
        "cannot VACUUM from within a transaction"
    );
}

/// `VACUUM ... INTO` writes the database again into the file the
/// expression after `INTO` names.
#[test]
fn what_a_vacuum_into_writes() {
    let mut writer = ran(&["CREATE TABLE t(a)", "INSERT INTO t VALUES(1),(2)"]);
    writer.run(b"VACUUM main INTO 'out.db'").unwrap();
    // The file stands beside the databases the connection attached, and
    // holds the rows the database holds.
    let (named, bytes) = writer
        .attached_files()
        .into_iter()
        .find(|(name, _)| name == b"out.db")
        .expect("the file the statement wrote");
    assert_eq!(named, b"out.db");
    let database = Database::open(&bytes).unwrap();
    assert_eq!(
        database.query(b"SELECT count(*) FROM t").unwrap().rows,
        [alloc::vec![Value::Int(2)]]
    );
    // The expression is answered against the database, so a name no
    // column carries and a value that is no text are both refused, and
    // a file the client holds bytes for is not written over.
    assert_eq!(writer.run(b"VACUUM INTO null"), Err(Error::NonTextFilename));
    assert_eq!(
        writer.run(b"VACUUM INTO x").unwrap_err().message(),
        "no such column: x"
    );
    writer.run(b"VACUUM INTO (SELECT 'other.db')").unwrap();
    writer.opens(|name| (name == b"out.db").then(|| alloc::vec![0_u8]));
    assert_eq!(
        writer.run(b"VACUUM INTO 'out.db'"),
        Err(Error::OutputExists)
    );
    // A database of one page is no file, so it is written whatever the
    // client holds.
    writer.run(b"VACUUM INTO ':memory:'").unwrap();
    // The two refusals carry the text the C library writes.
    assert_eq!(Error::OutputExists.message(), "output file already exists");
    assert_eq!(Error::NonTextFilename.message(), "non-text filename");
}

/// `PRAGMA default_synchronous`: a name no version of the library holds
/// answers no row, which is `sqlite3Pragma` for a name it does not know.
#[test]
fn a_pragma_the_library_does_not_know_answers_no_row() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    let none: alloc::vec::Vec<alloc::vec::Vec<Value>> = alloc::vec::Vec::new();
    assert_eq!(writer.run(b"PRAGMA default_synchronous").unwrap(), none);
    assert_eq!(writer.run(b"PRAGMA default_synchronous=2").unwrap(), none);
    assert_eq!(writer.run(b"PRAGMA optimize").unwrap(), none);
    // A name the pragma table of the C library does not hold changes
    // nothing, and one it holds that this crate does not write is
    // refused.
    assert_eq!(writer.run(b"PRAGMA bogus_name").unwrap(), none);
    assert_eq!(writer.run(b"PRAGMA autovacuum = 0").unwrap(), none);
    assert_eq!(writer.run(b"PRAGMA table_list"), Err(Error::Unsupported));
}

/// `autovacuum-1.*` of `test/autovacuum.test`: a file that vacuums
/// itself whole holds every page it has not given up after a row with
/// overflow pages is deleted.
#[test]
fn a_file_that_vacuums_itself_holds_every_page_after_a_delete() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.vacuuming(false);
    writer.run(b"CREATE TABLE av1(a)").unwrap();
    writer.run(b"CREATE INDEX av1_idx ON av1(a)").unwrap();
    for row in 1..=20_i64 {
        let text = alloc::string::String::from_utf8(alloc::vec![b'a'; 3500]).unwrap();
        writer
            .run(alloc::format!("INSERT INTO av1(oid, a) VALUES({row}, '{text}')").as_bytes())
            .unwrap();
    }
    assert_eq!(
        answered(&writer, "PRAGMA integrity_check"),
        [alloc::vec![Value::Text(b"ok".to_vec())]]
    );
    for row in 1..=20_i64 {
        writer
            .run(alloc::format!("DELETE FROM av1 WHERE oid={row}").as_bytes())
            .unwrap();
        assert_eq!(
            answered(&writer, "PRAGMA integrity_check"),
            [alloc::vec![Value::Text(b"ok".to_vec())]],
            "after deleting row {row}"
        );
    }
    // Every page the rows took is given up, so the file is the header,
    // the map, the table and the index.
    assert_eq!(
        answered(&writer, "PRAGMA page_count"),
        [alloc::vec![Value::Int(4)]]
    );
    assert_eq!(writer.written().len(), 4 * 1024);
}

/// `incrvacuum-4.*` of `test/incrvacuum.test`: `PRAGMA
/// incremental_vacuum` gives up the pages at the end of the file one at
/// a time and answers one row of no column per page it gave up.
#[test]
fn what_an_incremental_vacuum_gives_up() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.vacuuming(true);
    let text = alloc::string::String::from_utf8(alloc::vec![b'x'; 1100]).unwrap();
    for sql in [
        "CREATE TABLE t(a)".to_string(),
        alloc::format!("INSERT INTO t VALUES('{text}'),('{text}'),('{text}')"),
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    let held = answered(&writer, "PRAGMA page_count")[0][0].clone();
    writer.run(b"DELETE FROM t").unwrap();
    // The pages the rows took are on the free list, and the file is as
    // long as it was.
    assert_eq!(answered(&writer, "PRAGMA page_count"), [alloc::vec![held]]);
    let Value::Int(free) = answered(&writer, "PRAGMA freelist_count")[0][0] else {
        panic!("the pragma answers a number")
    };
    // Two steps give up two pages and answer two rows of no column.
    assert_eq!(
        writer.run(b"PRAGMA incremental_vacuum(2)").unwrap(),
        [alloc::vec![], alloc::vec![]]
    );
    assert_eq!(
        answered(&writer, "PRAGMA freelist_count"),
        [alloc::vec![Value::Int(free - 2)]]
    );
    // Every page of the free list is given up where the statement names
    // no count, and the file holds the header, the map and the table.
    assert_eq!(
        writer.run(b"PRAGMA incremental_vacuum").unwrap().len(),
        usize::try_from(free - 2).unwrap()
    );
    assert_eq!(
        answered(&writer, "PRAGMA page_count"),
        [alloc::vec![Value::Int(3)]]
    );
    assert_eq!(
        answered(&writer, "PRAGMA integrity_check"),
        [alloc::vec![Value::Text(b"ok".to_vec())]]
    );
    // A file that holds no free page gives up none, and so does one that
    // does not vacuum itself.
    assert!(writer.run(b"PRAGMA incremental_vacuum").unwrap().is_empty());
    let mut plain = ran(&[
        "CREATE TABLE t(a)",
        "INSERT INTO t VALUES(1)",
        "DELETE FROM t",
    ]);
    assert!(plain.run(b"PRAGMA incremental_vacuum").unwrap().is_empty());
}

/// `incrvacuum-6.*`: how many pages a `PRAGMA incremental_vacuum` gives
/// up is what `sqlite3GetInt32` reads after the name, and every page of
/// the free list where that is no number above nought.
#[test]
fn how_many_pages_an_incremental_vacuum_is_asked_for() {
    let text = alloc::string::String::from_utf8(alloc::vec![b'x'; 1100]).unwrap();
    let steps = |asked: &str| {
        let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
        writer.vacuuming(true);
        for sql in [
            "CREATE TABLE t(a)".to_string(),
            alloc::format!(
                "INSERT INTO t VALUES('{text}'),('{text}'),('{text}'),('{text}'),('{text}')"
            ),
            "DELETE FROM t".to_string(),
            alloc::format!("PRAGMA incremental_vacuum{asked}"),
        ] {
            let rows = writer.run(sql.as_bytes()).unwrap();
            if sql.starts_with("PRAGMA") {
                return rows.len();
            }
        }
        0
    };
    let every = steps("(0)");
    assert!(every > 3, "the file holds free pages to give up");
    assert_eq!(steps("(1)"), 1);
    assert_eq!(steps("('1')"), 1);
    assert_eq!(steps("(\"+3\")"), 3);
    assert_eq!(steps(" = 2"), 2);
    // A count past what a signed word holds, one that is no number at
    // all, and one that is nought or below all name every page.
    assert_eq!(steps("(2147483649)"), every);
    assert_eq!(steps("(bogus)"), every);
    assert_eq!(steps("=-1"), every);
}

/// `incrvacuum-3.4`: `PRAGMA auto_vacuum` over a file that vacuums
/// itself already writes which of the two ways it does, and a word no
/// way carries names none.
#[test]
fn what_an_auto_vacuum_over_a_file_with_a_table_writes() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.vacuuming(true);
    writer.run(b"CREATE TABLE t(a)").unwrap();
    assert_eq!(
        answered(&writer, "PRAGMA auto_vacuum"),
        [alloc::vec![Value::Int(2)]]
    );
    // The file vacuums itself whole from here on, so the pages a delete
    // frees are given up at the commit.
    assert!(writer.run(b"PRAGMA auto_vacuum=1").unwrap().is_empty());
    assert_eq!(
        answered(&writer, "PRAGMA auto_vacuum"),
        [alloc::vec![Value::Int(1)]]
    );
    let text = alloc::string::String::from_utf8(alloc::vec![b'x'; 1100]).unwrap();
    writer
        .run(alloc::format!("INSERT INTO t VALUES('{text}')").as_bytes())
        .unwrap();
    let held = answered(&writer, "PRAGMA page_count");
    writer.run(b"DELETE FROM t").unwrap();
    assert_ne!(answered(&writer, "PRAGMA page_count"), held);
    assert_eq!(
        answered(&writer, "PRAGMA freelist_count"),
        [alloc::vec![Value::Int(0)]]
    );
    // A word no way carries names none, which changes nothing, and
    // `none` over a file that vacuums itself changes nothing either.
    assert!(writer.run(b"PRAGMA auto_vacuum=bogus").unwrap().is_empty());
    assert!(writer.run(b"PRAGMA auto_vacuum=none").unwrap().is_empty());
    assert_eq!(
        answered(&writer, "PRAGMA auto_vacuum"),
        [alloc::vec![Value::Int(1)]]
    );
    // A file that vacuums itself not at all is left as it stands.
    let mut plain = ran(&["CREATE TABLE t(a)"]);
    assert!(plain.run(b"PRAGMA auto_vacuum=2").unwrap().is_empty());
    assert_eq!(
        answered(&plain, "PRAGMA auto_vacuum"),
        [alloc::vec![Value::Int(0)]]
    );
}

/// `incrvacuum-5.3.*`: a step moves the page at the end of the file into
/// a free page below it, taking that page off the free list wherever the
/// list holds it.
#[test]
fn what_a_step_moves_the_last_page_into() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.vacuuming(true);
    writer.run(b"CREATE TABLE t(a, b)").unwrap();
    writer.run(b"CREATE INDEX i ON t(b)").unwrap();
    let text = alloc::string::String::from_utf8(alloc::vec![b'y'; 400]).unwrap();
    let long = alloc::string::String::from_utf8(alloc::vec![b'w'; 1500]).unwrap();
    for row in 1..=300_i64 {
        // Every fifth row runs onto a chain of overflow pages and the
        // others do not, so a page holds a cell of each.
        let held = if row % 5 == 0 { &long } else { &text };
        writer
            .run(alloc::format!("INSERT INTO t VALUES({row}, '{held}')").as_bytes())
            .unwrap();
    }
    // The rows below the last hundred are deleted, so the free list holds
    // the pages they took and the pages at the end of the file are the
    // ones the table still holds. The list runs over more than one trunk,
    // because a trunk of a page of 512 bytes carries 120 leaves.
    writer.run(b"DELETE FROM t WHERE a<=200").unwrap();
    let Value::Int(free) = answered(&writer, "PRAGMA freelist_count")[0][0] else {
        panic!("the pragma answers a number")
    };
    assert!(free > 120, "the free list runs over more than one trunk");
    let steps = writer.run(b"PRAGMA incremental_vacuum").unwrap().len();
    assert!(steps > 120, "every page of the list is given up");
    assert_eq!(
        answered(&writer, "PRAGMA freelist_count"),
        [alloc::vec![Value::Int(0)]]
    );
    assert_eq!(
        answered(&writer, "PRAGMA integrity_check"),
        [alloc::vec![Value::Text(b"ok".to_vec())]]
    );
    // Every row the delete left stands, and the index answers for it.
    assert_eq!(
        answered(&writer, "SELECT count(*) FROM t"),
        [alloc::vec![Value::Int(100)]]
    );
    assert_eq!(
        answered(&writer, "SELECT a FROM t WHERE b=x'00' OR a=300"),
        [alloc::vec![Value::Int(300)]]
    );
}

/// What a step refuses over a file whose pointer map or whose free list
/// says what the pages of the file do not.
#[test]
fn what_a_step_refuses_over_a_file_that_says_what_it_does_not_hold() {
    let held = || {
        let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
        writer.vacuuming(true);
        writer.run(b"CREATE TABLE t(a, b)").unwrap();
        let text = alloc::string::String::from_utf8(alloc::vec![b'z'; 1100]).unwrap();
        for row in 1..=4_i64 {
            writer
                .run(alloc::format!("INSERT INTO t VALUES({row}, '{text}')").as_bytes())
                .unwrap();
        }
        writer.run(b"DELETE FROM t WHERE a<=2").unwrap();
        writer.written()
    };
    // The map says the last page is the root of a tree, which no page
    // past the end the file is cut back to is.
    let mut image = held();
    let last = image.len() / 1024;
    let at = 1024 + (last - 3) * 5;
    image[at] = 1;
    let mut writer = Writer::opened(&image).unwrap();
    assert_eq!(
        writer.run(b"PRAGMA incremental_vacuum"),
        Err(Error::Image(crate::error::Error::Page(
            u32::try_from(last).unwrap()
        )))
    );
    // The free list names no page at or below the end, because the
    // header says it begins nowhere.
    let mut image = held();
    for byte in image.iter_mut().take(36).skip(32) {
        *byte = 0;
    }
    let mut writer = Writer::opened(&image).unwrap();
    assert_eq!(
        writer.run(b"PRAGMA incremental_vacuum"),
        Err(Error::Image(crate::error::Error::Balance))
    );
    // The header counts more free pages than the file holds pages.
    let mut image = held();
    for (at, byte) in (36..40).enumerate() {
        image[byte] = [0xff, 0xff, 0xff, 0xff][at];
    }
    let mut writer = Writer::opened(&image).unwrap();
    assert!(writer.run(b"PRAGMA incremental_vacuum").is_err());
}

/// A file that ends on a pointer-map page gives that page up and moves
/// nothing, because the map page carries the entries of the pages the
/// file gave up before it.
#[test]
fn what_a_step_over_a_file_that_ends_on_a_map_page_gives_up() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.vacuuming(true);
    writer.run(b"CREATE TABLE t(a, b)").unwrap();
    let text = alloc::string::String::from_utf8(alloc::vec![b'y'; 400]).unwrap();
    for row in 1..=150_i64 {
        writer
            .run(alloc::format!("INSERT INTO t VALUES({row}, '{text}')").as_bytes())
            .unwrap();
    }
    // A page of 512 bytes carries 102 entries, so page 105 is the second
    // map page. The image is cut back to it and the header counts the
    // pages and the free list of the file it names.
    let mut image = writer.written();
    image.truncate(105 * 512);
    for (at, byte) in (28..32).enumerate() {
        image[byte] = 105_u32.to_be_bytes()[at];
    }
    for (at, byte) in (36..40).enumerate() {
        image[byte] = 50_u32.to_be_bytes()[at];
    }
    let mut writer = Writer::opened(&image).unwrap();
    assert_eq!(
        writer.run(b"PRAGMA incremental_vacuum(1)").unwrap().len(),
        1
    );
    assert_eq!(writer.written().len(), 104 * 512);
}

/// `avtrans-4.98` of `test/avtrans.test`: the second of two tables
/// dropped in a row out of a file that vacuums itself whole.
#[test]
fn the_second_table_a_file_that_vacuums_itself_drops_is_dropped() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.vacuuming(false);
    for sql in [
        b"CREATE TABLE one(a int PRIMARY KEY, b text)".as_slice(),
        b"INSERT INTO one VALUES(1,'one')",
        b"CREATE TABLE two(a int PRIMARY KEY, b text)",
        b"INSERT INTO two VALUES(1,'I')",
        b"DROP TABLE one",
        b"DROP TABLE two",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(
        writer
            .run(b"PRAGMA integrity_check")
            .unwrap()
            .first()
            .and_then(|row| row.first()),
        Some(&Value::Text(b"ok".to_vec()))
    );
}

/// `autovacuum-2.2.3` of `test/autovacuum.test`: the roots of a file
/// that vacuums itself run from page three up with no gap.
#[test]
fn a_root_of_a_file_that_vacuums_itself_takes_the_page_after_the_largest() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.vacuuming(false);
    writer.run(b"CREATE TABLE av1(x)").unwrap();
    // The row fills the page and runs onto a chain, so the page the
    // second root takes holds part of that chain.
    writer
        .run(b"INSERT INTO av1 VALUES(zeroblob(3000))")
        .unwrap();
    writer.run(b"CREATE TABLE av2(x)").unwrap();
    writer.run(b"CREATE TABLE av3(x)").unwrap();
    let bytes = writer.written();
    let read = |sql: &[u8]| {
        Database::open(&bytes)
            .unwrap()
            .query(sql)
            .unwrap()
            .rows
            .into_iter()
            .flatten()
            .collect::<alloc::vec::Vec<Value>>()
    };
    assert_eq!(
        read(b"SELECT rootpage FROM sqlite_master ORDER BY rootpage"),
        [Value::Int(3), Value::Int(4), Value::Int(5)]
    );
    assert_eq!(read(b"SELECT count(*) FROM av1"), [Value::Int(1)]);
    assert_eq!(
        writer
            .run(b"PRAGMA integrity_check")
            .unwrap()
            .first()
            .and_then(|row| row.first()),
        Some(&Value::Text(b"ok".to_vec()))
    );
}

/// `trans-9.2.5` of `test/trans.test`: what a connection was told for
/// `PRAGMA fullfsync`, which says a sync holds the file on the disk of
/// the machine and not only in the cache of its driver.
#[test]
fn what_the_connection_was_told_for_fullfsync() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    assert!(!writer.kept().fullfsync());
    writer.run(b"PRAGMA fullfsync=ON").unwrap();
    assert!(writer.kept().fullfsync());
    writer.run(b"PRAGMA fullfsync=OFF").unwrap();
    assert!(!writer.kept().fullfsync());
}

/// The steps of `incrvacuum3.test`: a file in incremental vacuum that a
/// transaction writes, vacuums and rolls back stands after every one of
/// them.
#[test]
fn what_a_vacuum_inside_a_transaction_leaves() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.randomness(11);
    let check = |writer: &Writer, at: usize| {
        let rows = answered(writer, "PRAGMA integrity_check");
        let held = rows
            .first()
            .and_then(|row| row.first())
            .and_then(Value::text)
            .unwrap_or_default();
        assert_eq!(
            alloc::string::String::from_utf8_lossy(&held),
            "ok",
            "step {at}"
        );
        // The file the connection holds is read again from its bytes,
        // which is what a copy of it is read as.
        let image = writer.written();
        let database = Database::open(&image).unwrap_or_else(|_| panic!("step {at} opens"));
        let rows = database
            .query(b"PRAGMA integrity_check")
            .expect("rows")
            .rows;
        let held = rows
            .first()
            .and_then(|row| row.first())
            .and_then(Value::text)
            .unwrap_or_default();
        assert_eq!(
            alloc::string::String::from_utf8_lossy(&held),
            "ok",
            "step {at} read again"
        );
    };
    for (at, sql) in [
        "PRAGMA auto_vacuum = 2",
        "CREATE TABLE t1(x UNIQUE)",
        "INSERT INTO t1 VALUES(randomblob(400))",
        "INSERT INTO t1 VALUES(randomblob(400))",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "DELETE FROM t1 WHERE rowid%8",
        "BEGIN",
        "PRAGMA incremental_vacuum = 100",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "ROLLBACK",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "PRAGMA incremental_vacuum = 1000",
    ]
    .into_iter()
    .enumerate()
    {
        writer.run(sql.as_bytes()).unwrap_or_else(|error| {
            panic!("step {at} `{sql}`: {}", error.message());
        });
        check(&writer, at);
    }
}

/// A file in incremental vacuum whose free list is given up page by
/// page stands, and the pages come off the end of the file.
#[test]
fn what_an_incremental_vacuum_of_many_pages_leaves() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.randomness(7);
    for sql in [
        "PRAGMA auto_vacuum = 2",
        "CREATE TABLE t1(x UNIQUE)",
        "INSERT INTO t1 VALUES(randomblob(400))",
        "INSERT INTO t1 VALUES(randomblob(400))",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "DELETE FROM t1 WHERE rowid%8",
    ] {
        writer.run(sql.as_bytes()).expect(sql);
    }
    // Every page the deletions freed is given up, and the file holds
    // what it held before them.
    let free = |writer: &Writer| -> i64 {
        answered(writer, "PRAGMA freelist_count")
            .first()
            .and_then(|row| row.first())
            .map_or(-1, Value::to_integer)
    };
    assert!(free(&writer) > 0, "the deletions freed pages");
    writer.run(b"PRAGMA incremental_vacuum = 1000").unwrap();
    assert_eq!(free(&writer), 0);
    assert_eq!(
        answered(&writer, "PRAGMA integrity_check")
            .first()
            .and_then(|row| row.first())
            .and_then(Value::text),
        Some(b"ok".to_vec())
    );
    // The file the connection wrote is read again from its bytes, which
    // is what a copy of it is read as.
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let rows = database.query(b"PRAGMA integrity_check").unwrap().rows;
    assert_eq!(
        rows.first()
            .and_then(|row| row.first())
            .and_then(Value::text),
        Some(b"ok".to_vec())
    );
}

/// `PRAGMA incremental_vacuum` answers one row of no column per page it
/// gives up, which is what the loop of `OP_IncrVacuum` and its
/// `OP_ResultRow` of no register answer and what the tester counts the
/// pages of a vacuum by.
#[test]
fn what_an_incremental_vacuum_answers_a_row_for() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.randomness(11);
    for sql in [
        "PRAGMA auto_vacuum = 2",
        "CREATE TABLE t1(x)",
        "INSERT INTO t1 VALUES(randomblob(400))",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "INSERT INTO t1 SELECT randomblob(400) FROM t1",
        "DELETE FROM t1",
    ] {
        writer.run(sql.as_bytes()).expect(sql);
    }
    let free = answered(&writer, "PRAGMA freelist_count")
        .first()
        .and_then(|row| row.first())
        .map_or(-1, Value::to_integer);
    assert!(free > 1, "the deletions freed more than one page");
    // One row per page, each of no value, and the pragma names no column
    // of its own.
    let rows = writer.run(b"PRAGMA incremental_vacuum").unwrap();
    assert_eq!(i64::try_from(rows.len()).unwrap_or(-1), free);
    assert!(rows.iter().all(alloc::vec::Vec::is_empty));
    assert!(
        crate::pragma::of_name(b"incremental_vacuum")
            .expect("the pragma")
            .columns(b"incremental_vacuum", false)
            .is_empty()
    );
    // A vacuum that finds no page to give up answers no row.
    assert!(writer.run(b"PRAGMA incremental_vacuum").unwrap().is_empty());
}
