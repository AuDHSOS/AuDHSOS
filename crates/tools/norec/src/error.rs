// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a run failed.

use std::fmt;
use std::path::Path;

/// What went wrong. A database that answers a query wrongly is not here:
/// that is a finding and not an error.
#[derive(Debug)]
pub(crate) enum Error {
    /// Wrong command line.
    Usage(String),
    /// An I/O operation failed.
    Io {
        /// What was being done.
        context: String,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The engine could not be run, or answered something unreadable.
    Engine(String),
}

impl Error {
    /// An I/O error with context.
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Error::Io {
            context: context.into(),
            source,
        }
    }

    /// An I/O error that names a file.
    pub(crate) fn path(verb: &str, path: &Path, source: std::io::Error) -> Self {
        Error::io(format!("{verb} {}", path.display()), source)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(message) | Error::Engine(message) => f.write_str(message),
            Error::Io { context, source } => write!(f, "{context}: {source}"),
        }
    }
}
