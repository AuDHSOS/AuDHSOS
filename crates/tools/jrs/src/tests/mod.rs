// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{Console, entry, read_source, run};
use jrs::{Error, Host, Value};
use std::io::{self, Read, Write};

fn command(args: &[&str], input: &[u8]) -> Result<(String, String), String> {
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    run(
        args.iter().map(|s| (*s).to_owned()),
        input,
        &mut output,
        &mut diagnostics,
    )?;
    Ok((
        String::from_utf8(output).map_err(|e| e.to_string())?,
        String::from_utf8(diagnostics).map_err(|e| e.to_string())?,
    ))
}

#[test]
fn cli_source_stdin_help_and_timings() -> Result<(), String> {
    assert_eq!(
        command(&["-e", "print('hello', 2); 42"], b"")?.0,
        "hello 2\n42\n"
    );
    assert_eq!(command(&["-"], b"6*7")?.0, "42\n");
    assert_eq!(command(&["-e", "print()"], b"")?.0, "\n");
    assert!(command(&["--help"], b"")?.0.contains("usage:"));
    let (output, diagnostics) = command(
        &["--fuel", "100", "--stats", "--bench", "2", "-e", "2+3"],
        b"",
    )?;
    assert_eq!(output, "5\n");
    assert!(diagnostics.contains("iterations=2"));
    assert!(diagnostics.contains("compile="));
    Ok(())
}

#[test]
fn cli_invalid_options_and_runtime_errors_are_failures() {
    for args in [
        vec![],
        vec!["--bogus"],
        vec!["--test262"],
        vec!["--fuel"],
        vec!["--fuel", "-1"],
        vec!["--bench"],
        vec!["--bench", "0"],
        vec!["-e"],
        vec!["-e", "1", "extra"],
        vec!["-e", "let"],
        vec!["--fuel", "2", "-e", "while(true){}"],
        vec!["-e", "missing"],
        vec!["/this-path-does-not-exist/jrs.js"],
    ] {
        assert!(command(&args, b"").is_err(), "{args:?}");
    }
}

#[test]
fn cli_file_is_read_as_source() -> Result<(), String> {
    // A project-owned fixture: no shared temp filename and no external source.
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tests/answer.js");
    assert_eq!(command(&[file], b"")?.0, "42\n");
    Ok(())
}

#[test]
fn cli_realm_files_preserve_state_and_checkpoints() -> Result<(), String> {
    let first = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tests/fixtures/realm-first.js"
    );
    let second = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tests/fixtures/realm-second.js"
    );
    assert_eq!(command(&["--realm", first, second], b"")?.0, "42\n");
    assert!(command(&["--realm"], b"").is_err());
    assert!(command(&["--realm", second], b"").is_err());
    assert!(command(&["--realm", "/missing-script.js"], b"").is_err());
    assert!(command(&["--fuel", "0", "--realm", first], b"").is_err());
    let a = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tests/fixtures/realm-model-a.js"
    );
    let b = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tests/fixtures/realm-model-b.js"
    );
    let c = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tests/fixtures/realm-model-c.js"
    );
    assert_eq!(
        command(&["--realm", a, b, c], b"")?.0,
        "undefined\n6\n7 5 6 7\n2 undefined undefined\n13\ntrue undefined false\n99 18 13 7\n2\nfalse\n"
    );
    Ok(())
}

struct Broken;
impl Read for Broken {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("broken input"))
    }
}
impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("broken output"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn source_reader_enforces_length_encoding_and_io() {
    assert_eq!(read_source(&b"123"[..], 3), Ok("123".to_owned()));
    assert!(read_source(&b"1234"[..], 3).is_err());
    assert!(read_source(&b"\xff"[..], 3).is_err());
    assert!(read_source(Broken, 3).is_err());
}

#[test]
fn output_errors_are_returned() {
    for args in [vec!["--help"], vec!["-e", "42"], vec!["-e", "print(1)"]] {
        assert!(
            run(
                args.into_iter().map(str::to_owned),
                &b""[..],
                &mut Broken,
                &mut Vec::new()
            )
            .is_err()
        );
    }
    assert!(
        run(
            ["--stats", "-e", "1"].into_iter().map(str::to_owned),
            &b""[..],
            &mut Vec::new(),
            &mut Broken
        )
        .is_err()
    );
    assert_eq!(
        Console(&mut Broken).print(&[Value::Number(1.0)]),
        Err(Error::Host)
    );
}

#[test]
fn entry_returns_process_status_and_reports_errors() {
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    assert_eq!(
        entry(
            ["-e", "42"].into_iter().map(str::to_owned),
            &b""[..],
            &mut output,
            &mut diagnostics
        ),
        std::process::ExitCode::SUCCESS
    );
    assert_eq!(
        entry(
            ["-e", "missing"].into_iter().map(str::to_owned),
            &b""[..],
            &mut output,
            &mut diagnostics
        ),
        std::process::ExitCode::FAILURE
    );
    assert!(String::from_utf8_lossy(&diagnostics).contains("ReferenceError"));
}
