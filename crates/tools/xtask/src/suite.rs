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
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use db_sqlite::change::{Checkpointing, Does, Onto, Writer};
use db_sqlite::db::Database;
use db_sqlite::decimal::Decimal;
use db_sqlite::func::Counted;
use db_sqlite::func::Defined;
use db_sqlite::func::Safety;
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
    // `testfixture` is built with `-DSQLITE_DEFAULT_PAGE_SIZE=1024`,
    // which `research/sqlite/main.mk:1784` sets, so a file of the suite
    // that writes no page size of its own is read under this one.
    Configuration {
        name: "utf8-1024-delete",
        page: 1024,
        encoding: Encoding::Utf8,
        journal: None,
    },
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
/// moved by hundreds between runs; under pages of a thousand bytes
/// `savepoint6.test` answers 414 cases in under five minutes and 310
/// under three, so five is past every file of the suite.
const DEADLINE: Duration = Duration::from_secs(300);

/// How many cases in a row one file may have refused for the same reason
/// before the file is ended. A loop whose end a command this harness has
/// none of decides runs without bound, and the file is scored with what
/// it answered up to there.
const LOOPING: usize = 5000;

/// The capabilities an `ifcapable` may name that this engine does not
/// have. Every other name is answered as held.
const MISSING: [&str; 22] = [
    "vtab",
    // `SQLITE_DEBUG` writes the program of a statement out as it runs and
    // gives the library the seven pragmas that turn each listing on. This
    // engine builds no program, so it holds none of them.
    "debug",
    "fts1",
    "fts2",
    "fts3",
    "fts4",
    "fts5",
    "rtree",
    "icu",
    "shared_cache",
    "memdebug",
    "codec",
    "atomicwrite",
    "explain",
    "compound_select",
    "unlock_notify",
    "session",
    "update_delete_limit",
    // `SQLITE_DIRECT_OVERFLOW_READ` reads an overflow page past the
    // cache, which this engine does not, and `SQLITE_THREADSAFE=2` is a
    // build that holds a lock per connection, which this engine holds
    // none of.
    "direct_read",
    "threadsafe2",
    // `SQLITE_ENABLE_MEMORY_MANAGEMENT` gives the library
    // `sqlite3_release_memory`, which writes a dirty page out of a cache
    // on demand. This engine writes a page out where the cache it is
    // held to is full and on no other call.
    "memorymanage",
    // `SQLITE_ENABLE_STAT4` writes `sqlite_stat4`, which holds samples of
    // the keys of an index. `ANALYZE` writes `sqlite_stat1` here and no
    // other table.
    "stat4",
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
    /// The database, the table and the name each of those columns comes
    /// from, and three empty texts for a column of an expression.
    origins: Vec<[String; 3]>,
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
    /// The number the schema of the connection stood at where the
    /// statement was made, which a step reads again to answer
    /// `SQLITE_SCHEMA` for a schema that has changed since.
    cookie: u32,
    /// How many reasons of its own the connection had given where the
    /// statement was made, which a database taken away, a function, a
    /// collation and an authorizer each count as one of.
    stamp: u64,
}

/// The three files of one database while the writes of a run are applied
/// to them, which a crash stops part way through.
struct Crashing {
    /// The database file.
    main: Vec<u8>,
    /// The rollback journal beside it.
    journal: Vec<u8>,
    /// The write-ahead log beside it.
    log: Vec<u8>,
}

/// One backup `sqlite3_backup_init` began: the connection it writes, the
/// connection it reads, the page it copies next and the image the source
/// held when the last step ran.
struct Backing {
    /// The connection whose database the backup writes.
    into: String,
    /// The connection whose database the backup reads.
    from: String,
    /// The page the next step copies, counting from one.
    next: u32,
    /// The pages the source held when its count was last read.
    pages: u32,
    /// The pages the connections over the source had written then, which
    /// a step reads again: a source written since begins the backup
    /// again, which `sqlite3BackupRestart` of
    /// `research/sqlite/src/backup.c:719` does.
    wrote: u64,
}

/// A child process a crash runs its statements in: the three files of the
/// path as it found them, the connection its script runs over,
/// everything that connection has done, and where the machine loses
/// power.
struct Crashed {
    /// The path the child opened.
    path: String,
    /// The files as the child found them, which the crash writes part of.
    files: Crashing,
    /// The connection the statements of the script run on.
    writer: Writer,
    /// Everything the statements have done so far.
    did: Vec<Does>,
    /// Whether the machine counts the writes of every file rather than
    /// the syncs of one, which `crash_on_write` says.
    counts_writes: bool,
    /// Which sync of that file the machine loses power at, nought where
    /// it loses power at none.
    delay: usize,
    /// The file whose syncs the machine counts.
    onto: Onto,
    /// The seed the crash chooses its torn write by.
    seed: usize,
    /// Whether the script said the machine loses power now, which
    /// `sqlite3_crash_now` says.
    now: bool,
}

impl Crashing {
    /// The files as the session holds them.
    fn new(files: (Vec<u8>, Vec<u8>, Vec<u8>)) -> Self {
        Crashing {
            main: files.0,
            journal: files.1,
            log: files.2,
        }
    }

    /// A connection over the files as they stand, which plays back a
    /// journal that is hot and reads a log that holds frames.
    ///
    /// Three files of no bytes are a database no process has written,
    /// which a connection makes under the settings this run opens every
    /// database with. A file of no bytes beside a log is the database a
    /// connection in write-ahead logging left, whose pages are all in
    /// the log.
    fn opened(&self) -> Result<Writer, String> {
        if self.main.is_empty() && self.journal.is_empty() && self.log.is_empty() {
            let under = configured();
            let mut writer =
                Writer::new(under.page, 0, under.encoding).map_err(|error| refusal(&error))?;
            if let Some(journal) = under.journal {
                let mut sql = b"PRAGMA journal_mode=".to_vec();
                sql.extend_from_slice(journal);
                let _ = writer.run(&sql);
            }
            return Ok(writer);
        }
        Writer::recovered(&self.main, &self.journal, &self.log).map_err(|error| refusal(&error))
    }

    /// The connection the session holds after the crash, which recovers
    /// the files the crash left.
    fn recovered(&self) -> Result<Writer, String> {
        self.opened()
    }

    /// The file one write reaches.
    const fn file(&mut self, onto: Onto) -> &mut Vec<u8> {
        match onto {
            Onto::Main => &mut self.main,
            Onto::Journal => &mut self.journal,
            Onto::Log => &mut self.log,
        }
    }

    /// Does one thing to the files.
    fn does(&mut self, held: &Does) {
        match held {
            Does::Write { onto, at, bytes } => {
                let file = self.file(*onto);
                let at = usize::try_from(*at).unwrap_or(usize::MAX);
                let end = at.saturating_add(bytes.len());
                if file.len() < end {
                    file.resize(end, 0);
                }
                for (slot, byte) in file.iter_mut().skip(at).zip(bytes) {
                    *slot = *byte;
                }
            }
            Does::Truncate { onto, at } => {
                let at = usize::try_from(*at).unwrap_or(usize::MAX);
                self.file(*onto).truncate(at);
            }
            Does::Remove(onto) => self.file(*onto).clear(),
            Does::Sync(_) => {}
        }
    }

    /// Applies the writes of `did` until the `delay`th of them, which the
    /// machine never reaches, and answers whether it stopped there.
    ///
    /// `writecrashWrite` of `research/sqlite/src/test_devsym.c` counts
    /// every write of every file and ends the process before the one the
    /// count reaches, so the writes before it stand on the disk and the
    /// one it stopped at reaches nothing.
    fn on_write(&mut self, did: &[Does], delay: usize) -> bool {
        let mut left = delay;
        for held in did {
            if matches!(held, Does::Write { .. }) {
                left = left.saturating_sub(1);
                if left == 0 {
                    return true;
                }
            }
            self.does(held);
        }
        false
    }

    /// Holds every write back until the sync of its file and stops at the
    /// `delay`th sync of `onto`, which the machine loses power at, and
    /// answers whether it stopped there.
    ///
    /// `writeListSync` of `research/sqlite/src/test6.c` writes the list a
    /// sync holds back and, where the sync is the one that crashes,
    /// writes each entry of the whole list, leaves it out, or fills the
    /// sectors it covers with garbage, one of the three per entry.
    fn on_sync(&mut self, did: &[Does], delay: usize, onto: Onto, seed: usize) -> bool {
        let mut left = delay;
        let mut pending: Vec<&Does> = Vec::new();
        let mut random = Rolling::new(seed);
        for held in did {
            // `cfDelete` of `research/sqlite/src/test6.c:661` takes a
            // file away at once, where `cfWrite` and `cfTruncate` hold
            // what they do back for the sync, so a journal a commit
            // disposed of is gone whatever the machine does after it and
            // the writes it held are gone with it.
            if let Does::Remove(onto) = held {
                pending.retain(|entry| onto_of(entry) != *onto);
                self.does(held);
                continue;
            }
            let Does::Sync(synced) = held else {
                pending.push(held);
                continue;
            };
            if *synced == onto {
                left = left.saturating_sub(1);
            }
            if *synced == onto && left == 0 {
                self.torn(&pending, &mut random);
                return true;
            }
            let (now, later): (Vec<&Does>, Vec<&Does>) = pending
                .into_iter()
                .partition(|held| onto_of(held) == *synced);
            for held in &now {
                self.does(held);
            }
            pending = later;
        }
        for held in &pending {
            self.does(held);
        }
        false
    }

    /// Writes what the machine that lost power had written of the list:
    /// each entry in turn until one of them, which is written in part or
    /// not at all, and nothing after it.
    ///
    /// A machine stops once, so the entries before the one it stopped in
    /// stand whole, the one it stopped in stands as far as its bytes
    /// reached, and the ones after it reach the disk at all. This is
    /// narrower than `writeListSync` of `research/sqlite/src/test6.c`,
    /// which fills whole sectors with garbage: a frame of the log is
    /// shorter than a sector and holds no sector of its own, so garbage
    /// over the sector a frame lies in would take back frames the log
    /// had already held.
    fn torn(&mut self, pending: &[&Does], random: &mut Rolling) {
        let stops = random.next().checked_rem(pending.len().max(1)).unwrap_or(0);
        for (at, held) in pending.iter().enumerate() {
            if at < stops {
                self.does(held);
                continue;
            }
            let Does::Write { onto, at, bytes } = held else {
                return;
            };
            let reached = random.next().checked_rem(bytes.len().max(1)).unwrap_or(0);
            self.does(&Does::Write {
                onto: *onto,
                at: *at,
                bytes: bytes.get(..reached).unwrap_or_default().to_vec(),
            });
            return;
        }
    }
}

/// The values every statement of `sql` answered and the message the first
/// one refused with, where one did.
///
/// `held` carries whether another connection holds the transaction of the
/// path and whether this connection waits on it, which the statements of
/// a text may change.
fn ran_each(
    writer: &mut Writer,
    sql: &str,
    reads_as: (&'static [Collating], &'static [Defined], &str),
    held: (bool, &mut bool),
) -> (Vec<String>, Result<(), String>) {
    let (collating, defines, null) = reads_as;
    let (outside, waiting) = held;
    let mut out = Vec::new();
    for statement in statements(sql) {
        // The bytes after the last one that carries meaning are the
        // statement's own: `SELECT 1 /* ` is a comment that runs to the
        // end and `SELECT 1 /*` is a slash and a star.
        let text = statement.trim_start();
        if text.trim().is_empty() {
            continue;
        }
        if outside {
            match locked(text, waiting) {
                Locked::Waits => continue,
                Locked::Refused => return (out, Err("database is locked".to_owned())),
                Locked::Reads => {}
            }
        }
        match run_one(writer, text, collating, defines, outside) {
            Ok(values) => out.extend(values.iter().map(|value| listed(value, null))),
            Err(message) => return (out, Err(message)),
        }
    }
    (out, Ok(()))
}

/// What one statement of a connection that does not hold the transaction
/// of the path may do.
enum Locked {
    /// It waits: the statement opens or ends a transaction of its own,
    /// which takes no lock and runs nothing.
    Waits,
    /// It reads the file as the transaction of the other connection found
    /// it.
    Reads,
    /// It writes, which the lock the other connection holds refuses.
    Refused,
}

/// What `text` may do while another connection holds the transaction of
/// the path, and whether the connection is waiting on that transaction
/// once the statement has run.
///
/// `sqlite3BeginTrans` opens a deferred transaction, which takes no lock,
/// so the `BEGIN` runs nothing and the first write of it is what
/// `sqlite3BtreeBeginTrans` refuses for a `RESERVED` lock another
/// connection holds.
fn locked(text: &str, waiting: &mut bool) -> Locked {
    if begins(text) {
        *waiting = true;
        return Locked::Waits;
    }
    if *waiting && ends(text) {
        *waiting = false;
        return Locked::Waits;
    }
    if reads(text) {
        Locked::Reads
    } else {
        Locked::Refused
    }
}

/// The logs and the rollback journals beside the files an `ATTACH`
/// added, each with its file name.
type Beside = (Vec<(Vec<u8>, Vec<u8>)>, Vec<(Vec<u8>, Vec<u8>)>);

/// The log and the rollback journal the harness holds beside one file,
/// each where the file has one.
type Besides = (Option<Vec<u8>>, Option<Vec<u8>>);

/// The database a name stands for and which of the two files beside it
/// the name holds, where the name holds one: `test.db-wal` is the log of
/// `test.db` and `test.db` is the database itself.
fn named_beside(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('-') {
        Some((base, tail @ ("wal" | "journal"))) => (base, Some(tail)),
        _ => (name, None),
    }
}

/// Whether the statement begins a transaction, which `BEGIN` and `BEGIN
/// TRANSACTION` both do.
fn begins(text: &str) -> bool {
    first_word(text).eq_ignore_ascii_case("begin")
}

/// Whether the statement ends one, which `COMMIT`, `END` and `ROLLBACK`
/// all do.
fn ends(text: &str) -> bool {
    let word = first_word(text);
    ["commit", "end", "rollback"]
        .iter()
        .any(|held| word.eq_ignore_ascii_case(held))
}

/// The first word of a statement, which is the letters it opens with.
fn first_word(text: &str) -> &str {
    let held = text.trim_start();
    let at = held
        .find(|held: char| !held.is_ascii_alphabetic())
        .unwrap_or(held.len());
    held.get(..at).unwrap_or_default()
}

/// One path with the steps that name nothing taken out: an empty step, a
/// step of one dot, and a step of two dots with the step before it.
///
/// `sqlite3_open` names the file the path reaches, so two spellings of one
/// path are one database, which `lock.test` opens a second connection
/// over `./tempdir/../tempdir/t1/.//t2/../../..//test.db` to state.
///
/// Simplifying one path costs O(n) in its steps.
fn simplified(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut steps: Vec<&str> = Vec::new();
    for step in path.split('/') {
        match step {
            "" | "." => {}
            ".." => {
                // A step of two dots at the front of a relative path
                // names the directory above, which no step before it
                // names.
                if steps.last().is_none_or(|held| *held == "..") {
                    steps.push(step);
                } else {
                    steps.pop();
                }
            }
            held => steps.push(held),
        }
    }
    let held = steps.join("/");
    if absolute { format!("/{held}") } else { held }
}

/// The image a backup writes into `path`, with the two version bytes of
/// its header read back to one where the path names a database the
/// connection holds of its own.
///
/// `pPager->memDb` of `research/sqlite/src/pager.c` is never read through
/// a log whatever page one says, so a copy of a database in write-ahead
/// logging mode is held in memory as a database that keeps a journal.
fn held_alone(image: Vec<u8>, path: &str) -> Vec<u8> {
    if !path.is_empty() && !path.contains('\0') && !path.eq_ignore_ascii_case(":memory:") {
        return image;
    }
    let mut image = image;
    for slot in image.iter_mut().skip(18).take(2) {
        *slot = 1;
    }
    image
}

/// The path of the file a crash counts on, which is the database itself
/// where the name carries the tail of a journal or of a log.
fn pathed(named: &str) -> &str {
    named
        .strip_suffix("-wal")
        .or_else(|| named.strip_suffix("-journal"))
        .unwrap_or(named)
}

/// Which of the three files a crash counts the syncs of, read off the
/// name it counts on.
fn onto_named(named: &str) -> Onto {
    match named.rsplit_once('-').map(|(_, tail)| tail) {
        Some("wal") => Onto::Log,
        Some("journal") => Onto::Journal,
        _ => Onto::Main,
    }
}

/// The file one thing a commit does reaches.
const fn onto_of(held: &Does) -> Onto {
    match held {
        Does::Write { onto, .. }
        | Does::Truncate { onto, .. }
        | Does::Remove(onto)
        | Does::Sync(onto) => *onto,
    }
}

/// The numbers the crash chooses by, which one seed answers the same
/// list of every time: `x[n+1] = x[n] * 6364136223846793005 + 1`, the
/// multiplier Knuth names for a linear congruential source.
struct Rolling(u64);

impl Rolling {
    /// The source one seed begins.
    fn new(seed: usize) -> Self {
        Rolling(u64::try_from(seed).unwrap_or(0))
    }

    /// The next number, which is the high bits of the state because the
    /// low ones of such a source repeat in short cycles.
    fn next(&mut self) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        usize::try_from(self.0 >> 33).unwrap_or(0)
    }
}

/// One run of one file: the interpreter on one side of the line and the
/// engine on the other.
struct Session {
    /// The name of the file, for what `--show` writes.
    file: String,
    /// One writer per path a connection opened, which is what two
    /// connections over one path share.
    held: BTreeMap<String, Writer>,
    /// The paths the client may only read, which `file attributes NAME
    /// -readonly 1` and a mode of `-permissions` that lets the owner not
    /// write name: every connection over such a path refuses a statement
    /// that would write a page of the file.
    readonly: BTreeSet<String>,
    /// The connections `sqlite3 NAME FILE -readonly 1` opened, which
    /// `SQLITE_OPEN_READONLY` of `sqlite3_open_v2` names: such a
    /// connection refuses a statement that would write a page of the file
    /// and another connection over the same path writes it.
    readers: BTreeSet<String>,
    /// The bytes the machine wrote for a path and the image its writer
    /// answered then, kept where the two differ. Writing a database out
    /// again drops the bytes past the pages its header names and writes
    /// the count of pages into the header, which a file the tester wrote
    /// by hand carries neither of, so these bytes stand while the writer
    /// has written nothing since.
    raw: BTreeMap<String, (Vec<u8>, Vec<u8>)>,
    /// The log and the rollback journal beside the file of each database
    /// an `ATTACH` added, which the writer that attached it holds and the
    /// writer of that path does not.
    beside: BTreeMap<String, Besides>,
    /// Which path each connection reads.
    connections: BTreeMap<String, String>,
    /// What a `NULL` prints as, per connection.
    nulls: BTreeMap<String, String>,
    /// How many times each connection was told a function, a collation
    /// or an authorizer, or took a database away, which
    /// `sqlite3ExpirePreparedStatements` of
    /// `research/sqlite/src/vdbeaux.c:5337` counts as a reason to make
    /// every statement of it again.
    stamps: BTreeMap<String, u64>,
    /// The three counters of each connection, which belong to a
    /// connection and not to the file the connection opened.
    counters: BTreeMap<String, Counted>,
    /// Which connection holds the transaction open on each path, where
    /// one does. A connection that did not begin it reads the file as
    /// the transaction found it.
    owners: BTreeMap<String, String>,
    /// Which connections wrote a `BEGIN` the transaction of another
    /// connection over the same path stood in the way of, which take no
    /// lock until they write.
    waiting: BTreeSet<String>,
    /// What the last statement of each connection counted, which
    /// `db status` answers.
    stepped: BTreeMap<String, db_sqlite::db::Stepped>,
    /// How many times the last statement of the run, on whichever
    /// connection, sorted, which `sqlite3_sort_count` counts and
    /// `::sqlite_sort_count` answers.
    sorted: u64,
    /// How many times a commit held a file on the disk since the tester
    /// last counted from nought, which `sqlite3_sync_count` counts and
    /// `::sqlite_sync_count` answers, and how many of those the
    /// connection was under `PRAGMA fullfsync` for.
    synced: (u64, u64),
    /// The descents and the steps the last statement of the run took,
    /// which `sqlite3_search_count` counts and `::sqlite_search_count`
    /// answers.
    searched: i64,
    /// What `PRAGMA data_version` answers for each connection, and the
    /// count of commits the file had when the connection last read it:
    /// the value rises by one per commit another connection made, which
    /// `pPager->iDataVersion` of `research/sqlite/src/pager.c:669` and
    /// `iBDataVersion` of `research/sqlite/src/btreeInt.h:354` together
    /// answer.
    dated: BTreeMap<String, (i64, u32)>,
    /// The temp schema each connection holds, which is a database of its
    /// own that no other connection reads.
    temps: BTreeMap<String, Vec<u8>>,
    /// Which connection's temp schema the writer of each path carries,
    /// because the harness holds one writer per path and a temp schema
    /// belongs to a connection.
    tempers: BTreeMap<String, String>,
    /// The child a crash opened, whose script the tester runs one
    /// statement at a time and whose crash is applied when the script
    /// ends.
    crashing: Option<Crashed>,
    /// The backups `sqlite3_backup_init` began, under the names the
    /// tester holds them by.
    backups: BTreeMap<String, Backing>,
    /// The directory the interpreter runs in, which is where a file the
    /// session holds is written when the tester opens it as a file of
    /// the machine.
    over: PathBuf,
    /// What the machine answered the last open of a file with, which
    /// `sqlite3_system_errno` reads: two for a directory that is not
    /// there, and nought where the file opened.
    errno: i32,
    /// How many pages the connections over each path have written into
    /// the database file, which `PAGER_STAT_WRITE` of
    /// `research/sqlite/src/pager.c` counts and `btree_pager_stats`
    /// answers under `write`.
    writes: BTreeMap<String, u64>,
    /// The limits each connection holds, which `sqlite3_limit` sets and
    /// reads and which a connection that is opened again holds the hard
    /// ones of.
    limits: BTreeMap<String, db_sqlite::db::Limits>,
    /// Whether a file name that begins `file:` is read as a URI, which
    /// `sqlite3_config_uri` sets for every connection opened after it.
    uri: bool,
    /// Which connections read a file name as a URI: the setting above,
    /// or the `SQLITE_OPEN_URI` flag the open carried.
    uris: BTreeMap<String, bool>,
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
    /// The hooks the tester told each connection, each by the name of the
    /// request that named its script, which the writer is told for each
    /// request.
    hooked: BTreeMap<String, BTreeSet<String>>,
    /// The statements the tester prepared, by the name it holds each at.
    statements: BTreeMap<String, Prepared>,
    /// How many statements the tester has prepared, which names the next
    /// one.
    prepared: u64,
    /// How many databases of a connection's own this run has opened,
    /// which names the next one.
    opened: u64,
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
            temps: BTreeMap::new(),
            tempers: BTreeMap::new(),
            crashing: None,
            backups: BTreeMap::new(),
            over: std::env::temp_dir().join(format!("audhsos-suite-{file}")),
            file: file.to_owned(),
            held: BTreeMap::new(),
            readonly: BTreeSet::new(),
            readers: BTreeSet::new(),
            raw: BTreeMap::new(),
            beside: BTreeMap::new(),
            connections: BTreeMap::new(),
            nulls: BTreeMap::new(),
            stamps: BTreeMap::new(),
            counters: BTreeMap::new(),
            owners: BTreeMap::new(),
            waiting: BTreeSet::new(),
            stepped: BTreeMap::new(),
            sorted: 0,
            synced: (0, 0),
            searched: 0,
            dated: BTreeMap::new(),
            errno: 0,
            writes: BTreeMap::new(),
            limits: BTreeMap::new(),
            uri: false,
            uris: BTreeMap::new(),
            pragmas: BTreeMap::new(),
            collations: BTreeMap::new(),
            functions: BTreeMap::new(),
            authorizers: BTreeMap::new(),
            hooked: BTreeMap::new(),
            statements: BTreeMap::new(),
            prepared: 0,
            opened: 0,
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
        // The writer of a path carries the temp schema of the connection
        // that last read it, so the connection this request names is
        // given its own before the request is answered.
        self.tempering(verb, first);
        match verb {
            "open" => Ok(self.open(
                first,
                second,
                (
                    args.get(2).map_or("", String::as_str),
                    args.get(3).map_or("", String::as_str),
                ),
            )),
            // `sqlite3_config_uri` of `research/sqlite/src/main.c:707`,
            // which every connection opened after it reads a file name
            // that begins `file:` as a URI for.
            "config_uri" => Ok(self.reads_uri(first == "1")),
            // `btree_pager_stats` of `research/sqlite/src/test3.c:147`,
            // which answers the eleven counts of `sqlite3PagerStats`.
            "pager" => self.pager(first),
            // `btree_ismemdb` of `research/sqlite/src/test3.c`, which
            // answers whether no file holds the database.
            "ismemdb" => Ok(vec![usize::from(self.in_memory(first)).to_string()]),
            // `sqlite3_system_errno` of `research/sqlite/src/main.c`:
            // what the machine answered the last open of a file with.
            "system_errno" => Ok(vec![self.errno.to_string()]),
            "close" => Ok(self.closed(first)),
            "delete" => Ok(self.removed(first)),
            "exists" => Ok(vec![usize::from(self.sized(first).is_some()).to_string()]),
            // `sqlite3_test_control SQLITE_TESTCTRL_LOCALTIME_FAULT`,
            // which names the zone `localtime` and `utc` read.
            "zone" => Ok(zoning(first)),
            // The tester opens a file of the machine, so the bytes the
            // session holds are written there first, and read back when
            // it closes the channel.
            // `file attributes NAME -readonly` and `-permissions` of
            // `research/sqlite/test/tester.tcl`: whether the client may
            // only read the file, and minus one for a name the harness
            // holds no file under.
            "permissions" => Ok(vec![self.permissions(first, second).to_string()]),
            "flush" => Ok(vec![self.flushed(first).to_string()]),
            "take" => Ok(vec![self.taken(first).to_string()]),
            // `crashsql` and `crash_on_write` of
            // `research/sqlite/test/tester.tcl`.
            "crash" => self.crashed(args),
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
            // `sqlite3BitvecBuiltinTest` of
            // `research/sqlite/src/bitvec.c:400`.
            "bitvec" => Ok(alloc_one(
                &bitvec_test(i64::try_from(number_of(first)?).unwrap_or(0), second).to_string(),
            )),
            "complete" => Ok(vec![
                usize::from(db_sqlite::token::complete(second.as_bytes())).to_string(),
            ]),
            "copy" => self.copy(first, second),
            // `sqlite3_backup_init` and the four calls that drive the
            // backup it answers.
            "backup" => self.backed(args),
            // `sqlite3_create_collation` and `sqlite3_create_function`
            // name a proc the engine reaches back over the line, and
            // `load_static_extension` names a module.
            "collate" | "function" | "extension" => {
                let third = args.get(2).map_or("", String::as_str);
                self.told(verb, first, (second, third));
                Ok(Vec::new())
            }
            // `SQLITE_TESTCTRL_INTERNAL_FUNCTIONS` turns the functions
            // the library keeps for its own use on and off again.
            "internal" => {
                self.internals(first);
                Ok(Vec::new())
            }
            // `sqlite3_create_collation` with no function deletes the
            // collation of that name.
            "uncollate" => {
                self.uncollates(first, second);
                Ok(Vec::new())
            }
            "null" => {
                self.nulls.insert(first.to_owned(), second.to_owned());
                Ok(Vec::new())
            }
            // `db status (step|sort|autoindex|vmstep)`.
            "status" => self.status(first, second),
            // `sqlite3_sort_count` of `vdbe.c:79`.
            "sorts" => Ok(alloc_one(&self.sorted.to_string())),
            // `sqlite3_sync_count` and `sqlite3_fullsync_count` of
            // `research/sqlite/src/os_unix.c`.
            "syncs" => Ok(alloc_one(&self.synced.0.to_string())),
            "fullsyncs" => Ok(alloc_one(&self.synced.1.to_string())),
            "syncs_as" => {
                self.synced.0 = u64::try_from(number_of(first)?).unwrap_or(0);
                Ok(Vec::new())
            }
            "fullsyncs_as" => {
                self.synced.1 = u64::try_from(number_of(first)?).unwrap_or(0);
                Ok(Vec::new())
            }
            // `sqlite3_search_count` of `vdbe.c:56`.
            "searches" => Ok(alloc_one(&self.searched.to_string())),
            "clock" => self.ticks(first),
            // `sqlite3_set_authorizer`, `sqlite3_commit_hook`,
            // `sqlite3_rollback_hook` and `sqlite3_update_hook`.
            "authorizer" | "commit_hook" | "rollback_hook" | "update_hook" | "preupdate_hook" => {
                self.hooks(verb, first, second);
                Ok(Vec::new())
            }
            // `sqlite3_prepare`, the commands that read a statement it
            // answered, and the three that read what it is.
            "prepare" | "autocommit" | "step" | "finalize" | "reset" | "clear_binds" | "bind"
            | "column" | "stmt" | "next_stmt" | "readonly" | "busy" | "isexplain" | "expired"
            | "checkpoint" => self.of_statement(verb, args),
            // `sqlite3_errcode` and `sqlite3_extended_errcode`.
            "errcode" => Ok(alloc_one(&last_code(first))),
            "normalize" => Ok(alloc_one(&normalized(first))),
            "columnmeta" => self.column_meta(first, second, args.get(2).map_or("", String::as_str)),
            "eval" | "names" | "exec" | "exec_names" | "rows" => self.of_sql(verb, first, second),
            // `DB deserialize BYTES` of
            // `research/sqlite/src/tclsqlite.c`: the database of the
            // connection is read again from the bytes the tester hands
            // over, which it carries as hexadecimal digits.
            "deserialize" | "serialize" => self.deserialize(verb, first, second),

            // `sqlite3_blob_open`, `sqlite3_blob_bytes`,
            // `sqlite3_blob_read` and `sqlite3_blob_write` of
            // `research/sqlite/src/test_blob.c`.
            "blob_bytes" | "blob_read" | "blob_write" | "refused" => self.blob(verb, args),
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
            other => self.apart(other, args),
        }
    }

    /// What one request that reads the library itself answers, rather
    /// than a database or a connection the session holds.
    fn apart(&mut self, verb: &str, args: &[String]) -> Result<Vec<String>, String> {
        let first = args.first().map_or("", String::as_str);
        let second = args.get(1).map_or("", String::as_str);
        match verb {
            "varint" => varint(args),
            // `sqlite3_quota_glob` of
            // `research/sqlite/src/test_quota.c:254`.
            "strglob" => Ok(globbed(first, second)),
            // `sqlite3_limit` of `research/sqlite/src/main.c:2990`,
            // which answers what the limit was and sets it.
            "limit" => Ok(self.limited(first, second, args.get(2).map_or("", String::as_str))),
            // `sqlite3_test_errstr` of
            // `research/sqlite/src/test1.c:3674`: the words the code of
            // that name stands for.
            "errstr" => Ok(vec![db_sqlite::db::errstr(numbered_code(first)).to_owned()]),
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
        self.bytes_of(name).map(|bytes| bytes.len())
    }

    /// The bytes the harness holds beside the database `base` under the
    /// name `tail`, which is `wal` for the log and `journal` for the
    /// rollback journal.
    ///
    /// The writer of the path holds them where a connection opened that
    /// path, and the writer that attached the file holds them where no
    /// connection did, which is what `beside` carries.
    fn beside_bytes(&self, base: &str, tail: &str) -> Option<Vec<u8>> {
        let held = self.held.get(base);
        let mine = match tail {
            "wal" => held.and_then(|writer| writer.log().map(<[u8]>::to_vec)),
            "journal" => held.and_then(|writer| writer.journal().map(<[u8]>::to_vec)),
            _ => return None,
        };
        if mine.is_some() {
            return mine;
        }
        let (log, journal) = self.beside.get(base)?;
        match tail {
            "wal" => log.clone(),
            _ => journal.clone(),
        }
    }

    /// The bytes the session holds under `name` written into the
    /// directory the interpreter runs in, so that the tester may open
    /// the file as a file of the machine.
    ///
    /// The answer is whether the file was written. Writing costs O(n) in
    /// its bytes.
    /// Whether the client may only read the file `name`, and where
    /// `told` holds a number, that the file is held that way from now on.
    ///
    /// The answer is one for a file the client may only read, nought for
    /// one it may write, and minus one for a name the harness holds no
    /// file under, which is what makes the tester's own `file attributes`
    /// answer for every other name.
    fn permissions(&mut self, name: &str, told: &str) -> i64 {
        let path = simplified(name);
        // A journal or a log stands beside a database the harness holds,
        // and the harness answers for it while the database is there
        // whether the file beside it holds bytes now or not.
        let (base, beside) = named_beside(&path);
        let held = match beside {
            Some(_) => self.bytes_of(base).is_some(),
            None => self.bytes_of(&path).is_some(),
        };
        if !held {
            return -1;
        }
        if told.is_empty() {
            return i64::from(self.readonly.contains(&path));
        }
        if told == "0" {
            self.readonly.remove(&path);
            return 0;
        }
        self.readonly.insert(path);
        1
    }

    /// Refuses a statement on a connection whose database carries a
    /// journal the client may only read.
    ///
    /// `unixOpen` of `research/sqlite/src/os_unix.c` answers
    /// `SQLITE_CANTOPEN` for such a journal, and `sqlite3PagerSharedLock`
    /// reads the database only once the journal is played back, so the
    /// database cannot be read at all.
    ///
    /// # Errors
    ///
    /// The words `unable to open database file`.
    fn cannot_open(&self, name: &str) -> Result<(), String> {
        let Some(path) = self.connections.get(name) else {
            return Ok(());
        };
        if !self.readonly.contains(&format!("{path}-journal")) {
            return Ok(());
        }
        refused_as("SQLITE_CANTOPEN", "14");
        Err(String::from("unable to open database file"))
    }

    fn flushed(&self, name: &str) -> usize {
        let Some(bytes) = self.bytes_of(name) else {
            return 0;
        };
        let path = self.over.join(name);
        if path.parent().is_some_and(|over| !over.is_dir()) {
            return 0;
        }
        usize::from(std::fs::write(&path, &bytes).is_ok())
    }

    /// The file of the machine read back into the session, which the
    /// tester wrote through a channel of its own.
    ///
    /// The answer is whether the session took it. A name the session
    /// holds no database under is left where it is, because a file the
    /// tester writes beside the databases is its own. Reading costs O(n)
    /// in the bytes of the database.
    fn taken(&mut self, name: &str) -> usize {
        let path = self.over.join(name);
        let Ok(bytes) = std::fs::read(&path) else {
            return 0;
        };
        let (base, tail) = named_beside(name);
        if !self.held.contains_key(base) {
            return 0;
        }
        if self.bytes_of(name).is_some_and(|held| held == bytes) {
            return 0;
        }
        let mut files = self.files_of(base);
        match tail {
            Some("wal") => files.2 = bytes,
            Some("journal") => files.1 = bytes,
            _ => files.0 = bytes,
        }
        usize::from(self.opened_again(base, files).is_ok())
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
        let image = writer.written();
        self.held.insert(name.to_owned(), writer);
        self.machined(name, bytes, image);
        Ok(vec![written.len().to_string()])
    }

    /// The blob handle commands of `research/sqlite/src/test_blob.c`,
    /// which answer the name of the code they carry, what they read as
    /// hexadecimal digits, and what the refusal says.
    ///
    /// The values are the connection, the schema, the table, the column,
    /// the key of the row and whether the handle writes, then the offset
    /// and either how many bytes to read or the digits to write. The
    /// handle itself is the tester's, which holds the six and hands them
    /// back with every command, so a handle that is reopened under
    /// another key is the same six with that key.
    fn blob(&mut self, verb: &str, args: &[String]) -> Result<Vec<String>, String> {
        let held = |at: usize| args.get(at).map_or("", String::as_str);
        // What the tester refused a command of a blob handle with, which
        // `sqlite3_errcode` answers as every other refusal.
        if verb == "refused" {
            refused_as(held(0), held(1));
            return Ok(Vec::new());
        }
        let name = held(0);
        let path = self
            .connections
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no such connection: {name}"))?;
        let rowid: i64 = held(4).parse().unwrap_or(0);
        let at = number_of(held(6)).unwrap_or(0);
        let schema = held(1).as_bytes().to_vec();
        let table = held(2).as_bytes().to_vec();
        let column = held(3).as_bytes().to_vec();
        let written = binary(held(7));
        let wanted = number_of(held(7)).unwrap_or(0);
        let writer = self
            .held
            .get_mut(&path)
            .ok_or_else(|| format!("no such database: {path}"))?;
        let asked = db_sqlite::change::Blob {
            schema: (!schema.is_empty()).then_some(schema.as_slice()),
            table: &table,
            column: &column,
            rowid,
            writing: held(5) != "0",
        };
        let answered = match verb {
            "blob_bytes" => writer.blob_bytes(&asked).map(|held| held.to_string()),
            "blob_read" => writer
                .blob_read(&asked, at, wanted)
                .map(|bytes| digits(&bytes)),
            _ => writer
                .blob_write(&asked, at, &written)
                .map(|()| String::new()),
        };
        Ok(match answered {
            Ok(held) => {
                stood();
                vec!["SQLITE_OK".to_owned(), held, String::new()]
            }
            Err(error) => {
                let code = String::from_utf8_lossy(error.code().extended_name).into_owned();
                vec![code, String::new(), refusal(&error)]
            }
        })
    }

    /// `DB serialize` answers the bytes of the database of a connection,
    /// and `DB deserialize` reads the database again from the bytes the
    /// tester hands over, each as hexadecimal digits.
    ///
    /// Either costs O(n) in the bytes of the database.
    fn deserialize(&mut self, verb: &str, name: &str, data: &str) -> Result<Vec<String>, String> {
        let path = self
            .connections
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no such connection: {name}"))?;
        if verb == "serialize" {
            let writer = self
                .held
                .get(&path)
                .ok_or_else(|| format!("no such database: {path}"))?;
            return Ok(vec![digits(&writer.written())]);
        }
        let bytes = binary(data);
        let mut writer = Writer::opened(&bytes).map_err(|error| refusal(&error))?;
        writer.defines(DEFINED);
        writer.groups(GROUPED);
        self.held.insert(path, writer);
        self.stamped(name);
        Ok(Vec::new())
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
            let image = writer.written();
            if let Some((bytes, held)) = self.raw.get(name)
                && *held == image
            {
                return Some(bytes.clone());
            }
            return Some(image);
        }
        let (base, tail) = name.rsplit_once('-')?;
        self.beside_bytes(base, tail)
    }

    /// The bytes the machine wrote for `path` kept beside its writer,
    /// and given up where writing the database out again answers them.
    fn machined(&mut self, path: &str, bytes: Vec<u8>, image: Vec<u8>) {
        if bytes == image {
            self.raw.remove(path);
            return;
        }
        self.raw.insert(path.to_owned(), (bytes, image));
    }

    /// One connection closed, which is `sqlite3_close`.
    ///
    /// The connection rolls back the transaction it began itself and
    /// leaves the transaction another connection over the same path began
    /// where it stands, which the harness names the owner of. The last
    /// connection over a path writes the log of that path into the file
    /// and removes the log, which `sqlite3WalClose` does.
    fn closed(&mut self, name: &str) -> Vec<String> {
        if let Some(path) = self.connections.get(name).cloned()
            && self.owners.get(&path).is_some_and(|held| held == name)
            && let Some(writer) = self.held.get_mut(&path)
        {
            let _ = writer.run(b"ROLLBACK");
            self.owners.remove(&path);
        }
        if let Some(path) = self.connections.get(name).cloned()
            && !self
                .connections
                .iter()
                .any(|(held, over)| held != name && over == &path)
            && let Some(writer) = self.held.get_mut(&path)
        {
            // A connection holds the databases it attached, which
            // `sqlite3Close` of `research/sqlite/src/main.c` gives up, so
            // the writer the session keeps for the path holds none once
            // the last connection over it closes. The temp schema is
            // given up below.
            for named in writer.attached_names() {
                if named.eq_ignore_ascii_case(b"temp") {
                    continue;
                }
                let mut sql = b"DETACH '".to_vec();
                sql.extend_from_slice(&named);
                sql.push(b'\'');
                let _ = writer.run(&sql);
            }
            writer.closing();
        }
        if let Some(path) = self.connections.get(name).cloned()
            && self.tempers.get(&path).is_some_and(|held| held == name)
        {
            self.tempers.remove(&path);
            if let Some(writer) = self.held.get_mut(&path) {
                let _ = writer.temps(None);
            }
        }
        self.connections.remove(name);
        self.temps.remove(name);
        self.nulls.remove(name);
        self.counters.remove(name);
        self.pragmas.remove(name);
        self.collations.remove(name);
        self.functions.remove(name);
        Vec::new()
    }

    /// Runs `sql` over the files the session holds, takes the machine to
    /// lose power part way through, and leaves the files as that machine
    /// left them.
    ///
    /// The arguments are the kind of crash, how long it waits, the file
    /// it counts on, the seed of its choices and the statements: `crashsql`
    /// of `research/sqlite/test/tester.tcl` counts the syncs of one file
    /// and `crash_on_write` counts the writes of every file. Both run the
    /// statements in a process of their own, which the crash ends, so the
    /// session opens the files again afterwards and recovers them.
    ///
    /// The answer is the two values `catch` leaves: one and a message
    /// where the crash happened, and nought and nothing where the
    /// statements ran to their end.
    fn crashed(&mut self, args: &[String]) -> Result<Vec<String>, String> {
        let kind = args.first().map_or("", String::as_str);
        // A script the tester runs one statement at a time opens the
        // child, evaluates statements on it, arms the crash again where
        // the script says so, says the machine loses power now, reads
        // whether the child holds a transaction, and ends the child.
        match kind {
            "open" => return self.crash_open(args, false),
            "eval" => return self.crash_eval(args.get(1).map_or("", String::as_str)),
            "arm" => return self.crash_arm(args),
            "now" => {
                let child = self.crash_child()?;
                child.now = true;
                return Ok(Vec::new());
            }
            "autocommit" => {
                let held = self.crash_child()?.writer.began();
                return Ok(alloc_one(if held { "0" } else { "1" }));
            }
            "close" => return self.crash_close(),
            _ => {}
        }
        self.crash_open(args, kind == "write")?;
        for statement in statements(args.get(4).map_or("", String::as_str)) {
            let text = statement.trim();
            if text.is_empty() {
                continue;
            }
            // A statement that only reads writes no byte of any file, so
            // it counts for nothing here and the writer takes none.
            if reads(text) {
                continue;
            }
            if self.crash_eval(text).is_err() {
                break;
            }
        }
        self.crash_close()
    }

    /// The child a crash opened.
    ///
    /// # Errors
    ///
    /// The words for a request that names no child.
    fn crash_child(&mut self) -> Result<&mut Crashed, String> {
        self.crashing
            .as_mut()
            .ok_or_else(|| "no crash is open".to_owned())
    }

    /// Opens the child a crash runs its statements in: the three files of
    /// the path as the child finds them, a connection over them, and
    /// where the machine loses power.
    ///
    /// The arguments are the kind of crash, how long it waits, the file
    /// it counts on and the seed of its choices. `counts_writes` says the
    /// machine counts the writes of every file rather than the syncs of
    /// one, which `crash_on_write` does.
    fn crash_open(&mut self, args: &[String], counts_writes: bool) -> Result<Vec<String>, String> {
        let delay = number_of(args.get(1).map_or("0", String::as_str))?;
        let named = args.get(2).map_or("", String::as_str);
        let seed = number_of(args.get(3).map_or("0", String::as_str))?;
        let path = pathed(named);
        let files = Crashing::new(self.files_of(path));
        let mut writer = files.opened()?;
        writer.defines(DEFINED);
        writer.groups(GROUPED);
        // `crashsql` of `research/sqlite/test/tester.tcl` holds the cache
        // of the child to ten pages, so the transaction writes pages into
        // the file before it commits.
        writer.caching(10);
        ticked(&mut writer, self.clock);
        self.crashing = Some(Crashed {
            path: path.to_owned(),
            files,
            writer,
            did: Vec::new(),
            counts_writes,
            delay,
            onto: onto_named(named),
            seed,
            now: false,
        });
        Ok(Vec::new())
    }

    /// Runs `sql` on the child and answers the values its statements
    /// answered, which a script of the child reads.
    fn crash_eval(&mut self, sql: &str) -> Result<Vec<String>, String> {
        let clock = self.clock;
        let child = self.crash_child()?;
        ticked(&mut child.writer, clock);
        let mut waiting = false;
        let (out, ran) = ran_each(
            &mut child.writer,
            sql,
            (&[], DEFINED, ""),
            (false, &mut waiting),
        );
        child.did.extend(child.writer.did());
        ran.map(|()| out)
    }

    /// Arms the crash of the child again: `sqlite3_crashparams` names how
    /// long it waits and the file it counts on, and a wait of nought is
    /// a machine that loses power at no sync at all.
    fn crash_arm(&mut self, args: &[String]) -> Result<Vec<String>, String> {
        let delay = number_of(args.get(1).map_or("0", String::as_str))?;
        let named = args.get(2).map_or("", String::as_str).to_owned();
        let child = self.crash_child()?;
        child.delay = delay;
        child.onto = onto_named(&named);
        Ok(Vec::new())
    }

    /// Ends the child: the machine loses power where the script said so
    /// or where the wait names a sync the child reached, the session
    /// opens the files the child left, and the answer is the two values
    /// `catch` leaves.
    fn crash_close(&mut self) -> Result<Vec<String>, String> {
        let mut child = self
            .crashing
            .take()
            .ok_or_else(|| "no crash is open".to_owned())?;
        let crashed = if child.delay == 0 {
            // A child that loses power at no sync writes every byte its
            // statements wrote, which a machine that loses power after
            // the last of them has on the disk already.
            for held in &child.did {
                child.files.does(held);
            }
            child.now
        } else if child.counts_writes {
            child.files.on_write(&child.did, child.delay)
        } else {
            child
                .files
                .on_sync(&child.did, child.delay, child.onto, child.seed)
        };
        let recovered = child.files.recovered()?;
        self.held.insert(child.path.clone(), recovered);
        if !crashed {
            return Ok(vec!["0".to_owned(), String::new()]);
        }
        let message = if child.counts_writes {
            "child killed: SIGABRT"
        } else {
            "child process exited abnormally"
        };
        Ok(vec!["1".to_owned(), message.to_owned()])
    }

    /// The three files a database is kept in, as the session holds them:
    /// the file itself, the rollback journal beside it and the
    /// write-ahead log beside it.
    fn files_of(&self, path: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let Some(writer) = self.held.get(path) else {
            return (Vec::new(), Vec::new(), Vec::new());
        };
        (
            writer.written(),
            writer.journal().unwrap_or_default().to_vec(),
            writer.log().unwrap_or_default().to_vec(),
        )
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

    /// What the limit of that name was on the connection, with the value
    /// set where it is not below nought, which is what `sqlite3_limit`
    /// answers.
    ///
    /// A name no limit carries answers minus one, which `sqlite3_limit`
    /// answers for a number out of range.
    fn limited(&mut self, name: &str, limit: &str, value: &str) -> Vec<String> {
        let Some(limit) = named_limit(limit) else {
            return vec![String::from("-1")];
        };
        let held = self.limits.entry(name.to_owned()).or_default();
        let value = value.parse::<i64>().unwrap_or(-1);
        vec![held.set(limit, value).to_string()]
    }

    /// Whether a file name that begins `file:` is read as a URI, which
    /// every connection opened after it reads one for.
    const fn reads_uri(&mut self, uri: bool) -> Vec<String> {
        self.uri = uri;
        Vec::new()
    }

    /// The counts `btree_pager_stats` answers for the connection, each
    /// with the name `sqlite3PagerStats` of
    /// `research/sqlite/src/pager.c:6900` writes it under.
    ///
    /// This harness holds the pages of a file as one image and no cache
    /// of its own, so the counts of a cache are nought: no page is held
    /// open between two statements, none stands in a cache, and none is
    /// read or missed there. The pages of the file and the pages
    /// written are counted, and the state is the one a pager that holds
    /// no file open reads.
    fn pager(&self, held: &str) -> Result<Vec<String>, String> {
        let (name, place) = named_place(held);
        // The temp schema is a database of the connection's own that no
        // file holds, so no page of it was written into a file.
        if place != 0 {
            let bytes = self.temps.get(name).cloned().unwrap_or_default();
            return Ok(pager_counts(pages_of(&bytes), 0));
        }
        let path = self
            .connections
            .get(name)
            .ok_or_else(|| format!("no such connection: {name}"))?;
        let held = self
            .held
            .get(path)
            .ok_or_else(|| format!("no such database: {path}"))?;
        let wrote = self.writes.get(path).copied().unwrap_or(0);
        Ok(pager_counts(pages_of(&held.written()), wrote))
    }

    /// Whether no file of the harness holds the database the name
    /// stands for, which a database of the connection's own and the
    /// temp schema are the two of.
    fn in_memory(&self, held: &str) -> bool {
        let (name, place) = named_place(held);
        if place != 0 {
            return true;
        }
        self.connections
            .get(name)
            .is_some_and(|path| path.contains('\0') || path.is_empty())
    }

    /// Opens a connection over a path, making the database where no
    /// connection has opened that path yet.
    ///
    /// A path of no bytes and `:memory:` each name a database of the
    /// connection's own, which `sqlite3BtreeOpen` of
    /// `research/sqlite/src/btree.c:2170` makes fresh for every
    /// connection and no other connection reads, so the harness holds
    /// one under a name no file has.
    fn open(&mut self, name: &str, path: &str, flags: (&str, &str)) -> Vec<String> {
        let (flag, only) = flags;
        let under = configured();
        let uri = self.uri || flag == "1";
        // `sqlite3 NAME FILE -readonly 1` opens the file with
        // `SQLITE_OPEN_READONLY`, and a connection opened again without
        // the option writes it.
        if only == "1" {
            self.readers.insert(name.to_owned());
        } else {
            self.readers.remove(name);
        }
        let named = match db_sqlite::uri::named(path.as_bytes(), uri) {
            Ok(named) => named,
            Err(refusal) => {
                self.connections.insert(name.to_owned(), path.to_owned());
                refused_as("SQLITE_ERROR", "1");
                return vec![refusal.message()];
            }
        };
        self.uris.insert(name.to_owned(), uri);
        let path = String::from_utf8_lossy(&named.path).into_owned();
        // A URI that asks for a database of the connection's own names
        // no file, which `mode=memory` of `sqlite3ParseUri` says.
        let path = if named.mode == Some(db_sqlite::uri::Mode::Memory) {
            ":memory:"
        } else {
            path.as_str()
        };
        // A database of the connection's own stands in memory alone where
        // the path names `:memory:`, and stands in a file the library
        // makes where the path holds no byte.
        let memory = path.eq_ignore_ascii_case(":memory:");
        // `zFile==0 ? "" : zFile` of
        // `research/sqlite/src/tclsqlite.c:4376`: a connection opened
        // under no file name writes a file the library makes and takes
        // away again, which no name of the client's names.
        let unnamed = path.is_empty();
        let held = if path.is_empty() || memory {
            self.opened = self.opened.saturating_add(1);
            format!("{path}\0{name}\0{}", self.opened)
        } else {
            simplified(path)
        };
        let path = held.as_str();
        // `sqlite3OsOpen` makes the file where it is not there, and
        // answers `SQLITE_CANTOPEN` where the directory that would hold
        // it is not there, which the machine says `ENOENT` for.
        if !path.starts_with("file:")
            && let Some(over) = Path::new(path).parent()
            && !over.as_os_str().is_empty()
            && !over.is_dir()
        {
            self.connections.insert(name.to_owned(), path.to_owned());
            self.errno = 2;
            refused_as("SQLITE_CANTOPEN", "14");
            return vec![String::from("unable to open database file")];
        }
        if !self.held.contains_key(path)
            && let Ok(mut writer) = Writer::new(under.page, 0, under.encoding)
        {
            writer.defines(DEFINED);
            writer.groups(GROUPED);
            ticked(&mut writer, self.clock);
            if memory {
                writer.memoried();
            } else if unnamed {
                writer.unnamed();
            }
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
        // `PRAGMA data_version` answers one on a connection that just
        // opened, whatever the file has had written to it.
        let commits = self
            .held
            .get(path)
            .map_or(0, db_sqlite::change::Writer::counted_commits);
        self.dated.insert(name.to_owned(), (1, commits));
        // `sqlite3_create_collation` holds a collation on one
        // connection, so a connection that opens again defines none, and
        // the temp tables of the connection that closed are gone.
        self.temps.remove(name);
        self.tempers.retain(|_, held| held != name);
        self.collations.remove(name);
        self.functions.remove(name);
        self.limits.remove(name);
        self.errno = 0;
        stood();
        vec![String::new()]
    }

    /// Keeps a collation the tester defined under its name, leaking the
    /// name and the list so that both outlive the file.
    ///
    /// A file defines a handful of collations, so leaking one list per
    /// definition costs O(n^2) bytes over n definitions and nothing
    /// that matters.
    fn collates(&mut self, connection: &str, name: &str) {
        self.stamped(connection);
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

    /// Turns the functions the C library keeps for its own use on where
    /// the connection holds none of them and off where it holds them,
    /// which `SQLITE_TESTCTRL_INTERNAL_FUNCTIONS` does.
    fn internals(&mut self, connection: &str) {
        self.stamped(connection);
        let held = self.functions.entry(connection.to_owned()).or_default();
        let on = held
            .iter()
            .any(|one| INTERNAL.iter().any(|carried| carried.name == one.name));
        let mut defined: Vec<Defined> = held
            .iter()
            .filter(|one| !INTERNAL.iter().any(|carried| carried.name == one.name))
            .copied()
            .collect();
        if defined.is_empty() {
            defined.extend_from_slice(DEFINED);
        }
        if !on {
            defined.extend_from_slice(INTERNAL);
        }
        *held = Box::leak(defined.into_boxed_slice());
    }

    /// Takes the collation of that name off the connection, which
    /// `sqlite3_create_collation` with no comparison function does.
    fn uncollates(&mut self, connection: &str, name: &str) {
        self.stamped(connection);
        let held = self.collations.entry(connection.to_owned()).or_default();
        let collating: Vec<Collating> = held
            .iter()
            .filter(|one| one.name != name.as_bytes())
            .copied()
            .collect();
        *held = Box::leak(collating.into_boxed_slice());
    }

    /// Keeps a function the tester defined under its name, beside the
    /// ones this harness holds, leaking the name and the list so that
    /// both outlive the file.
    ///
    /// The function takes any number of arguments, which `db function`
    /// of `testfixture` registers as `nArg` at -1.
    fn functions(&mut self, connection: &str, name: &str, safety: Safety) {
        self.stamped(connection);
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
            safety,
        });
        *held = Box::leak(defined.into_boxed_slice());
    }

    /// The name `sqlite3_create_collation`, `sqlite3_create_function`
    /// or `load_static_extension` adds to the connection, which the
    /// statements of the connection reach from there on.
    fn told(&mut self, verb: &str, connection: &str, named: (&str, &str)) {
        let (name, safety) = named;
        match verb {
            "collate" => self.collates(connection, name),
            "function" => self.functions(connection, name, safety_of(safety)),
            _ => self.extension(connection, name),
        }
    }

    /// `load_static_extension`: the functions the module named carries,
    /// where this harness holds them, registered on the connection the
    /// way `sqlite3_regexp_init` registers its own.
    ///
    /// A module this harness holds none of leaves the connection as it
    /// stands, so the functions it carries stay missing.
    fn extension(&mut self, connection: &str, module: &str) {
        let carried: &[Defined] = match module {
            "regexp" => EXTENDED,
            "decimal" => DECIMALS,
            _ => return,
        };
        self.stamped(connection);
        let held = self.functions.entry(connection.to_owned()).or_default();
        let mut defined: Vec<Defined> = held
            .iter()
            .filter(|one| !carried.iter().any(|beside| beside.name == one.name))
            .copied()
            .collect();
        if defined.is_empty() {
            defined.extend_from_slice(DEFINED);
        }
        defined.extend_from_slice(carried);
        *held = Box::leak(defined.into_boxed_slice());
        // `sqlite3_decimal_init` registers the collation `decimal` beside
        // the functions of the module.
        if module == "decimal" {
            let held = self.collations.entry(connection.to_owned()).or_default();
            let mut collating: Vec<Collating> = held
                .iter()
                .filter(|one| one.name != b"decimal")
                .copied()
                .collect();
            collating.push(Collating {
                name: b"decimal",
                by: decimal_collate,
            });
            *held = Box::leak(collating.into_boxed_slice());
        }
    }

    /// `db status (step|sort|autoindex|vmstep)` of `tclsqlite.c:3766`:
    /// what the last statement the connection ran counted. This harness
    /// counts no automatic index and no step of a program, so both
    /// answer nought.
    fn status(&self, connection: &str, what: &str) -> Result<Vec<String>, String> {
        let held = self.stepped.get(connection).copied().unwrap_or_default();
        let count = match what {
            "step" => held.steps,
            "sort" => held.sorts,
            "autoindex" | "vmstep" => 0,
            _ => {
                return Err("bad argument: should be autoindex, step, sort or vmstep".to_owned());
            }
        };
        Ok(alloc_one(&count.to_string()))
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
            "checkpoint" => self.checkpointed(args),
            "step" => self.step(first),
            "finalize" => Ok(self.finalize(first)),
            "reset" => Ok(self.reset_statement(first, false)),
            "clear_binds" => Ok(self.reset_statement(first, true)),
            "bind" => self.bind(first, second, third, args.get(3).map(String::as_str)),
            "column" => self.column(first, second, third),
            "next_stmt" => Ok(self.next_statement(first, second)),
            "readonly" | "busy" | "isexplain" | "expired" => {
                Ok(alloc_one(&self.statement_is(verb, first)))
            }
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
                // The tail begins after the semicolon, so the text of
                // the statement carries it, which is what
                // `sqlite3_sql` and `sqlite3_expanded_sql` answer.
                if sql.get(at.saturating_sub(1)..at) == Some(";") {
                    text.push(';');
                }
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
            origins: Vec::new(),
            rows: Vec::new(),
            at: 0,
            ran: false,
            row: false,
            legacy: false,
            cookie: self.cookie(connection),
            stamp: self.stamp(connection),
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
            prepared.origins = prepared.names.iter().map(|_| nowhere()).collect();
        } else if reads(&text) {
            let read = self.read_statement(connection, &bound_into(&text, &BTreeMap::new()));
            match read {
                Ok(answered) => {
                    prepared.names = texts(&answered.names);
                    prepared.declared = texts(&answered.declared);
                    prepared.origins = origins_of(&answered.origins);
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
    /// `sqlite3_stmt_readonly` answers one for and the three beside it
    /// nought.
    fn statement_is(&self, verb: &str, name: &str) -> String {
        let Some(prepared) = self.statements.get(name) else {
            return usize::from(verb == "readonly").to_string();
        };
        let answered = match verb {
            "busy" => i64::from(prepared.ran && prepared.row),
            "isexplain" => explaining(&prepared.sql),
            // `sqlite3_expired` answers whether the statement must be
            // made again, which a change to the schema since it was made
            // is what this harness holds.
            "expired" => i64::from(
                prepared.cookie != self.cookie(&prepared.connection)
                    || prepared.stamp != self.stamp(&prepared.connection),
            ),
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

    /// `sqlite3_wal_checkpoint_v2` of `research/sqlite/src/test1.c:7685`:
    /// the pages the log of the database `name` names are written into
    /// that file, or the pages of every database of the connection where
    /// the call names none, and the three values the C API writes.
    ///
    /// The arguments are the connection, the word of the mode and the
    /// name of the database, which is empty where the call names none.
    fn checkpointed(&mut self, args: &[String]) -> Result<Vec<String>, String> {
        let connection = args.first().map_or("", String::as_str);
        let word = args.get(1).map_or("", String::as_str);
        let named = args.get(2).map_or("", String::as_str);
        let mode = match word {
            "noop" => Checkpointing::Noop,
            "full" => Checkpointing::Full,
            "restart" => Checkpointing::Restart,
            "truncate" => Checkpointing::Truncate,
            _ => Checkpointing::Passive,
        };
        let path = self
            .connections
            .get(connection)
            .cloned()
            .ok_or_else(|| format!("no such connection: {connection}"))?;
        let writer = self
            .held
            .get_mut(&path)
            .ok_or_else(|| format!("no such database: {path}"))?;
        let schema = named.as_bytes().to_vec();
        let answered = writer
            .checkpointed(mode, (!named.is_empty()).then_some(schema).as_deref())
            .map_err(|error| refusal(&error))?;
        stood();
        Ok(answered.iter().map(|value| listed(value, "")).collect())
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
        answered_rows(writer, sql, collating, defines, false)
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
        // `sqlite3_prepare` holds the schema the statement was made
        // under, and a step past a change to it is refused
        // `SQLITE_SCHEMA`; `sqlite3_prepare_v2` makes the statement
        // again, which `sqlite3Reprepare` does.
        if held.legacy
            && (held.cookie != self.cookie(&connection) || held.stamp != self.stamp(&connection))
        {
            if let Some(held) = self.statements.get_mut(name) {
                held.ran = true;
                held.row = false;
            }
            return Err(stale());
        }
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
                held.origins = origins_of(&answered.origins);
            }
            held.row = !held.rows.is_empty();
        }
        Ok(())
    }

    /// Counts one more reason for the statements of the connection to be
    /// made again.
    fn stamped(&mut self, connection: &str) {
        let held = self.stamps.entry(connection.to_owned()).or_default();
        *held = held.saturating_add(1);
    }

    /// How many reasons the connection has given for its statements to be
    /// made again.
    fn stamp(&self, connection: &str) -> u64 {
        self.stamps.get(connection).copied().unwrap_or_default()
    }

    /// The number the schema of the connection stands at, which every
    /// change to it raises and `PRAGMA schema_version` answers.
    ///
    /// Reading the header costs O(1) over the image the writer holds.
    fn cookie(&self, connection: &str) -> u32 {
        let Some(path) = self.connections.get(connection) else {
            return 0;
        };
        let Some(writer) = self.held.get(path) else {
            return 0;
        };
        let bytes = writer.written();
        db_sqlite::image::Image::open(&bytes)
            .map(|image| image.header().schema_cookie)
            .unwrap_or_default()
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
            .run(&sql_bytes(sql))
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
    fn bind(
        &mut self,
        name: &str,
        at: &str,
        value: &str,
        kind: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let place: usize = at
            .parse()
            .map_err(|source| format!("a place is a whole number: {source}"))?;
        // A text and a blob are carried as the digits of their bytes,
        // because a value of the tester may hold a quote, a nought and
        // bytes that are no text at all; every other kind is carried as
        // the literal the tester wrote.
        let value = match kind {
            Some("text") => quoted_text(&bytes_of_hex(value)),
            Some("blob") => format!("X'{value}'"),
            _ => value.to_owned(),
        };
        let value = value.as_str();
        if let Some(held) = self.statements.get_mut(name) {
            // `sqlite3_bind_*` refuses a place below one and one past
            // the parameters the statement holds, which
            // `vdbeUnbind` of `research/sqlite/src/vdbeapi.c` answers
            // `SQLITE_RANGE` for.
            if place < 1 || place > count_binds(&held.sql) {
                refused_as("SQLITE_RANGE", "25");
                return Err(String::from("column index out of range"));
            }
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
        // `sqlite3_column_database_name`, `sqlite3_column_table_name`
        // and `sqlite3_column_origin_name`.
        if let Some(part) = ["database", "table", "origin"]
            .iter()
            .position(|held| *held == which)
        {
            let named = held
                .origins
                .get(place)
                .and_then(|origin| origin.get(part))
                .cloned()
                .unwrap_or_default();
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
        self.stamped(connection);
        if name.is_empty() {
            self.authorizers.remove(connection);
        } else {
            self.authorizers.insert(connection.to_owned(), true);
        }
    }

    /// `sqlite3_set_authorizer`, `sqlite3_commit_hook`,
    /// `sqlite3_rollback_hook` and `sqlite3_update_hook`: the tester
    /// names a script for one connection, or nothing to tell it none.
    fn hooks(&mut self, verb: &str, connection: &str, name: &str) {
        if verb == "authorizer" {
            self.authorizes(connection, name);
            return;
        }
        let held = self.hooked.entry(connection.to_owned()).or_default();
        if name.is_empty() {
            held.remove(verb);
        } else {
            held.insert(verb.to_owned());
        }
    }

    /// The functions one connection reads, which are the ones this
    /// harness holds where the tester defined none.
    fn defines(&self, connection: &str) -> &'static [Defined] {
        self.functions.get(connection).copied().unwrap_or(DEFINED)
    }

    /// One database written again under another path, which is what a
    /// file that saves itself and reads the save back asks for.
    /// `sqlite3_backup_init` of `research/sqlite/src/backup.c:129` and
    /// the calls that drive the backup it answers: `init` begins one,
    /// `step` copies pages, `remaining` and `pagecount` count them,
    /// `finish` ends it, and `file` and `into` are the `backup` and
    /// `restore` methods of a connection, which write the database into
    /// a file and read it back.
    ///
    /// A step copies the image the source held when the backup began; a
    /// step that finds the source written since then begins the backup
    /// again, which `sqlite3BackupRestart` does. The destination is
    /// written when the last page is copied, because
    /// `sqlite3_backup_finish` rolls the destination back where the
    /// backup did not run to its end.
    ///
    /// # Errors
    ///
    /// The words the C API writes for a backup it refuses to begin.
    fn backed(&mut self, args: &[String]) -> Result<Vec<String>, String> {
        let verb = args.first().map_or("", String::as_str);
        let held = args.get(1).map_or("", String::as_str).to_owned();
        if verb == "init" {
            return self.backup_init(args);
        }
        if verb == "file" || verb == "into" {
            return self.backup_file(verb, args);
        }
        let from = self
            .backups
            .get(&held)
            .map(|backing| backing.from.clone())
            .ok_or_else(|| format!("no such backup: {held}"))?;
        let wrote = self.wrote_of(&from)?;
        let written = self
            .backups
            .get(&held)
            .is_some_and(|backing| backing.wrote != wrote);
        let pages = if written {
            u32::try_from(pages_of(&self.image_of(&from)?)).unwrap_or(0)
        } else {
            0
        };
        let backing = self
            .backups
            .get_mut(&held)
            .ok_or_else(|| format!("no such backup: {held}"))?;
        if written {
            backing.wrote = wrote;
            backing.pages = pages;
            backing.next = 1;
        }
        let pages = backing.pages;
        match verb {
            "remaining" => Ok(alloc_one(
                &pages
                    .saturating_add(1)
                    .saturating_sub(backing.next)
                    .to_string(),
            )),
            "pagecount" => Ok(alloc_one(&pages.to_string())),
            "finish" => {
                self.backups.remove(&held);
                Ok(alloc_one("SQLITE_OK"))
            }
            "step" => {
                // `sqlite3_backup_step` of
                // `research/sqlite/src/backup.c:408` answers
                // `SQLITE_BUSY` where a connection holds a transaction on
                // the source or on the destination, because the pages it
                // would copy are not the ones a commit left.
                let into = backing.into.clone();
                if self.began_of(&from)? || self.began_of(&into)? {
                    return Ok(alloc_one("SQLITE_BUSY"));
                }
                let backing = self
                    .backups
                    .get_mut(&held)
                    .ok_or_else(|| format!("no such backup: {held}"))?;
                let asked = args
                    .get(2)
                    .map_or("", String::as_str)
                    .parse::<i64>()
                    .unwrap_or(0);
                let copied = if asked < 0 {
                    pages
                } else {
                    u32::try_from(asked).unwrap_or(0)
                };
                backing.next = backing
                    .next
                    .saturating_add(copied)
                    .min(pages.saturating_add(1));
                if backing.next <= pages {
                    return Ok(alloc_one("SQLITE_OK"));
                }
                let path = self.path_of(&into)?;
                let bytes = held_alone(self.image_of(&from)?, &path);
                self.opened_again(&path, (bytes, Vec::new(), Vec::new()))?;
                self.stamped(&into);
                Ok(alloc_one("SQLITE_DONE"))
            }
            other => Err(format!("this harness has no backup {other}")),
        }
    }

    /// Begins one backup: the connection it writes, the connection it
    /// reads and the pages the source holds.
    ///
    /// # Errors
    ///
    /// The words the C API writes for a backup it refuses to begin.
    fn backup_init(&mut self, args: &[String]) -> Result<Vec<String>, String> {
        let held = args.get(1).map_or("", String::as_str).to_owned();
        let into = args.get(2).map_or("", String::as_str).to_owned();
        let from = args.get(4).map_or("", String::as_str).to_owned();
        for (connection, schema) in [(&into, args.get(3)), (&from, args.get(5))] {
            let named = schema.map_or("", String::as_str);
            if !named.eq_ignore_ascii_case("main") {
                return Err(format!("unknown database {named}"));
            }
            if !self.connections.contains_key(connection) {
                return Err(format!("no such connection: {connection}"));
            }
        }
        if self.path_of(&into)? == self.path_of(&from)? {
            return Err("source and destination must be distinct".to_owned());
        }
        let pages = u32::try_from(pages_of(&self.image_of(&from)?)).unwrap_or(0);
        let wrote = self.wrote_of(&from)?;
        self.backups.insert(
            held,
            Backing {
                into,
                from,
                next: 1,
                pages,
                wrote,
            },
        );
        Ok(Vec::new())
    }

    /// The `backup` and `restore` methods of a connection: the database
    /// written into a file the session holds, and the file read back.
    ///
    /// # Errors
    ///
    /// The words for a schema that is not `main` and for a name no
    /// connection carries.
    fn backup_file(&mut self, verb: &str, args: &[String]) -> Result<Vec<String>, String> {
        let held = args.get(1).map_or("", String::as_str).to_owned();
        let schema = args.get(2).map_or("", String::as_str);
        if !schema.eq_ignore_ascii_case("main") {
            return Err(format!("unknown database {schema}"));
        }
        let file = args.get(3).map_or("", String::as_str).to_owned();
        if verb == "file" {
            let image = held_alone(self.image_of(&held)?, &file);
            return self.opened_again(&file, (image, Vec::new(), Vec::new()));
        }
        let path = self.path_of(&held)?;
        let image = held_alone(self.bytes_of(&file).unwrap_or_default(), &path);
        self.opened_again(&path, (image, Vec::new(), Vec::new()))
    }

    /// The path a connection opened.
    ///
    /// # Errors
    ///
    /// The words for a name no connection carries.
    fn path_of(&self, connection: &str) -> Result<String, String> {
        self.connections
            .get(connection)
            .cloned()
            .ok_or_else(|| format!("no such connection: {connection}"))
    }

    /// The pages the connections over the database of a connection have
    /// written, which says whether it was written since a backup read it.
    ///
    /// # Errors
    ///
    /// The words for a name no connection carries.
    fn wrote_of(&self, connection: &str) -> Result<u64, String> {
        let path = self.path_of(connection)?;
        Ok(self.writes.get(&path).copied().unwrap_or(0))
    }

    /// The image the database of a connection holds, with the pages a log
    /// beside it has taken, which is the file a reader of that connection
    /// sees and the file a backup copies.
    ///
    /// # Errors
    ///
    /// The words for a name no connection carries.
    fn image_of(&self, connection: &str) -> Result<Vec<u8>, String> {
        let path = self.path_of(connection)?;
        let writer = self
            .held
            .get(&path)
            .ok_or_else(|| format!("no such database: {path}"))?;
        Ok(writer.inside())
    }

    /// Whether a connection holds a transaction, which a backup over its
    /// database answers `SQLITE_BUSY` for.
    ///
    /// # Errors
    ///
    /// The words for a name no connection carries.
    fn began_of(&self, connection: &str) -> Result<bool, String> {
        let path = self.path_of(connection)?;
        Ok(self
            .held
            .get(&path)
            .is_some_and(db_sqlite::change::Writer::began))
    }

    fn copy(&mut self, from: &str, to: &str) -> Result<Vec<String>, String> {
        let Some(bytes) = self.bytes_of(from) else {
            self.removed(to);
            return Ok(Vec::new());
        };
        let (base, tail) = named_beside(to);
        let mut files = self.files_of(base);
        match tail {
            Some("wal") => files.2 = bytes,
            Some("journal") => files.1 = bytes,
            _ => files.0 = bytes,
        }
        self.opened_again(base, files)
    }

    /// One file the session holds given up, which `forcedelete` asks
    /// for: the database with the log and the journal beside it, or one
    /// of those two alone, which leaves the database where it is.
    fn removed(&mut self, name: &str) -> Vec<String> {
        self.readonly.remove(name);
        if self.held.remove(name).is_some() {
            self.raw.remove(name);
            return Vec::new();
        }
        let (base, tail) = named_beside(name);
        let mut files = self.files_of(base);
        match tail {
            Some("wal") => files.2 = Vec::new(),
            Some("journal") => files.1 = Vec::new(),
            _ => return Vec::new(),
        }
        let _ = self.opened_again(base, files);
        Vec::new()
    }

    /// The connection over `path` made again out of the three files, which
    /// is what a copy or a removal of one of them leaves.
    ///
    /// # Errors
    ///
    /// The message the engine refused the files with.
    fn opened_again(
        &mut self,
        path: &str,
        files: (Vec<u8>, Vec<u8>, Vec<u8>),
    ) -> Result<Vec<String>, String> {
        // A journal or a log beside the file is played back as the
        // writer opens, so the bytes the machine wrote are no longer the
        // file and only a database that stands alone keeps them.
        let alone = files.1.is_empty() && files.2.is_empty();
        let bytes = files.0.clone();
        let mut writer = Crashing::new(files).opened()?;
        writer.defines(DEFINED);
        writer.groups(GROUPED);
        let image = writer.written();
        self.held.insert(path.to_owned(), writer);
        if alone {
            self.machined(path, bytes, image);
        } else {
            self.raw.remove(path);
        }
        Ok(Vec::new())
    }

    /// `eval DB SQL` and `names DB SQL`, which answer the rows and the
    /// column names of a statement, and `exec DB SQL` and `exec_names DB
    /// SQL` beside them, which read the `%XX` escapes of the statement as
    /// the bytes they name: that is how a file writes a byte it cannot
    /// hold as text.
    ///
    /// # Errors
    ///
    /// The message the engine refused the statement with.
    fn of_sql(&mut self, verb: &str, name: &str, sql: &str) -> Result<Vec<String>, String> {
        self.cannot_open(name)?;
        let sql = if verb.starts_with("exec") {
            escaped(sql)
        } else {
            sql.to_owned()
        };
        if verb.ends_with("names") {
            return self.names(name, &sql);
        }
        // `DB eval SQL SCRIPT` of `research/sqlite/src/tclsqlite.c:2087`
        // runs the script once per row whatever the count of columns is,
        // so a statement of no column is run for its count of rows.
        if verb == "rows" {
            self.eval(name, &sql)?;
            return Ok(alloc_one(&ANSWERED.with(core::cell::Cell::get).to_string()));
        }
        self.eval(name, &sql)
    }

    /// The temp schema of the connection this request names handed to the
    /// writer of its path, where the request reads one.
    ///
    /// A request that names a statement names its connection through it.
    fn tempering(&mut self, verb: &str, first: &str) {
        match verb {
            "eval" | "names" | "exec" | "exec_names" | "prepare" | "deserialize" | "serialize"
            | "columnmeta" | "blob_bytes" | "blob_read" | "blob_write" => self.tempered(first),
            "step" | "column" | "stmt" | "bind" | "finalize" | "reset" | "clear_binds" => {
                if let Some(connection) = self
                    .statements
                    .get(first)
                    .map(|held| held.connection.clone())
                {
                    self.tempered(&connection);
                }
            }
            _ => {}
        }
    }

    /// The writer of the path the connection reads given the temp schema
    /// of that connection, and the one it carried handed back to
    /// whichever connection made it.
    ///
    /// A temp table belongs to the connection that made it, which
    /// `sqlite3TwoPartName` of `research/sqlite/src/build.c` writes into
    /// the schema at place one, and this harness holds one writer per
    /// path, so the writer carries the temp schema of the connection
    /// that last read it. Swapping one costs O(n) in the pages of the
    /// two temp schemas.
    fn tempered(&mut self, connection: &str) {
        let Some(path) = self.connections.get(connection).cloned() else {
            return;
        };
        let held = self.tempers.get(&path).cloned();
        if held.as_deref() == Some(connection) {
            return;
        }
        if let Some(made) = held
            && let Some(writer) = self.held.get(&path)
        {
            match writer.temp() {
                Some(bytes) => {
                    self.temps.insert(made, bytes);
                }
                None => {
                    self.temps.remove(&made);
                }
            }
        }
        let wanted = self.temps.get(connection).cloned();
        if let Some(writer) = self.held.get_mut(&path) {
            let _ = writer.temps(wanted.as_deref());
        }
        self.tempers.insert(path, connection.to_owned());
    }

    /// The statements of one text, in order, answered as one list.
    ///
    /// # Errors
    ///
    /// The message the engine refused the statement with.
    fn eval(&mut self, name: &str, sql: &str) -> Result<Vec<String>, String> {
        if names_file(sql) {
            self.telling_files();
        }
        if names_word(sql, b"attach") {
            // `sqlite3DetachDatabase` makes every statement of the
            // connection again, because the databases it reads have
            // moved.
            self.stamped(name);
        }
        let null = self.nulls.get(name).cloned().unwrap_or_default();
        let path = self
            .connections
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no such connection: {name}"))?;
        let counted = self.counters.get(name).copied().unwrap_or_default();
        let mut kept = self.pragmas.get(name).cloned().unwrap_or_default();
        let collating = self.collations.get(name).copied().unwrap_or_default();
        let defines = self.defines(name);
        let asks = self.authorizers.contains_key(name);
        let hooked = self.hooked.get(name).cloned().unwrap_or_default();
        // A connection that did not begin the transaction open on the
        // path reads the file as that transaction found it.
        let outside = self.owners.get(&path).is_some_and(|held| held != name);
        let uri = self.uris.get(name).copied().unwrap_or(false);
        let limits = self.limits.get(name).copied().unwrap_or_default();
        // The client may only read the file where its permissions say so
        // or where this connection was opened that way.
        let reading = self.readonly.contains(&path) || self.readers.contains(name);
        let writer = self
            .held
            .get_mut(&path)
            .ok_or_else(|| format!("no such database: {path}"))?;
        // The counters belong to the connection and the pages to the
        // file, so the writer stands at this connection's counters for
        // the statements of this request and answers them back.
        writer.counts_as(counted);
        // The commits another connection made since this one last read
        // the file each raise its `data_version` by one.
        let commits = writer.counted_commits();
        let dated = self.dated.entry(name.to_owned()).or_insert((1, commits));
        let held = i64::from(commits.saturating_sub(dated.1));
        dated.0 = dated.0.saturating_add(held);
        dated.1 = commits;
        kept.tells(b"data_version", dated.0);
        writer.kept_as(kept);
        writer.collates(collating);
        writer.defines(defines);
        if asks {
            writer.asks(asking);
        } else {
            writer.asks_nothing();
        }
        hooking(writer, &hooked);
        WHO.with(|who| who.borrow_mut().clone_from(&name.to_owned()));
        NULLED.with(|text| text.borrow_mut().clone_from(&null));
        writer.opens(opening);
        writer.in_zone(zoned);
        writer.reads_uri(uri);
        writer.limited(limits);
        writer.only_reading(reading);
        let mut waiting = self.waiting.contains(name);
        let (out, ran) = ran_each(
            writer,
            sql,
            (collating, defines, &null),
            (outside, &mut waiting),
        );
        if waiting {
            self.waiting.insert(name.to_owned());
        } else {
            self.waiting.remove(name);
        }
        // `sqlite3OsSync` counts every sync, and counts it twice where
        // `PRAGMA fullfsync` says the file is held on the disk of the
        // machine rather than in the cache of its driver.
        // The commits this connection made are ones it has seen, so its
        // own writes raise no `data_version` of its own.
        let after = writer.counted_commits();
        let did = writer.did();
        let syncs = did
            .iter()
            .filter(|held| matches!(held, Does::Sync(_)))
            .count();
        let wrote = wrote_pages(&did);
        let counted = writer.counts();
        let kept = writer.kept();
        let full = kept.fullfsync();
        let began = writer.began();
        let files = writer.attached_files();
        let beside = (writer.attached_logs(), writer.attached_journals());
        if began {
            self.owners.entry(path.clone()).or_insert(name.to_owned());
        } else {
            self.owners.remove(&path);
        }
        let syncs = u64::try_from(syncs).unwrap_or(0);
        let held = self.writes.entry(path.clone()).or_default();
        *held = held.saturating_add(wrote);
        self.synced.0 = self.synced.0.saturating_add(syncs);
        if full {
            self.synced.1 = self.synced.1.saturating_add(syncs);
        }
        let stepped = STEPPED.with(core::cell::Cell::take);
        self.sorted = stepped.sorts;
        self.searched = stepped.searched;
        self.stepped.insert(name.to_owned(), stepped);
        self.counters.insert(name.to_owned(), counted);
        self.pragmas.insert(name.to_owned(), kept);
        if let Some(held) = self.dated.get_mut(name) {
            held.1 = after;
        }
        self.mirror(&path, files, &beside);
        ran?;
        Ok(out)
    }

    /// The image of every attached database written back into the file
    /// the session holds under its name, so a connection over that path
    /// reads what the statement wrote, except for `held`, which is the
    /// path the connection itself reads.
    ///
    /// Reading one file again costs O(n) in its pages.
    fn mirror(&mut self, held: &str, files: Vec<(Vec<u8>, Vec<u8>)>, beside: &Beside) {
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
            // The copy is opened out of the image alone, so the log and
            // the journal beside the file are kept where the size of
            // either is read.
            let found = |held: &Vec<(Vec<u8>, Vec<u8>)>| {
                held.iter()
                    .find(|(name, _)| *name == file)
                    .map(|(_, bytes)| bytes.clone())
            };
            self.beside
                .insert(path.clone(), (found(&beside.0), found(&beside.1)));
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
        // A statement that writes names the columns of its `RETURNING`,
        // which the engine reads out of the statement without running it.
        let returning = statements(sql)
            .into_iter()
            .map(|text| writer.returning_names(&sql_bytes(text.trim())))
            .rfind(|names| !names.is_empty());
        if let Some(names) = returning {
            return Ok(names
                .iter()
                .map(|name| String::from_utf8_lossy(name).into_owned())
                .collect());
        }
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
                    .trusting(writer.trusts_schema())
                    .limited(writer.limits())
                    .encoded(writer.encoding())
                    .journalling(writer.journalled())
                    .in_zone(zoned);
                match held {
                    Some(seconds) => database.clocked(seconds),
                    None => database,
                }
            })
            .and_then(|database| attaching(database, &beside))
            .map_err(|error| refusal(&error))?;
        let answered = database
            .query(&sql_bytes(last))
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
                    shortened(args.get(2).map_or("", String::as_str)),
                    shortened(args.get(3).map_or("", String::as_str))
                ));
            }
        }
    }
}

/// The first bytes of a value a case answered, which is what the report
/// of a case that answered differently writes.
///
/// A run of one file writes its report onto a pipe the run of every file
/// reads once the file ended, so a value longer than the pipe holds would
/// stop the file until the deadline ended it. The tail of a value tells a
/// reader nothing the head does not.
fn shortened(held: &str) -> String {
    const MOST: usize = 300;
    if held.len() <= MOST {
        return held.to_owned();
    }
    let mut out: String = held.chars().take(MOST).collect();
    out.push_str("...");
    out
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
    /// Which zone `localtime` and `utc` read, which
    /// `SQLITE_TESTCTRL_LOCALTIME_FAULT` names: nought the zone of the
    /// machine, one a zone that fails, and two the zone
    /// `testLocaltime` answers.
    static ZONED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };

    /// How many rows the last statement of a run answered, which a
    /// statement of no column answers as many of as one that carries
    /// columns, because `run_one` carries no connection.
    static ANSWERED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };

    /// What the walks and the sorts of the last statement counted, which
    /// `db status` answers, because `run_one` carries no connection.
    static STEPPED: core::cell::Cell<db_sqlite::db::Stepped> =
        const { core::cell::Cell::new(db_sqlite::db::Stepped { steps: 0, sorts: 0, searched: 0 }) };

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

/// Records the code the tester refused a call with, which the harness
/// itself did not answer.
fn refused_as(name: &str, number: &str) {
    CODE.with(|held| {
        *held.borrow_mut() = (
            name.to_owned(),
            name.to_owned(),
            number.parse().unwrap_or(1),
        );
    });
}

/// Records that the last call of the connection stood, which
/// `sqlite3_errcode` answers `SQLITE_OK` for.
fn stood() {
    CODE.with(|held| {
        *held.borrow_mut() = (String::new(), String::new(), 0);
    });
}

/// Records that the schema of the connection changed since the statement
/// was made, which `sqlite3_step` of a statement `sqlite3_prepare` made
/// is refused with.
pub(crate) fn stale() -> String {
    CODE.with(|held| {
        *held.borrow_mut() = (
            String::from("SQLITE_SCHEMA"),
            String::from("SQLITE_SCHEMA"),
            17,
        );
    });
    String::from("database schema has changed")
}

/// The code of the last refusal, as the name of the primary code, the
/// name of the extended one or the number of the primary one, which is
/// `SQLITE_OK` where the statements of this run all stood.
pub(crate) fn last_code(which: &str) -> String {
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

/// Whether the commit hook the tester named refuses the commit, which is
/// one `CALL` onto the line and the `RET` that answers it.
///
/// A script that answers other than a number lets the commit stand,
/// which is what `tclsqlite.c` reads for it.
fn committing() -> bool {
    hooked_answer("commit_hook")
        .and_then(|text| text.trim().parse::<i64>().ok())
        .is_some_and(|answered| answered != 0)
}

/// Tells the rollback hook the tester named that a transaction went
/// back, which is one `CALL` onto the line and the `RET` that answers
/// it.
fn rolling() {
    drop(hooked_answer("rollback_hook"));
}

/// Tells the update hook the tester named of one row a statement wrote,
/// which is one `CALL` onto the line and the `RET` that answers it.
///
/// Costs one round trip per row, so a statement that writes n rows costs
/// O(n) of them.
fn writing(wrote: &db_sqlite::change::Wrote<'_>) {
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    LINE.with(|line| {
        let mut held = line.borrow_mut();
        let Some(line) = held.as_mut() else {
            return;
        };
        let values = [
            WHO.with(|who| who.borrow().clone()),
            text(wrote.did.word()),
            text(wrote.schema),
            text(wrote.table),
            wrote.rowid.to_string(),
        ];
        if write_call(&mut line.writer, "update_hook", &values).is_ok() {
            drop(returned(&mut line.reader));
        }
    });
}

/// Tells the preupdate hook the tester named of one row a statement is
/// about to write, which is one `CALL` onto the line and the `RET` that
/// answers it.
///
/// The call carries the row itself, so the tester answers
/// `sqlite3_preupdate_old`, `sqlite3_preupdate_new`,
/// `sqlite3_preupdate_count` and `sqlite3_preupdate_depth` out of what it
/// was handed rather than asking the engine again, which the line has no
/// path for. A row of n columns costs O(n) bytes on the line.
fn peeking(peeked: &db_sqlite::change::Peeked<'_>) {
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let mut values = vec![
        WHO.with(|who| who.borrow().clone()),
        text(peeked.did.word()),
        text(peeked.schema),
        text(peeked.table),
        peeked.was.to_string(),
        peeked.key.to_string(),
        peeked.depth.to_string(),
    ];
    for row in [peeked.old, peeked.new] {
        match row {
            None => values.push(String::from("-1")),
            Some(row) => {
                values.push(row.len().to_string());
                values.extend(row.iter().map(|value| listed(value, "")));
            }
        }
    }
    LINE.with(|line| {
        let mut held = line.borrow_mut();
        let Some(line) = held.as_mut() else {
            return;
        };
        if write_call(&mut line.writer, "preupdate", &values).is_ok() {
            drop(returned(&mut line.reader));
        }
    });
}

/// What the script one connection named answers, written as `CALL` with
/// the name of the connection.
fn hooked_answer(kind: &str) -> Option<String> {
    LINE.with(|line| {
        let mut held = line.borrow_mut();
        let line = held.as_mut()?;
        let values = [WHO.with(|who| who.borrow().clone())];
        write_call(&mut line.writer, kind, &values).ok()?;
        returned(&mut line.reader)
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
        // `tclSqlFunc` of `research/sqlite/src/tclsqlite.c:1046` builds
        // one Tcl object per argument, of the kind the value is, so a
        // proc that answers its argument answers an object of that kind
        // and `value_kind` reads it back. The line carries text, so the
        // kind stands in front of every value.
        for value in args {
            values.push(String::from(match value {
                Value::Int(_) => "int",
                Value::Real(_) => "real",
                _ => "text",
            }));
            values.push(listed(value, &null));
        }
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

/// What a schema object may do with a function the tester defined, which
/// is what `-innocuous` and `-directonly` of `db function` name and which
/// `SQLITE_INNOCUOUS` and `SQLITE_DIRECTONLY` carry into the library.
fn safety_of(word: &str) -> Safety {
    match word {
        "innocuous" => Safety::Innocuous,
        "direct" => Safety::Direct,
        _ => Safety::Unsafe,
    }
}

/// The functions `testfixture` defines that this harness answers,
/// which SQLite's own files call in their statements.
static DEFINED: &[Defined] = &[Defined {
    name: b"randstr",
    count: Some(2),
    answer: randstr,
    safety: Safety::Innocuous,
}];

/// The functions the C library keeps for its own use, which
/// `SQLITE_TESTCTRL_INTERNAL_FUNCTIONS` of
/// `research/sqlite/src/main.c:4290` turns on for one connection and off
/// again; a statement of any other connection is refused `no such
/// function`.
static INTERNAL: &[Defined] = &[
    Defined {
        name: b"sqlite_rename_table",
        count: Some(7),
        answer: rename_table,
        safety: Safety::Innocuous,
    },
    // `affinity(X)` answers the name of the affinity of the expression
    // `X` is, which the engine reads off the walk of that expression, so
    // this stands for the name alone.
    Defined {
        name: b"affinity",
        count: Some(1),
        answer: nothing,
        safety: Safety::Innocuous,
    },
];

/// A function the engine answers itself, which this stands in the list
/// for.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the shape every function the application defines answers in"
)]
const fn nothing(
    _name: &'static [u8],
    _args: &[Value],
    _random: Option<&Source>,
) -> Result<Value, db_sqlite::eval::Error> {
    Ok(Value::Null)
}

/// `sqlite_rename_table(DB, TYPE, OBJECT, SQL, OLD, NEW, TEMP)`, which is
/// `renameTableFunc` of `research/sqlite/src/alter.c:1754`: the statement
/// with every place that names `OLD` written under `NEW` in quotes, which
/// is what an `ALTER TABLE ... RENAME TO` writes into the schema.
///
/// A statement the parser refuses is refused here as well, which
/// `renameParseSql` of the same file answers an error for.
///
/// It costs what reading the statement costs.
fn rename_table(
    _: &'static [u8],
    args: &[Value],
    _: Option<&Source>,
) -> Result<Value, db_sqlite::eval::Error> {
    let text = |at: usize| args.get(at).and_then(Value::text).unwrap_or_default();
    let sql = text(3);
    if db_sqlite::parse::definition(&sql).is_err() {
        return Err(db_sqlite::eval::Error::Malformed);
    }
    let places = db_sqlite::rename::places(&sql, &text(4), db_sqlite::rename::Marking::Every);
    Ok(Value::Text(db_sqlite::rename::written(
        &sql,
        &places,
        &text(5),
    )))
}

/// `sqlite3BitvecBuiltinTest` of `research/sqlite/src/bitvec.c:400`: the
/// program is run against a bit vector and against a bare array of bits
/// beside it, and the answer is the first bit the two disagree on, or
/// nought where they agree on every bit.
///
/// The words of the program are an operation and its arguments: one sets
/// a run of bits, two clears one, three sets a bit drawn at random, four
/// clears one drawn at random, and five sets the bit of the array alone,
/// which is how a case asks for a disagreement. A word of six or more is
/// passed over, and nought ends the program.
///
/// One run costs O(n) in the bits the program names.
fn bitvec_test(size: i64, program: &str) -> i64 {
    let mut held: Vec<i64> = program
        .split_ascii_whitespace()
        .filter_map(|word| word.parse::<i64>().ok())
        .collect();
    if size <= 0 {
        return -1;
    }
    let width = usize::try_from(size).unwrap_or(0).saturating_add(2);
    let mut vector = Bits::of(width);
    let mut beside = Bits::of(width);
    let mut drawn = 1_u64;
    let mut at = 0_usize;
    while let Some(op) = held.get(at).copied().filter(|op| *op != 0) {
        if op >= 6 {
            at = at.saturating_add(1);
            continue;
        }
        let (mut bit, mut step) = if matches!(op, 1 | 2 | 5) {
            let bit = word_at(&held, at.saturating_add(2)).saturating_sub(1);
            let by = word_at(&held, at.saturating_add(3));
            put_word(
                &mut held,
                at.saturating_add(2),
                bit.saturating_add(1).saturating_add(by),
            );
            (bit, 4)
        } else {
            // `sqlite3_randomness` draws the bit, and the two sides of the
            // comparison are told the same bit, so the draw itself says
            // nothing about the answer.
            drawn = drawn
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (i64::try_from(drawn >> 33_u32).unwrap_or(0), 2)
        };
        let left = word_at(&held, at.saturating_add(1)).saturating_sub(1);
        put_word(&mut held, at.saturating_add(1), left);
        if left > 0 {
            step = 0;
        }
        at = at.saturating_add(step);
        bit = (bit & 0x7fff_ffff).checked_rem(size).unwrap_or(0);
        let bit = usize::try_from(bit.saturating_add(1)).unwrap_or(0);
        if op & 1 != 0 {
            beside.set(bit, true);
            if op != 5 {
                vector.set(bit, true);
            }
        } else {
            beside.set(bit, false);
            vector.set(bit, false);
        }
    }
    for bit in 1..=usize::try_from(size).unwrap_or(0) {
        if beside.holds(bit) != vector.holds(bit) {
            return i64::try_from(bit).unwrap_or(0);
        }
    }
    0
}

/// One word of the program, and nought past its end.
fn word_at(held: &[i64], at: usize) -> i64 {
    held.get(at).copied().unwrap_or(0)
}

/// One word of the program written, where the program holds that many
/// words.
fn put_word(held: &mut [i64], at: usize, word: i64) {
    for slot in held.iter_mut().skip(at).take(1) {
        *slot = word;
    }
}

/// An array of bits, one bit per bit and not one per byte.
struct Bits {
    /// The words the bits stand in, sixty-four bits per word.
    held: Vec<u64>,
}

impl Bits {
    /// An array of `width` bits, every one of them nought.
    fn of(width: usize) -> Self {
        Bits {
            held: vec![0; width.saturating_div(64).saturating_add(1)],
        }
    }

    /// One bit written.
    fn set(&mut self, at: usize, held: bool) {
        let word = at.saturating_div(64);
        let bit = 1_u64 << u32::try_from(at % 64).unwrap_or(0);
        for slot in self.held.iter_mut().skip(word).take(1) {
            if held {
                *slot |= bit;
            } else {
                *slot &= !bit;
            }
        }
    }

    /// Whether one bit is written.
    fn holds(&self, at: usize) -> bool {
        let word = at.saturating_div(64);
        let bit = 1_u64 << u32::try_from(at % 64).unwrap_or(0);
        self.held.get(word).is_some_and(|slot| slot & bit != 0)
    }
}

/// The functions `ext/misc/regexp.c` registers, which
/// `load_static_extension db regexp` reaches.
static EXTENDED: &[Defined] = &[
    Defined {
        name: b"regexp",
        count: Some(2),
        answer: regexp,
        safety: Safety::Innocuous,
    },
    Defined {
        name: b"regexpi",
        count: Some(2),
        answer: regexp,
        safety: Safety::Innocuous,
    },
];

/// `regexp(P,S)` and `regexpi(P,S)` of `ext/misc/regexp.c`: whether `S`
/// holds a run the pattern `P` matches, where `regexpi` reads a capital
/// and its small letter as one character.
///
/// A null argument answers nothing, which is the result
/// `re_sql_func` leaves unset.
fn regexp(
    name: &'static [u8],
    args: &[Value],
    _: Option<&Source>,
) -> Result<Value, db_sqlite::eval::Error> {
    let Some(pattern) = args.first().and_then(Value::text) else {
        return Ok(Value::Null);
    };
    let compiled = db_sqlite::regexp::compile(&pattern, name == b"regexpi")
        .map_err(db_sqlite::eval::Error::Regexp)?;
    let Some(text) = args.get(1).and_then(Value::text) else {
        return Ok(Value::Null);
    };
    Ok(Value::Int(i64::from(compiled.matches(&text))))
}

/// The aggregates `testfixture` defines that this harness answers.
static GROUPED: &[db_sqlite::func::Grouped] = &[
    db_sqlite::func::Grouped {
        name: b"md5sum",
        count: None,
        answer: md5sum,
    },
    db_sqlite::func::Grouped {
        name: b"decimal_sum",
        count: Some(1),
        answer: decimal_sum,
    },
];

/// The nine functions `sqlite3_decimal_init` of
/// `research/sqlite/ext/misc/decimal.c:920` registers, which
/// `load_static_extension db decimal` adds to a connection.
static DECIMALS: &[Defined] = &[
    Defined {
        name: b"decimal",
        count: None,
        answer: decimal_text,
        safety: Safety::Innocuous,
    },
    Defined {
        name: b"decimal_exp",
        count: None,
        answer: decimal_text,
        safety: Safety::Innocuous,
    },
    Defined {
        name: b"decimal_cmp",
        count: Some(2),
        answer: decimal_compare,
        safety: Safety::Innocuous,
    },
    Defined {
        name: b"decimal_add",
        count: Some(2),
        answer: decimal_arithmetic,
        safety: Safety::Innocuous,
    },
    Defined {
        name: b"decimal_sub",
        count: Some(2),
        answer: decimal_arithmetic,
        safety: Safety::Innocuous,
    },
    Defined {
        name: b"decimal_mul",
        count: Some(2),
        answer: decimal_arithmetic,
        safety: Safety::Innocuous,
    },
    Defined {
        name: b"decimal_pow2",
        count: Some(1),
        answer: decimal_power,
        safety: Safety::Innocuous,
    },
];

/// One value read as a decimal, which is `decimal_new` of
/// `research/sqlite/ext/misc/decimal.c:196`: a text and a whole number
/// are read as the text they spell, a real number and a blob of eight
/// bytes as the binary64 number they hold, and every other value as
/// nothing.
///
/// `text` says that the value is read as text whatever it holds, which is
/// the `bTextOnly` argument every function but `decimal` and
/// `decimal_exp` passes.
fn decimal_of(value: Option<&Value>, text: bool) -> Option<Decimal> {
    match value {
        Some(Value::Text(held)) => Some(Decimal::of_text(held)),
        Some(held @ Value::Int(_)) => Some(Decimal::of_text(&held.stringify().unwrap_or_default())),
        Some(held @ Value::Real(_)) if text => {
            Some(Decimal::of_text(&held.stringify().unwrap_or_default()))
        }
        Some(Value::Real(held)) => db_sqlite::decimal::of_double(*held),
        Some(Value::Blob(held)) if text => Some(Decimal::of_text(held)),
        Some(Value::Blob(held)) => held
            .as_slice()
            .try_into()
            .ok()
            .map(f64::from_be_bytes)
            .and_then(db_sqlite::decimal::of_double),
        _ => None,
    }
}

/// `decimal(X)`, `decimal(X,N)`, `decimal_exp(X)` and `decimal_exp(X,N)`
/// of `ext/misc/decimal.c`: the value written as a decimal, rounded to
/// `N` significant digits where a second argument names one, and in
/// exponential notation under `decimal_exp`.
///
/// It costs what reading and writing the digits costs.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the shape every function the application defines answers in"
)]
fn decimal_text(
    name: &'static [u8],
    args: &[Value],
    _: Option<&Source>,
) -> Result<Value, db_sqlite::eval::Error> {
    let Some(mut held) = decimal_of(args.first(), false) else {
        return Ok(Value::Null);
    };
    let count = args.get(1).map_or(0, db_sqlite::value::Value::to_integer);
    let count = usize::try_from(count).unwrap_or(0);
    if count > 0 {
        held.round(count);
    }
    Ok(Value::Text(if name == b"decimal_exp" {
        held.scientific(count)
    } else {
        held.text()
    }))
}

/// `decimal_cmp(X,Y)`: minus one, nought or one as `X` stands below `Y`,
/// beside it or above it, and nothing where either holds no number.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the shape every function the application defines answers in"
)]
fn decimal_compare(
    _: &'static [u8],
    args: &[Value],
    _: Option<&Source>,
) -> Result<Value, db_sqlite::eval::Error> {
    let (Some(one), Some(other)) = (
        decimal_of(args.first(), true),
        decimal_of(args.get(1), true),
    ) else {
        return Ok(Value::Null);
    };
    Ok(Value::Int(match db_sqlite::decimal::order(&one, &other) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }))
}

/// `decimal_add(X,Y)`, `decimal_sub(X,Y)` and `decimal_mul(X,Y)`: the
/// sum, the difference and the product of the two values, and nothing
/// where either holds no number.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the shape every function the application defines answers in"
)]
fn decimal_arithmetic(
    name: &'static [u8],
    args: &[Value],
    _: Option<&Source>,
) -> Result<Value, db_sqlite::eval::Error> {
    let (Some(one), Some(other)) = (
        decimal_of(args.first(), true),
        decimal_of(args.get(1), true),
    ) else {
        return Ok(Value::Null);
    };
    let held = match name {
        b"decimal_mul" => db_sqlite::decimal::multiplied(one, &other),
        b"decimal_sub" => db_sqlite::decimal::added(one, other.negated()),
        _ => db_sqlite::decimal::added(one, other),
    };
    Ok(Value::Text(held.text()))
}

/// `decimal_pow2(N)`: two to the power `N` in exponential notation, and
/// nothing for a value that is no whole number or a power past twenty
/// thousand either way.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the shape every function the application defines answers in"
)]
fn decimal_power(
    _: &'static [u8],
    args: &[Value],
    _: Option<&Source>,
) -> Result<Value, db_sqlite::eval::Error> {
    let Some(Value::Int(power)) = args.first() else {
        return Ok(Value::Null);
    };
    let held = i32::try_from(*power)
        .ok()
        .and_then(db_sqlite::decimal::power_of_two);
    Ok(held.map_or(Value::Null, |held| Value::Text(held.scientific(0))))
}

/// `decimal_sum(X)`: the sum of the values of the group, which is
/// `decimalSumStep` of `ext/misc/decimal.c:812` over each row, with a
/// null argument adding nothing.
///
/// It costs O(n) sums of O(m) digits each.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the shape every function the application defines answers in"
)]
fn decimal_sum(_: &'static [u8], rows: &[Vec<Value>]) -> Result<Value, db_sqlite::eval::Error> {
    let mut held = Decimal::zero();
    for row in rows {
        if let Some(value) = decimal_of(row.first(), true) {
            held = db_sqlite::decimal::added(held, value);
        }
    }
    Ok(Value::Text(held.text()))
}

/// The collation `decimal`, which orders two texts by the numbers they
/// spell.
fn decimal_collate(_: &'static [u8], left: &[u8], right: &[u8]) -> std::cmp::Ordering {
    db_sqlite::decimal::collate(left, right)
}

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
    outside: bool,
) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    // `tclsqlite.c:1790` reads the counters of each statement in turn,
    // so the last statement of a run is the one `db status` answers for.
    STEPPED.with(|held| held.set(db_sqlite::db::Stepped::default()));
    if reads(text) {
        // A statement that names the temp schema opens it, and a reader
        // is built over the images the connection holds, so the writer
        // opens it before the reader is built.
        writer
            .opens_temp(&sql_bytes(text))
            .map_err(|error| shape(text, refusal(&error)))?;
        let answered = answered_rows(writer, text, collating, defines, outside)?;
        STEPPED.with(|held| held.set(answered.stepped));
        ANSWERED.with(|held| held.set(answered.rows.len()));
        for row in &answered.rows {
            out.extend(row.iter().cloned());
        }
        return Ok(out);
    }
    let rows = writer
        .run(&sql_bytes(text))
        .map_err(|error| shape(text, refusal(&error)))?;
    ANSWERED.with(|held| held.set(rows.len()));
    for row in &rows {
        out.extend(row.iter().cloned());
    }
    Ok(out)
}

/// The lowest character of the private use area that stands for a byte
/// a `%XX` escape read, which is that area's first character plus the
/// byte.
const ESCAPED: u32 = 0xe000;

/// A statement with every `%XX` escape of `test_exec` of
/// `research/sqlite/src/test1.c:442` read as the byte it names.
///
/// A byte over `0x7f` is no character of its own, so it is carried as
/// the character `ESCAPED` plus the byte and read back by `sql_bytes`;
/// the line between this harness and the tester carries text and not
/// bytes. It costs O(n) in the length of the statement.
fn escaped(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |at: usize| {
        let digits = text.get(at..at.saturating_add(2))?;
        digits
            .bytes()
            .all(|digit| digit.is_ascii_hexdigit())
            .then(|| u8::from_str_radix(digits, 16).ok())?
    };
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < bytes.len() {
        let byte = hex(at.saturating_add(1)).filter(|_| bytes.get(at) == Some(&b'%'));
        let Some(byte) = byte else {
            let rest = text.get(at..).unwrap_or("");
            let value = rest.chars().next().unwrap_or('\0');
            out.push(value);
            at = at.saturating_add(value.len_utf8());
            continue;
        };
        if byte < 0x80 {
            out.push(char::from(byte));
        } else {
            let value = ESCAPED.saturating_add(u32::from(byte));
            out.push(char::from_u32(value).unwrap_or('?'));
        }
        at = at.saturating_add(3);
    }
    out
}

/// Which zone the run reads local time through, which
/// `SQLITE_TESTCTRL_LOCALTIME_FAULT` names.
fn zoning(text: &str) -> Vec<String> {
    ZONED.with(|held| held.set(number_of(text).unwrap_or(0)));
    Vec::new()
}

/// The local time of a moment, as the seconds of the unix epoch a clock
/// of the zone reads.
///
/// `testLocaltime` of `research/sqlite/src/test1.c:7937` is half an hour
/// later than UTC on an odd day and half an hour earlier on an even
/// one, and fails for the one moment `2000-05-29 14:16:00`. A run that
/// names no zone reads the machine's, which is UTC here.
fn zoned(seconds: i64) -> Option<i64> {
    match ZONED.with(core::cell::Cell::get) {
        1 => None,
        2 => {
            if seconds == 959_609_760 {
                return None;
            }
            let half = 1800;
            if (seconds / 86_400) & 1 == 1 {
                Some(seconds.saturating_add(half))
            } else {
                Some(seconds.saturating_sub(half))
            }
        }
        _ => Some(seconds),
    }
}

/// The bytes a run of hexadecimal digits names.
fn bytes_of_hex(digits: &str) -> Vec<u8> {
    let held: Vec<u8> = digits
        .bytes()
        .filter_map(|digit| char::from(digit).to_digit(16))
        .filter_map(|half| u8::try_from(half).ok())
        .collect();
    held.chunks(2)
        .map(|pair| {
            pair.first()
                .copied()
                .unwrap_or(0)
                .wrapping_shl(4)
                .wrapping_add(pair.get(1).copied().unwrap_or(0))
        })
        .collect()
}

/// A text literal holding those bytes, with every quote doubled.
///
/// A byte the line cannot carry as text is held as the character
/// `carried_text` writes for it, which `sql_bytes` reads back where the
/// statement reaches the engine, so the literal carries the bytes
/// themselves whatever encoding the database keeps its text in.
fn quoted_text(bytes: &[u8]) -> String {
    let mut held = Vec::with_capacity(bytes.len().saturating_add(2));
    held.push(b'\'');
    for byte in bytes {
        if *byte == b'\'' {
            held.push(b'\'');
        }
        held.push(*byte);
    }
    held.push(b'\'');
    carried_text(&held)
}

/// Whether the character stands for a byte the line cannot carry as
/// text, which is one of the private use area from `ESCAPED` on.
fn escaped_char(value: char) -> bool {
    (ESCAPED..ESCAPED.saturating_add(0x100)).contains(&u32::from(value))
}

/// The bytes written as capital hexadecimal digits, two per byte.
fn hex_of(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        for half in [byte >> 4, byte & 0xf] {
            out.push(char::from_digit(u32::from(half), 16).unwrap_or('0'));
        }
    }
    out.to_uppercase()
}

/// The bytes of a statement, with every character `escaped` wrote for a
/// byte over `0x7f` read back as that byte.
///
/// It costs O(n) in the length of the statement.
fn sql_bytes(text: &str) -> Vec<u8> {
    let carried = escaped_char;
    if !text.chars().any(carried) {
        return text.as_bytes().to_vec();
    }
    let mut out = Vec::with_capacity(text.len());
    let mut buffer = [0_u8; 4];
    for value in text.chars() {
        if carried(value) {
            out.push(u8::try_from(u32::from(value) & 0xff).unwrap_or(0));
        } else {
            out.extend_from_slice(value.encode_utf8(&mut buffer).as_bytes());
        }
    }
    out
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
    outside: bool,
) -> Result<db_sqlite::db::Answer, String> {
    {
        let beside = attached_images(writer);
        // A connection in write-ahead logging holds its newest pages in
        // the log, so a connection that did not write them follows the
        // log beside the file; the connection that wrote them reads its
        // own pages, which hold what a log has taken and what an open
        // transaction has written besides.
        let bytes = if outside {
            writer.outside()
        } else {
            writer.inside()
        };
        let log = if outside {
            writer.log().map(db_sqlite::wal::Wal::open)
        } else {
            None
        };
        let opened = match &log {
            Some(Ok(log)) => Database::open_log_collating(&bytes, log, collating),
            Some(Err(error)) => return Err(format!("{error}")),
            None => Database::open_collating(&bytes, collating),
        };
        let counted = writer.counts();
        let encoding = writer.encoding();
        let naming = writer.naming();
        let journalled = writer.journalled();
        let sensitive = writer.sensitive();
        let trusted = writer.trusts_schema();
        let limits = writer.limits();
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
                    .trusting(trusted)
                    .limited(limits)
                    .encoded(encoding)
                    .journalling(journalled)
                    .in_zone(zoned);
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
            .and_then(|database| database.query(&sql_bytes(text)))
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

/// Whether a text may name a file of its own, which an `ATTACH` and a
/// `VACUUM ... INTO` are the two of, and which is what the session tells
/// the files it holds for.
///
/// Reading the text costs O(n) in its bytes.
fn names_file(sql: &str) -> bool {
    names_word(sql, b"attach") || names_word(sql, b"vacuum")
}

/// Whether the text holds `word`, whatever case it is written in.
fn names_word(sql: &str, word: &[u8]) -> bool {
    sql.as_bytes()
        .windows(word.len())
        .any(|window| window.eq_ignore_ascii_case(word))
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

/// The hooks the tester told the connection, told to the writer, with
/// the ones it told none of taken off it.
fn hooking(writer: &mut Writer, hooked: &BTreeSet<String>) {
    if hooked.contains("commit_hook") {
        writer.commits(committing);
    } else {
        writer.commits_nothing();
    }
    if hooked.contains("rollback_hook") {
        writer.rolls_back(rolling);
    } else {
        writer.rolls_back_nothing();
    }
    if hooked.contains("update_hook") {
        writer.writes_rows(writing);
    } else {
        writer.writes_nothing();
    }
    if hooked.contains("preupdate_hook") {
        writer.peeks(peeking);
    } else {
        writer.peeks_nothing();
    }
}

/// How many pages of the database file the commits of one request
/// wrote, which `PAGER_STAT_WRITE` of `research/sqlite/src/pager.c`
/// counts one of per page written.
fn wrote_pages(did: &[Does]) -> u64 {
    let count = did
        .iter()
        .filter(|held| {
            matches!(
                held,
                Does::Write {
                    onto: Onto::Main,
                    ..
                }
            )
        })
        .count();
    u64::try_from(count).unwrap_or(0)
}

/// The connection and the place of the database a handle of
/// `btree_from_db` names, which the tester writes as the name of the
/// connection, an `@`, and the place.
fn named_place(held: &str) -> (&str, usize) {
    let (name, place) = held.split_once('@').unwrap_or((held, "0"));
    (name, place.parse().unwrap_or(0))
}

/// How many pages of the file the image holds, which is nothing for
/// bytes no header reads.
fn pages_of(bytes: &[u8]) -> usize {
    db_sqlite::image::Image::open(bytes)
        .ok()
        .and_then(|image| {
            bytes
                .len()
                .checked_div(usize::try_from(image.header().page_size).unwrap_or(1))
        })
        .unwrap_or(0)
}

/// The eleven counts `sqlite3PagerStats` of
/// `research/sqlite/src/pager.c:6900` answers, each with the name it
/// writes under.
///
/// This harness holds the pages of a file as one image and no cache of
/// its own, so every count of a cache is nought: no page is held open
/// between two statements, none stands in a cache, and none is read or
/// missed there. The pager state is the one a pager that holds no file
/// open reads.
fn pager_counts(pages: usize, wrote: u64) -> Vec<String> {
    let counts = [
        ("ref", 0),
        ("page", 0),
        ("max", 0),
        ("size", i64::try_from(pages).unwrap_or(0)),
        ("state", 0),
        ("err", 0),
        ("hit", 0),
        ("miss", 0),
        ("ovfl", 0),
        ("read", 0),
        ("write", i64::try_from(wrote).unwrap_or(0)),
    ];
    let mut out = Vec::new();
    for (name, count) in counts {
        out.push(name.to_owned());
        out.push(count.to_string());
    }
    out
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
        // A value the line cannot carry as text is carried as the
        // hexadecimal digits of its bytes, which the length line says
        // with an `x` in front of it.
        if value.chars().any(escaped_char) {
            let digits = hex_of(&sql_bytes(value));
            out.extend_from_slice(format!("x{}\n", digits.len()).as_bytes());
            out.extend_from_slice(digits.as_bytes());
            out.push(b'\n');
            continue;
        }
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

/// The limit a name of `research/sqlite/src/sqlite.h.in:4409` stands
/// for, and nothing for a name the library holds no limit under.
fn named_limit(name: &str) -> Option<db_sqlite::db::Limit> {
    use db_sqlite::db::Limit;
    Some(match name.strip_prefix("SQLITE_LIMIT_")? {
        "LENGTH" => Limit::Length,
        "SQL_LENGTH" => Limit::SqlLength,
        "COLUMN" => Limit::Column,
        "EXPR_DEPTH" => Limit::ExprDepth,
        "COMPOUND_SELECT" => Limit::CompoundSelect,
        "VDBE_OP" => Limit::VdbeOp,
        "FUNCTION_ARG" => Limit::FunctionArg,
        "ATTACHED" => Limit::AttachedDatabases,
        "LIKE_PATTERN_LENGTH" => Limit::LikePattern,
        "VARIABLE_NUMBER" => Limit::VariableNumber,
        "TRIGGER_DEPTH" => Limit::TriggerDepth,
        "WORKER_THREADS" => Limit::WorkerThreads,
        "PARSER_DEPTH" => Limit::ParserDepth,
        _ => return None,
    })
}

/// The result codes by name, which `sqlite3ErrName` of
/// `research/sqlite/src/main.c:1545` writes for each: the primary codes,
/// which are the ones `sqlite3ErrStr` holds words for.
const NAMED: [(&str, i64); 25] = [
    ("SQLITE_OK", 0),
    ("SQLITE_ERROR", 1),
    ("SQLITE_INTERNAL", 2),
    ("SQLITE_PERM", 3),
    ("SQLITE_ABORT", 4),
    ("SQLITE_BUSY", 5),
    ("SQLITE_LOCKED", 6),
    ("SQLITE_NOMEM", 7),
    ("SQLITE_READONLY", 8),
    ("SQLITE_INTERRUPT", 9),
    ("SQLITE_IOERR", 10),
    ("SQLITE_CORRUPT", 11),
    ("SQLITE_NOTFOUND", 12),
    ("SQLITE_FULL", 13),
    ("SQLITE_CANTOPEN", 14),
    ("SQLITE_PROTOCOL", 15),
    ("SQLITE_EMPTY", 16),
    ("SQLITE_SCHEMA", 17),
    ("SQLITE_TOOBIG", 18),
    ("SQLITE_CONSTRAINT", 19),
    ("SQLITE_MISMATCH", 20),
    ("SQLITE_MISUSE", 21),
    ("SQLITE_AUTH", 23),
    ("SQLITE_RANGE", 25),
    ("SQLITE_NOTADB", 26),
];

/// The code a name stands for, and two hundred for a name that stands
/// for none, which `test_errstr` of `research/sqlite/src/test1.c:3674`
/// reads as the code past every one it looks up.
fn numbered_code(name: &str) -> i64 {
    NAMED
        .iter()
        .find(|(held, _)| *held == name)
        .map_or(200, |(_, code)| *code)
}

/// What `sqlite3_quota_glob` answers: one where the text matches the
/// pattern and nought where it does not.
fn globbed(pattern: &str, text: &str) -> Vec<String> {
    let held = quota_glob(&sql_bytes(pattern), &sql_bytes(text));
    vec![usize::from(held).to_string()]
}

/// Whether the text matches the pattern, which `quotaStrglob` of
/// `research/sqlite/src/test_quota.c:254` answers: the rules of `GLOB`,
/// and a `/` of the pattern matching a `/` or a `\` of the text.
///
/// Matching a pattern of m bytes against a text of n bytes costs O(m*n)
/// in the worst case, because a `*` is tried at every place the byte
/// after it stands at.
fn quota_glob(pattern: &[u8], text: &[u8]) -> bool {
    let byte = |bytes: &[u8], at: usize| bytes.get(at).copied().unwrap_or(0);
    let mut at = 0;
    let mut over = 0;
    while let Some(held) = pattern.get(at).copied() {
        at = at.saturating_add(1);
        match held {
            b'*' => return many(pattern, text, (at, over)),
            b'?' => {
                if text.get(over).is_none() {
                    return false;
                }
                over = over.saturating_add(1);
            }
            b'[' => {
                let Some(held) = text.get(over).copied() else {
                    return false;
                };
                over = over.saturating_add(1);
                let (seen, past) = one_of(pattern, at, held);
                at = past;
                if !seen {
                    return false;
                }
            }
            // A `/` of the pattern matches either separator of a path.
            b'/' => {
                if !matches!(text.get(over), Some(&b'/' | &b'\\')) {
                    return false;
                }
                over = over.saturating_add(1);
            }
            held => {
                if byte(text, over) != held {
                    return false;
                }
                over = over.saturating_add(1);
            }
        }
    }
    over == text.len()
}

/// Whether the rest of the text matches the rest of the pattern after a
/// `*`, which stands for as many bytes as it takes.
///
/// Every `*` and `?` after the first `*` is read there: each `?` takes
/// one byte of the text, and the byte after them is looked for at every
/// place of the text the rest of the pattern is then tried at.
fn many(pattern: &[u8], text: &[u8], held: (usize, usize)) -> bool {
    let byte = |bytes: &[u8], at: usize| bytes.get(at).copied().unwrap_or(0);
    let (mut at, mut over) = held;
    let mut wanted;
    loop {
        wanted = byte(pattern, at);
        at = at.saturating_add(1);
        if wanted != b'*' && wanted != b'?' {
            break;
        }
        if wanted == b'?' {
            if text.get(over).is_none() {
                return false;
            }
            over = over.saturating_add(1);
        }
    }
    // A `*` the pattern ends on matches the rest of the text.
    if wanted == 0 {
        return true;
    }
    // A set after a `*` is tried at every place, because the byte it
    // matches is not one byte the text has to hold.
    if wanted == b'[' {
        let from = at.saturating_sub(1);
        while over < text.len()
            && !quota_glob(
                pattern.get(from..).unwrap_or_default(),
                text.get(over..).unwrap_or_default(),
            )
        {
            over = over.saturating_add(1);
        }
        return over < text.len();
    }
    let other = if wanted == b'/' { b'\\' } else { wanted };
    while let Some(mut held) = text.get(over).copied() {
        over = over.saturating_add(1);
        while held != wanted && held != other {
            let Some(next) = text.get(over).copied() else {
                return false;
            };
            held = next;
            over = over.saturating_add(1);
        }
        if quota_glob(
            pattern.get(at..).unwrap_or_default(),
            text.get(over..).unwrap_or_default(),
        ) {
            return true;
        }
    }
    false
}

/// Whether the byte is one the set that begins at `at` holds, and where
/// the pattern stands after the set.
///
/// A set that the pattern does not close holds nothing, which
/// `quotaStrglob` answers no match for.
fn one_of(pattern: &[u8], at: usize, held: u8) -> (bool, usize) {
    let byte = |at: usize| pattern.get(at).copied().unwrap_or(0);
    let mut at = at;
    let mut prior = 0;
    let mut seen = false;
    let mut invert = false;
    let mut wanted = byte(at);
    at = at.saturating_add(1);
    if wanted == b'^' {
        invert = true;
        wanted = byte(at);
        at = at.saturating_add(1);
    }
    // A `]` the set opens with is one of its own bytes.
    if wanted == b']' {
        seen = held == b']';
        wanted = byte(at);
        at = at.saturating_add(1);
    }
    while wanted != 0 && wanted != b']' {
        if wanted == b'-' && byte(at) != b']' && byte(at) != 0 && prior > 0 {
            wanted = byte(at);
            at = at.saturating_add(1);
            if held >= prior && held <= wanted {
                seen = true;
            }
            prior = 0;
        } else {
            if held == wanted {
                seen = true;
            }
            prior = wanted;
        }
        wanted = byte(at);
        at = at.saturating_add(1);
    }
    (wanted != 0 && seen != invert, at)
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
    writer.in_zone(zoned);
}

/// A whole number a request carries, which every request that names a
/// place in a file carries two of.
fn number_of(text: &str) -> Result<usize, String> {
    text.parse::<usize>()
        .map_err(|source| format!("a place in a file is a whole number: {source}"))
}

/// The hexadecimal digits of `bytes`, which is how the line carries what
/// no text holds.
///
/// Writing them costs O(n) in the bytes.
fn digits(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        for half in [byte >> 4, byte & 0xf] {
            out.push(char::from_digit(u32::from(half), 16).unwrap_or('0'));
        }
    }
    out.to_uppercase()
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
    reading(&words(sql))
}

/// The same over the words of a statement, which `EXPLAIN QUERY PLAN`
/// is read again without.
fn reading(words: &[String]) -> bool {
    let first = words.first().map_or("", String::as_str);
    if first.eq_ignore_ascii_case("select") || first.eq_ignore_ascii_case("values") {
        return true;
    }
    // `EXPLAIN QUERY PLAN` answers the plan of the statement under it
    // and writes nothing, so the reader answers it where the reader
    // answers that statement.
    if first.eq_ignore_ascii_case("explain") {
        let second = words.get(1).map_or("", String::as_str);
        let third = words.get(2).map_or("", String::as_str);
        return second.eq_ignore_ascii_case("query")
            && third.eq_ignore_ascii_case("plan")
            && reading(words.get(3..).unwrap_or_default());
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

/// The statements of one case, split at the semicolons that end one.
///
/// `sqlite3_complete` reads how far a text is a statement, which
/// [`db_sqlite::token::complete`] answers: a semicolon inside a string,
/// a comment or the body of a `CREATE TRIGGER` ends none. Each
/// semicolon is read against the statement it may end, so the walk is
/// O(n) in the text and O(k) again for each semicolon of one statement
/// of k bytes.
pub(crate) fn statements(sql: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (at, character) in sql.char_indices() {
        if character != ';' {
            continue;
        }
        let after = at.saturating_add(1);
        let piece = sql.get(start..after).unwrap_or_default();
        if db_sqlite::token::complete(piece.as_bytes()) {
            out.push(sql.get(start..at).unwrap_or_default());
            start = after;
        }
    }
    if let Some(piece) = sql.get(start..) {
        out.push(piece);
    }
    out
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
            // The interpreter writes a real with `%.15g` and puts `.0`
            // after one that reads as a whole number, so a value with an
            // exponent carries no `.0` in front of that exponent.
            let held = String::from_utf8_lossy(&db_sqlite::fp::text(*number, 15)).into_owned();
            held.replace(".0e", "e")
        }
        // `dbEvalColumnValue` of `research/sqlite/src/tclsqlite.c` hands
        // the tester a text as a C string, which ends at the first
        // nought and which the interpreter holds as text, and a blob as
        // the bytes it holds.
        Value::Text(bytes) => {
            let end = bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(bytes.len());
            String::from_utf8_lossy(bytes.get(..end).unwrap_or_default()).into_owned()
        }
        Value::Blob(bytes) => carried_text(bytes),
    }
}

/// A value of the engine as the line carries it: the text itself where
/// the bytes are text, and one character of the private use area per
/// byte where they are not, which `write_ok` writes as hexadecimal
/// digits.
///
/// A blob holds whatever bytes were written into it, and the line
/// between this harness and the tester carries text, so a blob reaches
/// the tester as the bytes it holds and not as the text they are
/// nearest. It costs O(n) in the bytes.
fn carried_text(bytes: &[u8]) -> String {
    if let Ok(text) = core::str::from_utf8(bytes) {
        return text.to_owned();
    }
    bytes
        .iter()
        .map(|byte| char::from_u32(ESCAPED.saturating_add(u32::from(*byte))).unwrap_or('?'))
        .collect()
}

/// The three empty texts a column of an expression names.
const fn nowhere() -> [String; 3] {
    [String::new(), String::new(), String::new()]
}

/// The database, the table and the name each column comes from, as the
/// three texts the line carries.
fn origins_of(origins: &[db_sqlite::db::Origin]) -> Vec<[String; 3]> {
    origins
        .iter()
        .map(|origin| {
            [&origin.schema, &origin.table, &origin.column]
                .map(|text| String::from_utf8_lossy(text).into_owned())
        })
        .collect()
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
    // The largest place any parameter so far stands for, which is `nVar`
    // of `sqlite3ExprAssignVarNumber`: a `?` and a name not written
    // before take the place after it, and a `?N` raises it only where
    // `N` is larger.
    let mut counted: usize = 0;
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    for (kind, name) in held {
        let mark = find_parameter(bytes, at, kind, &name);
        out.push_str(sql.get(at..mark).unwrap_or(""));
        let width = 1usize.saturating_add(name.len());
        at = mark.saturating_add(width);
        let place = match (kind, name.as_str()) {
            (b'?', "") => counted.saturating_add(1),
            (b'?', digits) => digits.parse().unwrap_or(counted.saturating_add(1)),
            _ => *names
                .entry(written_parameter(kind, &name))
                .or_insert(counted.saturating_add(1)),
        };
        counted = counted.max(place);
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
    let mut counted: usize = 0;
    for (kind, name) in parameters(sql) {
        let (held, named) = match (kind, name.as_str()) {
            (b'?', "") => (counted.saturating_add(1), String::new()),
            (b'?', digits) => (
                digits.parse().unwrap_or(counted.saturating_add(1)),
                String::new(),
            ),
            _ => {
                let written = written_parameter(kind, &name);
                match out.iter().position(|first| *first == written) {
                    Some(at) => (at.saturating_add(1), written),
                    None => (counted.saturating_add(1), written),
                }
            }
        };
        counted = counted.max(held);
        while out.len() < held {
            out.push(String::new());
        }
        for slot in out.iter_mut().skip(held.saturating_sub(1)).take(1) {
            slot.clone_from(&named);
        }
    }
    out
}

/// The text a named parameter is written as, which is the character that
/// opens it and the name after it, and which two parameters stand for one
/// place under.
fn written_parameter(kind: u8, name: &str) -> String {
    let mut out = String::new();
    out.push(char::from(kind));
    out.push_str(name);
    out
}
