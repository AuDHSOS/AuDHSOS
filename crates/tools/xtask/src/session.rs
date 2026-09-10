// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A run of the machine that is read while it runs.
//!
//! The test kernels are read when they are over: they write, they end the
//! machine, and the runner reads what came out. An end-to-end run cannot be
//! read that way. The console driver owns the serial port, so the only way
//! to send it something is to write into the same stream the machine is
//! writing out of, and the moment to write is when the program on the other
//! side says it is ready.
//!
//! Invariants: the machine is stopped when the session ends, whether the run
//! succeeded, failed, or was never answered; everything that came out is
//! kept, so a failure can be reported with the whole of it.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

use crate::error::Error;
use crate::qemu::{Machine, Options};

/// How long a wait sleeps between two looks at what has arrived.
const POLL: Duration = Duration::from_millis(20);

/// A machine that is running, with its serial port readable and writable.
pub(crate) struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    seen: Vec<String>,
}

impl Session {
    /// Starts `image` on `machine`.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when QEMU cannot be started.
    pub(crate) fn start(machine: &Machine, image: &Path, options: &Options) -> Result<Self, Error> {
        let mut command = Command::new(machine.qemu());
        command
            .args(machine.arguments(image, options))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|source| Error::io("starting QEMU", source))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let (sender, lines) = channel();
        std::thread::spawn(move || {
            let Some(stdout) = stdout else {
                return;
            };
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    return;
                };
                if sender.send(line).is_err() {
                    return;
                }
            }
        });
        Ok(Session {
            child,
            stdin,
            lines,
            seen: Vec::new(),
        })
    }

    /// Waits until a line holding `needle` arrives, or until `timeout` is
    /// up.
    ///
    /// Everything that arrives on the way is kept, so a caller that waits
    /// for the second of two lines does not lose the first.
    pub(crate) fn wait_for(&mut self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now().checked_add(timeout);
        // What has already arrived counts: a line the machine wrote before
        // the wait began is a line that arrived.
        if self.seen.iter().any(|line| line.contains(needle)) {
            return true;
        }
        loop {
            match self.lines.try_recv() {
                Ok(line) => {
                    let found = line.contains(needle);
                    self.seen.push(line);
                    if found {
                        return true;
                    }
                    continue;
                }
                Err(TryRecvError::Disconnected) => return false,
                Err(TryRecvError::Empty) => {}
            }
            if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
                return false;
            }
            std::thread::sleep(POLL);
        }
    }

    /// Waits until one more line holding `needle` has arrived than had
    /// arrived when the wait began, or until `timeout` is up.
    ///
    /// This is what a caller wants when the same line comes more than once
    /// and only the next one is meant. [`Session::wait_for`] is satisfied
    /// by one that came before the wait began, which for a line like
    /// `[canvas] cursor ...` is every earlier move of the pointer.
    pub(crate) fn wait_for_another(&mut self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now().checked_add(timeout);
        let already = self.count_of(needle);
        loop {
            if self.count_of(needle) > already {
                return true;
            }
            match self.lines.try_recv() {
                Ok(line) => {
                    self.seen.push(line);
                    continue;
                }
                Err(TryRecvError::Disconnected) => return false,
                Err(TryRecvError::Empty) => {}
            }
            if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
                return false;
            }
            std::thread::sleep(POLL);
        }
    }

    /// How many lines that have arrived hold `needle`.
    fn count_of(&self, needle: &str) -> usize {
        self.seen
            .iter()
            .filter(|line| line.contains(needle))
            .count()
    }

    /// Writes `bytes` to the serial port of the machine.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the machine is no longer reading.
    pub(crate) fn send(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| Error::Usage("the machine takes no input".to_owned()))?;
        stdin
            .write_all(bytes)
            .map_err(|source| Error::io("writing to the serial port", source))?;
        stdin
            .flush()
            .map_err(|source| Error::io("writing to the serial port", source))
    }

    /// Waits until the machine has ended by itself and answers with its
    /// exit status, or `None` when `timeout` was up first.
    ///
    /// The end of a run belongs to the run: the root task writes to the
    /// exit device when its children are done, and a session that killed
    /// the machine instead would never find out whether it does.
    pub(crate) fn wait_for_end(&mut self, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now().checked_add(timeout);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.code(),
                Ok(None) => {}
                Err(_) => return None,
            }
            if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
                return None;
            }
            std::thread::sleep(POLL);
        }
    }

    /// Everything the machine has written so far.
    pub(crate) fn output(&mut self) -> String {
        while let Ok(line) = self.lines.try_recv() {
            self.seen.push(line);
        }
        self.seen.join("\n")
    }

    /// Stops the machine and answers with everything it wrote.
    pub(crate) fn finish(mut self) -> String {
        // The input is dropped first: a machine waiting on the serial port
        // sees the end of it and stops asking.
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.output()
    }
}
