// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Running child processes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

use crate::error::Error;
use crate::out;

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

    /// Reports one completed parallel job. Only the collecting thread
    /// writes, so the command and its output stay together.
    fn report(&self, result: Result<Output, Error>) -> Result<(), Error> {
        let output = result?;
        if !out::quiet() || !output.status.success() {
            eprintln!("$ {}", self.display());
            Self::report_streams(&output.stdout, &output.stderr);
        }
        if output.status.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: self.display(),
                code: output.status.code(),
            })
        }
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

    /// Runs and returns standard output, or `None` when the command exits
    /// with a failure status. For a command whose failure is an answer:
    /// `git rev-parse HEAD` in a checkout without a commit fails, and what
    /// that says is that there is no commit, not that anything went wrong.
    pub(crate) fn capture_optional(&self) -> Result<Option<String>, Error> {
        let output = self
            .command()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|source| Error::io(format!("running `{}`", self.display()), source))?;
        if output.status.success() {
            Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
        } else {
            Ok(None)
        }
    }
}

/// Maximum concurrent test processes. An explicit setting must be positive.
pub(crate) fn test_jobs() -> Result<usize, Error> {
    match std::env::var("AUDHSOS_TEST_JOBS") {
        Ok(value) => {
            value.parse().ok().filter(|jobs| *jobs > 0).ok_or_else(|| {
                Error::Usage("AUDHSOS_TEST_JOBS must be a positive integer".to_owned())
            })
        }
        Err(std::env::VarError::NotPresent) => {
            Ok(thread::available_parallelism().map_or(1, std::num::NonZero::get))
        }
        Err(error) => Err(Error::Usage(format!("AUDHSOS_TEST_JOBS: {error}"))),
    }
}

/// Runs at most `jobs` processes at once and prints completed output blocks.
/// All queued jobs finish, even after a failure; the step then fails.
pub(crate) fn run_parallel(commands: &[Cmd], jobs: usize) -> Result<(), Error> {
    run_parallel_report(commands, jobs, Cmd::report)
}

/// Collects completions on the calling thread, with an injectable reporter.
pub(crate) fn run_parallel_report(
    commands: &[Cmd],
    jobs: usize,
    mut report: impl FnMut(&Cmd, Result<Output, Error>) -> Result<(), Error>,
) -> Result<(), Error> {
    let jobs = jobs.max(1).min(commands.len());
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::sync_channel(jobs);
    thread::scope(|scope| {
        let mut failure = None;
        for _ in 0..jobs {
            let sender = sender.clone();
            let next = &next;
            let worker = thread::Builder::new().spawn_scoped(scope, move || {
                while let Some(cmd) = commands.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let result = cmd.command().output().map_err(|source| {
                        Error::io(format!("running `{}`", cmd.display()), source)
                    });
                    if sender.send((cmd, result)).is_err() {
                        break;
                    }
                }
            });
            if let Err(source) = worker {
                failure = Some(Error::io("starting a test worker", source));
                break;
            }
        }
        drop(sender);
        for (cmd, result) in receiver {
            if let Err(error) = report(cmd, result) {
                eprintln!("error: {error}");
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    })
}

/// The Cargo that started the xtask (`$CARGO`, set by Cargo itself).
fn cargo_path() -> PathBuf {
    PathBuf::from(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}
