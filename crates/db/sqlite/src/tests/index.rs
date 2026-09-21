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

/// The rowids the fixture answers for `sql`.
fn keys(bytes: &[u8], sql: &[u8]) -> Vec<Value> {
    answers(bytes, sql)
        .unwrap()
        .into_iter()
        .filter_map(|row| row.first().cloned())
        .collect()
}

#[test]
fn an_or_whose_every_branch_names_a_key_answers_the_branches_one_after_another() {
    // `q='b'` reaches `mq` and `p=2` reaches `mpq`, so the rows come in
    // the order the branches are written and not in the table's.
    let rows = answers(super::INDEXED, b"SELECT rowid, p FROM m WHERE q='b' OR p=2").unwrap();
    assert_eq!(
        rows,
        [
            alloc::vec![Value::Int(2), Value::Int(1)],
            alloc::vec![Value::Int(3), Value::Int(2)],
            alloc::vec![Value::Int(4), Value::Int(2)],
        ]
    );
}

#[test]
fn a_row_two_branches_of_an_or_name_is_answered_once() {
    let rows = keys(super::INDEXED, b"SELECT rowid FROM m WHERE p=2 OR q='c'");
    assert_eq!(rows, [Value::Int(3), Value::Int(4)]);
}

#[test]
fn an_or_is_read_where_a_term_of_the_and_spine_above_it_names_no_key() {
    let rows = keys(
        super::INDEXED,
        b"SELECT rowid FROM m WHERE r IS NOT NULL AND (q='b' OR p=2)",
    );
    assert_eq!(rows, [Value::Int(2), Value::Int(3)]);
}

#[test]
fn an_or_one_branch_of_which_names_no_key_leaves_the_table_scanned() {
    // `mr` holds its entries in the other order, so no index answers
    // `r=30` and the branch holds any row.
    let rows = keys(super::INDEXED, b"SELECT rowid FROM m WHERE q='b' OR r=30");
    assert_eq!(rows, [Value::Int(2), Value::Int(3)]);
}

#[test]
fn a_where_that_holds_no_or_is_read_by_the_table() {
    let rows = keys(super::INDEXED, b"SELECT rowid FROM m WHERE q IS NOT NULL");
    assert_eq!(
        rows,
        [Value::Int(1), Value::Int(2), Value::Int(3), Value::Int(4)]
    );
}

#[test]
fn an_index_held_backwards_is_walked_from_the_high_end_of_the_bounds() {
    // `mr` holds `r` from the largest value down, so a walk of it
    // answers the rows in that order.
    assert_eq!(
        listed(super::INDEXED, b"SELECT rowid FROM m WHERE r>20"),
        "5,3"
    );
    assert_eq!(
        listed(super::INDEXED, b"SELECT rowid FROM m WHERE r<=30"),
        "3,2,1"
    );
    assert_eq!(
        listed(super::INDEXED, b"SELECT rowid FROM m WHERE r>10 AND r<50"),
        "3,2"
    );
    assert_eq!(
        listed(super::INDEXED, b"SELECT rowid FROM m ORDER BY r DESC"),
        "5,3,2,1,4"
    );
}

#[test]
fn an_or_whose_branch_names_an_index_the_descent_cannot_read_is_scanned() {
    let bytes = damaged(b"mq", 1);
    let rows = keys(&bytes, b"SELECT rowid FROM m WHERE q='A' OR p=3");
    assert_eq!(rows, [Value::Int(1), Value::Int(3), Value::Int(5)]);
}

#[test]
fn an_or_whose_branch_names_an_index_the_walk_cannot_read_refuses() {
    let bytes = damaged(b"mq", 3);
    assert_eq!(
        answers(&bytes, b"SELECT rowid FROM m WHERE q='A' OR p=3"),
        Err(crate::db::Error::Image(crate::error::Error::Overrun))
    );
}

#[test]
fn a_term_that_carries_a_collate_names_no_key_of_an_index() {
    // `mq` holds the entries of `q` under `NOCASE`, so a term that
    // compares under `BINARY` asks about an order the index is not in.
    let rows = keys(
        super::INDEXED,
        b"SELECT rowid FROM m WHERE q='A' COLLATE binary",
    );
    assert_eq!(rows, [Value::Int(1), Value::Int(3)]);
}

#[test]
fn a_walk_of_an_index_is_held_between_the_bounds_the_terms_name() {
    // `mpq` holds `p` and then `q`, so a term over `p` holds the walk
    // to a range of it and a second term over `q` holds the key.
    for (sql, wanted) in [
        (
            b"SELECT rowid FROM m WHERE p>=2".as_slice(),
            [3, 4, 5].as_slice(),
        ),
        (b"SELECT rowid FROM m WHERE p>2", &[5]),
        (b"SELECT rowid FROM m WHERE p<=1", &[1, 2]),
        (b"SELECT rowid FROM m WHERE p<2", &[1, 2]),
        (b"SELECT rowid FROM m WHERE p>1 AND p<3", &[3, 4]),
        (b"SELECT rowid FROM m WHERE p>=1 AND p<=2", &[1, 2, 3, 4]),
        (b"SELECT rowid FROM m WHERE p=2 AND q='c'", &[4]),
        (b"SELECT rowid FROM m WHERE p=2 AND q>'b'", &[4]),
    ] {
        let rows = keys(super::INDEXED, sql);
        let held: Vec<Value> = wanted.iter().map(|key| Value::Int(*key)).collect();
        assert_eq!(
            rows,
            held,
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
}

#[test]
fn what_index_no_term_reaches_a_key_or_a_bound_of() {
    // `ea` holds what an expression answers, `ec` holds its entries
    // under another collation, and `eb` holds fewer rows than the table
    // has.
    assert_eq!(
        keys(super::INDEXED, b"SELECT rowid FROM e WHERE a>1"),
        [Value::Int(2), Value::Int(3)]
    );
    assert_eq!(
        keys(super::INDEXED, b"SELECT rowid FROM e WHERE b>0"),
        [Value::Int(1), Value::Int(3)]
    );
    // A statement that names a column no index begins with reads the
    // table, and one that names none reads it as well.
    assert_eq!(
        keys(super::INDEXED, b"SELECT rowid FROM m WHERE q>'b'"),
        [Value::Int(4)]
    );
    assert_eq!(
        keys(super::INDEXED, b"SELECT rowid FROM m WHERE r IS NULL"),
        [Value::Int(4)]
    );
}

#[test]
fn a_walk_held_to_a_key_answers_the_order_of_the_columns_after_it() {
    // `mpq` holds `p` and then `q`, so a walk held to one `p` answers
    // its entries in the order of `q`, and one held to a range of `p`
    // answers them in the order of `p` and then `q`.
    for (sql, wanted) in [
        (
            b"SELECT rowid FROM m WHERE p=2 ORDER BY q".as_slice(),
            [3, 4].as_slice(),
        ),
        (b"SELECT rowid FROM m WHERE p=2 ORDER BY p, q", &[3, 4]),
        (b"SELECT rowid FROM m WHERE p>=2 ORDER BY p, q", &[3, 4, 5]),
        (b"SELECT rowid FROM m WHERE p=2 ORDER BY q DESC", &[4, 3]),
        (b"SELECT rowid FROM m WHERE p>=2 ORDER BY q", &[5, 3, 4]),
    ] {
        let rows = keys(super::INDEXED, sql);
        let held: Vec<Value> = wanted.iter().map(|key| Value::Int(*key)).collect();
        assert_eq!(
            rows,
            held,
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
}

#[test]
fn an_entry_that_runs_onto_an_overflow_page_is_read_whole_to_be_bounded() {
    // The index `ot` holds one entry of eight thousand letters, which no
    // page holds, so the value the high end of the bounds is compared
    // against is read off the chain that follows.
    let rows = answers(super::INDEXED, b"SELECT a FROM o WHERE t<'z'").unwrap();
    assert_eq!(
        rows,
        [alloc::vec![Value::Int(2)], alloc::vec![Value::Int(1)]]
    );
}

#[test]
fn a_column_of_real_affinity_reaches_no_key_and_no_bound() {
    use crate::change::Writer;
    use crate::header::Encoding;
    // A whole number stands in the row and in the entry as itself, and
    // `OP_RealAffinity` reads it out of the row as the real nearest it,
    // which is a larger number: an entry held to the whole number would
    // leave the row out.
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t0(c0 REAL UNIQUE)".as_slice(),
        b"INSERT INTO t0(c0) VALUES (3175546974276630385)",
    ] {
        writer.run(sql).unwrap();
    }
    let bytes = writer.written();
    assert_eq!(
        answers(&bytes, b"SELECT 1 FROM t0 WHERE 3175546974276630385 < c0").unwrap(),
        [alloc::vec![Value::Int(1)]]
    );
    assert_eq!(
        answers(&bytes, b"SELECT 1 FROM t0 WHERE c0 = 3175546974276630528").unwrap(),
        [alloc::vec![Value::Int(1)]]
    );
}

#[test]
fn a_bound_the_affinity_of_the_column_changes_is_widened() {
    // `a` is a whole number and the bound is text, so the affinity of
    // the column makes the bound a number: the walk begins at the entry
    // holding it rather than at the one after it, which is one row the
    // term leaves out and the `WHERE` reads again.
    let rows = answers(
        super::INDEXED,
        b"SELECT count(*), min(a) FROM k WHERE a > '5'",
    )
    .unwrap();
    assert_eq!(rows, [alloc::vec![Value::Int(95), Value::Int(6)]]);
    // A column that compares as text and a bound that is a number reach
    // no row, because a number stands before every text.
    assert!(keys(super::INDEXED, b"SELECT rowid FROM m WHERE p>'1'").is_empty());
}

/// The rowids of `sql` over the fixture, as one text.
fn listed(bytes: &[u8], sql: &[u8]) -> alloc::string::String {
    keys(bytes, sql)
        .iter()
        .map(|key| match key {
            Value::Int(number) => alloc::format!("{number}"),
            other => alloc::format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn a_pattern_that_begins_with_characters_holds_the_column_between_two_bounds() {
    // `ot` compares under `BINARY`, which `GLOB` matches under, and `mq`
    // under `NOCASE`, which `LIKE` matches under.
    assert_eq!(
        listed(super::INDEXED, b"SELECT a FROM o WHERE t GLOB 'sh*'"),
        "2"
    );
    assert_eq!(
        listed(super::INDEXED, b"SELECT rowid FROM m WHERE q LIKE 'a%'"),
        "1,3"
    );
    assert_eq!(
        keys(super::INDEXED, b"SELECT count(*) FROM k WHERE b GLOB 'v1*'"),
        [Value::Int(12)]
    );
    // A blob the pattern matches stands after every text, and the high
    // end of the bounds is a blob so that the walk reaches it.
    assert_eq!(
        listed(super::INDEXED, b"SELECT a FROM o WHERE t GLOB '12*'"),
        ""
    );
}

#[test]
fn what_pattern_holds_the_column_between_no_bounds() {
    for (sql, wanted) in [
        // `LIKE` matches under `NOCASE` and `ot` compares under
        // `BINARY`; `GLOB` matches under `BINARY` and `mq` compares
        // under `NOCASE`.
        (b"SELECT a FROM o WHERE t LIKE 'sh%'".as_slice(), "2"),
        (b"SELECT rowid FROM m WHERE q GLOB 'A*'", "1,3"),
        // A pattern that begins with a wildcard, one that begins with a
        // character past the first 128, one written after `NOT`, one
        // with an `ESCAPE`, and one that is no text of its own.
        (b"SELECT a FROM o WHERE t GLOB '*x'", ""),
        (b"SELECT a FROM o WHERE t GLOB '\xc3\xa9*'", ""),
        (b"SELECT rowid FROM m WHERE q NOT LIKE 'a%'", "2,4"),
        (b"SELECT a FROM o WHERE t LIKE 'sh%' ESCAPE '\\'", "2"),
        (
            b"SELECT rowid FROM m WHERE q LIKE q ORDER BY rowid",
            "1,2,3,4",
        ),
        (
            b"SELECT rowid FROM m WHERE 'abc' LIKE 'a%' ORDER BY rowid",
            "1,2,3,4,5",
        ),
        // A prefix that reads as a number, and a lone minus with it,
        // where the column is of no text affinity.
        (b"SELECT rowid FROM e WHERE a GLOB '1*'", ""),
        (b"SELECT rowid FROM e WHERE a GLOB '-*'", "1"),
        (
            b"SELECT a FROM k WHERE a GLOB '1*'",
            "1,10,11,12,13,14,15,16,17,18,19,100",
        ),
        (b"SELECT a FROM k WHERE a GLOB 'x1*'", ""),
        // A prefix that is no number and the value one past it that is,
        // and a prefix that begins as a number and is none.
        (b"SELECT rowid FROM e WHERE a GLOB '/*'", ""),
        (b"SELECT a FROM k WHERE a GLOB '1x*'", ""),
    ] {
        assert_eq!(
            listed(super::INDEXED, sql),
            wanted,
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
}

#[test]
fn a_pattern_no_index_matches_under_is_refused_where_no_function_answers_it() {
    assert_eq!(
        answers(super::INDEXED, b"SELECT rowid FROM m WHERE q REGEXP 'a'"),
        Err(crate::db::Error::Eval(crate::eval::Error::NoFunction(
            b"REGEXP".to_vec()
        )))
    );
}

#[test]
fn a_branch_of_an_or_that_names_the_rowid_is_read_out_of_the_tables_own_tree() {
    assert_eq!(
        listed(
            super::INDEXED,
            b"SELECT rowid FROM m WHERE rowid=2 OR q='c'"
        ),
        "2,4"
    );
    assert_eq!(
        listed(
            super::INDEXED,
            b"SELECT rowid FROM m WHERE rowid>3 OR q='b'"
        ),
        "4,5,2"
    );
    // A rowid the term compares against text names no range, so the
    // table is scanned.
    assert_eq!(
        listed(
            super::INDEXED,
            b"SELECT rowid FROM m WHERE rowid='x' OR q='b'"
        ),
        "2"
    );
    // A term about another side names no range of this one.
    assert_eq!(
        keys(
            super::INDEXED,
            b"SELECT count(*) FROM m, k WHERE (m.rowid=2 AND k.a=1) OR m.q='c'"
        ),
        [Value::Int(101)]
    );
}

#[test]
fn a_between_holds_the_column_at_both_ends() {
    assert_eq!(
        listed(
            super::INDEXED,
            b"SELECT rowid FROM m WHERE rowid BETWEEN 2 AND 4"
        ),
        "2,3,4"
    );
    assert_eq!(
        listed(
            super::INDEXED,
            b"SELECT rowid FROM m WHERE p BETWEEN 2 AND 3"
        ),
        "3,4,5"
    );
    // A `BETWEEN` written after `NOT`, one over a value that is no
    // column, and one whose end is a column read no key and no bound.
    assert_eq!(
        listed(
            super::INDEXED,
            b"SELECT rowid FROM m WHERE q NOT BETWEEN 'a' AND 'b'"
        ),
        "4"
    );
    assert_eq!(
        keys(
            super::INDEXED,
            b"SELECT count(*) FROM m WHERE 1 BETWEEN 0 AND 2"
        ),
        [Value::Int(5)]
    );
    assert_eq!(
        listed(
            super::INDEXED,
            b"SELECT rowid FROM m WHERE p BETWEEN q AND 3"
        ),
        ""
    );
}

#[test]
fn an_index_read_from_its_last_entry_answers_the_order_the_other_way_round() {
    // `mq` holds `q` forwards and `mr` holds `r` backwards, so each
    // answers the other order read backwards, and the rows come as the
    // C library answers them.
    assert_eq!(
        listed(super::INDEXED, b"SELECT rowid FROM m ORDER BY q DESC"),
        "4,2,3,1,5"
    );
    assert_eq!(
        listed(super::INDEXED, b"SELECT rowid FROM m ORDER BY r"),
        "4,1,2,3,5"
    );
    assert_eq!(
        listed(
            super::INDEXED,
            b"SELECT rowid FROM m ORDER BY p DESC, q DESC"
        ),
        "5,4,3,2,1"
    );
}

#[test]
fn an_index_tall_enough_to_carry_interior_pages_is_read_backwards_as_well() {
    // The tree of `ts` carries interior pages, so the walk descends
    // into the right-most subtree of each of them and reads the entries
    // of every page from the last to the first.
    let answer = Database::open(super::TALL_INDEX)
        .unwrap()
        .query(b"SELECT s FROM t ORDER BY s DESC")
        .unwrap();
    let forwards = Database::open(super::TALL_INDEX)
        .unwrap()
        .query(b"SELECT s FROM t ORDER BY s")
        .unwrap();
    assert_eq!(answer.rows.len(), 600);
    assert_eq!(answer.stepped.sorts, 0);
    let mut backwards = forwards.rows;
    backwards.reverse();
    assert_eq!(answer.rows, backwards);
}

#[test]
fn a_walk_held_between_bounds_is_read_backwards_where_that_answers_the_order() {
    // `mr` holds `r` backwards, so an `ORDER BY r` over a walk held
    // between bounds is read from the last entry the bounds hold.
    assert_eq!(
        listed(super::INDEXED, b"SELECT rowid FROM m WHERE r>20 ORDER BY r"),
        "3,5"
    );
    assert_eq!(
        listed(
            super::INDEXED,
            b"SELECT rowid FROM m WHERE r<=30 ORDER BY r"
        ),
        "1,2,3"
    );
    assert_eq!(
        listed(
            super::INDEXED,
            b"SELECT rowid FROM m WHERE r>10 AND r<50 ORDER BY r"
        ),
        "2,3"
    );
    // `mq` holds `q` forwards, so the walk of it runs the other way.
    assert_eq!(
        listed(
            super::INDEXED,
            b"SELECT rowid FROM m WHERE q>='A' ORDER BY q DESC"
        ),
        "4,2,3,1"
    );
    assert_eq!(
        listed(
            super::INDEXED,
            b"SELECT rowid FROM m WHERE p=1 ORDER BY q DESC"
        ),
        "2,1"
    );
}

#[test]
fn a_walk_of_a_tall_index_held_between_bounds_is_read_backwards_as_well() {
    // The descent of a backwards walk descends into the subtree the
    // bound stands in and reads the pages of the path from the last
    // position of each of them.
    // A low bound leaves the descent in the right-most subtree of every
    // page of the path, and a high bound leaves it in the subtree the
    // bound stands in.
    for (sql, held, count) in [
        (
            b"SELECT s FROM t WHERE s>='row 5' ORDER BY s DESC".as_slice(),
            b"SELECT s FROM t WHERE s>='row 5' ORDER BY s".as_slice(),
            303,
        ),
        (
            b"SELECT s FROM t WHERE s<='row 5' ORDER BY s DESC",
            b"SELECT s FROM t WHERE s<='row 5' ORDER BY s",
            297,
        ),
    ] {
        let answer = Database::open(super::TALL_INDEX)
            .unwrap()
            .query(sql)
            .unwrap();
        let forwards = Database::open(super::TALL_INDEX)
            .unwrap()
            .query(held)
            .unwrap();
        assert_eq!(answer.rows.len(), count);
        assert_eq!(answer.stepped.sorts, 0);
        let mut backwards = forwards.rows;
        backwards.reverse();
        assert_eq!(answer.rows, backwards);
    }
}

/// The lines `EXPLAIN QUERY PLAN` answers for `sql`.
fn planned(bytes: &[u8], sql: &[u8]) -> alloc::string::String {
    Database::open(bytes)
        .unwrap()
        .query(sql)
        .unwrap()
        .rows
        .iter()
        .map(|row| match row.get(3) {
            Some(Value::Text(text)) => alloc::string::String::from_utf8_lossy(text).into_owned(),
            _ => alloc::string::String::new(),
        })
        .collect::<Vec<_>>()
        .join("|")
}

#[test]
fn explain_query_plan_names_the_walk_of_every_side() {
    assert_eq!(
        planned(super::INDEXED, b"EXPLAIN QUERY PLAN SELECT * FROM m"),
        "SCAN m"
    );
    assert_eq!(
        planned(
            super::INDEXED,
            b"EXPLAIN QUERY PLAN SELECT * FROM m WHERE q='b'"
        ),
        "SEARCH m USING INDEX mq (q=?)"
    );
    assert_eq!(
        planned(
            super::INDEXED,
            b"EXPLAIN QUERY PLAN SELECT * FROM m WHERE p=1 AND q>'a'"
        ),
        "SEARCH m USING INDEX mpq (p=? AND q>?)"
    );
    assert_eq!(
        planned(
            super::INDEXED,
            b"EXPLAIN QUERY PLAN SELECT * FROM m WHERE rowid>2"
        ),
        "SEARCH m USING INTEGER PRIMARY KEY (rowid>?)"
    );
    assert_eq!(
        planned(
            super::INDEXED,
            b"EXPLAIN QUERY PLAN SELECT * FROM m WHERE rowid=2"
        ),
        "SEARCH m USING INTEGER PRIMARY KEY (rowid=?)"
    );
    assert_eq!(
        planned(super::INDEXED, b"EXPLAIN QUERY PLAN SELECT * FROM m AS z"),
        "SCAN z"
    );
    assert_eq!(
        planned(
            super::INDEXED,
            b"EXPLAIN QUERY PLAN SELECT * FROM m ORDER BY q"
        ),
        "SCAN m USING INDEX mq"
    );
    assert_eq!(
        planned(
            super::INDEXED,
            b"EXPLAIN QUERY PLAN SELECT * FROM m ORDER BY r+1"
        ),
        "SCAN m|USE TEMP B-TREE FOR ORDER BY"
    );
    assert_eq!(
        planned(
            super::INDEXED,
            b"EXPLAIN QUERY PLAN SELECT * FROM m WHERE q='b' OR p=2"
        ),
        "MULTI-INDEX OR|INDEX 1|SEARCH m USING INDEX mq (q=?)|INDEX 2|SEARCH m USING INDEX mpq (p=?)"
    );
    assert_eq!(
        planned(
            super::INDEXED,
            b"EXPLAIN QUERY PLAN SELECT * FROM m WHERE rowid<3"
        ),
        "SEARCH m USING INTEGER PRIMARY KEY (rowid<?)"
    );
    assert_eq!(
        planned(
            super::INDEXED,
            b"EXPLAIN QUERY PLAN SELECT * FROM m WHERE rowid>1 AND rowid<4"
        ),
        "SEARCH m USING INTEGER PRIMARY KEY (rowid>? AND rowid<?)"
    );
    assert_eq!(
        planned(
            super::INDEXED,
            b"EXPLAIN QUERY PLAN SELECT * FROM m, k ON k.a=m.p"
        ),
        "SCAN m|SEARCH k USING INDEX ka (a=?)"
    );
    assert_eq!(
        planned(
            super::INDEXED,
            b"EXPLAIN QUERY PLAN SELECT * FROM (SELECT * FROM m)"
        ),
        "SCAN m|SCAN "
    );
    assert_eq!(
        planned(
            super::INDEXED,
            b"EXPLAIN QUERY PLAN SELECT * FROM m WHERE q<'c'"
        ),
        "SEARCH m USING INDEX mq (q<?)"
    );
}

#[test]
fn an_order_by_that_names_an_alias_answers_what_the_alias_names() {
    assert_eq!(
        listed(super::INDEXED, b"SELECT r AS q FROM m ORDER BY q"),
        "Null,10,20,30,50"
    );
}

#[test]
fn what_words_no_explain_query_plan_stands_in() {
    // A bare `EXPLAIN` names the program the statement compiles to, and
    // the two words after it are read as one prefix or as none.
    for sql in [
        b"EXPLAIN SELECT 1".as_slice(),
        b"EXPLAIN QUERY SELECT 1",
        b"EXPLAIN QUERY PLAN",
    ] {
        assert!(Database::open(super::INDEXED).unwrap().query(sql).is_err());
    }
}
