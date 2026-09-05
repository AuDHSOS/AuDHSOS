// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the xtask says while it works.
//!
//! Progress belongs to whoever watches a run; the explanation of a failure
//! belongs to whoever reads the log afterwards. A quiet run keeps the
//! second and drops the first: `note!` and `note_raw!` write only while the
//! xtask is loud, and everything that says why something failed is a plain
//! `eprintln!` that quiet mode never touches.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether progress is suppressed. Set once, from the command line, before
/// any subcommand runs.
static QUIET: AtomicBool = AtomicBool::new(false);

/// Suppresses progress from here on.
pub(crate) fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// Whether progress is suppressed.
pub(crate) fn quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// One line of progress on standard error, unless the xtask is quiet.
macro_rules! note {
    ($($arg:tt)*) => {
        if !$crate::out::quiet() {
            eprintln!($($arg)*);
        }
    };
}

/// Progress on standard error without a line break, unless the xtask is
/// quiet.
macro_rules! note_raw {
    ($($arg:tt)*) => {
        if !$crate::out::quiet() {
            eprint!($($arg)*);
        }
    };
}

pub(crate) use {note, note_raw};
