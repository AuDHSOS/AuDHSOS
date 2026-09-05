// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Running child processes.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::Error;
use crate::out::{self, note};

/// A command line to run.
#[derive(Clone, Debug)]
pub(crate) struct Cmd {
    program: PathBuf,
    args: Vec<String>,
    envs: Vec<(String, String)>,
    cwd: Option<PathBuf>,
}

impl Cmd {
    /// A command for `program`.
    pub(crate) fn new(program: impl Into<PathBuf>) -> Self {
        Cmd {
            program: program.into(),
            args: Vec::new(),
            envs: Vec::new(),
            cwd: None,
        }
    }

    /// The Cargo of the toolchain in use (`$CARGO`, set by Cargo itself),
    /// with `RUSTC` and `RUSTDOC` pointing at the same toolchain so that a
    /// foreign `rustc` earlier on the `PATH` is never picked up.
    pub(crate) fn cargo() -> Self {
        let cargo = cargo_path();
        let mut cmd = Cmd::new(cargo.clone());
        if let Some(dir) = cargo.parent().filter(|d| !d.as_os_str().is_empty()) {
            cmd = cmd
                .env("RUSTC", dir.join("rustc").display().to_string())
                .env("RUSTDOC", dir.join("rustdoc").display().to_string());
        }
        cmd
    }

    /// The toolchain's Cargo without the `RUSTC`/`RUSTDOC` overrides, for
    /// subcommands that bring their own driver (Miri).
    pub(crate) fn cargo_plain() -> Self {
        Cmd::new(cargo_path())
    }

    /// A binary from the same directory as the toolchain's Cargo.
    pub(crate) fn toolchain_binary(name: &str) -> Self {
        let cargo = cargo_path();
        match cargo.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => Cmd::new(dir.join(name)),
            _ => Cmd::new(name),
        }
    }

    /// Adds one argument.
    pub(crate) fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Adds several arguments.
    pub(crate) fn args<I: IntoIterator<Item = S>, S: Into<String>>(mut self, args: I) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets an environment variable for the child.
    pub(crate) fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// Sets the working directory.
    pub(crate) fn cwd(mut self, dir: &Path) -> Self {
        self.cwd = Some(dir.to_path_buf());
        self
    }

    /// The command line for messages.
    pub(crate) fn display(&self) -> String {
        let mut text = self.program.display().to_string();
        for arg in &self.args {
            text.push(' ');
            text.push_str(arg);
        }
        text
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        for (key, value) in &self.envs {
            command.env(key, value);
        }
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        command
    }

    /// Runs with inherited output and fails on a non-zero status. A quiet
    /// xtask reads the output instead and prints it only for a command
    /// that failed.
    pub(crate) fn run(&self) -> Result<(), Error> {
        if out::quiet() {
            return self.run_buffered();
        }
        eprintln!("$ {}", self.display());
        let status = self
            .command()
            .status()
            .map_err(|source| Error::io(format!("running `{}`", self.display()), source))?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: self.display(),
                code: status.code(),
            })
        }
    }

    /// `run` for a quiet xtask. A command that succeeds says nothing; a
    /// command that fails prints its command line and both its streams,
    /// each in its own order, since the two are read apart and cannot be
    /// interleaved as a terminal would have shown them.
    fn run_buffered(&self) -> Result<(), Error> {
        let output = self
            .command()
            .output()
            .map_err(|source| Error::io(format!("running `{}`", self.display()), source))?;
        if output.status.success() {
            return Ok(());
        }
        eprintln!("$ {}", self.display());
        Self::report_streams(&output.stdout, &output.stderr);
        Err(Error::CommandFailed {
            command: self.display(),
            code: output.status.code(),
        })
    }

    /// Prints what was read of a failed command's streams. What was
    /// inherited rather than read arrives here empty and prints nothing.
    fn report_streams(stdout: &[u8], stderr: &[u8]) {
        eprint!("{}", String::from_utf8_lossy(stdout));
        eprint!("{}", String::from_utf8_lossy(stderr));
    }

    /// Piped while the xtask is quiet, inherited while it is loud.
    fn quiet_stdio() -> Stdio {
        if out::quiet() {
            Stdio::piped()
        } else {
            Stdio::inherit()
        }
    }

    /// Runs and returns standard output; standard error is inherited, or
    /// read and printed on a failure while the xtask is quiet.
    pub(crate) fn capture(&self) -> Result<String, Error> {
        let output = self
            .command()
            .stdout(Stdio::piped())
            .stderr(Self::quiet_stdio())
            .output()
            .map_err(|source| Error::io(format!("running `{}`", self.display()), source))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Self::report_streams(&[], &output.stderr);
            Err(Error::CommandFailed {
                command: self.display(),
                code: output.status.code(),
            })
        }
    }

    /// Runs and returns standard error; standard output is inherited, or
    /// read and printed on a failure while the xtask is quiet.
    pub(crate) fn capture_stderr(&self) -> Result<String, Error> {
        note!("$ {}", self.display());
        let output = self
            .command()
            .stdout(Self::quiet_stdio())
            .stderr(Stdio::piped())
            .output()
            .map_err(|source| Error::io(format!("running `{}`", self.display()), source))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stderr).into_owned())
        } else {
            if out::quiet() {
                eprintln!("$ {}", self.display());
            }
            Self::report_streams(&output.stdout, &output.stderr);
            Err(Error::CommandFailed {
                command: self.display(),
                code: output.status.code(),
            })
        }
    }
}

/// The Cargo that started the xtask (`$CARGO`, set by Cargo itself).
fn cargo_path() -> PathBuf {
    PathBuf::from(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}
