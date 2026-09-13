// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The SQLite file format reader against arbitrary bytes: no input may
//! panic, no walk may run forever, and nothing a reader hands back may
//! reach past the page it came from.
//!
//! What a record holds is checked the other way as well: the values of
//! every record are written back as a record, and reading that record
//! answers the values it was written from.
//!
//! A database is a file of pointers — page numbers, cell offsets, payload
//! lengths, overflow links — and every one of them is a number a file can
//! lie about. What this target holds is that a lie is answered with a
//! refusal: a cell that reaches past its page, a chain that turns back on
//! itself, a tree that points into itself.

use db_sqlite::db::Database;
use db_sqlite::value::Value as Owned;
use db_sqlite::{Cell, Header, Image, Kind, Page, Record, Value};

/// What is asked of every table the file names, before its name.
const SHAPES: [&[u8]; 2] = [
    b"SELECT * FROM ",
    b"SELECT count(*), sum(rowid), total(rowid), avg(rowid), min(rowid), \
      max(rowid), group_concat(rowid) FROM ",
];

/// The most rows a walk reads. A file can describe more; reading them adds
/// no coverage and costs the fuzzer its time.
const MAX_ROWS: usize = 4096;

/// The largest payload this target assembles. A record can claim a length
/// no machine holds, and refusing to allocate it is the point.
const MAX_PAYLOAD: usize = 1 << 20;

/// The names of the first columns of the table `name`, each with its
/// quotes doubled, because a column's name is text the file decides.
fn columns_of(database: &Database<'_>, name: &[u8]) -> Vec<Vec<u8>> {
    let Some(table) = database.tables().find(|table| table.name == name) else {
        return Vec::new();
    };
    table
        .columns
        .iter()
        .take(2)
        .map(|column| {
            let mut out = Vec::new();
            for byte in &column.name {
                if *byte == b'"' {
                    out.push(b'"');
                }
                out.push(*byte);
            }
            out
        })
        .collect()
}

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    // The schema of a file is text the file decides, and a statement
    // over it is a walk the file decides the shape of. Neither may
    // panic and neither may run forever.
    if let Ok(database) = Database::open(bytes) {
        let names: Vec<Vec<u8>> = database
            .tables()
            .map(|table| table.name.clone())
            .collect();
        for name in names.iter().take(4) {
            let mut quoted = b"\"".to_vec();
            for byte in name {
                if *byte == b'"' {
                    quoted.push(b'"');
                }
                quoted.push(*byte);
            }
            quoted.push(b'"');
            // A scan and a grouping, over the columns the file names and
            // over the key it does not: the second is where every
            // aggregate accumulates.
            for shape in SHAPES {
                for tail in [
                    b" ORDER BY 1 LIMIT 64".as_slice(),
                    b" GROUP BY rowid%4 ORDER BY 1 LIMIT 64".as_slice(),
                ] {
                    let mut sql = shape.to_vec();
                    sql.extend_from_slice(&quoted);
                    sql.extend_from_slice(tail);
                    answers(&database, &sql);
                }
            }
            // A compound, which sorts and merges whatever the two sides
            // answered.
            let mut sql = b"SELECT * FROM ".to_vec();
            sql.extend_from_slice(&quoted);
            sql.extend_from_slice(b" UNION SELECT * FROM ");
            sql.extend_from_slice(&quoted);
            sql.extend_from_slice(b" LIMIT 64");
            answers(&database, &sql);
            // A join, which is the loops nested, and the pass over the
            // rows nothing matched that a `FULL` join adds to them.
            let mut sql = b"SELECT * FROM ".to_vec();
            sql.extend_from_slice(&quoted);
            sql.extend_from_slice(b" AS p FULL JOIN ");
            sql.extend_from_slice(&quoted);
            sql.extend_from_slice(b" AS q ON p.rowid=q.rowid+1 LIMIT 16");
            answers(&database, &sql);
            // A statement inside a statement, once written in the
            // `FROM` and once named by a `WITH`, which answers its rows
            // before the outer walk begins.
            let mut sql = b"WITH s AS (SELECT * FROM ".to_vec();
            sql.extend_from_slice(&quoted);
            sql.extend_from_slice(b" LIMIT 16) SELECT * FROM s, (SELECT * FROM ");
            sql.extend_from_slice(&quoted);
            sql.extend_from_slice(b" LIMIT 4) ORDER BY 1 LIMIT 16");
            answers(&database, &sql);
            // A statement used as a value, which is answered once per
            // row of the statement that encloses it.
            let mut sql = b"SELECT rowid, (SELECT count(*) FROM ".to_vec();
            sql.extend_from_slice(&quoted);
            sql.extend_from_slice(b" AS q WHERE q.rowid<p.rowid) FROM ");
            sql.extend_from_slice(&quoted);
            sql.extend_from_slice(b" AS p WHERE EXISTS (SELECT 1 FROM ");
            sql.extend_from_slice(&quoted);
            sql.extend_from_slice(b") AND p.rowid IN (SELECT rowid FROM ");
            sql.extend_from_slice(&quoted);
            sql.extend_from_slice(b") LIMIT 16");
            answers(&database, &sql);
            // A `WHERE` that names the rowid, which holds the walk to a
            // range instead of scanning the tree. The rows it answers
            // have to be the rows the same statement answers without the
            // range, which the terms below bracket.
            for held in [
                b"rowid=3".as_slice(),
                b"rowid>=2 AND rowid<=4".as_slice(),
                b"rowid>9223372036854775806".as_slice(),
                b"rowid<-9223372036854775807".as_slice(),
            ] {
                let mut sql = b"SELECT count(*), min(rowid), max(rowid) FROM ".to_vec();
                sql.extend_from_slice(&quoted);
                sql.extend_from_slice(b" WHERE ");
                sql.extend_from_slice(held);
                answers(&database, &sql);
            }
            // A `WHERE` that names a column an index may be over, which
            // holds the walk to the entries of that index. The rows it
            // answers have to be the rows the same statement answers
            // without one, and a file decides both the index and what
            // its entries hold.
            for column in columns_of(&database, name) {
                for value in [b"0".as_slice(), b"''".as_slice(), b"x'00'".as_slice()] {
                    let mut sql = b"SELECT count(*) FROM ".to_vec();
                    sql.extend_from_slice(&quoted);
                    sql.extend_from_slice(b" WHERE \"");
                    sql.extend_from_slice(&column);
                    sql.extend_from_slice(b"\"=");
                    sql.extend_from_slice(value);
                    answers(&database, &sql);
                }
            }
        }
    }

    let Ok(image) = Image::open(bytes) else {
        return;
    };
    let header = *image.header();
    assert!(header.page_size.is_power_of_two(), "a page size that is not");
    assert!(header.usable() <= header.page_size, "usable past the page");

    // Every page of the file, read as a page: the pointer array and the
    // cells of whatever type the first byte claims.
    for number in 1..=image.pages().min(64) {
        let Ok(page) = image.page(number) else {
            continue;
        };
        let mut cells = Vec::new();
        for index in 0..page.cells() {
            let Ok(cell) = page.cell(index) else {
                continue;
            };
            check(&image, page.kind(), &cell);
            cells.push(db_sqlite::page::write_cell(&cell));
        }
        built_back(&page, number, &header, &cells);
        changed(&image, &page, number, &header);
        assert_eq!(
            page.right_most().is_some(),
            page.kind().is_interior(),
            "a right-most pointer on a page without children"
        );
    }

    // The schema, and then every tree it names, as a table tree and as
    // an index tree: a root is a number the file chooses, and either
    // walk of it must refuse rather than run away.
    let mut roots = Vec::new();
    walk(&image, 1, &mut roots);
    for root in roots.iter().take(32) {
        walk(&image, *root, &mut Vec::new());
        for entry in image.entries(*root).take(MAX_ROWS) {
            let Ok(entry) = entry else {
                break;
            };
            assert!(
                entry.local.len() <= entry.total,
                "more of an entry on the page than the entry has"
            );
        }
    }
});

/// Answers one statement, where it is one this engine answers, and holds
/// every row it answers to the width of the answer.
fn answers(database: &Database<'_>, sql: &[u8]) {
    let Ok(answer) = database.query(sql) else {
        return;
    };
    for row in &answer.rows {
        assert_eq!(
            row.len(),
            answer.names.len(),
            "a row of another width than the answer"
        );
    }
}

/// Walks one table tree, reading every record it holds and collecting the
/// root pages the records name.
fn walk(image: &Image<'_>, root: u32, roots: &mut Vec<u32>) {
    for row in image.rows(root).take(MAX_ROWS) {
        let Ok(row) = row else {
            return;
        };
        assert!(
            row.payload.local.len() <= row.payload.total,
            "more of a payload on the page than the payload has"
        );
        let mut whole = Vec::new();
        let payload = if row.payload.is_whole() {
            row.payload.local
        } else if row.payload.total <= MAX_PAYLOAD {
            whole.resize(row.payload.total, 0);
            if image.read_payload(&row.payload, &mut whole).is_err() {
                continue;
            }
            &whole
        } else {
            continue;
        };
        let Ok(record) = Record::parse(payload) else {
            continue;
        };
        for value in record.values() {
            match value {
                Ok(Value::Text(text) | Value::Blob(text)) => {
                    assert!(text.len() <= payload.len(), "a value longer than its record");
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        written_back(&record);
        if let Ok(Some(Value::Int(page))) = record.value(3) {
            if let Ok(page) = u32::try_from(page) {
                roots.push(page);
            }
        }
    }
}

/// The cells of a page, written back into a page of their own, which
/// reads as the same cells. The bytes need not be the bytes the file
/// holds: a file may write a length as a varint longer than the value
/// needs, and this engine writes the shortest one.
fn built_back(page: &Page<'_>, number: u32, header: &Header, cells: &[Vec<u8>]) {
    if cells.len() != page.cells() {
        return;
    }
    let page_size = usize::try_from(header.page_size).unwrap_or(0);
    let usable = usize::try_from(header.usable()).unwrap_or(0);
    let Some(built) = db_sqlite::page::build(
        page.kind(),
        number,
        page_size,
        usable,
        cells,
        page.right_most(),
    ) else {
        return;
    };
    let again = Page::parse(&built, number, header.usable())
        .expect("a page this engine built is one it reads");
    assert_eq!(again.cells(), page.cells(), "a page of another width");
    assert_eq!(again.kind(), page.kind(), "a page of another kind");
    assert_eq!(
        again.right_most(),
        page.right_most(),
        "a right-most pointer that moved"
    );
    for index in 0..page.cells() {
        let Ok(was) = page.cell(index) else {
            continue;
        };
        let now = again
            .cell(index)
            .expect("a cell this engine wrote is one it reads");
        assert_eq!(was, now, "a cell that moved");
    }
}

/// A page changed where it lies: a cell taken off it and a cell put on
/// it, of a page a file decided the shape of. Neither may panic, and
/// what either leaves behind is still a page a reader reads or refuses.
fn changed(image: &Image<'_>, page: &Page<'_>, number: u32, header: &Header) {
    let cell: &[u8] = match page.kind() {
        Kind::LeafTable => b"\x02\x01\x00\x00",
        Kind::LeafIndex => b"\x02\x00\x00",
        Kind::InteriorTable => b"\x00\x00\x00\x02\x01",
        Kind::InteriorIndex => b"\x00\x00\x00\x02\x02\x00\x00",
    };
    let Ok(bytes) = image.page_bytes(number) else {
        return;
    };
    let mut copy = bytes.to_vec();
    let Ok(mut writer) = db_sqlite::page::Writer::open(&mut copy, number, header.usable()) else {
        return;
    };
    let _ = writer.free();
    let _ = writer.remove(0);
    let _ = writer.insert(0, cell);
    let _ = writer.remove(page.cells().saturating_sub(1));
    if let Ok(again) = Page::parse(&copy, number, header.usable()) {
        for at in 0..again.cells() {
            let _ = again.cell(at);
        }
    }
}

/// The values of a record, or nothing where one of them is refused.
fn owned(record: &Record<'_>) -> Option<Vec<Owned>> {
    let mut out = Vec::new();
    for value in record.values() {
        out.push(match value.ok()? {
            Value::Null => Owned::Null,
            Value::Int(number) => Owned::Int(number),
            Value::Real(number) => Owned::Real(number),
            Value::Text(bytes) => Owned::Text(bytes.to_vec()),
            Value::Blob(bytes) => Owned::Blob(bytes.to_vec()),
        });
    }
    Some(out)
}

/// The values of a record, written back as a record and read again: the
/// two runs of values are the same, whatever serial types the file chose
/// and whichever this engine chooses for them.
fn written_back(record: &Record<'_>) {
    let Some(values) = owned(record) else {
        return;
    };
    let bytes = db_sqlite::record::write(&values, &[], 4);
    let again = Record::parse(&bytes).expect("a record this engine wrote is one it reads");
    let back = owned(&again).expect("every value of a record this engine wrote");
    assert_eq!(back.len(), values.len(), "a record of another width");
    for (was, now) in values.iter().zip(&back) {
        // A double is compared by its bits, because a NaN is a value a
        // file may hold and is equal to nothing, itself included.
        let same = match (was, now) {
            (Owned::Real(before), Owned::Real(after)) => before.to_bits() == after.to_bits(),
            _ => was == now,
        };
        assert!(same, "a value that moved: {was:?} became {now:?}");
    }
}

/// What every cell of a page of this type has to hold.
fn check(image: &Image<'_>, kind: Kind, cell: &Cell<'_>) {
    match (kind, cell) {
        (Kind::LeafTable, Cell::TableLeaf { payload, .. })
        | (Kind::LeafIndex, Cell::IndexLeaf { payload })
        | (Kind::InteriorIndex, Cell::IndexInterior { payload, .. }) => {
            assert!(
                payload.local.len() <= payload.total,
                "more of a payload on the page than the payload has"
            );
            if let Some(first) = payload.overflow {
                assert!(first > 0, "an overflow chain that begins at page zero");
                let _ = image.page_bytes(first);
            }
        }
        (Kind::InteriorTable, Cell::TableInterior { child, .. }) => {
            assert!(*child > 0, "a child page of zero");
        }
        _ => panic!("a cell of the wrong shape for its page"),
    }
}
