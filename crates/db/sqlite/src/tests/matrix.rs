// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The configuration matrix of document 16, section 16.11.
//!
//! A test that ran under one configuration tested one configuration. Every
//! fixture here holds the same three rows and the same index, written by
//! the shell under a different page size, text encoding, reserved space,
//! journal mode or vacuum setting, and every one of them has to read back
//! the same. `sh tools/sqlite-fixtures.sh` is what writes them.

#![allow(clippy::arithmetic_side_effects)]

use crate::header::Encoding;
use crate::image::Image;
use crate::record::Value;

/// One fixture and what it was written under.
struct Case {
    /// What it is called on disk.
    name: &'static str,
    /// Its bytes.
    bytes: &'static [u8],
    /// The page size it was written with.
    page_size: u32,
    /// The encoding its text is in.
    encoding: Encoding,
    /// How many bytes of every page the b-tree layer may not use.
    reserved: u8,
    /// The write version: 2 once the file has been in write-ahead logging.
    write_version: u8,
    /// Whether the file vacuums itself, which gives it pointer maps.
    vacuums: bool,
}

/// The bytes of one of the matrix fixtures, by the name it has on disk.
pub(super) fn fixture(name: &str) -> Option<&'static [u8]> {
    MATRIX
        .iter()
        .find(|case| case.name == name)
        .map(|case| case.bytes)
}

/// Every configuration the shell can write the same rows under.
const MATRIX: [Case; 19] = [
    Case {
        name: "m-utf8-512.db",
        bytes: include_bytes!("fixtures/m-utf8-512.db"),
        page_size: 512,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-utf8-1024.db",
        bytes: include_bytes!("fixtures/m-utf8-1024.db"),
        page_size: 1024,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-utf8-4096.db",
        bytes: include_bytes!("fixtures/m-utf8-4096.db"),
        page_size: 4096,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-utf8-65536.db",
        bytes: include_bytes!("fixtures/m-utf8-65536.db"),
        page_size: 65536,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-utf16le-512.db",
        bytes: include_bytes!("fixtures/m-utf16le-512.db"),
        page_size: 512,
        encoding: Encoding::Utf16Le,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-utf16le-4096.db",
        bytes: include_bytes!("fixtures/m-utf16le-4096.db"),
        page_size: 4096,
        encoding: Encoding::Utf16Le,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-utf16be-4096.db",
        bytes: include_bytes!("fixtures/m-utf16be-4096.db"),
        page_size: 4096,
        encoding: Encoding::Utf16Be,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-reserved32.db",
        bytes: include_bytes!("fixtures/m-reserved32.db"),
        page_size: 4096,
        encoding: Encoding::Utf8,
        reserved: 32,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-wal.db",
        bytes: include_bytes!("fixtures/m-wal.db"),
        page_size: 4096,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 2,
        vacuums: false,
    },
    Case {
        name: "m-autovacuum-full.db",
        bytes: include_bytes!("fixtures/m-autovacuum-full.db"),
        page_size: 4096,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 1,
        vacuums: true,
    },
    Case {
        name: "m-autovacuum-incr.db",
        bytes: include_bytes!("fixtures/m-autovacuum-incr.db"),
        page_size: 4096,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 1,
        vacuums: true,
    },
    // The journal modes. What the mode leaves in the file is the write
    // version, which is two once the file has been in write-ahead
    // logging and one for every other mode; the journal itself is a
    // second file, which 6.6.94 and 6.6.95 read.
    Case {
        name: "m-delete.db",
        bytes: include_bytes!("fixtures/m-delete.db"),
        page_size: 4096,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-truncate.db",
        bytes: include_bytes!("fixtures/m-truncate.db"),
        page_size: 4096,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-persist.db",
        bytes: include_bytes!("fixtures/m-persist.db"),
        page_size: 4096,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-memory.db",
        bytes: include_bytes!("fixtures/m-memory.db"),
        page_size: 4096,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-off.db",
        bytes: include_bytes!("fixtures/m-off.db"),
        page_size: 4096,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-utf16be-512.db",
        bytes: include_bytes!("fixtures/m-utf16be-512.db"),
        page_size: 512,
        encoding: Encoding::Utf16Be,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-utf8-8192.db",
        bytes: include_bytes!("fixtures/m-utf8-8192.db"),
        page_size: 8192,
        encoding: Encoding::Utf8,
        reserved: 0,
        write_version: 1,
        vacuums: false,
    },
    Case {
        name: "m-reserved4.db",
        bytes: include_bytes!("fixtures/m-reserved4.db"),
        page_size: 1024,
        encoding: Encoding::Utf8,
        reserved: 4,
        write_version: 1,
        vacuums: false,
    },
];

/// The text of a value, decoded out of the encoding the file uses.
fn text(bytes: &[u8], encoding: Encoding) -> String {
    match encoding {
        Encoding::Utf8 => String::from_utf8(bytes.to_vec()).unwrap(),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| {
                    if encoding == Encoding::Utf16Le {
                        u16::from_le_bytes(*pair)
                    } else {
                        u16::from_be_bytes(*pair)
                    }
                })
                .collect();
            String::from_utf16(&units).unwrap()
        }
    }
}

/// The root page of the object named `name`, and its type.
fn objects(image: &Image<'_>, encoding: Encoding) -> Vec<(String, String, u32)> {
    let mut out = Vec::new();
    for row in image.schema() {
        let record = row.unwrap().record().unwrap();
        let (Some(Value::Text(kind)), Some(Value::Text(name)), Some(Value::Int(root))) = (
            record.value(0).unwrap(),
            record.value(1).unwrap(),
            record.value(3).unwrap(),
        ) else {
            panic!("a schema row that is not type, name and root page");
        };
        out.push((
            text(kind, encoding),
            text(name, encoding),
            u32::try_from(root).unwrap(),
        ));
    }
    out
}

#[test]
fn every_configuration_holds_the_header_it_was_written_under() {
    for case in &MATRIX {
        let image = Image::open(case.bytes).unwrap();
        let header = image.header();
        assert_eq!(header.page_size, case.page_size, "{}", case.name);
        assert_eq!(header.encoding, case.encoding, "{}", case.name);
        assert_eq!(header.reserved, case.reserved, "{}", case.name);
        assert_eq!(header.write_version, case.write_version, "{}", case.name);
        assert_eq!(header.read_version, case.write_version, "{}", case.name);
        assert_eq!(
            header.usable(),
            case.page_size - u32::from(case.reserved),
            "{}",
            case.name
        );
        assert_eq!(header.largest_root != 0, case.vacuums, "{}", case.name);
        assert_eq!(image.pages(), header.pages, "{}", case.name);
    }
}

#[test]
fn every_configuration_names_the_same_table_and_the_same_index() {
    for case in &MATRIX {
        let image = Image::open(case.bytes).unwrap();
        let found = objects(&image, case.encoding);
        let names: Vec<(&str, &str)> = found
            .iter()
            .map(|(kind, name, _)| (kind.as_str(), name.as_str()))
            .collect();
        assert_eq!(names, [("table", "m"), ("index", "mi")], "{}", case.name);
        assert!(
            found.iter().all(|(_, _, root)| *root > 1),
            "{}: a root page of one",
            case.name
        );
    }
}

#[test]
fn every_configuration_reads_back_the_same_three_rows() {
    for case in &MATRIX {
        let image = Image::open(case.bytes).unwrap();
        let root = objects(&image, case.encoding)
            .into_iter()
            .find(|(kind, _, _)| kind == "table")
            .map(|(_, _, root)| root)
            .unwrap();
        let mut rows = Vec::new();
        for row in image.rows(root) {
            let row = row.unwrap();
            let record = row.record().unwrap();
            let values: Vec<_> = record.values().collect::<Result<_, _>>().unwrap();
            let [
                Value::Int(i),
                Value::Text(t),
                Value::Real(r),
                Value::Blob(b),
            ] = values[..]
            else {
                panic!("{}: a row of the wrong shape: {values:?}", case.name);
            };
            rows.push((row.rowid, i, text(t, case.encoding), r, b.to_vec()));
        }
        assert_eq!(
            rows,
            [
                (1, 1, "one".to_owned(), 1.5, vec![1]),
                (2, 2, "two".to_owned(), 2.5, vec![2, 2]),
                (3, 3, "three".to_owned(), 3.5, vec![3, 3, 3]),
            ],
            "{}",
            case.name
        );
    }
}

#[test]
fn every_configuration_holds_its_index_entries_in_index_pages() {
    for case in &MATRIX {
        let image = Image::open(case.bytes).unwrap();
        let root = objects(&image, case.encoding)
            .into_iter()
            .find(|(kind, _, _)| kind == "index")
            .map(|(_, _, root)| root)
            .unwrap();
        let page = image.page(root).unwrap();
        assert!(!page.kind().is_table(), "{}", case.name);
        assert_eq!(page.cells(), 3, "{}", case.name);
        let mut keys = Vec::new();
        for index in 0..page.cells() {
            let crate::page::Cell::IndexLeaf { payload } = page.cell(index).unwrap() else {
                panic!("{}: an index page with a cell of another shape", case.name);
            };
            let record = crate::record::Record::parse(payload.local).unwrap();
            let Some(Value::Text(key)) = record.value(0).unwrap() else {
                panic!("{}: an index entry whose key is not its column", case.name);
            };
            keys.push(text(key, case.encoding));
        }
        // The index is over the text column, so its entries are in the
        // order that column collates in, which is not the rowid order.
        assert_eq!(keys, ["one", "three", "two"], "{}", case.name);
    }
}

/// The statements every configuration is put through.
///
/// Each reads the three rows the matrix holds, and between them they
/// reach the parts of the engine a configuration can change: the text
/// out of the encoding, the numbers out of the record, a walk of the
/// table's tree and of the index's, a grouping, a sort, a join of the
/// table to itself, a statement used as a value, and the rowid a
/// `WHERE` holds the walk to.
///
/// The three that answer the bytes as they are stored are left out and
/// held to their difference by
/// [`the_three_that_show_the_stored_bytes_answer_the_encoding`].
const STATEMENTS: [&str; 16] = [
    "SELECT count(*) FROM m",
    "SELECT i, t, r, quote(b) FROM m ORDER BY i",
    "SELECT t FROM m ORDER BY t DESC",
    "SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
    "SELECT length(t), unicode(t) FROM m ORDER BY i",
    "SELECT upper(t), lower(t) FROM m WHERE i=2",
    "SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
    "SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
    "SELECT rowid, i FROM m WHERE rowid=2",
    "SELECT i FROM m WHERE rowid>1 ORDER BY i",
    "SELECT i FROM m WHERE t='two'",
    "SELECT i FROM m WHERE t>'one' ORDER BY i",
    "SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
    "SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
    "SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
    "SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
];

/// One answer as text, so that two configurations are compared by what
/// they answer and not by how they hold it.
fn written(answer: &crate::db::Answer) -> String {
    use core::fmt::Write as _;
    let mut out = String::new();
    for name in &answer.names {
        out.push_str(&String::from_utf8_lossy(name));
        out.push('|');
    }
    for row in &answer.rows {
        out.push('\n');
        for value in row {
            let _ = write!(out, "{value:?}|");
        }
    }
    out
}

#[test]
fn every_configuration_answers_every_statement_the_same() {
    // A configuration changes how the rows are held and not what they
    // are, so the answers are the same under all of them. The first is
    // what the rest are compared against; what SQLite answers for it is
    // `query.corpus`, which names these fixtures as well.
    for sql in STATEMENTS {
        let mut wanted: Option<String> = None;
        for case in &MATRIX {
            let database = crate::db::Database::open(case.bytes)
                .unwrap_or_else(|error| panic!("{}: {error:?}", case.name));
            let answer = database
                .query(sql.as_bytes())
                .unwrap_or_else(|error| panic!("{sql} over {}: {error:?}", case.name));
            let answer = written(&answer);
            match &wanted {
                None => wanted = Some(answer),
                Some(wanted) => assert_eq!(*wanted, answer, "{sql} over {}", case.name),
            }
        }
    }
}

#[test]
fn the_three_that_show_the_stored_bytes_answer_the_encoding() {
    // `hex`, `octet_length` and a cast to a blob read the bytes the file
    // holds rather than the text they spell, so these three are the ones
    // a configuration is allowed to change. A text of three letters is
    // three bytes in UTF-8 and six in either UTF-16.
    for case in &MATRIX {
        let database = crate::db::Database::open(case.bytes).unwrap();
        let answer = database
            .query(b"SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1")
            .unwrap();
        let wanted = match case.encoding {
            Encoding::Utf8 => alloc::vec![
                crate::value::Value::Int(3),
                crate::value::Value::Text(b"6F6E65".to_vec()),
                crate::value::Value::Int(1),
            ],
            Encoding::Utf16Le => alloc::vec![
                crate::value::Value::Int(6),
                crate::value::Value::Text(b"6F006E006500".to_vec()),
                crate::value::Value::Int(0),
            ],
            Encoding::Utf16Be => alloc::vec![
                crate::value::Value::Int(6),
                crate::value::Value::Text(b"006F006E0065".to_vec()),
                crate::value::Value::Int(0),
            ],
        };
        assert_eq!(answer.rows, [wanted], "{}", case.name);
    }
}
