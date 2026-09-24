// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `PRAGMA case_sensitive_like` and `PRAGMA default_cache_size`, the two
//! pragmas a connection holds outside [`crate::pragma::HELD`].

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// A connection over one table of three spellings of one word.
fn spelled() -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(x)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES('abc'),('ABC'),('aBc')")
        .unwrap();
    writer
}

/// What the three spellings the statement matches are, run over a
/// database the connection opens as the caller of this crate opens one.
fn matched(writer: &Writer, sql: &[u8]) -> alloc::string::String {
    let bytes = writer.written();
    let database = Database::open(&bytes)
        .unwrap()
        .sensitively(writer.sensitive());
    let mut out = alloc::string::String::new();
    for row in database.query(sql).unwrap().rows {
        for value in row {
            out.push_str(&alloc::string::String::from_utf8_lossy(
                &value.text().unwrap_or_default(),
            ));
        }
    }
    out
}

/// `PRAGMA case_sensitive_like` sets whether `LIKE` tells the twenty-six
/// letters apart, answers no row either way, and leaves `GLOB` as it is.
#[test]
fn what_like_matches_where_the_letters_are_told_apart() {
    let mut writer = spelled();
    let read = b"SELECT x FROM t WHERE x LIKE 'ABC' ORDER BY rowid".as_slice();
    assert_eq!(matched(&writer, read), "abcABCaBc");
    // The pragma answers no row, set or read, which `PragFlg_NoColumns`
    // says of its name.
    assert!(
        writer
            .run(b"PRAGMA case_sensitive_like=on")
            .unwrap()
            .is_empty()
    );
    assert!(
        writer
            .run(b"PRAGMA case_sensitive_like")
            .unwrap()
            .is_empty()
    );
    assert_eq!(matched(&writer, read), "ABC");
    // The function of the same name is the operator, so it tells the
    // letters apart as well.
    assert_eq!(matched(&writer, b"SELECT like('ab%','AB1')"), "0");
    // `GLOB` tells them apart whatever this pragma says.
    assert_eq!(
        matched(&writer, b"SELECT x FROM t WHERE x GLOB 'ABC'"),
        "ABC"
    );
    // A word that names no truth value turns the pragma off, which is
    // `sqlite3GetBoolean(zRight, 0)`.
    writer.run(b"PRAGMA case_sensitive_like=maybe").unwrap();
    assert_eq!(matched(&writer, read), "abcABCaBc");
    writer.run(b"PRAGMA case_sensitive_like=on").unwrap();
    writer.run(b"PRAGMA case_sensitive_like=off").unwrap();
    assert_eq!(matched(&writer, read), "abcABCaBc");
}

/// A statement that writes reads the pragma as a statement that reads
/// does, and a database the caller opens without it folds the letters.
#[test]
fn what_a_statement_that_writes_matches_under_the_pragma() {
    let mut writer = spelled();
    writer.run(b"PRAGMA case_sensitive_like=on").unwrap();
    writer.run(b"DELETE FROM t WHERE x LIKE 'ABC'").unwrap();
    assert_eq!(
        matched(&writer, b"SELECT x FROM t ORDER BY rowid"),
        "abcaBc"
    );
    // A database told no pragma folds the letters, so it matches both
    // rows that are left.
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    assert_eq!(
        database
            .query(b"SELECT count(*) FROM t WHERE x LIKE 'ABC'")
            .unwrap()
            .rows,
        alloc::vec![alloc::vec![Value::Int(2)]]
    );
    // An `UPDATE` reads the pragma as the `DELETE` did.
    writer
        .run(b"UPDATE t SET x='z' WHERE x LIKE 'ABC'")
        .unwrap();
    assert_eq!(
        matched(&writer, b"SELECT x FROM t ORDER BY rowid"),
        "abcaBc"
    );
    writer.run(b"PRAGMA case_sensitive_like=off").unwrap();
    writer
        .run(b"UPDATE t SET x='z' WHERE x LIKE 'ABC'")
        .unwrap();
    assert_eq!(matched(&writer, b"SELECT x FROM t ORDER BY rowid"), "zz");
}

/// `PRAGMA default_cache_size` writes the header word at offset 48 and
/// the cache size of the connection together, and the word stands after
/// a reopen.
#[test]
fn what_the_header_word_holds_of_the_default_cache_size() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    let read = |writer: &mut Writer| {
        (
            writer.run(b"PRAGMA cache_size").unwrap(),
            writer.run(b"PRAGMA default_cache_size").unwrap(),
        )
    };
    let fallback = alloc::vec![alloc::vec![Value::Int(-2000)]];
    assert_eq!(read(&mut writer), (fallback.clone(), fallback.clone()));
    // The minus sign is dropped, so a size written either way answers
    // the same, and setting it answers no row.
    assert!(
        writer
            .run(b"PRAGMA default_cache_size=-123")
            .unwrap()
            .is_empty()
    );
    let one_two_three = alloc::vec![alloc::vec![Value::Int(123)]];
    assert_eq!(
        read(&mut writer),
        (one_two_three.clone(), one_two_three.clone())
    );
    // A reopen reads the word, which `sqlite3InitOne` does through
    // `sqlite3AbsInt32`.
    let bytes = writer.written();
    let mut again = Writer::opened(&bytes).unwrap();
    assert_eq!(
        read(&mut again),
        (one_two_three.clone(), one_two_three.clone())
    );
    // `VACUUM` writes the file again and keeps the word.
    again.run(b"VACUUM").unwrap();
    assert_eq!(read(&mut again), (one_two_three.clone(), one_two_three));
    // A word of nought is the built-in size for the pragma that reads
    // the word and nought for the connection, which is
    // `pDb->pSchema->cache_size = size`.
    again.run(b"PRAGMA default_cache_size=0").unwrap();
    assert_eq!(
        read(&mut again),
        (alloc::vec![alloc::vec![Value::Int(0)]], fallback.clone())
    );
    // Text that names no number is nought, which `sqlite3Atoi` answers
    // for it, and so is a number no signed word of 32 bits holds.
    again.run(b"PRAGMA default_cache_size=500").unwrap();
    again.run(b"PRAGMA default_cache_size='abc'").unwrap();
    assert_eq!(read(&mut again).1, fallback);
    again.run(b"PRAGMA default_cache_size=-3000000000").unwrap();
    assert_eq!(
        read(&mut again).1,
        alloc::vec![alloc::vec![Value::Int(-2000)]]
    );
}

/// `PRAGMA cache_size` is the connection's own, so it stands over the
/// header word and no reopen holds it.
#[test]
fn what_the_connection_holds_of_its_cache_size() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"PRAGMA default_cache_size=-123").unwrap();
    writer.run(b"PRAGMA cache_size=-4321").unwrap();
    assert_eq!(
        writer.run(b"PRAGMA cache_size").unwrap(),
        alloc::vec![alloc::vec![Value::Int(-4321)]]
    );
    // The header word is what it was, because `PRAGMA cache_size` writes
    // no byte of the file.
    assert_eq!(
        writer.run(b"PRAGMA default_cache_size").unwrap(),
        alloc::vec![alloc::vec![Value::Int(123)]]
    );
    // The pragma that writes the word writes the connection's own size
    // as well, so it stands over what the connection was told.
    writer.run(b"PRAGMA default_cache_size=456").unwrap();
    assert_eq!(
        writer.run(b"PRAGMA cache_size").unwrap(),
        alloc::vec![alloc::vec![Value::Int(456)]]
    );
    // A reopen reads the word and holds nothing the connection was told.
    let bytes = writer.written();
    let mut again = Writer::opened(&bytes).unwrap();
    assert_eq!(
        again.run(b"PRAGMA cache_size").unwrap(),
        alloc::vec![alloc::vec![Value::Int(456)]]
    );
}

/// A header word below nought answers its negation, which is a file
/// another writer left the word in.
#[test]
fn what_a_header_word_below_nought_answers() {
    assert_eq!(crate::pragma::default_cache(0), -2000);
    assert_eq!(crate::pragma::default_cache(500), 500);
    // 0xFFFFFF00 as a signed word of 32 bits is -256.
    assert_eq!(crate::pragma::default_cache(0xffff_ff00), 256);
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    let mut bytes = writer.written();
    bytes.splice(48..52, [0xff, 0xff, 0xff, 0x00]);
    let mut again = Writer::opened(&bytes).unwrap();
    assert_eq!(
        again.run(b"PRAGMA default_cache_size").unwrap(),
        alloc::vec![alloc::vec![Value::Int(256)]]
    );
    assert_eq!(
        again.run(b"PRAGMA cache_size").unwrap(),
        alloc::vec![alloc::vec![Value::Int(256)]]
    );
}

/// The three words of the header a pragma writes, and the two pragmas
/// that refuse rather than change what they name.
#[test]
fn what_a_pragma_writes_of_the_header_words() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    let held = writer.run(b"PRAGMA schema_version").unwrap();
    assert_eq!(held, alloc::vec![alloc::vec![Value::Int(1)]]);
    for (sql, read, value) in [
        (
            b"PRAGMA schema_version=105".as_slice(),
            b"PRAGMA schema_version".as_slice(),
            105,
        ),
        (b"PRAGMA user_version=7", b"PRAGMA user_version", 7),
        (
            b"PRAGMA application_id=1234",
            b"PRAGMA application_id",
            1234,
        ),
    ] {
        assert!(writer.run(sql).unwrap().is_empty());
        assert_eq!(
            writer.run(read).unwrap(),
            alloc::vec![alloc::vec![Value::Int(value)]]
        );
    }
    // The words stand in the file, so a reopen reads them.
    let bytes = writer.written();
    let mut again = Writer::opened(&bytes).unwrap();
    assert_eq!(
        again.run(b"PRAGMA user_version").unwrap(),
        alloc::vec![alloc::vec![Value::Int(7)]]
    );
    // Text that names no number writes nought, which `sqlite3Atoi`
    // answers for it, and a number below nought is kept as the bytes of
    // a word of 32 bits.
    again.run(b"PRAGMA user_version='abc'").unwrap();
    assert_eq!(
        again.run(b"PRAGMA user_version").unwrap(),
        alloc::vec![alloc::vec![Value::Int(0)]]
    );
    again.run(b"PRAGMA user_version=-1").unwrap();
    assert_eq!(
        again.run(b"PRAGMA user_version").unwrap(),
        alloc::vec![alloc::vec![Value::Int(4_294_967_295)]]
    );
    // A word `PragFlg_ReadOnly` stands against reads the file whatever
    // stands after the equals sign, and so does `PRAGMA page_count`.
    assert_eq!(
        again.run(b"PRAGMA freelist_count=3").unwrap(),
        alloc::vec![alloc::vec![Value::Int(0)]]
    );
    assert_eq!(
        again.run(b"PRAGMA page_count=44").unwrap(),
        again.run(b"PRAGMA page_count").unwrap()
    );
    assert_eq!(
        again.run(b"PRAGMA data_version=9").unwrap(),
        alloc::vec![alloc::vec![Value::Int(1)]]
    );
    // A name no encoding carries is refused whatever the file holds,
    // because `PragTyp_ENCODING` reads the name first.
    assert_eq!(
        again.run(b"PRAGMA encoding=bogus").unwrap_err().message(),
        "unsupported encoding: bogus"
    );
    assert!(again.run(b"PRAGMA encoding='UTF-8'").unwrap().is_empty());
    // `PRAGMA synchronous` refuses inside a transaction, which
    // `PragTyp_SYNCHRONOUS` does because a commit is waiting, and the
    // connection says it has one open, which `sqlite3_get_autocommit`
    // answers nought for.
    assert!(!again.began());
    again.run(b"BEGIN").unwrap();
    assert!(again.began());
    assert_eq!(
        again.run(b"PRAGMA synchronous=OFF").unwrap_err().message(),
        "Safety level may not be changed inside a transaction"
    );
    assert_eq!(
        again.run(b"PRAGMA synchronous").unwrap(),
        alloc::vec![alloc::vec![Value::Int(2)]]
    );
    again.run(b"COMMIT").unwrap();
    assert!(!again.began());
    assert!(again.run(b"PRAGMA synchronous=OFF").unwrap().is_empty());
    assert_eq!(
        again.run(b"PRAGMA synchronous").unwrap(),
        alloc::vec![alloc::vec![Value::Int(0)]]
    );
}

/// A connection under `PRAGMA query_only` refuses every statement that
/// writes a page or a word of the header, leaves the file as the
/// statement found it, and takes them again once the pragma is off.
#[test]
fn what_a_connection_under_query_only_refuses() {
    let mut writer = spelled();
    let was = writer.written();
    writer.run(b"PRAGMA query_only=ON").unwrap();
    for sql in [
        b"INSERT INTO t VALUES('d')".as_slice(),
        b"DELETE FROM t",
        b"UPDATE t SET x='e'",
        b"CREATE TABLE u(y)",
        b"CREATE INDEX i ON t(x)",
        b"DROP TABLE t",
        b"ANALYZE",
        b"VACUUM",
    ] {
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "attempt to write a readonly database",
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
    assert_eq!(writer.written(), was);
    // A statement that writes nothing runs under the pragma, which
    // `OP_Transaction` reads the write flag of the statement for.
    assert!(writer.run(b"REINDEX").unwrap().is_empty());
    assert!(
        writer
            .run(b"DELETE FROM t WHERE x='none'")
            .unwrap()
            .is_empty()
    );
    writer.run(b"PRAGMA query_only=OFF").unwrap();
    assert!(writer.run(b"INSERT INTO t VALUES('d')").unwrap().is_empty());
    assert_eq!(
        Database::open(&writer.written())
            .unwrap()
            .query(b"SELECT count(*) FROM t")
            .unwrap()
            .rows,
        alloc::vec![alloc::vec![Value::Int(4)]]
    );
}

/// A connection over a file the client may only read refuses every
/// statement that would write a page of it, and writes a database of its
/// own as before.
#[test]
fn what_a_connection_over_a_file_it_may_only_read_refuses() {
    let mut writer = spelled();
    let was = writer.written();
    writer.only_reading();
    for sql in [
        b"INSERT INTO t VALUES('d')".as_slice(),
        b"DELETE FROM t",
        b"CREATE TABLE u(y)",
    ] {
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "attempt to write a readonly database",
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
    assert_eq!(writer.written(), was);
    // The pragma answers what the connection was told and not what the
    // file allows, which `SQLITE_QueryOnly` of `sqlite3Pragma` holds.
    assert_eq!(
        writer.run(b"PRAGMA query_only").unwrap(),
        alloc::vec![alloc::vec![Value::Int(0)]]
    );
    // The temp schema is a database of the connection's own, so the
    // permissions of the file the client holds say nothing about it.
    writer.run(b"CREATE TEMP TABLE v(y)").unwrap();
    writer.run(b"INSERT INTO v VALUES(1)").unwrap();
    let temp = writer.attached_written(b"temp").expect("the temp schema");
    assert_eq!(
        Database::open(&temp)
            .unwrap()
            .query(b"SELECT count(*) FROM v")
            .unwrap()
            .rows,
        alloc::vec![alloc::vec![Value::Int(1)]]
    );
}
