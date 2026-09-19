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

use std::cell::RefCell;
use std::cmp::Ordering;
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
use db_sqlite::func::Defined;
use db_sqlite::header::Encoding;
use db_sqlite::pragma::Kept;
use db_sqlite::random::Source;
use db_sqlite::value::{Collating, Value};

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

/// One run-time configuration a connection of this harness opens under.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Configuration {
    /// The name the option carries.
    pub(crate) name: &'static str,
    /// The bytes one page holds.
    page: u32,
    /// The encoding the text of the file is written in.
    encoding: Encoding,
    /// The journal mode the connection is set to as it opens, and
    /// nothing where it opens in the mode a new file carries.
    journal: Option<&'static [u8]>,
}

/// The configurations `--configuration` names, the first of which is
/// the one a run that names none opens under.
///
/// `permutations.test` of SQLite's own suite runs its files under a
/// table of configurations; this is the part of that table this harness
/// opens a connection under.
pub(crate) const CONFIGURATIONS: [Configuration; 9] = [
    Configuration {
        name: "utf8-4096-delete",
        page: 4096,
        encoding: Encoding::Utf8,
        journal: None,
    },
    Configuration {
        name: "utf16le-4096-delete",
        page: 4096,
        encoding: Encoding::Utf16Le,
        journal: None,
    },
    Configuration {
        name: "utf16be-4096-delete",
        page: 4096,
        encoding: Encoding::Utf16Be,
        journal: None,
    },
    Configuration {
        name: "utf8-512-delete",
        page: 512,
        encoding: Encoding::Utf8,
        journal: None,
    },
    Configuration {
        name: "utf8-1024-delete",
        page: 1024,
        encoding: Encoding::Utf8,
        journal: None,
    },
    Configuration {
        name: "utf8-65536-delete",
        page: 65536,
        encoding: Encoding::Utf8,
        journal: None,
    },
    Configuration {
        name: "utf8-4096-persist",
        page: 4096,
        encoding: Encoding::Utf8,
        journal: Some(b"persist"),
    },
    Configuration {
        name: "utf8-4096-truncate",
        page: 4096,
        encoding: Encoding::Utf8,
        journal: Some(b"truncate"),
    },
    Configuration {
        name: "utf8-4096-wal",
        page: 4096,
        encoding: Encoding::Utf8,
        journal: Some(b"wal"),
    },
];

/// Which of [`CONFIGURATIONS`] this run opens its connections under.
static CHOSEN: AtomicUsize = AtomicUsize::new(0);

/// Opens every connection of this run under the configuration `name`.
///
/// # Errors
///
/// [`Error::Usage`] names the configurations there are where `name` is
/// none of them.
pub(crate) fn configure(name: &str) -> Result<(), Error> {
    let at = CONFIGURATIONS
        .iter()
        .position(|one| one.name == name)
        .ok_or_else(|| {
            let held: Vec<&str> = CONFIGURATIONS.iter().map(|one| one.name).collect();
            Error::Usage(format!(
                "no configuration `{name}`; the ones there are: {}",
                held.join(", ")
            ))
        })?;
    CHOSEN.store(at, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// The configuration this run opens its connections under.
fn configured() -> Configuration {
    let at = CHOSEN.load(std::sync::atomic::Ordering::Relaxed);
    CONFIGURATIONS.get(at).copied().unwrap_or(CONFIGURATIONS[0])
}

/// How long one file may run before the interpreter is ended. A file
/// that writes more rows than the engine answers for in this long is
/// scored with what it answered up to there.
///
/// `rowvalue2.test` answers 3834 cases in about two minutes and was
/// cut at some 2900 under a deadline of one, so the count a run answers
/// moved by hundreds between runs; three minutes is past every file but
/// the handful that answer for tens of thousands of rows.
const DEADLINE: Duration = Duration::from_secs(180);

/// How many cases in a row one file may have refused for the same reason
/// before the file is ended. A loop whose end a command this harness has
/// none of decides runs without bound, and the file is scored with what
/// it answered up to there.
const LOOPING: usize = 5000;

/// The capabilities an `ifcapable` may name that this engine does not
/// have. Every other name is answered as held.
const MISSING: [&str; 20] = [
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
            .arg("--configuration")
            .arg(configured().name)
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

/// One statement the tester prepared, which it steps and reads the
/// columns of.
///
/// `sqlite3_prepare` compiles a statement and `sqlite3_step` runs it one
/// row at a time. This harness runs the whole statement at the first
/// step and holds its rows, so a step reads the row it stands on rather
/// than the next row of a walk.
struct Prepared {
    /// The connection it was prepared on.
    connection: String,
    /// The text of the statement, as the tester wrote it.
    sql: String,
    /// What the tester bound at each place, counting from one.
    bound: BTreeMap<usize, String>,
    /// The names of the columns the statement answers.
    names: Vec<String>,
    /// The type the schema declares for each of those columns.
    declared: Vec<String>,
    /// The rows the statement answered, where it has run.
    rows: Vec<Vec<Value>>,
    /// Which row the reader stands on, counting from nought.
    at: usize,
    /// Whether the statement has run.
    ran: bool,
    /// Whether the last step answered a row.
    row: bool,
    /// Whether `sqlite3_prepare` and not `sqlite3_prepare_v2` made it,
    /// which is what makes a step answer `SQLITE_ERROR` rather than the
    /// code the refusal carries.
    legacy: bool,
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
    /// What each connection was told for the pragmas it keeps a value
    /// for, which belong to a connection and not to the file.
    pragmas: BTreeMap<String, Kept>,
    /// The collations the tester defined on each connection, which a
    /// connection that is opened again holds none of.
    collations: BTreeMap<String, &'static [Collating]>,
    /// The functions the tester defined on each connection, beside the
    /// ones this harness holds.
    functions: BTreeMap<String, &'static [Defined]>,
    /// Whether the tester told each connection an authorizer, which a
    /// connection that is opened again holds none of.
    authorizers: BTreeMap<String, bool>,
    /// The statements the tester prepared, by the name it holds each at.
    statements: BTreeMap<String, Prepared>,
    /// How many statements the tester has prepared, which names the next
    /// one.
    prepared: u64,
    /// What `save_prng_state` held, which `restore_prng_state` hands
    /// back to every writer.
    prng: u64,
    /// The moment `now` names, as the seconds since 1970, and nothing
    /// where the file set none: `sqlite_current_time` at nought is the
    /// clock of the machine, which this harness has none of.
    clock: Option<i64>,
    /// The reason the cases before this one were refused for and how
    /// many of them in a row carried it, which stops a file that loops
    /// until a command this harness has none of answers.
    repeated: (String, usize),
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
            pragmas: BTreeMap::new(),
            collations: BTreeMap::new(),
            functions: BTreeMap::new(),
            authorizers: BTreeMap::new(),
            statements: BTreeMap::new(),
            prepared: 0,
            prng: 0,
            clock: None,
            repeated: (String::new(), 0),
            score: Score::default(),
            started: Instant::now(),
        }
    }

    /// Answers the requests of the tester until the line closes.
    ///
    /// The line is held on the thread, because a collation the tester
    /// defined writes a call onto it from inside the engine.
    fn serve(&mut self, stream: &TcpStream) -> Result<(), Error> {
        let reader = BufReader::new(
            stream
                .try_clone()
                .map_err(|source| Error::io("reading the line", source))?,
        );
        let writing = stream
            .try_clone()
            .map_err(|source| Error::io("reading the line", source))?;
        LINE.with(|line| {
            *line.borrow_mut() = Some(Line {
                reader,
                writer: writing,
            });
        });
        let out = self.answering(stream);
        LINE.with(|line| line.borrow_mut().take());
        out
    }

    /// Reads one request at a time and answers it.
    fn answering(&mut self, stream: &TcpStream) -> Result<(), Error> {
        let mut writer = stream;
        while let Some((verb, args)) = LINE.with(|line| {
            line.borrow_mut()
                .as_mut()
                .map_or(Ok(None), |line| request(&mut line.reader))
        })? {
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
                // `sqlite3_close` rolls back the transaction the
                // connection had open, and the harness holds one
                // transaction per file, so the file the connection
                // closed leaves none.
                if let Some(path) = self.connections.get(first).cloned()
                    && let Some(writer) = self.held.get_mut(&path)
                {
                    let _ = writer.run(b"ROLLBACK");
                }
                self.connections.remove(first);
                self.nulls.remove(first);
                self.counters.remove(first);
                self.pragmas.remove(first);
                self.collations.remove(first);
                self.functions.remove(first);
                Ok(Vec::new())
            }
            "delete" => {
                self.held.remove(first);
                Ok(Vec::new())
            }
            "exists" => Ok(vec![usize::from(self.sized(first).is_some()).to_string()]),
            // `md5` and `md5file` of `test_md5.c`: the digest of a text
            // and the digest of a file the harness holds.
            "md5" => Ok(vec![crate::md5::digest(first.as_bytes())]),
            "md5file" => self.digest_of(first),
            // `hexio_read` and `hexio_write` of `test_hexio.c`: the
            // bytes of a file this harness holds, read out as
            // hexadecimal digits and written back from them.
            "read" => self.read_bytes(first, second, args.get(2).map_or("", String::as_str)),
            "write" => self.write_bytes(first, second, args.get(2).map_or("", String::as_str)),
            // `file size`: the bytes the harness holds under a name,
            // and minus one where it holds nothing under it.
            "size" => Ok(vec![
                self.sized(first)
                    .map_or(-1, |bytes| i64::try_from(bytes).unwrap_or(i64::MAX))
                    .to_string(),
            ]),
            // `sqlite3_complete` reads the text alone, so the
            // connection the request names says nothing about it.
            "complete" => Ok(vec![
                usize::from(db_sqlite::token::complete(second.as_bytes())).to_string(),
            ]),
            "copy" => self.copy(first, second),
            // `sqlite3_create_collation`: the tester names a proc, and
            // the engine reaches that proc back over the line.
            "collate" => {
                self.collates(first, second);
                Ok(Vec::new())
            }
            // `sqlite3_create_function`: the tester names a proc, and
            // the engine reaches that proc back over the line.
            "function" => {
                self.functions(first, second);
                Ok(Vec::new())
            }
            "null" => {
                self.nulls.insert(first.to_owned(), second.to_owned());
                Ok(Vec::new())
            }
            "clock" => self.ticks(first),
            // `sqlite3_set_authorizer`, and
            // `sqlite3_table_column_metadata DB SCHEMA TABLE COLUMN`.
            "authorizer" => {
                self.authorizes(first, second);
                Ok(Vec::new())
            }
            // `sqlite3_prepare`, the commands that read a statement it
            // answered, and the three that read what it is.
            "prepare" | "autocommit" | "step" | "finalize" | "reset" | "clear_binds" | "bind"
            | "column" | "stmt" | "next_stmt" | "readonly" | "busy" | "isexplain" => {
                self.of_statement(verb, args)
            }
            // `sqlite3_errcode` and `sqlite3_extended_errcode`.
            "errcode" => Ok(alloc_one(&last_code(first))),
            "normalize" => Ok(alloc_one(&normalized(first))),
            "columnmeta" => self.column_meta(first, second, args.get(2).map_or("", String::as_str)),
            "eval" => self.eval(first, second),
            "names" => self.names(first, second),
            "changes" | "total_changes" | "rowid" => self.counted(verb, first),
            // What the engine's writer does not answer. A case that
            // reads one of these is refused rather than scored against
            // a number this harness made up.
            "errorcode" => Err(format!("this harness has no {verb}")),
            // `save_prng_state` and `restore_prng_state`: SQLite draws
            // for the whole process from one source, so the state one
            // writer answers is handed to every writer.
            "save_prng" => {
                self.prng = self.held.values().next().map_or(0, Writer::randomness_held);
                Ok(Vec::new())
            }
            "restore_prng" => {
                let state = self.prng;
                for writer in self.held.values_mut() {
                    writer.randomness(state);
                }
                Ok(Vec::new())
            }
            "varint" => varint(args),
            "mprintf" => mprintf(args),
            "capable" => Ok(vec![usize::from(capable(first)).to_string()]),
            "case" => {
                self.case(args);
                if self.repeated.1 > LOOPING {
                    return Err(format!(
                        "{} cases in a row were refused for: {}",
                        self.repeated.1, self.repeated.0
                    ));
                }
                Ok(Vec::new())
            }
            "stopped" => {
                say(&format!("W stopped: {}", first_line(first)));
                Ok(Vec::new())
            }
            "done" => Ok(Vec::new()),
            other => Err(format!("this harness has no request {other}")),
        }
    }

    /// How many bytes the harness holds under `name`: the database
    /// itself, the log beside it, or the journal beside it.
    ///
    /// Nothing where the harness holds no such file, which is what
    /// `file exists` reads. Writing the database out costs O(n) in its
    /// pages.
    fn sized(&self, name: &str) -> Option<usize> {
        if let Some(writer) = self.held.get(name) {
            return Some(writer.written().len());
        }
        let (base, tail) = name.rsplit_once('-')?;
        let writer = self.held.get(base)?;
        match tail {
            "wal" => writer.log().map(<[u8]>::len),
            "journal" => writer.journal().map(<[u8]>::len),
            _ => None,
        }
    }

    /// `hexio_read FILENAME OFFSET AMT`: `amt` bytes of the file from
    /// `offset`, as the capital hexadecimal digits `sqlite3TestBinToHex`
    /// writes.
    ///
    /// A read past the end of the file answers the bytes up to it.
    /// Reading costs O(n) in the bytes it answers.
    fn read_bytes(&self, name: &str, offset: &str, amount: &str) -> Result<Vec<String>, String> {
        let bytes = self
            .bytes_of(name)
            .ok_or_else(|| format!("cannot open input file {name}"))?;
        let at = number_of(offset)?;
        let wanted = number_of(amount)?;
        let end = at.saturating_add(wanted).min(bytes.len());
        let mut out = String::with_capacity(end.saturating_sub(at).saturating_mul(2));
        for byte in bytes.get(at..end).unwrap_or_default() {
            for half in [byte >> 4, byte & 0xf] {
                out.push(char::from_digit(u32::from(half), 16).unwrap_or('0'));
            }
        }
        Ok(vec![out.to_uppercase()])
    }

    /// `hexio_write FILENAME OFFSET HEXDATA`: the bytes those digits
    /// name, written into the file from `offset`, and how many of them
    /// were written.
    ///
    /// The file is read again from the bytes the write left, so a write
    /// that leaves a file this engine cannot read is refused. Writing
    /// costs O(n) in the bytes of the file.
    fn write_bytes(&mut self, name: &str, offset: &str, data: &str) -> Result<Vec<String>, String> {
        let mut bytes = self
            .bytes_of(name)
            .ok_or_else(|| format!("cannot open output file {name}"))?;
        let at = number_of(offset)?;
        let written = binary(data);
        for (slot, byte) in bytes.iter_mut().skip(at).zip(written.iter()) {
            *slot = *byte;
        }
        let mut writer = Writer::opened(&bytes).map_err(|error| refusal(&error))?;
        writer.defines(DEFINED);
        writer.groups(GROUPED);
        self.held.insert(name.to_owned(), writer);
        Ok(vec![written.len().to_string()])
    }

    /// `sqlite3_table_column_metadata`: what the schema says about one
    /// column — the type it was declared with, the collation it compares
    /// under, whether it may be nothing, whether it stands in the
    /// primary key, and whether that key counts up.
    ///
    /// `rowid`, `oid` and `_rowid_` name the key of a table that holds
    /// its rows under one, unless a column of the table carries that
    /// name. Reading the schema costs O(n) in its bytes.
    fn column_meta(&self, name: &str, table: &str, column: &str) -> Result<Vec<String>, String> {
        let missing = || format!("no such table column: {table}.{column}");
        let path = self
            .connections
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no such connection: {name}"))?;
        let writer = self
            .held
            .get(&path)
            .ok_or_else(|| format!("no such database: {path}"))?;
        let bytes = writer.written();
        let collating = self.collations.get(name).copied().unwrap_or_default();
        let database =
            Database::open_collating(&bytes, collating).map_err(|error| refusal(&error))?;
        let (held, _) = database.table(table.as_bytes()).ok_or_else(missing)?;
        let keyed = !held.without_rowid;
        let at = held
            .columns
            .iter()
            .position(|one| one.name.eq_ignore_ascii_case(column.as_bytes()));
        let found = at.and_then(|at| held.columns.get(at));
        let Some(found) = found else {
            // The key of a table that holds its rows under one answers
            // to three names, and every one of them is a column the
            // schema does not carry.
            let rowid = ["rowid", "oid", "_rowid_"]
                .iter()
                .any(|held| column.eq_ignore_ascii_case(held));
            if !rowid || !keyed {
                return Err(missing());
            }
            return Ok(vec![
                "INTEGER".to_owned(),
                "BINARY".to_owned(),
                "0".to_owned(),
                "1".to_owned(),
                usize::from(held.autoincrement).to_string(),
            ]);
        };
        let alias = held.rowid_alias.is_some() && held.rowid_alias == at;
        Ok(vec![
            String::from_utf8_lossy(&found.declared).into_owned(),
            collation_named(found.collation),
            usize::from(found.not_null).to_string(),
            usize::from(found.key != 0 || alias).to_string(),
            usize::from(alias && held.autoincrement).to_string(),
        ])
    }

    /// `md5file`: the digest of the file the harness holds under `name`.
    ///
    /// Reading it costs O(n) in the bytes of the file.
    fn digest_of(&self, name: &str) -> Result<Vec<String>, String> {
        let bytes = self
            .bytes_of(name)
            .ok_or_else(|| format!("cannot open input file {name}"))?;
        Ok(vec![crate::md5::digest(&bytes)])
    }

    /// The bytes the harness holds under `name`: the database itself,
    /// the log beside it, or the journal beside it.
    ///
    /// Writing the database out costs O(n) in its pages.
    fn bytes_of(&self, name: &str) -> Option<Vec<u8>> {
        if let Some(writer) = self.held.get(name) {
            return Some(writer.written());
        }
        let (base, tail) = name.rsplit_once('-')?;
        let writer = self.held.get(base)?;
        match tail {
            "wal" => writer.log().map(<[u8]>::to_vec),
            "journal" => writer.journal().map(<[u8]>::to_vec),
            _ => None,
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
        let under = configured();
        if !self.held.contains_key(path)
            && let Ok(mut writer) = Writer::new(under.page, 0, under.encoding)
        {
            writer.defines(DEFINED);
            writer.groups(GROUPED);
            ticked(&mut writer, self.clock);
            if let Some(journal) = under.journal {
                let mut sql = b"PRAGMA journal_mode=".to_vec();
                sql.extend_from_slice(journal);
                let _ = writer.run(&sql);
            }
            self.held.insert(path.to_owned(), writer);
        }
        self.connections.insert(name.to_owned(), path.to_owned());
        self.nulls.insert(name.to_owned(), String::new());
        // A connection that is opened again counts from nought and was
        // told no pragma, which is what `sqlite3 db test.db` in a file
        // relies on.
        self.counters.insert(name.to_owned(), Counted::default());
        self.pragmas.insert(name.to_owned(), Kept::default());
        // `sqlite3_create_collation` holds a collation on one
        // connection, so a connection that opens again defines none.
        self.collations.remove(name);
        self.functions.remove(name);
    }

    /// Keeps a collation the tester defined under its name, leaking the
    /// name and the list so that both outlive the file.
    ///
    /// A file defines a handful of collations, so leaking one list per
    /// definition costs O(n^2) bytes over n definitions and nothing
    /// that matters.
    fn collates(&mut self, connection: &str, name: &str) {
        let held = self.collations.entry(connection.to_owned()).or_default();
        let mut collating: Vec<Collating> = held
            .iter()
            .filter(|one| one.name != name.as_bytes())
            .copied()
            .collect();
        collating.push(Collating {
            name: Box::leak(name.as_bytes().to_vec().into_boxed_slice()),
            by: asked,
        });
        *held = Box::leak(collating.into_boxed_slice());
    }

    /// Keeps a function the tester defined under its name, beside the
    /// ones this harness holds, leaking the name and the list so that
    /// both outlive the file.
    ///
    /// The function takes any number of arguments, which `db function`
    /// of `testfixture` registers as `nArg` at -1.
    fn functions(&mut self, connection: &str, name: &str) {
        let held = self.functions.entry(connection.to_owned()).or_default();
        let mut defined: Vec<Defined> = held
            .iter()
            .filter(|one| one.name != name.as_bytes())
            .copied()
            .collect();
        if defined.is_empty() {
            defined.extend_from_slice(DEFINED);
        }
        defined.push(Defined {
            name: Box::leak(name.as_bytes().to_vec().into_boxed_slice()),
            count: None,
            answer: called,
        });
        *held = Box::leak(defined.into_boxed_slice());
    }

    /// `sqlite_current_time` of `test1.c`: the moment `now` names, as
    /// the seconds since 1970, and nought for the clock of the machine,
    /// which this harness has none of.
    fn ticks(&mut self, written: &str) -> Result<Vec<String>, String> {
        self.clock = written
            .parse::<i64>()
            .map_err(|source| format!("a moment is a whole number: {source}"))
            .map(|seconds| (seconds != 0).then_some(seconds))?;
        let held = self.clock;
        for writer in self.held.values_mut() {
            ticked(writer, held);
        }
        Ok(Vec::new())
    }

    /// The requests of a statement the tester prepared.
    fn of_statement(&mut self, verb: &str, args: &[String]) -> Result<Vec<String>, String> {
        let first = args.first().map_or("", String::as_str);
        let second = args.get(1).map_or("", String::as_str);
        let third = args.get(2).map_or("", String::as_str);
        match verb {
            "prepare" => Ok(self.prepare(first, second, third == "1")),
            "autocommit" => self.autocommit(first),
            "step" => self.step(first),
            "finalize" => Ok(self.finalize(first)),
            "reset" => Ok(self.reset_statement(first, false)),
            "clear_binds" => Ok(self.reset_statement(first, true)),
            "bind" => self.bind(first, second, third),
            "column" => self.column(first, second, third),
            "next_stmt" => Ok(self.next_statement(first, second)),
            "readonly" | "busy" | "isexplain" => Ok(alloc_one(&self.statement_is(verb, first))),
            _ => self.stmt_text(first, second, third),
        }
    }

    /// `sqlite3_prepare DB SQL`: the first statement of the text is
    /// held under a name of its own, and the text after it is the tail.
    ///
    /// A statement that reads is run once with every parameter left
    /// unbound, so that the names of its columns are known before the
    /// first step and a statement the engine refuses is refused here.
    fn prepare(&mut self, connection: &str, sql: &str, legacy: bool) -> Vec<String> {
        // `sqlite3Prepare` reads past the semicolons and the comments no
        // statement stands in, so the first statement of a text is the
        // first one that carries meaning and the tail begins after the
        // semicolon that ends it.
        let mut at: usize = 0;
        let mut text = String::new();
        for piece in statements(sql) {
            at = at.saturating_add(piece.len()).saturating_add(1);
            if !db_sqlite::parse::blank(piece.as_bytes()) {
                piece.clone_into(&mut text);
                break;
            }
        }
        let tail = sql.get(at.min(sql.len())..).unwrap_or("").to_owned();
        let mut prepared = Prepared {
            connection: connection.to_owned(),
            sql: text.clone(),
            bound: BTreeMap::new(),
            names: Vec::new(),
            declared: Vec::new(),
            rows: Vec::new(),
            at: 0,
            ran: false,
            row: false,
            legacy: false,
        };
        prepared.legacy = legacy;
        // A text that holds no statement answers no statement at all,
        // which is the null pointer `sqlite3_prepare` writes for it.
        if text.is_empty() {
            return vec![String::new(), tail, String::new()];
        }
        if pragmas(&text) {
            prepared.names = pragma_columns(&text);
            prepared.declared = prepared.names.iter().map(|_| String::new()).collect();
        } else if reads(&text) {
            let read = self.read_statement(connection, &bound_into(&text, &BTreeMap::new()));
            match read {
                Ok(answered) => {
                    prepared.names = texts(&answered.names);
                    prepared.declared = texts(&answered.declared);
                }
                Err(message) => return vec![String::new(), tail, message],
            }
        }
        self.prepared = self.prepared.saturating_add(1);
        // The name stands for the pointer the C library answers, which
        // a file may read as a run of hexadecimal digits.
        let name = format!("{:08X}", self.prepared.saturating_add(0x1000_0000));
        self.statements.insert(name.clone(), prepared);
        vec![name, tail, String::new()]
    }

    /// What one statement is: whether it writes nothing, whether it has
    /// stepped onto a row it has not left, and whether it is an
    /// `EXPLAIN`.
    ///
    /// A name no statement stands under is the null pointer, which
    /// `sqlite3_stmt_readonly` answers one for and the two beside it
    /// nought.
    fn statement_is(&self, verb: &str, name: &str) -> String {
        let Some(prepared) = self.statements.get(name) else {
            return usize::from(verb == "readonly").to_string();
        };
        let answered = match verb {
            "busy" => i64::from(prepared.ran && prepared.row),
            "isexplain" => explaining(&prepared.sql),
            _ => i64::from(readonly(&prepared.sql)),
        };
        answered.to_string()
    }

    /// `sqlite3_next_stmt DB STMT`: the name of the statement of
    /// `connection` that stands after `after`, and the first one where
    /// `after` is `0`, which is how a file reads every statement a
    /// connection holds. Nothing stands after the last one.
    ///
    /// The names are read in the order they were made, which is the
    /// order of the map, so a walk over them costs O(n log n) in the
    /// statements the session holds.
    fn next_statement(&self, connection: &str, after: &str) -> Vec<String> {
        let held: Vec<String> = self
            .statements
            .iter()
            .filter(|(_, prepared)| prepared.connection == connection)
            .map(|(name, _)| name.clone())
            .collect();
        alloc_one(&after_name(&held, after))
    }

    /// The names a statement of `connection` may read, which are the
    /// tables of its schema and their columns.
    fn named(&self, connection: &str) -> Vec<Vec<u8>> {
        let Some(path) = self.connections.get(connection) else {
            return Vec::new();
        };
        let Some(writer) = self.held.get(path) else {
            return Vec::new();
        };
        let bytes = writer.written();
        let Ok(database) = Database::open(&bytes) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for table in database.tables() {
            out.push(table.name.clone());
            for column in &table.columns {
                out.push(column.name.clone());
            }
        }
        out
    }

    /// `sqlite3_get_autocommit DB`: nought where the connection has a
    /// transaction open and one where it has none.
    fn autocommit(&self, connection: &str) -> Result<Vec<String>, String> {
        let path = self
            .connections
            .get(connection)
            .cloned()
            .ok_or_else(|| format!("no such connection: {connection}"))?;
        let writer = self
            .held
            .get(&path)
            .ok_or_else(|| format!("no such database: {path}"))?;
        Ok(alloc_one(if writer.began() { "0" } else { "1" }))
    }

    /// What one statement of `connection` answers, with the names of its
    /// columns.
    fn read_statement(&self, connection: &str, sql: &str) -> Result<db_sqlite::db::Answer, String> {
        let path = self
            .connections
            .get(connection)
            .cloned()
            .ok_or_else(|| format!("no such connection: {connection}"))?;
        let collating = self.collations.get(connection).copied().unwrap_or_default();
        let defines = self.defines(connection);
        let writer = self
            .held
            .get(&path)
            .ok_or_else(|| format!("no such database: {path}"))?;
        answered_rows(writer, sql, collating, defines)
    }

    /// `sqlite3_step STMT`: the code the step answers, which is
    /// `SQLITE_ROW` where the statement stands on a row and
    /// `SQLITE_DONE` where it has none left.
    fn step(&mut self, name: &str) -> Result<Vec<String>, String> {
        let held = self
            .statements
            .get(name)
            .ok_or_else(|| format!("no such statement: {name}"))?;
        if held.ran {
            if let Some(held) = self.statements.get_mut(name) {
                // `sqlite3_step` past the last row resets the statement
                // and answers its first row again, which is the
                // `SQLITE_OMIT_AUTORESET` the library is built without.
                if held.row {
                    held.at = held.at.saturating_add(1);
                } else {
                    held.at = 0;
                }
                held.row = held.at < held.rows.len();
            }
        } else {
            self.first_step(name)?;
        }
        let held = self
            .statements
            .get(name)
            .ok_or_else(|| format!("no such statement: {name}"))?;
        Ok(alloc_two(
            if held.row {
                String::from("SQLITE_ROW")
            } else {
                String::from("SQLITE_DONE")
            },
            if held.legacy {
                String::from("1")
            } else {
                String::new()
            },
        ))
    }

    /// The first step of one statement, which runs it and holds the rows
    /// it answered.
    fn first_step(&mut self, name: &str) -> Result<(), String> {
        let held = self
            .statements
            .get(name)
            .ok_or_else(|| format!("no such statement: {name}"))?;
        let (connection, sql) = (held.connection.clone(), held.sql.clone());
        let text = bound_into(&sql, &held.bound);
        let answered = if reads(&sql) {
            self.read_statement(&connection, &text)
        } else {
            self.ran_one(&connection, &text)
        };
        let answered = match answered {
            Ok(answered) => answered,
            Err(message) => {
                if let Some(held) = self.statements.get_mut(name) {
                    held.ran = true;
                    held.row = false;
                }
                return Err(message);
            }
        };
        if let Some(held) = self.statements.get_mut(name) {
            held.ran = true;
            held.at = 0;
            held.rows.clone_from(&answered.rows);
            if !answered.names.is_empty() {
                held.names = texts(&answered.names);
                held.declared = texts(&answered.declared);
            }
            held.row = !held.rows.is_empty();
        }
        Ok(())
    }

    /// One statement of `connection` run through its writer, which
    /// answers no row.
    fn ran_one(&mut self, connection: &str, sql: &str) -> Result<db_sqlite::db::Answer, String> {
        let path = self
            .connections
            .get(connection)
            .cloned()
            .ok_or_else(|| format!("no such connection: {connection}"))?;
        let collating = self.collations.get(connection).copied().unwrap_or_default();
        let defines = self.defines(connection);
        let writer = self
            .held
            .get_mut(&path)
            .ok_or_else(|| format!("no such database: {path}"))?;
        writer.collates(collating);
        writer.defines(defines);
        let ran = writer
            .run(sql.as_bytes())
            .map(|_| db_sqlite::db::Answer::default())
            .map_err(|error| shape(sql, refusal(&error)));
        let counted = writer.counts();
        self.counters.insert(connection.to_owned(), counted);
        ran
    }

    /// `sqlite3_finalize STMT`: the statement is let go, and the code it
    /// answers is `SQLITE_OK`.
    fn finalize(&mut self, name: &str) -> Vec<String> {
        self.statements.remove(name);
        alloc_one("SQLITE_OK")
    }

    /// `sqlite3_reset STMT` and `sqlite3_clear_bindings STMT`: the
    /// statement stands before its first step again, and the second
    /// drops what was bound.
    fn reset_statement(&mut self, name: &str, clearing: bool) -> Vec<String> {
        if let Some(held) = self.statements.get_mut(name) {
            held.ran = false;
            held.row = false;
            held.at = 0;
            held.rows.clear();
            if clearing {
                held.bound.clear();
            }
        }
        alloc_one("SQLITE_OK")
    }

    /// `sqlite3_bind_* STMT N VALUE`: the value the parameter at `at`
    /// stands for, written as a literal.
    fn bind(&mut self, name: &str, at: &str, value: &str) -> Result<Vec<String>, String> {
        let place: usize = at
            .parse()
            .map_err(|source| format!("a place is a whole number: {source}"))?;
        if let Some(held) = self.statements.get_mut(name) {
            held.bound.insert(place, value.to_owned());
        }
        Ok(alloc_one("SQLITE_OK"))
    }

    /// `sqlite3_column_* STMT N` and `sqlite3_column_count STMT`, which
    /// `which` names.
    fn column(&self, name: &str, which: &str, at: &str) -> Result<Vec<String>, String> {
        let held = self
            .statements
            .get(name)
            .ok_or_else(|| format!("no such statement: {name}"))?;
        if which == "count" {
            return Ok(alloc_one(&held.names.len().to_string()));
        }
        if which == "data" {
            let count = if held.row { held.names.len() } else { 0 };
            return Ok(alloc_one(&count.to_string()));
        }
        let place: usize = at.parse().unwrap_or(usize::MAX);
        if which == "name" {
            let named = held.names.get(place).cloned().unwrap_or_default();
            return Ok(alloc_one(&named));
        }
        if which == "decltype" {
            let named = held.declared.get(place).cloned().unwrap_or_default();
            return Ok(alloc_one(&named));
        }
        let value = held
            .rows
            .get(held.at)
            .and_then(|row| row.get(place))
            .cloned()
            .unwrap_or(Value::Null);
        let text = match which {
            "type" => type_word(&value).to_owned(),
            "int" => value.to_integer().to_string(),
            "double" => listed(&db_sqlite::value::Value::Real(real_of(&value)), ""),
            _ => listed(&value, ""),
        };
        Ok(alloc_one(&text))
    }

    /// `sqlite3_sql STMT` and `sqlite3_expanded_sql STMT`: the text the
    /// statement was prepared from, and the same with what was bound
    /// written into it.
    fn stmt_text(&self, name: &str, which: &str, at: &str) -> Result<Vec<String>, String> {
        let held = self
            .statements
            .get(name)
            .ok_or_else(|| format!("no such statement: {name}"))?;
        let text = match which {
            "expanded" => bound_into(&held.sql, &held.bound),
            "binds" => count_binds(&held.sql).to_string(),
            // `sqlite3_bind_parameter_name` answers the name of the
            // parameter at a place, which a `?` carries none of, and
            // `sqlite3_bind_parameter_index` the place of a name.
            "parameter" => {
                let place: usize = at.parse().unwrap_or(0);
                named_parameters(&held.sql)
                    .get(place.saturating_sub(1))
                    .cloned()
                    .unwrap_or_default()
            }
            // `sqlite3_normalized_sql` writes the statement again with
            // every literal as a `?`.
            "normalized" => {
                let names = self.named(&held.connection);
                String::from_utf8_lossy(&db_sqlite::normalize::normalized_sql(
                    held.sql.as_bytes(),
                    &names,
                ))
                .into_owned()
            }
            "index" => named_parameters(&held.sql)
                .iter()
                .position(|name| name == at)
                .map_or(0, |place| place.saturating_add(1))
                .to_string(),
            _ => held.sql.clone(),
        };
        Ok(alloc_one(&text))
    }

    /// `sqlite3_set_authorizer`: the tester names a proc for one
    /// connection, or nothing to tell it no authorizer at all.
    fn authorizes(&mut self, connection: &str, name: &str) {
        if name.is_empty() {
            self.authorizers.remove(connection);
        } else {
            self.authorizers.insert(connection.to_owned(), true);
        }
    }

    /// The functions one connection reads, which are the ones this
    /// harness holds where the tester defined none.
    fn defines(&self, connection: &str) -> &'static [Defined] {
        self.functions.get(connection).copied().unwrap_or(DEFINED)
    }

    /// One database written again under another path, which is what a
    /// file that saves itself and reads the save back asks for.
    fn copy(&mut self, from: &str, to: &str) -> Result<Vec<String>, String> {
        let Some(held) = self.held.get(from) else {
            self.held.remove(to);
            return Ok(Vec::new());
        };
        let bytes = held.written();
        let mut writer = Writer::opened(&bytes).map_err(|error| refusal(&error))?;
        writer.defines(DEFINED);
        writer.groups(GROUPED);
        self.held.insert(to.to_owned(), writer);
        Ok(Vec::new())
    }

    /// The statements of one text, in order, answered as one list.
    fn eval(&mut self, name: &str, sql: &str) -> Result<Vec<String>, String> {
        if may_attach(sql) {
            self.telling_files();
        }
        let null = self.nulls.get(name).cloned().unwrap_or_default();
        let path = self
            .connections
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no such connection: {name}"))?;
        let counted = self.counters.get(name).copied().unwrap_or_default();
        let kept = self.pragmas.get(name).cloned().unwrap_or_default();
        let collating = self.collations.get(name).copied().unwrap_or_default();
        let defines = self.defines(name);
        let asks = self.authorizers.contains_key(name);
        let writer = self
            .held
            .get_mut(&path)
            .ok_or_else(|| format!("no such database: {path}"))?;
        // The counters belong to the connection and the pages to the
        // file, so the writer stands at this connection's counters for
        // the statements of this request and answers them back.
        writer.counts_as(counted);
        writer.kept_as(kept);
        writer.collates(collating);
        writer.defines(defines);
        if asks {
            writer.asks(asking);
        } else {
            writer.asks_nothing();
        }
        WHO.with(|who| who.borrow_mut().clone_from(&name.to_owned()));
        NULLED.with(|text| text.borrow_mut().clone_from(&null));
        writer.opens(opening);
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
            match run_one(writer, text, collating, defines) {
                Ok(values) => out.extend(values.iter().map(|value| listed(value, &null))),
                Err(message) => {
                    ran = Err(message);
                    break;
                }
            }
        }
        let counted = writer.counts();
        let kept = writer.kept();
        let files = writer.attached_files();
        self.counters.insert(name.to_owned(), counted);
        self.pragmas.insert(name.to_owned(), kept);
        self.mirror(&path, files);
        ran?;
        Ok(out)
    }

    /// The image of every attached database written back into the file
    /// the session holds under its name, so a connection over that path
    /// reads what the statement wrote, except for `held`, which is the
    /// path the connection itself reads.
    ///
    /// Reading one file again costs O(n) in its pages.
    fn mirror(&mut self, held: &str, files: Vec<(Vec<u8>, Vec<u8>)>) {
        for (file, bytes) in files {
            let path = String::from_utf8_lossy(&file).into_owned();
            // A connection that attached its own file writes both
            // through the one writer the session holds for that path,
            // so writing the copy back would take away what the
            // connection holds.
            if path == held {
                continue;
            }
            let Ok(mut writer) = Writer::opened(&bytes) else {
                continue;
            };
            writer.defines(DEFINED);
            writer.groups(GROUPED);
            self.held.insert(path, writer);
        }
    }

    /// The files the session holds, told to the opening function every
    /// connection reads an `ATTACH` out of.
    ///
    /// Writing them out costs O(n) in the pages of every file, so the
    /// session tells them only where a text may attach one.
    fn telling_files(&self) {
        FILES.with(|files| {
            let mut held = files.borrow_mut();
            held.clear();
            for (path, writer) in &self.held {
                held.insert(path.as_bytes().to_vec(), writer.written());
            }
        });
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
        // The column names are the ones the last statement that answers
        // rows carries, which is what `exec_printf_cb` writes for the
        // first row it is given whatever stands after that statement.
        let Some(last) = statements(sql)
            .into_iter()
            .map(str::trim)
            .rfind(|text| reads(text) || pragmas(text))
        else {
            return Ok(Vec::new());
        };
        if pragmas(last) {
            return Ok(pragma_columns(last));
        }
        let collating = self.collations.get(name).copied().unwrap_or_default();
        let defines = self.defines(name);
        let beside = attached_images(writer);
        let bytes = writer.written();
        // A connection in write-ahead logging holds its newest pages in
        // the log, so the schema this reads is the one the log carries.
        let log = writer.log().map(db_sqlite::wal::Wal::open);
        let opened = match &log {
            Some(Ok(log)) => Database::open_log_collating(&bytes, log, collating),
            Some(Err(error)) => return Err(format!("{error}")),
            None => Database::open_collating(&bytes, collating),
        };
        let held = self.clock;
        let database = opened
            .map(|database| {
                let database = database
                    .naming(writer.naming())
                    .defining(defines)
                    .grouping(GROUPED)
                    .sensitively(writer.sensitive())
                    .journalling(writer.journalled());
                match held {
                    Some(seconds) => database.clocked(seconds),
                    None => database,
                }
            })
            .and_then(|database| attaching(database, &beside))
            .map_err(|error| refusal(&error))?;
        let answered = database
            .query(last.as_bytes())
            .map_err(|error| refusal(&error))?;
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
                self.repeated = (String::new(), 0);
                say("C passed");
            }
            "refused" => {
                self.score.refused = self.score.refused.saturating_add(1);
                say("C refused");
                let why = args.get(2).map_or("", String::as_str);
                say(&format!("W {why}"));
                if self.repeated.0 == why {
                    self.repeated.1 = self.repeated.1.saturating_add(1);
                } else {
                    self.repeated = (why.to_owned(), 1);
                }
            }
            _ => {
                self.score.failed = self.score.failed.saturating_add(1);
                self.repeated = (String::new(), 0);
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

/// The line to the tester while one file runs, which a collation the
/// tester defined writes a `CALL` onto.
struct Line {
    /// What the requests of the tester are read from.
    reader: BufReader<TcpStream>,
    /// What answers and calls are written onto.
    writer: TcpStream,
}

thread_local! {
    /// The line of the run on this thread, which `asked` reaches
    /// because `Comparing` is a bare function and carries nothing.
    static LINE: RefCell<Option<Line>> = const { RefCell::new(None) };

    /// The connection whose statement is running, which `asking` names
    /// in the call it writes because the function carries nothing.
    static WHO: RefCell<String> = const { RefCell::new(String::new()) };

    /// The text a NULL argument of a function call is written as, which
    /// is the null value of the connection whose statement is running,
    /// because `called` carries nothing.
    static NULLED: RefCell<String> = const { RefCell::new(String::new()) };

    /// The result code of the last statement this thread refused, which
    /// `sqlite3_errcode` and `sqlite3_extended_errcode` answer.
    static CODE: RefCell<(String, String, i64)> =
        const { RefCell::new((String::new(), String::new(), 0)) };

    /// The files the session holds, which `opening` answers an `ATTACH`
    /// out of because the function carries nothing.
    static FILES: RefCell<BTreeMap<Vec<u8>, Vec<u8>>> = const { RefCell::new(BTreeMap::new()) };
}

/// What one refusal says, with the code it carries kept for
/// `sqlite3_errcode`.
fn refusal(error: &db_sqlite::db::Error) -> String {
    let code = error.code();
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    CODE.with(|held| {
        *held.borrow_mut() = (text(code.name), text(code.extended_name), code.number);
    });
    error.message()
}

/// The code of the last refusal, as the name of the primary code, the
/// name of the extended one or the number of the primary one, which is
/// `SQLITE_OK` where the statements of this run all stood.
fn last_code(which: &str) -> String {
    CODE.with(|held| {
        let held = held.borrow();
        let name = if which == "extended" {
            &held.1
        } else {
            &held.0
        };
        if name.is_empty() {
            return if which == "number" {
                String::from("0")
            } else {
                String::from("SQLITE_OK")
            };
        }
        if which == "number" {
            return held.2.to_string();
        }
        name.clone()
    })
}

/// What the authorizer the tester defined answers, which is one `CALL`
/// onto the line and the `RET` that answers it.
///
/// `sqlite3_set_authorizer` hands the action to the application, so the
/// tester's own proc answers, as one of three words. A word it does not
/// know is a denial, which is what `tclsqlite.c` reads for it.
fn asking(asked: &db_sqlite::auth::Asked<'_>) -> db_sqlite::auth::Answer {
    use db_sqlite::auth::Answer;
    LINE.with(|line| {
        let mut held = line.borrow_mut();
        let Some(line) = held.as_mut() else {
            return Answer::Ok;
        };
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let values = [
            WHO.with(|who| who.borrow().clone()),
            text(asked.action.word()),
            text(asked.first),
            text(asked.second),
            text(asked.schema),
            text(asked.inner),
        ];
        if write_call(&mut line.writer, "auth", &values).is_err() {
            return Answer::Ok;
        }
        match returned(&mut line.reader).as_deref().map(str::trim) {
            Some("SQLITE_OK") => Answer::Ok,
            Some("SQLITE_IGNORE") => Answer::Ignore,
            _ => Answer::Deny,
        }
    })
}

/// The order of two values under a collation the tester defined, which
/// is one `CALL` onto the line and the `RET` that answers it.
///
/// `sqlite3_create_collation` hands the comparison to the application,
/// so the tester's own proc answers. Costs one round trip per pair,
/// which makes a sort of n values cost O(n log n) round trips.
fn asked(name: &'static [u8], left: &[u8], right: &[u8]) -> Ordering {
    LINE.with(|line| {
        let mut held = line.borrow_mut();
        let Some(line) = held.as_mut() else {
            return Ordering::Equal;
        };
        let values = [
            String::from_utf8_lossy(name).into_owned(),
            String::from_utf8_lossy(left).into_owned(),
            String::from_utf8_lossy(right).into_owned(),
        ];
        if write_call(&mut line.writer, "collate", &values).is_err() {
            return Ordering::Equal;
        }
        let answered = returned(&mut line.reader)
            .and_then(|text| text.trim().parse::<i64>().ok())
            .unwrap_or(0);
        answered.cmp(&0)
    })
}

/// Asks the tester to run a proc, written as `CALL`, what kind of proc
/// it is, and its values.
fn write_call(stream: &mut TcpStream, kind: &str, values: &[String]) -> Result<(), Error> {
    let mut out = format!("CALL {kind} {}\n", values.len()).into_bytes();
    for value in values {
        out.extend_from_slice(format!("{}\n", value.len()).as_bytes());
        out.extend_from_slice(value.as_bytes());
        out.push(b'\n');
    }
    stream
        .write_all(&out)
        .map_err(|source| Error::io("writing a call", source))
}

/// What the tester's proc answered, read as `RET` and its values.
fn returned_values(reader: &mut BufReader<TcpStream>) -> Option<Vec<String>> {
    let head = line(reader).ok()??;
    let mut words = head.split_whitespace();
    if words.next() != Some("RET") {
        return None;
    }
    let count: usize = words.next()?.trim().parse().ok()?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let length: usize = line(reader).ok()??.trim().parse().ok()?;
        let mut bytes = vec![0_u8; length];
        reader.read_exact(&mut bytes).ok()?;
        line(reader).ok()?;
        out.push(String::from_utf8_lossy(&bytes).into_owned());
    }
    Some(out)
}

/// The first value the tester's proc answered.
fn returned(reader: &mut BufReader<TcpStream>) -> Option<String> {
    returned_values(reader)?.into_iter().next()
}

/// What a function the tester defined answered: the kind of value the
/// result stands for and its text, which `tclSqlFunc` of
/// `research/sqlite/src/tclsqlite.c:1256` reads the Tcl type of the
/// result and the declared return type for.
pub(crate) fn valued(values: &[String]) -> Value {
    let [kind, text] = values else {
        return Value::Null;
    };
    match kind.as_str() {
        "int" => text.trim().parse::<i64>().map_or(Value::Null, Value::Int),
        "real" => text.trim().parse::<f64>().map_or(Value::Null, Value::Real),
        "blob" => Value::Blob(text.as_bytes().to_vec()),
        "text" => Value::Text(text.as_bytes().to_vec()),
        _ => Value::Null,
    }
}

/// What a function the tester defined answers, which is one `CALL` onto
/// the line and the `RET` that answers it.
///
/// `sqlite3_create_function` hands the call to the application, so the
/// tester's own proc answers, as text of one value. Costs one round
/// trip per call.
fn called(
    name: &'static [u8],
    args: &[Value],
    _random: Option<&Source>,
) -> Result<Value, db_sqlite::eval::Error> {
    LINE.with(|line| {
        let mut held = line.borrow_mut();
        let Some(line) = held.as_mut() else {
            return Ok(Value::Null);
        };
        let null = NULLED.with(|text| text.borrow().clone());
        let mut values = vec![String::from_utf8_lossy(name).into_owned()];
        values.extend(args.iter().map(|value| listed(value, &null)));
        if write_call(&mut line.writer, "function", &values).is_err() {
            return Ok(Value::Null);
        }
        Ok(returned_values(&mut line.reader).map_or(Value::Null, |values| valued(&values)))
    })
}

/// One line to the process that started this run, written as it
/// happens because that process may end this one at the deadline.
fn say(line: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// The functions `testfixture` defines that this harness answers,
/// which SQLite's own files call in their statements.
static DEFINED: &[Defined] = &[Defined {
    name: b"randstr",
    count: Some(2),
    answer: randstr,
}];

/// The aggregates `testfixture` defines that this harness answers.
static GROUPED: &[db_sqlite::func::Grouped] = &[db_sqlite::func::Grouped {
    name: b"md5sum",
    count: None,
    answer: md5sum,
}];

/// `md5sum(X,...)` of `src/test_md5.c`: the digest of the text of every
/// argument of every row of the group, in the order the rows were
/// stepped, with a null argument writing nothing.
///
/// It costs O(n) in the bytes it digests.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the shape every aggregate the application defines answers in"
)]
fn md5sum(_: &'static [u8], rows: &[Vec<Value>]) -> Result<Value, db_sqlite::eval::Error> {
    let mut held = Vec::new();
    for row in rows {
        for value in row {
            held.extend_from_slice(&value.text().unwrap_or_default());
        }
    }
    Ok(Value::Text(
        crate::md5::digest(&held).to_lowercase().into_bytes(),
    ))
}

/// The letters, digits and marks `randStr` of `src/test_func.c` draws
/// its bytes from.
const LETTERS: &[u8] =
    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-!,:*^+=_|?/<> ";

/// The most bytes `randStr` answers, which is the buffer it writes
/// into, one byte shorter than the thousand it holds.
const LONGEST: i64 = 999;

/// `randstr(N,M)` of `src/test_func.c`: a string of between N and M
/// bytes, each drawn from [`LETTERS`], held to [`LONGEST`] bytes.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the shape every function the application defines answers in"
)]
fn randstr(
    _name: &'static [u8],
    args: &[Value],
    random: Option<&Source>,
) -> Result<Value, db_sqlite::eval::Error> {
    let held = |value: Option<&Value>| value.map_or(0, Value::to_integer).clamp(0, LONGEST);
    let least = held(args.first());
    let most = held(args.get(1)).max(least);
    let mut count = least;
    if most > least {
        let drawn = random.map_or(0, |source| source.word() & 0x7fff_ffff);
        let range = u64::try_from(most.saturating_sub(least).saturating_add(1)).unwrap_or(1);
        let within = drawn.checked_rem(range).unwrap_or(0);
        count = count.saturating_add(i64::try_from(within).unwrap_or(0));
    }
    let bytes = random.map_or_else(Vec::new, |source| {
        source.bytes(usize::try_from(count).unwrap_or(0))
    });
    let text = bytes
        .iter()
        .map(|byte| {
            let at = usize::from(*byte).checked_rem(LETTERS.len()).unwrap_or(0);
            LETTERS.get(at).copied().unwrap_or(b'a')
        })
        .collect();
    Ok(Value::Text(text))
}

/// One statement: a reader answers one that reads and the writer
/// answers the rest, which is what a connection does with either.
fn run_one(
    writer: &mut Writer,
    text: &str,
    collating: &'static [Collating],
    defines: &'static [Defined],
) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    if reads(text) {
        let answered = answered_rows(writer, text, collating, defines)?;
        for row in &answered.rows {
            out.extend(row.iter().cloned());
        }
        return Ok(out);
    }
    let rows = writer
        .run(text.as_bytes())
        .map_err(|error| shape(text, refusal(&error)))?;
    for row in &rows {
        out.extend(row.iter().cloned());
    }
    Ok(out)
}

/// What one statement that reads answers: the names of its columns and
/// its rows, read over a database opened as the harness opens one.
///
/// # Errors
///
/// The message the engine refused the statement with.
fn answered_rows(
    writer: &Writer,
    text: &str,
    collating: &'static [Collating],
    defines: &'static [Defined],
) -> Result<db_sqlite::db::Answer, String> {
    {
        let beside = attached_images(writer);
        let bytes = writer.written();
        // A connection in write-ahead logging holds its newest pages in
        // the log, so a reader follows the log beside the file.
        let log = writer.log().map(db_sqlite::wal::Wal::open);
        let opened = match &log {
            Some(Ok(log)) => Database::open_log_collating(&bytes, log, collating),
            Some(Err(error)) => return Err(format!("{error}")),
            None => Database::open_collating(&bytes, collating),
        };
        let counted = writer.counts();
        let naming = writer.naming();
        let journalled = writer.journalled();
        let sensitive = writer.sensitive();
        let asks = writer.asking();
        let held = writer.clock();
        let answered = opened
            .map(|database| {
                let database = database
                    .counting(counted)
                    .naming(naming)
                    .defining(defines)
                    .grouping(GROUPED)
                    .sensitively(sensitive)
                    .journalling(journalled);
                let database = match asks {
                    Some(asking) => database.asked(asking),
                    None => database,
                };
                match held {
                    Some(seconds) => database.clocked(seconds),
                    None => database,
                }
            })
            .and_then(|database| attaching(database, &beside))
            .and_then(|database| database.query(text.as_bytes()))
            .map_err(|error| shape(text, refusal(&error)))?;
        Ok(answered)
    }
}

/// The image of the file an `ATTACH` names, out of what the session
/// holds.
///
/// A path the session holds no file under answers no bytes at all, which
/// the engine reads as a database of one page: `sqlite3OsOpen` makes the
/// file where it is not there.
fn opening(file: &[u8]) -> Option<Vec<u8>> {
    FILES.with(|files| Some(files.borrow().get(file).cloned().unwrap_or_default()))
}

/// Whether a text may attach a file, which is what the session tells the
/// files it holds for.
///
/// Reading the text costs O(n) in its bytes.
fn may_attach(sql: &str) -> bool {
    sql.as_bytes()
        .windows(6)
        .any(|window| window.eq_ignore_ascii_case(b"attach"))
}

/// The images of the databases an `ATTACH` added to a connection, each
/// with the name a statement names it by.
///
/// Building them costs O(n) in the pages of every attached database.
fn attached_images(writer: &Writer) -> Vec<(Vec<u8>, Vec<u8>)> {
    writer
        .attached_names()
        .into_iter()
        .filter_map(|name| writer.attached_written(&name).map(|bytes| (name, bytes)))
        .collect()
}

/// The same reader with every attached database beside it.
///
/// # Errors
///
/// What the engine refused one of the images for.
fn attaching<'a>(
    database: Database<'a>,
    beside: &'a [(Vec<u8>, Vec<u8>)],
) -> Result<Database<'a>, db_sqlite::db::Error> {
    let mut database = database;
    for (name, bytes) in beside {
        database = database.attaching(name, bytes)?;
    }
    Ok(database)
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

/// `btree_varint_test START MULTIPLIER COUNT INCREMENT`: `count`
/// values, the first `start * multiplier` and each `increment` past the
/// one before, written as a varint and read back.
///
/// The answer is empty where every value came back unchanged under the
/// same count of bytes, and names the first value that did not
/// otherwise. One value costs O(1), so the request costs O(n) in
/// `count`.
fn varint(args: &[String]) -> Result<Vec<String>, String> {
    let number = |at: usize| -> Result<u64, String> {
        args.get(at)
            .ok_or_else(|| format!("varint wants four numbers, not {}", args.len()))?
            .parse::<u64>()
            .map_err(|source| format!("varint reads numbers: {source}"))
    };
    let mut value = number(0)?.wrapping_mul(number(1)?);
    let count = number(2)?;
    let step = number(3)?;
    for _ in 0..count {
        let (read, written, taken) =
            db_sqlite::bytes::varint_again(value).map_err(|error| format!("{error}"))?;
        if read != value {
            return Err(format!("wrote {value} and read back {read}"));
        }
        if written != taken {
            return Err(format!("{value} wrote {written} bytes and read {taken}"));
        }
        value = value.wrapping_add(step);
    }
    Ok(Vec::new())
}

/// The `sqlite3_mprintf_*` commands of `src/test1.c`: the format of the
/// first argument read over the arguments after it, which is `printf`
/// of this engine.
///
/// Each argument after the format carries the C type the command hands
/// the format: `i` a 32-bit `int`, `l` a 64-bit one, `r` a double, `h`
/// the 16 hexadecimal digits of one, `s` a string, and `n` a null
/// pointer. The request costs O(n) in what the format writes.
fn mprintf(args: &[String]) -> Result<Vec<String>, String> {
    let format = args.first().map_or("", String::as_str);
    let mut values = vec![Value::Text(format.as_bytes().to_vec())];
    for text in args.iter().skip(1) {
        values.push(printed(text)?);
    }
    for (place, conversion) in conversions(format.as_bytes()) {
        let at = place.saturating_add(1);
        if !args.get(at).is_some_and(|text| text.starts_with('i')) {
            continue;
        }
        let Some(&Value::Int(number)) = values.get(at) else {
            continue;
        };
        // A conversion that reads an unsigned int reads the same 32 bits
        // without the sign, which is 2^32 past the signed number, and
        // `%c` writes the character the number names.
        let read = match conversion {
            b'x' | b'X' | b'o' | b'u' if number < 0 => Value::Int(number.saturating_add(1 << 32)),
            b'c' => Value::Text(character(number)),
            _ => continue,
        };
        values.splice(at..=at, [read]);
    }
    match db_sqlite::format::format(&values) {
        Ok(Value::Text(bytes)) => Ok(vec![String::from_utf8_lossy(&bytes).into_owned()]),
        Ok(other) => Err(format!("printf answered {other:?}")),
        Err(error) => Err(error.message()),
    }
}

/// One argument of [`mprintf`], read out of the kind its first byte
/// names.
///
/// Reading one costs O(n) in the text.
fn printed(text: &str) -> Result<Value, String> {
    let rest = text.get(1..).unwrap_or_default();
    let number = |radix: u32| {
        i64::from_str_radix(rest, radix).map_err(|source| format!("printf reads numbers: {source}"))
    };
    match text.as_bytes().first() {
        // A C `int` is 32 bits wide, so the number the format reads is
        // what those 32 bits hold as a signed number.
        Some(b'i') => Ok(Value::Int(i64::from(narrowed(number(10)?)))),
        Some(b'l') => Ok(Value::Int(number(10)?)),
        Some(b'r') => rest
            .parse::<f64>()
            .map(Value::Real)
            .map_err(|source| format!("printf reads reals: {source}")),
        Some(b'h') => u64::from_str_radix(rest, 16)
            .map(|bits| Value::Real(f64::from_bits(bits)))
            .map_err(|source| format!("printf reads the bits of a real: {source}")),
        Some(b's') => Ok(Value::Text(rest.as_bytes().to_vec())),
        // A command that hands the format no string hands it a null
        // pointer, which `%s` writes nothing for and `%q` writes
        // `(NULL)` for.
        Some(b'n') => Ok(Value::Null),
        _ => Err(format!("an argument of printf names no kind: {text}")),
    }
}

/// Which argument of [`mprintf`] each conversion of the format reads,
/// counted from the first argument after the format, and the conversion
/// character.
///
/// `sqlite3_str_vappendf` reads `%x`, `%X`, `%o` and `%u` as an
/// `unsigned int`, `%c` as the character a number names, and every other
/// whole number as an `int`. Scanning the format costs O(n) in its
/// bytes.
fn conversions(format: &[u8]) -> Vec<(usize, u8)> {
    let mut places = Vec::new();
    let mut argument = 0_usize;
    let mut at = 0;
    while at < format.len() {
        if format.get(at) != Some(&b'%') {
            at = at.saturating_add(1);
            continue;
        }
        at = at.saturating_add(1);
        while format.get(at).is_some_and(|byte| b"-+ 0#,!".contains(byte)) {
            at = at.saturating_add(1);
        }
        at = counted_out(format, at, &mut argument);
        if format.get(at) == Some(&b'.') {
            at = counted_out(format, at.saturating_add(1), &mut argument);
        }
        while format.get(at) == Some(&b'l') {
            at = at.saturating_add(1);
        }
        match format.get(at) {
            None => break,
            // Two per cent signs write one and read no argument.
            Some(&b'%') => (),
            Some(&byte) => {
                places.push((argument, byte));
                argument = argument.saturating_add(1);
            }
        }
        at = at.saturating_add(1);
    }
    places
}

/// Where the width or precision of one conversion ends, counting the
/// argument it reads where it is `*`.
///
/// Scanning one costs O(n) in its digits.
fn counted_out(format: &[u8], at: usize, argument: &mut usize) -> usize {
    if format.get(at) == Some(&b'*') {
        *argument = argument.saturating_add(1);
        return at.saturating_add(1);
    }
    let mut end = at;
    while format.get(end).is_some_and(u8::is_ascii_digit) {
        end = end.saturating_add(1);
    }
    end
}

/// The character a number names, which `%c` of the C library writes.
///
/// A number no character names writes nothing, and one past the first
/// 128 writes the byte it holds, which is what `sqlite3_str_appendchar`
/// writes. Reading one costs O(1).
fn character(number: i64) -> Vec<u8> {
    u32::try_from(number)
        .ok()
        .and_then(char::from_u32)
        .map(|found| found.to_string().into_bytes())
        .unwrap_or_default()
}

/// What the low 32 bits of `number` hold as a signed number, which is
/// the C `int` the tester hands `sqlite3_mprintf`.
///
/// Reading it costs O(1).
fn narrowed(number: i64) -> i32 {
    let low = number.to_le_bytes();
    let mut bytes = [0_u8; 4];
    bytes.copy_from_slice(low.get(..4).unwrap_or(&[0; 4]));
    i32::from_le_bytes(bytes)
}

/// Writes out what the engine refused a statement for, keyed by the
/// first words of the statement, and answers the message the tester
/// reads, which `catchsql` compares against the C library's own.
fn shape(text: &str, message: String) -> String {
    say(&format!("S {} {message}", first_words(text)));
    message
}

/// The name a collation answers to, which
/// `sqlite3_table_column_metadata` writes in capitals for the three the
/// library holds.
fn collation_named(collation: db_sqlite::value::Collation) -> String {
    match collation {
        db_sqlite::value::Collation::NoCase => "NOCASE".to_owned(),
        db_sqlite::value::Collation::Rtrim => "RTRIM".to_owned(),
        db_sqlite::value::Collation::Defined(name, _) => String::from_utf8_lossy(name).into_owned(),
        _ => "BINARY".to_owned(),
    }
}

/// Tells a writer what the clock says, or leaves it told none.
///
/// Telling one costs O(1).
const fn ticked(writer: &mut Writer, clock: Option<i64>) {
    if let Some(seconds) = clock {
        writer.clocking(seconds);
    }
}

/// A whole number a request carries, which every request that names a
/// place in a file carries two of.
fn number_of(text: &str) -> Result<usize, String> {
    text.parse::<usize>()
        .map_err(|source| format!("a place in a file is a whole number: {source}"))
}

/// The bytes a run of hexadecimal digits names, which is
/// `sqlite3TestHexToBin`: a digit that is none ends the run.
///
/// Reading them costs O(n) in the digits.
fn binary(digits: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(digits.len() / 2);
    let mut held: Option<u8> = None;
    for digit in digits.chars() {
        let Some(value) = digit.to_digit(16) else {
            break;
        };
        let low = u8::try_from(value).unwrap_or(0);
        match held.take() {
            Some(high) => out.push((high << 4) | low),
            None => held = Some(low),
        }
    }
    out
}

/// The first line of what a file stopped at, cut to 72 characters,
/// which names the command the file wanted.
fn first_line(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();
    match line.char_indices().nth(72) {
        Some((at, _)) => line.get(..at).unwrap_or_default().to_owned(),
        None => line.to_owned(),
    }
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

/// The name that stands after `after` in `held`, and the first one where
/// `after` is `0` or nothing, which is what `sqlite3_next_stmt` answers.
/// Nothing stands after the last name, and nothing after a name `held`
/// does not carry.
///
/// Reading the names costs O(n) in their number.
pub(crate) fn after_name(held: &[String], after: &str) -> String {
    let at = match after {
        "" | "0" => Some(0),
        _ => held
            .iter()
            .position(|name| name == after)
            .map(|at| at.saturating_add(1)),
    };
    at.and_then(|at| held.get(at)).cloned().unwrap_or_default()
}

/// `sqlite3_normalize SQL`: the same statement in lower case, and
/// nothing for a byte no rule accepts.
fn normalized(sql: &str) -> String {
    db_sqlite::normalize::normalized(sql.as_bytes())
        .map(|held| String::from_utf8_lossy(&held).into_owned())
        .unwrap_or_default()
}

/// Whether the statement writes nothing of the database file, which
/// `sqlite3_stmt_readonly` answers.
///
/// A transaction statement writes nothing itself, which `BEGIN`,
/// `COMMIT`, `ROLLBACK`, `SAVEPOINT` and `RELEASE` are, and `BEGIN
/// IMMEDIATE` and `BEGIN EXCLUSIVE` take the file and are not. An
/// `ATTACH` and a `DETACH` change what the connection holds and not
/// what a file holds. A pragma that sets something writes, and one that
/// asks does not. An `EXPLAIN` answers what the statement it names
/// answers, which `sqlite3_stmt_readonly` reads past.
pub(crate) fn readonly(sql: &str) -> bool {
    let text = past_explain(sql);
    if reads(&text) {
        return true;
    }
    let words = words(&text);
    let first = words
        .first()
        .map_or("", String::as_str)
        .to_ascii_lowercase();
    let second = words.get(1).map_or("", String::as_str).to_ascii_lowercase();
    if first == "begin" {
        return second != "immediate" && second != "exclusive";
    }
    if first == "pragma" {
        // `PRAGMA wal_checkpoint` writes the file the log holds frames
        // for, and every other pragma that asks writes nothing.
        return !text.contains('=')
            && !second.eq_ignore_ascii_case("wal_checkpoint")
            && !words
                .get(2)
                .is_some_and(|word| word.eq_ignore_ascii_case("wal_checkpoint"));
    }
    [
        "attach",
        "detach",
        "commit",
        "end",
        "rollback",
        "savepoint",
        "release",
    ]
    .contains(&first.as_str())
}

/// What `sqlite3_stmt_isexplain` answers: one for an `EXPLAIN`, two for
/// an `EXPLAIN QUERY PLAN`, and nought for every other statement.
pub(crate) fn explaining(sql: &str) -> i64 {
    let words = words(sql);
    if !words
        .first()
        .is_some_and(|word| word.eq_ignore_ascii_case("explain"))
    {
        return 0;
    }
    let planned = words
        .get(1)
        .is_some_and(|word| word.eq_ignore_ascii_case("query"));
    if planned { 2 } else { 1 }
}

/// The statement an `EXPLAIN` names, and the text itself where it names
/// none.
pub(crate) fn past_explain(sql: &str) -> String {
    let text = uncommented(sql);
    let mut rest = text.trim_start();
    for word in ["explain", "query", "plan"] {
        let held = rest.get(..word.len()).unwrap_or("");
        if held.eq_ignore_ascii_case(word) {
            rest = rest.get(word.len()..).unwrap_or("").trim_start();
        }
    }
    rest.to_owned()
}

/// Whether the statement is a `PRAGMA`, which names the columns it
/// answers even though the connection that writes runs it.
fn pragmas(sql: &str) -> bool {
    words(sql)
        .first()
        .is_some_and(|word| word.eq_ignore_ascii_case("pragma"))
}

/// The columns a `PRAGMA` answers, each by name, which
/// `db_sqlite::pragma::Setting::columns` names off the pragma and
/// whether a value follows it.
///
/// The words of the statement are the name and the schema in front of
/// it where one is written, and a value follows the name where the text
/// holds `=` or a bracket, neither of which stands in a name.
fn pragma_columns(sql: &str) -> Vec<String> {
    let text = uncommented(sql);
    let words = words(sql);
    // `PRAGMA schema.name` writes the schema in front of the name, which
    // says nothing about the columns.
    let at = if text.contains('.') { 2 } else { 1 };
    let Some(name) = words.get(at) else {
        return Vec::new();
    };
    let Some(setting) = db_sqlite::pragma::of_name(name.as_bytes()) else {
        return Vec::new();
    };
    let valued = text.contains('=') || text.contains('(');
    setting
        .columns(name.as_bytes(), valued)
        .iter()
        .map(|column| String::from_utf8_lossy(column).into_owned())
        .collect()
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

/// The names of a run of values, each as text.
fn texts(values: &[Vec<u8>]) -> Vec<String> {
    values
        .iter()
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .collect()
}

/// One value, as one answer of a request.
fn alloc_one(text: &str) -> Vec<String> {
    vec![text.to_owned()]
}

/// Two values, as one answer of a request.
fn alloc_two(first: String, second: String) -> Vec<String> {
    vec![first, second]
}

/// The word `sqlite3_column_type` answers for a value.
const fn type_word(value: &Value) -> &'static str {
    match value {
        Value::Null => "NULL",
        Value::Int(_) => "INTEGER",
        Value::Real(_) => "FLOAT",
        Value::Text(_) => "TEXT",
        Value::Blob(_) => "BLOB",
    }
}

/// The real a value converts to, which is what `sqlite3_column_double`
/// answers for it.
fn real_of(value: &Value) -> f64 {
    match value {
        Value::Real(number) => *number,
        // `sqlite3_column_double` reads the whole number as a real,
        // which the text of the number reads as the same real.
        Value::Int(number) => db_sqlite::number::real(number.to_string().as_bytes()).value,
        Value::Text(bytes) | Value::Blob(bytes) => db_sqlite::number::real(bytes).value,
        Value::Null => 0.0,
    }
}

/// How many parameters one statement carries, which is what
/// `sqlite3_bind_parameter_count` answers: the largest place any of them
/// stands at.
pub(crate) fn count_binds(sql: &str) -> usize {
    named_parameters(sql).len()
}

/// The parameters one statement carries, in the order they are written,
/// each as the character that opens it and the name after it.
///
/// A parameter inside a string, a comment or an identifier in brackets is
/// text and is left, which is what `sqlite3GetToken` reads.
pub(crate) fn parameters(sql: &str) -> Vec<(u8, String)> {
    let bytes = sql.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes.get(at).copied().unwrap_or(0);
        let next = bytes.get(at.saturating_add(1)).copied().unwrap_or(0);
        match byte {
            b'\'' | b'"' | b'`' | b'[' => {
                let close = if byte == b'[' { b']' } else { byte };
                at = at.saturating_add(1);
                while at < bytes.len() {
                    if bytes.get(at).copied() == Some(close) {
                        break;
                    }
                    at = at.saturating_add(1);
                }
                at = at.saturating_add(1);
            }
            b'-' if next == b'-' => {
                while at < bytes.len() && bytes.get(at).copied() != Some(b'\n') {
                    at = at.saturating_add(1);
                }
            }
            b'/' if next == b'*' => {
                at = at.saturating_add(2);
                while at.saturating_add(1) < bytes.len() {
                    if bytes.get(at).copied() == Some(b'*')
                        && bytes.get(at.saturating_add(1)).copied() == Some(b'/')
                    {
                        break;
                    }
                    at = at.saturating_add(1);
                }
                at = at.saturating_add(2);
            }
            b'?' | b':' | b'@' | b'$' => {
                let mut end = at.saturating_add(1);
                while end < bytes.len() {
                    let held = bytes.get(end).copied().unwrap_or(0);
                    let named = held.is_ascii_alphanumeric() || held == b'_';
                    let held = if byte == b'?' {
                        held.is_ascii_digit()
                    } else {
                        named || (byte == b'$' && held == b':')
                    };
                    if !held {
                        break;
                    }
                    end = end.saturating_add(1);
                }
                // A `$name(...)` carries the brackets and what stands
                // inside them, which is how the TCL interface names a
                // member of an array.
                if byte == b'$' && bytes.get(end).copied() == Some(b'(') {
                    while end < bytes.len() {
                        let held = bytes.get(end).copied().unwrap_or(0);
                        end = end.saturating_add(1);
                        if held == b')' {
                            break;
                        }
                    }
                }
                let name = sql.get(at.saturating_add(1)..end).unwrap_or("").to_owned();
                if byte == b'?' || !name.is_empty() {
                    out.push((byte, name));
                }
                at = end;
            }
            _ => at = at.saturating_add(1),
        }
    }
    out
}

/// One statement with what the tester bound written into it as literals,
/// which is how this harness answers a parameter.
pub(crate) fn bound_into(sql: &str, bound: &BTreeMap<usize, String>) -> String {
    let held = parameters(sql);
    if held.is_empty() {
        return sql.to_owned();
    }
    let bytes = sql.as_bytes();
    let mut out = String::new();
    let mut at = 0;
    let mut place: usize = 0;
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    // The places are counted as `sqlite3ExprAssignVarNumber` counts
    // them: a `?` takes the next place, a `?N` takes the place it names,
    // and a name takes the place it was first written at.
    for (kind, name) in held {
        let mark = find_parameter(bytes, at, kind, &name);
        out.push_str(sql.get(at..mark).unwrap_or(""));
        let width = 1usize.saturating_add(name.len());
        at = mark.saturating_add(width);
        place = match (kind, name.as_str()) {
            (b'?', "") => place.saturating_add(1),
            (b'?', digits) => digits.parse().unwrap_or(place.saturating_add(1)),
            _ => {
                if let Some(held) = names.get(&name) {
                    *held
                } else {
                    let held = place.saturating_add(1);
                    names.insert(name.clone(), held);
                    held
                }
            }
        };
        out.push_str(bound.get(&place).map_or("NULL", String::as_str));
    }
    out.push_str(sql.get(at..).unwrap_or(""));
    out
}

/// Where the parameter of `kind` and `name` stands from `at` on.
fn find_parameter(bytes: &[u8], at: usize, kind: u8, name: &str) -> usize {
    let mut held = at;
    while held < bytes.len() {
        if bytes.get(held).copied() == Some(kind) {
            let width = 1usize.saturating_add(name.len());
            let end = held.saturating_add(width);
            if bytes.get(held.saturating_add(1)..end) == Some(name.as_bytes()) {
                return held;
            }
        }
        held = held.saturating_add(1);
    }
    bytes.len()
}

/// The name of the parameter at each place of one statement, counting
/// from one, which is what `sqlite3_bind_parameter_name` answers.
///
/// A `?` carries no name, a `?N` names the place `N`, and a name takes
/// the place it was first written at, which is
/// `sqlite3ExprAssignVarNumber`.
pub(crate) fn named_parameters(sql: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut place: usize = 0;
    for (kind, name) in parameters(sql) {
        let (held, named) = match (kind, name.as_str()) {
            (b'?', "") => (place.saturating_add(1), String::new()),
            (b'?', digits) => (
                digits.parse().unwrap_or(place.saturating_add(1)),
                String::new(),
            ),
            _ => {
                let mut written = String::new();
                written.push(char::from(kind));
                written.push_str(&name);
                match out.iter().position(|first| *first == written) {
                    Some(at) => (at.saturating_add(1), written),
                    None => (place.saturating_add(1), written),
                }
            }
        };
        place = held;
        while out.len() < held {
            out.push(String::new());
        }
        for slot in out.iter_mut().skip(held.saturating_sub(1)).take(1) {
            slot.clone_from(&named);
        }
    }
    out
}
