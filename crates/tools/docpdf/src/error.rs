// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a run failed.

use std::fmt;
use std::path::Path;

/// What went wrong.
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
    /// One or more documents failed; the rest were written.
    Documents(Vec<String>),
}

impl Error {
    /// An I/O error with the path it happened on.
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
            Error::Usage(message) => f.write_str(message),
            Error::Io { context, source } => write!(f, "{context}: {source}"),
            Error::Documents(failures) => {
                writeln!(f, "{} document(s) failed:", failures.len())?;
                for failure in failures {
                    writeln!(f, "  - {failure}")?;
                }
                Ok(())
            }
        }
    }
}
