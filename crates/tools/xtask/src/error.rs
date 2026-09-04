// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The error type of the xtask.

use std::fmt;

/// Why a subcommand failed.
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
    /// A child process exited with a failure status.
    CommandFailed {
        /// The command line.
        command: String,
        /// The exit code, if the process exited normally.
        code: Option<i32>,
    },
    /// The toolchain in use is not the pinned one.
    Toolchain(String),
    /// Output of a tool could not be understood.
    Parse(String),
    /// A policy was violated; the string lists the violations.
    Violations(Vec<String>),
}

impl Error {
    /// An I/O error with context.
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Error::Io {
            context: context.into(),
            source,
        }
    }

    /// `Ok` for an empty list, otherwise the violations.
    pub(crate) fn from_violations(violations: Vec<String>) -> Result<(), Self> {
        if violations.is_empty() {
            Ok(())
        } else {
            Err(Error::Violations(violations))
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(message) | Error::Toolchain(message) | Error::Parse(message) => {
                f.write_str(message)
            }
            Error::Io { context, source } => write!(f, "{context}: {source}"),
            Error::CommandFailed {
                command,
                code: Some(code),
            } => {
                write!(f, "`{command}` exited with status {code}")
            }
            Error::CommandFailed {
                command,
                code: None,
            } => {
                write!(f, "`{command}` was terminated by a signal")
            }
            Error::Violations(violations) => {
                writeln!(f, "{} violation(s):", violations.len())?;
                for violation in violations {
                    writeln!(f, "  - {violation}")?;
                }
                Ok(())
            }
        }
    }
}
