// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The lines the runner on the machine writes and the runner on the host
//! parses.
//!
//! Invariant: every line this module writes matches the grammar in
//! [03-target-platform.md 3.1.7]; the xtask parses exactly these lines.

use core::fmt::{self, Write};

use kernel_hal_api::console::DebugConsole;

/// The prefix of a line that reports one test.
pub const TEST_PREFIX: &str = "[test] ";

/// The prefix of the line that closes a run.
pub const SUMMARY_PREFIX: &str = "[summary] ";

/// What separates a test name from its outcome.
pub const SEPARATOR: &str = " ... ";

/// The outcome of a test that passed.
pub const OK: &str = "ok";

/// The outcome of a test that failed, followed by the message.
pub const FAILED: &str = "FAILED: ";

/// A writer over a debug console that drops formatting errors, because a
/// console that cannot take the bytes must not stop the run.
struct Line<'a, C: DebugConsole + ?Sized> {
    console: &'a mut C,
}

impl<C: DebugConsole + ?Sized> Write for Line<'_, C> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.console.write_bytes(text.as_bytes());
        Ok(())
    }
}

fn emit(console: &mut (impl DebugConsole + ?Sized), arguments: fmt::Arguments<'_>) {
    let _ = Line { console }.write_fmt(arguments);
}

/// Writes the start of a test line, without the outcome.
pub fn start(console: &mut (impl DebugConsole + ?Sized), name: &str) {
    emit(console, format_args!("{TEST_PREFIX}{name}{SEPARATOR}"));
}

/// Closes a test line with the passing outcome.
pub fn passed(console: &mut (impl DebugConsole + ?Sized)) {
    emit(console, format_args!("{OK}\n"));
}

/// Closes a test line with the failing outcome and a message.
pub fn failed(console: &mut (impl DebugConsole + ?Sized), message: fmt::Arguments<'_>) {
    emit(console, format_args!("{FAILED}{message}\n"));
}

/// Writes the summary line.
pub fn summary(console: &mut (impl DebugConsole + ?Sized), passed_count: u32, failed_count: u32) {
    emit(
        console,
        format_args!("{SUMMARY_PREFIX}passed={passed_count} failed={failed_count}\n"),
    );
}
