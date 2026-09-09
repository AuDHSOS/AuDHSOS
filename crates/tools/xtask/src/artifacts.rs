// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Executables reported by Cargo, including their package working directory.

use std::path::PathBuf;

use crate::error::Error;
use crate::json::{self, Value};
use crate::out::note;
use crate::process::Cmd;

/// One executable from a compiler-artifact message.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Executable {
    /// Cargo target name.
    pub(crate) name: String,
    /// Absolute executable path, including any custom target directory.
    pub(crate) path: PathBuf,
    /// Package root, as used by Cargo when running tests.
    pub(crate) directory: PathBuf,
    /// Whether the executable was compiled as a test harness.
    pub(crate) test: bool,
}

impl Executable {
    /// Runs from the package root and exposes that root to the child.
    pub(crate) fn command(&self) -> Cmd {
        Cmd::new(&self.path)
            .cwd(&self.directory)
            .env("CARGO_MANIFEST_DIR", self.directory.display().to_string())
    }

    /// Bounds harness threads as well as processes, unless the caller
    /// explicitly selected a harness thread count.
    pub(crate) fn test_command(&self, jobs: usize) -> Cmd {
        let command = self.command();
        if std::env::var_os("RUST_TEST_THREADS").is_some() {
            return command;
        }
        let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        command.env(
            "RUST_TEST_THREADS",
            cpus.checked_div(jobs).unwrap_or(1).max(1).to_string(),
        )
    }
}

/// Builds once, using Cargo's structured output instead of guessing paths.
pub(crate) fn build(command: Cmd) -> Result<Vec<Executable>, Error> {
    let command = command.arg("--message-format=json-render-diagnostics");
    note!("$ {}", command.display());
    executables_of(&command.capture()?)
}

/// Reads executable artifacts, ignoring non-JSON output and other messages.
pub(crate) fn executables_of(output: &str) -> Result<Vec<Executable>, Error> {
    let mut executables = Vec::new();
    for line in output.lines().filter(|line| line.starts_with('{')) {
        let value =
            json::parse(line).map_err(|error| Error::Parse(format!("Cargo output: {error}")))?;
        if value.get("reason").and_then(Value::text) != Some("compiler-artifact") {
            continue;
        }
        let Some(path) = value.get("executable").and_then(Value::text) else {
            continue;
        };
        let name = value
            .get("target")
            .and_then(|target| target.get("name"))
            .and_then(Value::text)
            .ok_or_else(|| Error::Parse("Cargo executable has no target name".to_owned()))?;
        let directory = value
            .get("manifest_path")
            .and_then(Value::text)
            .map(PathBuf::from)
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
            .ok_or_else(|| Error::Parse("Cargo executable has no package directory".to_owned()))?;
        let test = value.get("profile").and_then(|profile| profile.get("test"));
        let Some(Value::Bool(test)) = test else {
            return Err(Error::Parse(
                "Cargo executable has no test profile".to_owned(),
            ));
        };
        executables.push(Executable {
            name: name.to_owned(),
            path: PathBuf::from(path),
            directory,
            test: *test,
        });
    }
    Ok(executables)
}

/// Builds test executables, excluding ordinary binaries built as dependencies.
pub(crate) fn build_tests(command: Cmd) -> Result<Vec<Executable>, Error> {
    let mut executables = build(command.arg("--no-run"))?;
    executables.retain(|executable| executable.test);
    if executables.is_empty() {
        return Err(Error::Parse(
            "Cargo reported no test executables".to_owned(),
        ));
    }
    Ok(executables)
}
