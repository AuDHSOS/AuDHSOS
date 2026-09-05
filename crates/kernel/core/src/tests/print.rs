// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::print`.

use core::fmt::Write;

use kernel_hal_api::doubles::RecordingConsole;

use crate::print::{ConsoleWriter, write};
use crate::println;

#[test]
fn the_writer_passes_every_byte_to_the_console() {
    let mut console = RecordingConsole::new();
    let mut writer = ConsoleWriter::new(&mut console);
    write!(writer, "{}-{:#x}", 12, 255).unwrap();
    assert_eq!(console.text(), "12-0xff");
}

#[test]
fn the_line_macro_adds_exactly_one_newline() {
    let mut console = RecordingConsole::new();
    println!(&mut console, "one");
    println!(&mut console, "{} {}", "two", 3);
    assert_eq!(console.text(), "one\ntwo 3\n");
}

#[test]
fn writing_arguments_directly_reaches_the_console() {
    let mut console = RecordingConsole::new();
    write(&mut console, format_args!("plain"));
    assert_eq!(console.output(), b"plain");
}
