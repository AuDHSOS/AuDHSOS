// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The database under test, driven as a child process.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::Error;

/// What one run of a script produced.
#[derive(Clone, Debug, Default)]
pub(crate) struct Run {
    /// Everything the statements printed.
    pub(crate) stdout: String,
    /// Everything the engine complained about.
    pub(crate) stderr: String,
    /// Whether the process had to be killed for taking too long.
    pub(crate) timed_out: bool,
}

/// A database engine that runs a script and answers with its output.
pub(crate) trait Engine {
    /// Runs `script` from a fresh, empty database.
    fn run(&mut self, script: &str) -> Result<Run, Error>;
}

/// The `sqlite3` shell over an in-memory database. One process per case,
/// so no case can see what another left behind.
#[derive(Clone, Debug)]
pub(crate) struct Sqlite3 {
    /// The shell binary.
    pub(crate) program: PathBuf,
    /// Its arguments.
    pub(crate) args: Vec<String>,
    /// How long one case may take before the process is killed.
    pub(crate) timeout: Duration,
}

impl Sqlite3 {
    /// The shell at `program`, reading its script from standard input.
    /// `-init /dev/null` keeps a `.sqliterc` of the machine out of the
    /// run, and `:memory:` keeps the database off the disk.
    pub(crate) fn new(program: &Path, timeout: Duration) -> Self {
        Sqlite3 {
            program: program.to_path_buf(),
            args: ["-batch", "-init", "/dev/null", ":memory:"]
                .iter()
                .map(|arg| (*arg).to_owned())
                .collect(),
            timeout,
        }
    }
}

impl Engine for Sqlite3 {
    fn run(&mut self, script: &str) -> Result<Run, Error> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| Error::path("starting", &self.program, source))?;

        // The three streams are served by threads: a script larger than a
        // pipe buffer would otherwise block the write against an engine
        // that is blocked printing.
        let script = script.to_owned();
        let feed = child.stdin.take().map(|mut stdin| {
            thread::spawn(move || {
                let _ = stdin.write_all(script.as_bytes());
            })
        });
        let out = child.stdout.take().map(reader);
        let err = child.stderr.take().map(reader);

        let deadline = Instant::now()
            .checked_add(self.timeout)
            .unwrap_or_else(Instant::now);
        let mut timed_out = false;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(source) => return Err(Error::path("waiting for", &self.program, source)),
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }

        if timed_out {
            // The engine was killed. A child of its own may still hold the
            // pipes, so the threads are left to finish rather than waited
            // for: what a killed engine printed says nothing anyway.
            return Ok(Run {
                timed_out: true,
                ..Run::default()
            });
        }
        if let Some(feed) = feed {
            let _ = feed.join();
        }
        Ok(Run {
            stdout: out.map(collect).unwrap_or_default(),
            stderr: err.map(collect).unwrap_or_default(),
            timed_out,
        })
    }
}

/// Reads one stream to its end on a thread of its own.
fn reader<R: Read + Send + 'static>(mut stream: R) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut text = String::new();
        // Output that is not UTF-8 is the engine's business and not this
        // tool's; what is read up to that point is what the run reports.
        let _ = stream.read_to_string(&mut text);
        text
    })
}

/// The text a reader thread collected, empty if the thread died.
fn collect(handle: thread::JoinHandle<String>) -> String {
    handle.join().unwrap_or_default()
}
