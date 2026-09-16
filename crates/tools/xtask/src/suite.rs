// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! SQLite's own test files, run against `db-sqlite`.
//!
//! `research/sqlite/test` holds the suite `testfixture` drives. This
//! runs each file under `tclsh`, which reads `tools/suite/tester.tcl`
//! for the commands the file drives and reaches the engine over a
//! socket on the loopback address. One database is kept per path a
//! connection opened, and the cases of a file run against it in the
//! order they are written, because each case builds on the ones before
//! it.
//!
//! `docs/17-the-suite-on-the-machine.md` says why the interpreter is
//! `tclsh` and why the tester is this repository's own.
//!
//! Neither the checkout nor the interpreter is part of this repository,
//! so this is never a step of `cargo xtask check`; `sh tools/sqlite.sh`
//! brings the checkout.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use db_sqlite::change::Writer;
use db_sqlite::db::Database;
use db_sqlite::func::Counted;
use db_sqlite::header::Encoding;
use db_sqlite::value::Value;

use crate::error::Error;

/// How a case ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Score {
    /// Cases whose answer is the one the file writes.
    pub(crate) passed: usize,
    /// Cases the engine answered differently.
    pub(crate) failed: usize,
    /// Cases the engine refused, which name what it does not answer.
    pub(crate) refused: usize,
}

impl Score {
    /// How many cases ran.
    pub(crate) const fn ran(self) -> usize {
        self.passed
            .saturating_add(self.failed)
            .saturating_add(self.refused)
    }

    /// Adds another file's score to this one.
    pub(crate) const fn and(&mut self, other: Self) {
        self.passed = self.passed.saturating_add(other.passed);
        self.failed = self.failed.saturating_add(other.failed);
        self.refused = self.refused.saturating_add(other.refused);
    }
}

/// What a file stopped at, counted by the first words of it, which
/// `--why` answers.
static WHY: std::sync::Mutex<Option<BTreeMap<String, usize>>> = std::sync::Mutex::new(None);

/// Counts what each refusal was for from here on.
pub(crate) fn why() {
    if let Ok(mut held) = WHY.lock() {
        *held = Some(BTreeMap::new());
    }
    if let Ok(mut held) = SHAPES.lock() {
        *held = Some(BTreeMap::new());
    }
}

/// What the refusals were for, most first.
pub(crate) fn reasons() -> Vec<(String, usize)> {
    let Ok(held) = WHY.lock() else {
        return Vec::new();
    };
    let mut out: Vec<(String, usize)> = held
        .as_ref()
        .map(|counts| counts.iter().map(|(k, v)| (k.clone(), *v)).collect())
        .unwrap_or_default();
    out.sort_by(|one, other| other.1.cmp(&one.1).then_with(|| one.0.cmp(&other.0)));
    out
}

/// Records that `what` was refused.
fn refused(what: &str) {
    if let Ok(mut held) = WHY.lock()
        && let Some(counts) = held.as_mut()
    {
        let count = counts.entry(what.to_owned()).or_insert(0);
        *count = count.saturating_add(1);
    }
}

/// What the engine refused a statement for, counted by the first
/// words of the statement and the message, which `--why` answers
/// beside the cases.
static SHAPES: std::sync::Mutex<Option<BTreeMap<String, usize>>> = std::sync::Mutex::new(None);

/// What the statements the engine refused were, most first.
pub(crate) fn shapes() -> Vec<(String, usize)> {
    let Ok(held) = SHAPES.lock() else {
        return Vec::new();
    };
    let mut out: Vec<(String, usize)> = held
        .as_ref()
        .map(|counts| counts.iter().map(|(k, v)| (k.clone(), *v)).collect())
        .unwrap_or_default();
    out.sort_by(|one, other| other.1.cmp(&one.1).then_with(|| one.0.cmp(&other.0)));
    out
}

/// Records that the engine refused a statement `what`.
fn shaped(what: &str) {
    if let Ok(mut held) = SHAPES.lock()
        && let Some(counts) = held.as_mut()
    {
        let count = counts.entry(what.to_owned()).or_insert(0);
        *count = count.saturating_add(1);
    }
}

/// Whether a case that did not pass is printed with what each side
/// answered, which `--show` turns on.
static SHOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Prints every case that did not pass from here on.
pub(crate) fn show() {
    SHOW.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// How long one file may run before the interpreter is ended. A file
/// that writes more rows than the engine answers for in this long is
/// scored with what it answered up to there.
const DEADLINE: Duration = Duration::from_secs(60);

/// The capabilities an `ifcapable` may name that this engine does not
/// have. Every other name is answered as held.
const MISSING: [&str; 22] = [
    "vtab",
    "fts1",
    "fts2",
    "fts3",
    "fts4",
    "fts5",
    "rtree",
    "icu",
    "incrblob",
    "shared_cache",
    "memdebug",
    "crashtest",
    "codec",
    "atomicwrite",
    "vacuum",
    "attach",
    "explain",
    "autovacuum",
    "compound_select",
    "unlock_notify",
    "session",
    "update_delete_limit",
];

/// Runs every file of the suite, or the one `only` names, and answers
/// what each scored.
///
/// Each file runs in a process of its own, because a statement the
/// engine answers slowly cannot be stopped from inside: the process is
/// ended when the deadline passes and the file is counted with what it
/// scored up to there. `process::test_jobs` many of those processes run
/// beside each other, since a file collides with no other one: each
/// binds a port the system hands out and writes under a directory its
/// own name keys.
///
/// What the processes wrote is read here, on this thread and in the
/// order of the names, so that `WHY`, `SHAPES` and what `--show` prints
/// are what a run of one file after another wrote.
///
/// # Errors
///
/// [`Error::Usage`] where the suite or the runner is not on the
/// machine; the errors of reading a file.
pub(crate) fn run(root: &Path, only: Option<&str>) -> Result<BTreeMap<String, Score>, Error> {
    let dir = root.join("research").join("sqlite").join("test");
    if !dir.is_dir() {
        return Err(Error::Usage(format!(
            "no suite at {}; run sh tools/sqlite.sh first",
            dir.display()
        )));
    }
    let runner = root.join("tools").join("suite").join("runner.tcl");
    if !runner.is_file() {
        return Err(Error::Usage(format!("no runner at {}", runner.display())));
    }
    let me = std::env::current_exe()
        .map_err(|source| Error::io("reading the path of this program", source))?;
    let wanted: Vec<PathBuf> = files(&dir)?
        .into_iter()
        .filter(|path| only.is_none_or(|wanted| wanted == named(path)))
        .collect();
    let asides = beside(&me, &wanted, crate::process::test_jobs()?)?;
    let mut scores = BTreeMap::new();
    for (path, aside) in wanted.iter().zip(asides) {
        let name = named(path);
        if !aside.ended {
            refused(&format!("{name} took longer than the deadline"));
        }
        let score = read_score(&name, &aside.text);
        if score.ran() != 0 {
            scores.insert(name, score);
        }
    }
    Ok(scores)
}

/// The name a score is keyed by, which is the name of the file.
fn named(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// What the process of one file wrote, and how that process ended.
pub(crate) struct Aside {
    /// What the run wrote on its standard output.
    pub(crate) text: String,
    /// Whether the process ended by itself rather than at the deadline.
    pub(crate) ended: bool,
}

/// Every file in a process of its own, at most `jobs` of them at once,
/// answered in the order the files were given.
///
/// The workers read one work index and the completions are collected
/// here, which is the shape [`crate::process::run_parallel_report`]
/// runs a step's commands in. That runner is not reused, because it
/// waits on a child for as long as the child runs and this one ends a
/// child at the deadline.
///
/// # Errors
///
/// The errors of starting a worker or of running one file.
pub(crate) fn beside(me: &Path, files: &[PathBuf], jobs: usize) -> Result<Vec<Aside>, Error> {
    let jobs = jobs.max(1).min(files.len());
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::sync_channel(jobs.max(1));
    let mut done: Vec<(usize, Aside)> = thread::scope(|scope| {
        let mut failure = None;
        let mut done = Vec::with_capacity(files.len());
        for _ in 0..jobs {
            let sender = sender.clone();
            let next = &next;
            let worker = thread::Builder::new().spawn_scoped(scope, move || {
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(path) = files.get(at) else {
                        break;
                    };
                    if sender.send((at, apart(me, path))).is_err() {
                        break;
                    }
                }
            });
            if let Err(source) = worker {
                failure = Some(Error::io("starting a worker for the suite", source));
                break;
            }
        }
        drop(sender);
        for (at, result) in receiver {
            match result {
                Ok(aside) => done.push((at, aside)),
                Err(error) => {
                    if failure.is_none() {
                        failure = Some(error);
                    }
                }
            }
        }
        failure.map_or(Ok(done), Err)
    })?;
    done.sort_by_key(|(at, _)| *at);
    Ok(done.into_iter().map(|(_, aside)| aside).collect())
}

/// How long a run waits for a descriptor another thread holds open to
/// the binary to be closed.
const BUSY: Duration = Duration::from_secs(5);

/// The process of one file, started.
///
/// A thread that forks while another writes a file holds a descriptor
/// open to it, so `execve` refuses the binary as busy for as long as
/// that descriptor stands; the run waits it out rather than answering
/// the refusal.
///
/// # Errors
///
/// [`Error`] names what starting the process refused.
fn started(me: &Path, path: &Path) -> Result<std::process::Child, Error> {
    let over = Instant::now();
    loop {
        let started = Command::new(me)
            .arg("sqlite-suite")
            .arg("--one")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        match started {
            Ok(child) => return Ok(child),
            Err(source)
                if source.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && over.elapsed() < BUSY => {}
            Err(source) => return Err(Error::io("starting the run of one file", source)),
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// One file in a process of its own, ended where the deadline passes.
///
/// The standard output is read after the process ended and not while it
/// runs, because a file that writes more than the pipe holds is one the
/// deadline ends, and reading along would score cases that a run of one
/// file after another never counted.
fn apart(me: &Path, path: &Path) -> Result<Aside, Error> {
    let mut child = started(me, path)?;
    let over = Instant::now();
    let ended = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) => {}
            Err(source) => return Err(Error::io("waiting for the run of one file", source)),
        }
        if over.elapsed() > DEADLINE {
            break false;
        }
        thread::sleep(Duration::from_millis(20));
    };
    if !ended {
        let _ = child.kill();
        let _ = child.wait();
    }
    // What the run wrote before it was ended is still in the pipe, so a
    // file the deadline ended is counted with the cases it ran.
    let mut text = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut text);
    }
    Ok(Aside { text, ended })
}

/// What a run of one file wrote about itself: one line per case, one
/// per refusal, and one per case that answered differently.
pub(crate) fn read_score(name: &str, text: &str) -> Score {
    let mut score = Score::default();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some((kind, rest)) = line.split_once(' ') else {
            continue;
        };
        match kind {
            "C" => match rest {
                "passed" => score.passed = score.passed.saturating_add(1),
                "refused" => score.refused = score.refused.saturating_add(1),
                _ => score.failed = score.failed.saturating_add(1),
            },
            "W" => refused(rest),
            "S" => shaped(rest),
            "F" if SHOW.load(std::sync::atomic::Ordering::Relaxed) => {
                let mine = lines.next().unwrap_or("");
                let want = lines.next().unwrap_or("");
                crate::out::note!("{name} {rest}\n{mine}\n{want}");
            }
            "F" => {
                lines.next();
                lines.next();
            }
            _ => {}
        }
    }
    score
}

/// One file, run here and written out for the process that started it.
///
/// # Errors
///
/// The errors of the line.
pub(crate) fn one(root: &Path, path: &Path) -> Result<(), Error> {
    let runner = root.join("tools").join("suite").join("runner.tcl");
    alone(&runner, path, &named(path))?;
    Ok(())
}

/// One file: the harness listens, the interpreter reads the file, and
/// every request of the tester is answered until the file is done.
fn alone(runner: &Path, path: &Path, name: &str) -> Result<(), Error> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|source| Error::io("listening for the interpreter", source))?;
    let port = listener
        .local_addr()
        .map_err(|source| Error::io("reading the port", source))?
        .port();
    // The interpreter runs in a directory of its own, because a file
    // of the suite writes beside itself: a log, a copy of a database,
    // a script it reads back.
    let scratch = std::env::temp_dir().join(format!("audhsos-suite-{name}"));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)
        .map_err(|source| Error::io("making the directory the file runs in", source))?;
    let mut child = Command::new("tclsh")
        .arg(runner)
        .arg(path)
        .arg(port.to_string())
        .current_dir(&scratch)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| Error::io("starting tclsh", source))?;
    let mut session = Session::new(name);
    if let Ok((stream, _)) = listener.accept() {
        let _ = stream.set_read_timeout(Some(DEADLINE));
        let _ = session.serve(&stream);
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}

/// Every `.test` file of the checkout, in the order their names run.
fn files(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir)
        .map_err(|source| Error::io(format!("reading {}", dir.display()), source))?;
    for entry in entries {
        let entry = entry.map_err(|source| Error::io("reading a suite entry", source))?;
        let path = entry.path();
        if path.extension().is_some_and(|kind| kind == "test") {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// One run of one file: the interpreter on one side of the line and the
/// engine on the other.
struct Session {
    /// The name of the file, for what `--show` writes.
    file: String,
    /// One writer per path a connection opened, which is what two
    /// connections over one path share.
    held: BTreeMap<String, Writer>,
    /// Which path each connection reads.
    connections: BTreeMap<String, String>,
    /// What a `NULL` prints as, per connection.
    nulls: BTreeMap<String, String>,
    /// The three counters of each connection, which belong to a
    /// connection and not to the file the connection opened.
    counters: BTreeMap<String, Counted>,
    /// What the file has scored.
    score: Score,
    /// When the file began, which is what the deadline is counted from.
    started: Instant,
}

impl Session {
    /// A run that has answered nothing.
    fn new(file: &str) -> Self {
        Session {
            file: file.to_owned(),
            held: BTreeMap::new(),
            connections: BTreeMap::new(),
            nulls: BTreeMap::new(),
            counters: BTreeMap::new(),
            score: Score::default(),
            started: Instant::now(),
        }
    }

    /// Answers the requests of the tester until the line closes.
    fn serve(&mut self, stream: &TcpStream) -> Result<(), Error> {
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .map_err(|source| Error::io("reading the line", source))?,
        );
        let mut writer = stream;
        while let Some((verb, args)) = request(&mut reader)? {
            // The deadline is counted over the file and not over one
            // request, because a file that writes a million rows spends
            // its time inside the engine and not on the line.
            if self.started.elapsed() > DEADLINE {
                say(&format!("W {} took longer than the deadline", self.file));
                return Ok(());
            }
            match self.answered(&verb, &args) {
                Ok(values) => write_ok(&mut writer, &values)?,
                Err(message) => write_error(&mut writer, &message)?,
            }
            if verb == "done" {
                return Ok(());
            }
        }
        Ok(())
    }

    /// What one request answers, or the message it raises.
    fn answered(&mut self, verb: &str, args: &[String]) -> Result<Vec<String>, String> {
        let first = args.first().map_or("", String::as_str);
        let second = args.get(1).map_or("", String::as_str);
        match verb {
            "open" => {
                self.open(first, second);
                Ok(Vec::new())
            }
            "close" => {
                self.connections.remove(first);
                self.counters.remove(first);
                Ok(Vec::new())
            }
            "delete" => {
                self.held.remove(first);
                Ok(Vec::new())
            }
            "exists" => Ok(vec![usize::from(self.held.contains_key(first)).to_string()]),
            "copy" => self.copy(first, second),
            "null" => {
                self.nulls.insert(first.to_owned(), second.to_owned());
                Ok(Vec::new())
            }
            "eval" => self.eval(first, second),
            "names" => self.names(first, second),
            "changes" | "total_changes" | "rowid" => self.counted(verb, first),
            // What the engine's writer does not answer. A case that
            // reads one of these is refused rather than scored against
            // a number this harness made up.
            "errorcode" => Err(format!("this harness has no {verb}")),
            "capable" => Ok(vec![usize::from(capable(first)).to_string()]),
            "case" => {
                self.case(args);
                Ok(Vec::new())
            }
            "stopped" => {
                say(&format!("W stopped: {}", first_words(first)));
                Ok(Vec::new())
            }
            "done" => Ok(Vec::new()),
            other => Err(format!("this harness has no request {other}")),
        }
    }

    /// One of the three counters of a connection: `db changes`,
    /// `db total_changes` and `db last_insert_rowid`.
    fn counted(&self, verb: &str, name: &str) -> Result<Vec<String>, String> {
        let counted = self
            .counters
            .get(name)
            .copied()
            .ok_or_else(|| format!("no such connection: {name}"))?;
        let answer = match verb {
            "changes" => counted.changes,
            "total_changes" => counted.total,
            _ => counted.rowid,
        };
        Ok(vec![answer.to_string()])
    }

    /// Opens a connection over a path, making the database where no
    /// connection has opened that path yet.
    fn open(&mut self, name: &str, path: &str) {
        if !self.held.contains_key(path)
            && let Ok(writer) = Writer::new(4096, 0, Encoding::Utf8)
        {
            self.held.insert(path.to_owned(), writer);
        }
        self.connections.insert(name.to_owned(), path.to_owned());
        self.nulls.entry(name.to_owned()).or_default();
        // A connection that is opened again counts from nought, which
        // is what `sqlite3 db test.db` in a file relies on.
        self.counters.insert(name.to_owned(), Counted::default());
    }

    /// One database written again under another path, which is what a
    /// file that saves itself and reads the save back asks for.
    fn copy(&mut self, from: &str, to: &str) -> Result<Vec<String>, String> {
        let Some(held) = self.held.get(from) else {
            self.held.remove(to);
            return Ok(Vec::new());
        };
        let bytes = held.written();
        let writer = Writer::opened(&bytes).map_err(|error| error.message())?;
        self.held.insert(to.to_owned(), writer);
        Ok(Vec::new())
    }

    /// The statements of one text, in order, answered as one list.
    fn eval(&mut self, name: &str, sql: &str) -> Result<Vec<String>, String> {
        let null = self.nulls.get(name).cloned().unwrap_or_default();
        let path = self
            .connections
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no such connection: {name}"))?;
        let counted = self.counters.get(name).copied().unwrap_or_default();
        let writer = self
            .held
            .get_mut(&path)
            .ok_or_else(|| format!("no such database: {path}"))?;
        // The counters belong to the connection and the pages to the
        // file, so the writer stands at this connection's counters for
        // the statements of this request and answers them back.
        writer.counts_as(counted);
        let mut out = Vec::new();
        let mut ran = Ok(());
        for statement in statements(sql) {
            // The bytes after the last one that carries meaning are the
            // statement's own: `SELECT 1 /* ` is a comment that runs to
            // the end and `SELECT 1 /*` is a slash and a star.
            let text = statement.trim_start();
            if text.trim().is_empty() {
                continue;
            }
            match run_one(writer, text) {
                Ok(values) => out.extend(values.iter().map(|value| listed(value, &null))),
                Err(message) => {
                    ran = Err(message);
                    break;
                }
            }
        }
        let counted = writer.counts();
        self.counters.insert(name.to_owned(), counted);
        ran?;
        Ok(out)
    }

    /// The names of the columns the last statement of a text answers.
    fn names(&self, name: &str, sql: &str) -> Result<Vec<String>, String> {
        let path = self
            .connections
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no such connection: {name}"))?;
        let writer = self
            .held
            .get(&path)
            .ok_or_else(|| format!("no such database: {path}"))?;
        let last = statements(sql)
            .into_iter()
            .map(str::trim)
            .rfind(|text| !text.is_empty())
            .unwrap_or("");
        if !reads(last) {
            return Ok(Vec::new());
        }
        let bytes = writer.written();
        let database = Database::open(&bytes).map_err(|error| error.message())?;
        let answered = database
            .query(last.as_bytes())
            .map_err(|error| error.message())?;
        Ok(answered
            .names
            .iter()
            .map(|name| String::from_utf8_lossy(name).into_owned())
            .collect())
    }

    /// Scores one case, written out as it is scored so that a file the
    /// deadline ends is counted with the cases it ran.
    fn case(&mut self, args: &[String]) {
        let name = args.first().map_or("", String::as_str);
        match args.get(1).map_or("", String::as_str) {
            "passed" => {
                self.score.passed = self.score.passed.saturating_add(1);
                say("C passed");
            }
            "refused" => {
                self.score.refused = self.score.refused.saturating_add(1);
                say("C refused");
                say(&format!("W {}", args.get(2).map_or("", String::as_str)));
            }
            _ => {
                self.score.failed = self.score.failed.saturating_add(1);
                say("C failed");
                say(&format!(
                    "F {name}\n  mine {:?}\n  want {:?}",
                    args.get(2).map_or("", String::as_str),
                    args.get(3).map_or("", String::as_str)
                ));
            }
        }
    }
}

/// One line to the process that started this run, written as it
/// happens because that process may end this one at the deadline.
fn say(line: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// One statement: a reader answers one that reads and the writer
/// answers the rest, which is what a connection does with either.
fn run_one(writer: &mut Writer, text: &str) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    if reads(text) {
        let bytes = writer.written();
        // A connection in write-ahead logging holds its newest pages in
        // the log, so a reader follows the log beside the file.
        let log = writer.log().map(db_sqlite::wal::Wal::open);
        let opened = match &log {
            Some(Ok(log)) => Database::open_with_log(&bytes, log),
            Some(Err(error)) => return Err(format!("{error}")),
            None => Database::open(&bytes),
        };
        let counted = writer.counts();
        let answered = opened
            .map(|database| database.counting(counted))
            .and_then(|database| database.query(text.as_bytes()))
            .map_err(|error| shape(text, error.message()))?;
        for row in &answered.rows {
            out.extend(row.iter().cloned());
        }
        return Ok(out);
    }
    let rows = writer
        .run(text.as_bytes())
        .map_err(|error| shape(text, error.message()))?;
    for row in &rows {
        out.extend(row.iter().cloned());
    }
    Ok(out)
}

/// Whether this engine has what an `ifcapable` names, which is a
/// condition of names, `!`, `&&`, `||` and brackets.
fn capable(expression: &str) -> bool {
    let mut words = Vec::new();
    let mut held = String::new();
    for character in expression.chars() {
        if character.is_alphanumeric() || character == '_' {
            held.push(character);
            continue;
        }
        if !held.is_empty() {
            words.push(core::mem::take(&mut held));
        }
        if !character.is_whitespace() {
            words.push(character.to_string());
        }
    }
    if !held.is_empty() {
        words.push(held);
    }
    // `&&` and `||` come out as two characters each, which are joined
    // here so that the walk over them reads one operator.
    let mut joined: Vec<String> = Vec::new();
    for word in words {
        if joined.last().is_some_and(|last| *last == word) && (word == "&" || word == "|") {
            continue;
        }
        joined.push(word);
    }
    let mut at = 0;
    condition(&joined, &mut at)
}

/// One condition, which is terms joined by `&&` and `||` read left to
/// right, because the suite writes no condition that needs more.
fn condition(words: &[String], at: &mut usize) -> bool {
    let mut held = term(words, at);
    while let Some(word) = words.get(*at) {
        let operator = word.clone();
        if operator != "&" && operator != "|" {
            break;
        }
        *at = at.saturating_add(1);
        let next = term(words, at);
        held = if operator == "&" {
            held && next
        } else {
            held || next
        };
    }
    held
}

/// One term: a name, a `!` before a term, or a condition in brackets.
fn term(words: &[String], at: &mut usize) -> bool {
    let Some(word) = words.get(*at).cloned() else {
        return true;
    };
    *at = at.saturating_add(1);
    match word.as_str() {
        "!" => !term(words, at),
        "(" => {
            let held = condition(words, at);
            if words.get(*at).is_some_and(|word| word == ")") {
                *at = at.saturating_add(1);
            }
            held
        }
        name => !MISSING.iter().any(|held| name.eq_ignore_ascii_case(held)),
    }
}

/// One request of the tester: the verb and its values.
fn request(reader: &mut BufReader<TcpStream>) -> Result<Option<(String, Vec<String>)>, Error> {
    let Some(head) = line(reader)? else {
        return Ok(None);
    };
    let mut parts = head.split_whitespace();
    if parts.next() != Some("REQ") {
        return Ok(None);
    }
    let verb = parts.next().unwrap_or_default().to_owned();
    let count: usize = parts.next().unwrap_or("0").parse().unwrap_or(0);
    let mut args = Vec::new();
    for _ in 0..count {
        let Some(length) = line(reader)? else {
            return Ok(None);
        };
        let length: usize = length.trim().parse().unwrap_or(0);
        let mut bytes = vec![0_u8; length];
        reader
            .read_exact(&mut bytes)
            .map_err(|source| Error::io("reading a value", source))?;
        line(reader)?;
        args.push(String::from_utf8_lossy(&bytes).into_owned());
    }
    Ok(Some((verb, args)))
}

/// One line of the line, without its newline, and nothing where it
/// closed.
fn line(reader: &mut BufReader<TcpStream>) -> Result<Option<String>, Error> {
    let mut held = String::new();
    let read = reader
        .read_line(&mut held)
        .map_err(|source| Error::io("reading the line", source))?;
    if read == 0 {
        return Ok(None);
    }
    Ok(Some(held.trim_end_matches(['\n', '\r']).to_owned()))
}

/// Answers a request with its values.
fn write_ok(stream: &mut &TcpStream, values: &[String]) -> Result<(), Error> {
    let mut out = format!("OK {}\n", values.len()).into_bytes();
    for value in values {
        out.extend_from_slice(format!("{}\n", value.len()).as_bytes());
        out.extend_from_slice(value.as_bytes());
        out.push(b'\n');
    }
    stream
        .write_all(&out)
        .map_err(|source| Error::io("writing an answer", source))
}

/// Answers a request with the message it raised.
fn write_error(stream: &mut &TcpStream, message: &str) -> Result<(), Error> {
    let mut out = format!("ERR {}\n", message.len()).into_bytes();
    out.extend_from_slice(message.as_bytes());
    out.push(b'\n');
    stream
        .write_all(&out)
        .map_err(|source| Error::io("writing a refusal", source))
}

/// Writes out what the engine refused a statement for, keyed by the
/// first words of the statement, and answers the message the tester
/// reads, which `catchsql` compares against the C library's own.
fn shape(text: &str, message: String) -> String {
    say(&format!("S {} {message}", first_words(text)));
    message
}

/// The first two words of a statement in capitals, which is enough to
/// say what kind of statement the engine refused.
fn first_words(sql: &str) -> String {
    sql.split_whitespace()
        .take(2)
        .map(str::to_uppercase)
        .collect::<Vec<String>>()
        .join(" ")
}

/// A `PRAGMA` goes to the connection either way, because a connection
/// answers one out of what it holds and a file with no table holds no
/// encoding.
fn reads(sql: &str) -> bool {
    let words = words(sql);
    let first = words.first().map_or("", String::as_str);
    if first.eq_ignore_ascii_case("select") || first.eq_ignore_ascii_case("values") {
        return true;
    }
    // A `WITH` clause stands in front of a statement that writes as
    // well, and the statement after it is what says which connection
    // runs the whole.
    first.eq_ignore_ascii_case("with")
        && !words.iter().any(|word| {
            ["insert", "update", "delete", "replace"]
                .iter()
                .any(|written| word.eq_ignore_ascii_case(written))
        })
}

/// `sql` with the comments taken out, so that the first word of a
/// statement is the word that says what the statement does.
///
/// A comment inside a string stays, because a string is not a comment.
/// Reading the text costs O(n) in its bytes.
fn uncommented(sql: &str) -> String {
    let mut out = String::new();
    let mut quote: Option<char> = None;
    let mut rest = sql;
    while let Some(character) = rest.chars().next() {
        if let Some(mark) = quote {
            quote = (character != mark).then_some(mark);
        } else if matches!(character, '\'' | '"' | '`') {
            quote = Some(character);
        } else if rest.starts_with("--") {
            let end = rest.find('\n').unwrap_or(rest.len());
            out.push(' ');
            rest = rest.get(end..).unwrap_or_default();
            continue;
        } else if rest.starts_with("/*") {
            let end = rest
                .get(2..)
                .and_then(|rest| rest.find("*/"))
                .map_or(rest.len(), |at| at.saturating_add(4));
            out.push(' ');
            rest = rest.get(end..).unwrap_or_default();
            continue;
        }
        out.push(character);
        rest = rest.get(character.len_utf8()..).unwrap_or_default();
    }
    out
}

/// The words of a statement that stand outside a string.
fn words(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut held = String::new();
    let mut quote: Option<char> = None;
    for character in uncommented(sql).chars() {
        match quote {
            Some(mark) if character == mark => quote = None,
            Some(_) => {}
            None if matches!(character, '\'' | '"' | '`') => {
                quote = Some(character);
                if !held.is_empty() {
                    out.push(std::mem::take(&mut held));
                }
            }
            None if character.is_alphanumeric() || character == '_' => held.push(character),
            None => {
                if !held.is_empty() {
                    out.push(std::mem::take(&mut held));
                }
            }
        }
    }
    if !held.is_empty() {
        out.push(held);
    }
    out
}

/// The statements of one case, split on the semicolons that stand
/// outside a string.
pub(crate) fn statements(sql: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    for (at, character) in sql.char_indices() {
        match quote {
            Some(mark) if character == mark => quote = None,
            Some(_) => {}
            None => match character {
                '\'' | '"' | '`' => quote = Some(character),
                // A `CREATE TRIGGER` carries semicolons inside its
                // body, so the one that ends it is the one after the
                // word `END`, which is what `sqlite3_complete` reads.
                ';' if ends(sql.get(start..at).unwrap_or_default()) => {
                    if let Some(piece) = sql.get(start..at) {
                        out.push(piece);
                    }
                    start = at.saturating_add(1);
                }
                _ => {}
            },
        }
    }
    if let Some(piece) = sql.get(start..) {
        out.push(piece);
    }
    out
}

/// Whether a semicolon after this much text ends the statement, which
/// it does for everything but a `CREATE TRIGGER` whose `END` has not
/// been read yet.
fn ends(text: &str) -> bool {
    let words = words(text);
    let mut names = words.iter().map(String::as_str);
    let Some(first) = names.next() else {
        return true;
    };
    if !first.eq_ignore_ascii_case("create") {
        return true;
    }
    if !words
        .iter()
        .take(6)
        .any(|word| word.eq_ignore_ascii_case("trigger"))
    {
        return true;
    }
    words
        .last()
        .is_some_and(|word| word.eq_ignore_ascii_case("end"))
}

/// One value as an element of a TCL list: nothing is the empty element,
/// and a real keeps the digits SQLite prints it with.
fn listed(value: &Value, null: &str) -> String {
    match value {
        Value::Null => null.to_owned(),
        Value::Int(number) => number.to_string(),
        // `tester.tcl` sets `tcl_precision 15`, so the answers a file
        // writes for a real are the fifteen significant digits the
        // interpreter wrote and not the seventeen the library writes.
        Value::Real(number) => {
            String::from_utf8_lossy(&db_sqlite::fp::text(*number, 15)).into_owned()
        }
        Value::Text(bytes) | Value::Blob(bytes) => String::from_utf8_lossy(bytes).into_owned(),
    }
}
