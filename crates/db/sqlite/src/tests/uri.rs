// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A file name written as a URI, which `ATTACH` reads apart where the
//! client asked for it.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::header::Encoding;
use crate::uri::{Mode, named};

/// The path and the parameters of a name, written as text with a bar
/// after the path and after every value.
fn read(name: &[u8]) -> alloc::string::String {
    let held = named(name, true).expect("a name this library reads");
    let mut out = alloc::string::String::new();
    out.push_str(&alloc::string::String::from_utf8_lossy(&held.path));
    out.push('|');
    for (key, value) in &held.parameters {
        out.push_str(&alloc::string::String::from_utf8_lossy(key));
        out.push('=');
        out.push_str(&alloc::string::String::from_utf8_lossy(value));
        out.push('|');
    }
    out
}

/// The message a name is refused with.
fn refused(name: &[u8]) -> alloc::string::String {
    named(name, true).expect_err("a refusal").message()
}

/// The path a URI names, which is the name itself where the client reads
/// no URI or the name begins with something else.
#[test]
fn what_path_a_uri_names() {
    // A client that reads no URI takes the name as the path, and so does
    // a name that does not begin `file:`.
    let held = named(b"file:test.db?mode=ro", false).expect("a path");
    assert_eq!(held.path, b"file:test.db?mode=ro");
    assert!(held.parameters.is_empty());
    assert_eq!(held.mode, None);
    assert_eq!(read(b"test.db"), "test.db|");
    assert_eq!(read(b"http:test.db"), "http:test.db|");
    assert_eq!(read(b"file"), "file|");
    // The authority stands between the third and the fourth slash and is
    // left out of the path, which is empty or `localhost` and no other.
    assert_eq!(read(b"file:test.db"), "test.db|");
    assert_eq!(read(b"file:/test.db"), "/test.db|");
    assert_eq!(read(b"file:///test.db"), "/test.db|");
    assert_eq!(read(b"file://localhost/test.db"), "/test.db|");
    assert_eq!(read(b"file://localhost"), "|");
    assert_eq!(
        refused(b"file://host/test.db"),
        "invalid uri authority: host"
    );
    // The fragment and everything after it names nothing.
    assert_eq!(read(b"file:test.db#boris"), "test.db|");
    assert_eq!(read(b"file:test.db?a=b#c=d"), "test.db|a=b|");
    assert_eq!(read(b"file:test.db?a#b"), "test.db|a=|");
}

/// What a `%HH` escape in a URI stands for.
#[test]
fn what_an_escape_in_a_uri_stands_for() {
    // Every digit of the sixteen, in either case, and the byte the two
    // of them count.
    assert_eq!(read(b"file:test%2Edb"), "test.db|");
    assert_eq!(read(b"file:%74%65%73%74"), "test|");
    assert_eq!(read(b"file:%7a%5A%39"), "zZ9|");
    // A `%` that two digits do not follow is the byte itself.
    assert_eq!(read(b"file:a%zzb"), "a%zzb|");
    assert_eq!(read(b"file:a%4"), "a%4|");
    assert_eq!(read(b"file:a%"), "a%|");
    // An escape stands for its byte before the byte is read, so a `?`
    // written as an escape opens no parameter.
    assert_eq!(read(b"file:test.db%3Fhello"), "test.db?hello|");
    assert_eq!(read(b"file:test.db?a%3Db=c%26d"), "test.db|a=b=c&d|");
    // `%00` is refused wherever it stands, which the build this crate
    // makes says.
    assert_eq!(refused(b"file:test.db%00trailing"), "unexpected %00 in uri");
    assert_eq!(refused(b"file:test.db?%00a=1"), "unexpected %00 in uri");
    assert_eq!(refused(b"file:test.db?a=%00"), "unexpected %00 in uri");
}

/// What the parameters after the path of a URI say.
#[test]
fn what_the_parameters_of_a_uri_say() {
    assert_eq!(read(b"file:test.db?hello=world"), "test.db|hello=world|");
    // A parameter of no value is read, and so is one the name ends on.
    assert_eq!(read(b"file:test.db?hello&world"), "test.db|hello=|world=|");
    assert_eq!(read(b"file:test.db?a=1&b=2"), "test.db|a=1|b=2|");
    // A parameter of no name is left out, with the value written after
    // it, and so is one the name ends on.
    assert_eq!(read(b"file:test.db?=world&xyz=abc"), "test.db|xyz=abc|");
    assert_eq!(read(b"file:test.db?=world#a"), "test.db|");
    assert_eq!(read(b"file:test.db?&&&a=b&&&"), "test.db|a=b|");
    assert_eq!(read(b"file:test.db?"), "test.db|");
    // The value of a parameter by name, and nothing for a name the URI
    // does not carry.
    let held = named(b"file:test.db?vfs=tvfs", true).expect("a path");
    assert_eq!(held.parameter(b"vfs"), Some(b"tvfs".as_slice()));
    assert_eq!(held.parameter(b"mode"), None);
}

/// What access a `mode=` or `cache=` parameter asks for.
#[test]
fn what_access_a_uri_asks_for() {
    for (value, mode) in [
        (b"ro".as_slice(), Mode::ReadOnly),
        (b"rw", Mode::ReadWrite),
        (b"rwc", Mode::Create),
        (b"memory", Mode::Memory),
    ] {
        let mut name = b"file:test.db?mode=".to_vec();
        name.extend_from_slice(value);
        assert_eq!(named(&name, true).expect("a path").mode, Some(mode));
    }
    for value in [b"shared".as_slice(), b"private"] {
        let mut name = b"file:test.db?cache=".to_vec();
        name.extend_from_slice(value);
        assert_eq!(named(&name, true).expect("a path").mode, None);
    }
    // A value the library holds no mode under, which is read in the case
    // it was written in.
    assert_eq!(refused(b"file:test.db?mode=Ro"), "no such access mode: Ro");
    assert_eq!(
        refused(b"file:test.db?cache=Shared"),
        "no such cache mode: Shared"
    );
}

/// The opening function of these tests, which answers a database of one
/// table for `one.db` and nothing for every other name.
fn opening(file: &[u8]) -> Option<Vec<u8>> {
    if file != b"one.db" {
        return None;
    }
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE u(b)").unwrap();
    Some(writer.written())
}

/// The names and file names one statement of the connection answers.
fn shown(writer: &mut Writer) -> alloc::string::String {
    let rows = writer.run(b"PRAGMA database_list").unwrap();
    let mut out = alloc::string::String::new();
    for row in rows {
        for value in row {
            out.push_str(&alloc::string::String::from_utf8_lossy(
                &value.text().unwrap_or_default(),
            ));
            out.push('|');
        }
    }
    out
}

/// A connection that reads URIs opens the file the path of the URI
/// names, and one that reads none opens the file the whole name does.
#[test]
fn what_file_an_attach_of_a_uri_opens() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.opens(opening);
    // A client that asked for no URI takes the name as the file name,
    // which the opening function answers nothing for.
    assert_eq!(
        writer
            .run(b"ATTACH 'file:one.db' AS aux")
            .expect_err("a refusal")
            .message(),
        "unable to open database: file:one.db"
    );
    writer.reads_uri(true);
    writer.run(b"ATTACH 'file:one.db?x=1' AS aux").unwrap();
    assert_eq!(shown(&mut writer), "0|main||2|aux|one.db|");
    // `mode=memory` names a database of the connection's own, which no
    // file holds and `PRAGMA database_list` writes no name for.
    writer
        .run(b"ATTACH 'file:one.db?mode=memory' AS two")
        .unwrap();
    assert_eq!(shown(&mut writer), "0|main||2|aux|one.db|3|two||");
    // The refusals of the URI are the refusals of the statement.
    assert_eq!(
        writer
            .run(b"ATTACH 'file:one.db%00x' AS three")
            .expect_err("a refusal")
            .message(),
        "unexpected %00 in uri"
    );
    assert_eq!(
        writer
            .run(b"ATTACH 'file://host/one.db' AS three")
            .expect_err("a refusal")
            .message(),
        "invalid uri authority: host"
    );
}
