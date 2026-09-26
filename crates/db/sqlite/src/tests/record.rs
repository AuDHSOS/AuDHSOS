// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::record`.

#![allow(clippy::arithmetic_side_effects)]

use crate::error::Error;
use crate::record::{Record, Serial, Value};

/// A record of the serial types `types` over the body `body`, written the
/// way section 2.1 describes.
fn record(types: &[u64], body: &[u8]) -> Vec<u8> {
    let mut header = Vec::new();
    for code in types {
        let (bytes, len) = crate::tests::varint(*code);
        header.extend_from_slice(&bytes[..len]);
    }
    let (size, size_len) = crate::tests::varint(u64::try_from(header.len() + 1).unwrap());
    let mut out = Vec::new();
    out.extend_from_slice(&size[..size_len]);
    out.extend_from_slice(&header);
    out.extend_from_slice(body);
    out
}

#[test]
fn every_serial_code_is_the_type_and_the_length_the_format_gives_it() {
    let cases = [
        (0, Serial::Null, 0),
        (1, Serial::Int(1), 1),
        (2, Serial::Int(2), 2),
        (3, Serial::Int(3), 3),
        (4, Serial::Int(4), 4),
        (5, Serial::Int(6), 6),
        (6, Serial::Int(8), 8),
        (7, Serial::Real, 8),
        (8, Serial::Zero, 0),
        (9, Serial::One, 0),
        (12, Serial::Blob(0), 0),
        (13, Serial::Text(0), 0),
        (14, Serial::Blob(1), 1),
        (15, Serial::Text(1), 1),
        (100, Serial::Blob(44), 44),
        (101, Serial::Text(44), 44),
    ];
    for (code, serial, len) in cases {
        assert_eq!(Serial::from_code(code), Ok(serial), "code {code}");
        assert_eq!(serial.len(), len, "code {code}");
        assert_eq!(serial.is_empty(), len == 0);
    }
}

#[test]
fn the_two_reserved_serial_types_are_refused() {
    assert_eq!(Serial::from_code(10), Err(Error::SerialType(10)));
    assert_eq!(Serial::from_code(11), Err(Error::SerialType(11)));
}

#[test]
fn an_integer_is_read_at_the_width_it_was_stored_in_and_keeps_its_sign() {
    assert_eq!(Serial::Int(1).read(&[0xff]), Ok(Value::Int(-1)));
    assert_eq!(Serial::Int(1).read(&[0x7f]), Ok(Value::Int(127)));
    assert_eq!(Serial::Int(2).read(&[0xff, 0xfe]), Ok(Value::Int(-2)));
    assert_eq!(
        Serial::Int(3).read(&[0x01, 0x00, 0x00]),
        Ok(Value::Int(65536))
    );
    assert_eq!(
        Serial::Int(6).read(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]),
        Ok(Value::Int(-1))
    );
    assert_eq!(
        Serial::Int(8).read(&[0x80, 0, 0, 0, 0, 0, 0, 0]),
        Ok(Value::Int(i64::MIN))
    );
    assert_eq!(Serial::Zero.read(&[]), Ok(Value::Int(0)));
    assert_eq!(Serial::One.read(&[]), Ok(Value::Int(1)));
    assert_eq!(Serial::Null.read(&[]), Ok(Value::Null));
}

#[test]
fn a_double_is_eight_big_endian_bytes() {
    assert_eq!(
        Serial::Real.read(&1.5f64.to_be_bytes()),
        Ok(Value::Real(1.5))
    );
    assert_eq!(
        Serial::Real.read(&(-0.0f64).to_be_bytes()),
        Ok(Value::Real(-0.0))
    );
    // A double that is no number is read as a null, which is the value
    // SQLite writes a NaN as, so a file another writer left one in
    // answers a null rather than a number no comparison holds.
    assert_eq!(Serial::Real.read(&f64::NAN.to_be_bytes()), Ok(Value::Null));
    assert_eq!(
        Serial::Real.read(&(-f64::NAN).to_be_bytes()),
        Ok(Value::Null)
    );
    assert_eq!(
        Serial::Real.read(&f64::INFINITY.to_be_bytes()),
        Ok(Value::Real(f64::INFINITY))
    );
}

#[test]
fn text_and_blob_borrow_the_bytes_they_were_stored_in() {
    assert_eq!(Serial::Text(3).read(b"abcdef"), Ok(Value::Text(b"abc")));
    assert_eq!(
        Serial::Blob(2).read(b"\x01\x02\x03"),
        Ok(Value::Blob(b"\x01\x02"))
    );
    assert_eq!(Serial::Text(4).read(b"abc"), Err(Error::Overrun));
    assert_eq!(Serial::Int(8).read(b"short"), Err(Error::Overrun));
}

#[test]
fn a_record_answers_its_values_in_the_order_its_header_names_them() {
    // Types: NULL, one-byte integer, three-byte text, the constant one.
    let bytes = record(&[0, 1, 13 + 3 * 2, 9], b"\x2aabc");
    let parsed = Record::parse(&bytes).unwrap();
    let values: Vec<_> = parsed.values().collect::<Result<_, _>>().unwrap();
    assert_eq!(
        values.as_slice(),
        [
            Value::Null,
            Value::Int(42),
            Value::Text(b"abc"),
            Value::Int(1)
        ]
    );
    assert_eq!(parsed.value(2).unwrap(), Some(Value::Text(b"abc")));
    assert_eq!(parsed.value(4).unwrap(), None);
}

#[test]
fn a_header_that_reaches_past_the_payload_is_refused() {
    assert_eq!(Record::parse(&[]), Err(Error::Varint));
    // A header that claims more bytes than the payload holds.
    assert_eq!(Record::parse(&[40, 0, 0]), Err(Error::Overrun));
    // A header size smaller than the varint that holds it.
    assert_eq!(Record::parse(&[0, 0]), Err(Error::Overrun));
}

#[test]
fn a_value_that_runs_past_the_body_ends_the_walk_once() {
    let bytes = record(&[6], b"\x01\x02");
    let parsed = Record::parse(&bytes).unwrap();
    let mut values = parsed.values();
    assert_eq!(values.next(), Some(Err(Error::Overrun)));
    assert_eq!(values.next(), None);
    assert_eq!(parsed.value(0), Err(Error::Overrun));
}

#[test]
fn a_reserved_type_inside_a_record_ends_the_walk_once() {
    let bytes = record(&[10], b"");
    let parsed = Record::parse(&bytes).unwrap();
    let mut values = parsed.values();
    assert_eq!(values.next(), Some(Err(Error::SerialType(10))));
    assert_eq!(values.next(), None);
}

#[test]
fn a_header_whose_last_serial_type_does_not_end_is_refused_once() {
    // A header of one byte that is the start of a varint and nothing else.
    let parsed = Record::parse(&[2, 0x81]).unwrap();
    let mut values = parsed.values();
    assert_eq!(values.next(), Some(Err(Error::Varint)));
    assert_eq!(values.next(), None);
}

/// The values of one record, as an expression holds them.
fn owned(record: &Record<'_>) -> Vec<crate::value::Value> {
    record
        .values()
        .map(|value| match value.unwrap() {
            Value::Null => crate::value::Value::Null,
            Value::Int(number) => crate::value::Value::Int(number),
            Value::Real(number) => crate::value::Value::Real(number),
            Value::Text(bytes) => crate::value::Value::Text(bytes.to_vec()),
            Value::Blob(bytes) => crate::value::Value::Blob(bytes.to_vec()),
        })
        .collect()
}

#[test]
fn every_row_of_every_fixture_is_written_back_as_the_bytes_it_was_read_from() {
    use crate::image::Image;
    let mut rows = 0;
    for (name, bytes) in crate::tests::WRITTEN {
        let image = Image::open(bytes).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let format = image.header().schema_format;
        for table in tables(&image) {
            // `places` answers where each column stands; the record is
            // written in that order, so the affinities are turned around.
            let places = crate::db::places(&table.table);
            let mut affinities =
                alloc::vec![crate::value::Affinity::None; table.table.columns.len()];
            for (at, place) in places.iter().enumerate() {
                if *place != usize::MAX {
                    affinities[*place] = table.table.columns[at].affinity;
                }
            }
            // A table that keeps its rows in the key's own tree is
            // walked as the index tree it is.
            let held: Vec<crate::page::Payload<'_>> = if table.table.without_rowid {
                image
                    .entries(table.root)
                    .map(|entry| entry.unwrap_or_else(|error| panic!("{name}: {error:?}")))
                    .collect()
            } else {
                image
                    .rows(table.root)
                    .map(|row| {
                        row.unwrap_or_else(|error| panic!("{name}: {error:?}"))
                            .payload
                    })
                    .collect()
            };
            let mut payload = Vec::new();
            for holds in held {
                payload.resize(holds.total, 0);
                let read = image.read_payload(&holds, &mut payload).unwrap();
                assert_eq!(read, holds.total);
                let record = Record::parse(&payload).unwrap();
                let written = crate::record::write(&owned(&record), &affinities, format);
                assert_eq!(
                    written,
                    payload,
                    "{name}, {}",
                    String::from_utf8_lossy(&table.table.name)
                );
                rows += 1;
            }
        }
    }
    // The count is what says the walk found the rows rather than none.
    assert!(rows > 200, "only {rows} rows were written back");
}

/// One table of a fixture: what its statement says and where its tree is.
struct Held {
    /// The table.
    table: crate::schema::Table,
    /// The page its tree begins at.
    root: u32,
}

/// Every table of a fixture, read the way the engine reads the schema.
fn tables(image: &crate::image::Image<'_>) -> Vec<Held> {
    let encoding = image.header().encoding;
    let mut out = Vec::new();
    let mut payload = Vec::new();
    for row in image.schema() {
        let row = row.unwrap();
        payload.resize(row.payload.total, 0);
        image.read_payload(&row.payload, &mut payload).unwrap();
        let record = Record::parse(&payload).unwrap();
        let text = |at: usize| match record.value(at).unwrap() {
            Some(Value::Text(bytes)) => match encoding {
                crate::header::Encoding::Utf8 => bytes.to_vec(),
                crate::header::Encoding::Utf16Le => crate::utf8::from_utf16(bytes, false),
                crate::header::Encoding::Utf16Be => crate::utf8::from_utf16(bytes, true),
            },
            _ => Vec::new(),
        };
        if text(0) != b"table" {
            continue;
        }
        let Some(Value::Int(root)) = record.value(3).unwrap() else {
            continue;
        };
        let sql = text(4);
        if sql.is_empty() {
            continue;
        }
        let (arena, definition) = crate::parse::definition(&sql).unwrap();
        let crate::ast::Definition::Table(written) = definition else {
            continue;
        };
        out.push(Held {
            table: crate::schema::table(&arena, &written, &sql, &[]).unwrap(),
            root: u32::try_from(root).unwrap(),
        });
    }
    out
}

#[test]
fn an_integer_a_real_column_holds_is_written_as_a_double_where_it_is_wide() {
    use crate::value::{Affinity, Value as Owned};
    // `MEM_IntReal`: a value that goes into a column of real affinity as
    // an integer is stored as an integer while six bytes hold it, and as
    // the double it stands for where they do not. `records.db` holds the
    // second in `typed`, which is where these bytes come from.
    let wide = crate::record::write(&[Owned::Int(i64::MAX)], &[Affinity::Real], 4);
    assert_eq!(wide, [2, 7, 0x43, 0xe0, 0, 0, 0, 0, 0, 0]);
    let narrow = crate::record::write(&[Owned::Int(5)], &[Affinity::Real], 4);
    assert_eq!(narrow, [2, 1, 5]);
}

#[test]
fn a_file_written_before_the_fourth_format_spells_a_zero_and_a_one_out() {
    use crate::value::{Affinity, Value as Owned};
    // Serial types 8 and 9 arrived with the fourth schema format, so a
    // zero and a one take a byte of the body each before it.
    let values = [Owned::Int(0), Owned::Int(1)];
    assert_eq!(crate::record::write(&values, &[], 1), [3, 1, 1, 0, 1]);
    assert_eq!(crate::record::write(&values, &[], 4), [3, 8, 9]);
    // Every other integer is written the same way under either format.
    assert_eq!(
        crate::record::write(&[Owned::Int(2)], &[Affinity::None], 1),
        crate::record::write(&[Owned::Int(2)], &[Affinity::None], 4)
    );
}

/// Where in a record the value of one column begins, which a blob
/// handle reads to know where the bytes of that column lie.
#[test]
fn where_in_a_record_the_value_of_a_column_begins() {
    use crate::record::{Serial, placed};
    // Two columns: a header of three bytes, an integer of one byte, and
    // then three bytes of text.
    let payload = [3, 1, 19, 42, b'a', b'b', b'c'];
    assert_eq!(placed(&payload, 0).unwrap(), Some((3, 1, Serial::Int(1))));
    assert_eq!(placed(&payload, 1).unwrap(), Some((4, 3, Serial::Text(3))));
    // A column the record does not hold.
    assert_eq!(placed(&payload, 2).unwrap(), None);
    // A header the payload ends before, and one shorter than the byte
    // that says how long it is.
    assert_eq!(placed(&[9, 1, 15], 0), Err(crate::error::Error::Overrun));
    assert_eq!(placed(&[0], 0), Err(crate::error::Error::Overrun));
}
