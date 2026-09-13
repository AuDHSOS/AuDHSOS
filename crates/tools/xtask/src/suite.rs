// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! SQLite's own test files, run against `db-sqlite`.
//!
//! `research/sqlite/test` holds the suite `testfixture` drives. This
//! runs the part of it that needs no TCL interpreter: a
//! `do_execsql_test` whose statements and whose answer are both written
//! out in the file, with no substitution left in either. One database
//! is kept per file and the cases of that file run against it in the
//! order they are written, because each case builds on the ones before
//! it.
//!
//! The checkout is not part of this repository, so this is never a step
//! of `cargo xtask check`; `sh tools/sqlite.sh` brings it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use db_sqlite::change::Writer;
use db_sqlite::db::Database;
use db_sqlite::header::Encoding;
use db_sqlite::value::Value;

use crate::error::Error;
use crate::fs;

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

/// One step of a file.
pub(crate) enum Step {
    /// Statements the file runs to set itself up, which the cases after
    /// it read what was written by.
    Setup(String),
    /// A case: what the file calls it, the statements, and the answer
    /// it writes for them as the elements of a list.
    Case {
        /// What the file calls it.
        name: String,
        /// The statements, as written.
        sql: String,
        /// The answer the file writes.
        want: Vec<String>,
    },
    /// Something the file runs that this harness cannot: a body that
    /// runs more than statements, or statements a substitution stands
    /// in. It leaves the database short of what the cases after it
    /// read, so it stops the file the way a refusal does.
    Opaque,
}

/// What the engine refused, counted by the first word of the statement
/// it refused, which `--why` answers.
static WHY: std::sync::Mutex<Option<BTreeMap<String, usize>>> = std::sync::Mutex::new(None);

/// Counts what each refusal was for from here on.
pub(crate) fn why() {
    if let Ok(mut held) = WHY.lock() {
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

/// Whether a case that did not pass is printed with what each side
/// answered, which `--show` turns on.
static SHOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Prints every case that did not pass from here on.
pub(crate) fn show() {
    SHOW.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Runs the suite and answers one score per file, by file name.
///
/// # Errors
///
/// [`Error`] names a checkout that is not there and a file that would
/// not be read.
pub(crate) fn run(root: &Path, only: Option<&str>) -> Result<BTreeMap<String, Score>, Error> {
    let dir = root.join("research").join("sqlite").join("test");
    if !dir.is_dir() {
        return Err(Error::Usage(format!(
            "no suite at {}; run sh tools/sqlite.sh first",
            dir.display()
        )));
    }
    let mut scores = BTreeMap::new();
    for path in files(&dir)? {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if only.is_some_and(|wanted| wanted != name) {
            continue;
        }
        let cases = cases(&fs::read(&path)?);
        if cases.is_empty() {
            continue;
        }
        scores.insert(name.clone(), score(&name, &cases));
    }
    Ok(scores)
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

/// Every case of one file whose statements and answer are both written
/// out with no substitution left in either.
///
/// A case carrying `$` or `[` needs the TCL interpreter to say what it
/// runs, so it is passed over rather than guessed at.
pub(crate) fn cases(text: &str) -> Vec<Step> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some((command, after)) = next_command(rest) {
        rest = after;
        match command {
            // `execsql` outside a case is the file setting itself up,
            // and the cases after it read what it wrote.
            "execsql" => {
                if let Some((sql, after)) = braced(rest) {
                    rest = after;
                    out.push(Step::Setup(sql.to_owned()));
                }
            }
            // A case that expects a refusal says so as a pair of a
            // code and a message, which this does not answer; the
            // block is read past so that the `execsql` inside it is
            // not taken for setup.
            "do_catchsql_test" => {
                rest = past(rest, 2);
                out.push(Step::Opaque);
            }
            // `do_test NAME { execsql { SQL } } { ANSWER }` is the
            // older way of writing `do_execsql_test`, and the only
            // body this reads is one `execsql` and nothing else.
            "do_test" => {
                let Some((name, after)) = word(rest) else {
                    continue;
                };
                let Some((body, after)) = braced(after) else {
                    rest = after;
                    out.push(Step::Opaque);
                    continue;
                };
                let Some((want, after)) = braced(after) else {
                    rest = after;
                    out.push(Step::Opaque);
                    continue;
                };
                rest = after;
                let read = body
                    .trim()
                    .strip_prefix("execsql")
                    .and_then(braced)
                    .filter(|(_, over)| over.trim().is_empty());
                match read {
                    Some((sql, _)) => push(&mut out, name, sql, want),
                    None => out.push(Step::Opaque),
                }
            }
            _ => {
                let Some((name, after)) = word(rest) else {
                    continue;
                };
                let Some((sql, after)) = braced(after) else {
                    rest = after;
                    out.push(Step::Opaque);
                    continue;
                };
                let Some((want, after)) = braced(after) else {
                    rest = after;
                    out.push(Step::Opaque);
                    continue;
                };
                rest = after;
                push(&mut out, name, sql, want);
            }
        }
    }
    out
}

/// Keeps a case whose statements and whose answer are both written out
/// with no substitution left in either.
///
/// A case carrying `$` or `[` needs the TCL interpreter to say what it
/// runs, so it stops the file rather than being guessed at.
fn push(out: &mut Vec<Step>, name: &str, sql: &str, want: &str) {
    if [sql, want]
        .iter()
        .any(|text| text.contains('$') || text.contains('['))
    {
        out.push(Step::Opaque);
        return;
    }
    out.push(Step::Case {
        name: name.to_owned(),
        sql: sql.to_owned(),
        want: elements(want),
    });
}

/// The next of the four commands this reads, and what follows its name.
fn next_command(text: &str) -> Option<(&'static str, &str)> {
    const COMMANDS: [&str; 4] = ["do_execsql_test", "do_catchsql_test", "do_test", "execsql"];
    let mut best: Option<(&'static str, usize)> = None;
    for command in COMMANDS {
        let mut from = 0;
        while let Some(at) = text.get(from..).and_then(|rest| rest.find(command)) {
            let at = from.saturating_add(at);
            let before = text.get(..at).and_then(|head| head.chars().next_back());
            let after = text.get(at.saturating_add(command.len())..);
            let bare = !before.is_some_and(|character| {
                character.is_alphanumeric() || character == '_' || character == '.'
            }) && !after.is_some_and(|rest| {
                rest.starts_with(|character: char| character.is_alphanumeric() || character == '_')
            });
            if bare {
                if best.is_none_or(|(_, held)| at < held) {
                    best = Some((command, at));
                }
                break;
            }
            from = at.saturating_add(command.len());
        }
    }
    let (command, at) = best?;
    Some((command, text.get(at.saturating_add(command.len())..)?))
}

/// What follows `count` braced blocks.
fn past(text: &str, count: usize) -> &str {
    let mut rest = text;
    if let Some((_, after)) = word(rest) {
        rest = after;
    }
    for _ in 0..count {
        match braced(rest) {
            Some((_, after)) => rest = after,
            None => return rest,
        }
    }
    rest
}

/// The next word and what follows it, where a word is what stands
/// before the next space or brace.
fn word(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start_matches([' ', '\t']);
    let end = text.find([' ', '\t', '\n', '{']).unwrap_or(text.len());
    let (name, rest) = text.split_at(end);
    if name.is_empty() {
        return None;
    }
    Some((name, rest))
}

/// What the next pair of braces holds and what follows it, or nothing
/// where the next thing written is not a brace.
pub(crate) fn braced(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start_matches([' ', '\t', '\n', '\\', '\r']);
    if !text.starts_with('{') {
        return None;
    }
    let mut depth: usize = 0;
    let mut escaped = false;
    for (at, character) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let inside = text.get(1..at)?;
                    let rest = text.get(at.saturating_add(1)..)?;
                    return Some((inside, rest));
                }
            }
            _ => {}
        }
    }
    None
}

/// Runs one file's cases in order against one database and counts how
/// each ended.
fn score(file: &str, cases: &[Step]) -> Score {
    let mut score = Score::default();
    let Ok(mut writer) = Writer::new(4096, 0, Encoding::Utf8) else {
        return score;
    };
    let show = SHOW.load(std::sync::atomic::Ordering::Relaxed);
    let mut stopped = false;
    for step in cases {
        let (name, sql, want) = match step {
            Step::Opaque => {
                stopped = true;
                continue;
            }
            Step::Setup(sql) => {
                if !stopped && answer(&mut writer, sql).is_none() {
                    stopped = true;
                }
                continue;
            }
            Step::Case { name, sql, want } => (name, sql, want),
        };
        // A case the engine refused, and a step this harness could not
        // run, each leave the database short of what the cases after
        // them read, so the rest of the file is refused with them
        // rather than counted wrong.
        if stopped {
            score.refused = score.refused.saturating_add(1);
            continue;
        }
        match answer(&mut writer, sql) {
            None => {
                score.refused = score.refused.saturating_add(1);
                stopped = true;
            }
            Some(ref mine) if mine == want => {
                score.passed = score.passed.saturating_add(1);
            }
            Some(mine) => {
                score.failed = score.failed.saturating_add(1);
                if show {
                    crate::out::note!(
                        "{file} {name}\n  sql  {}\n  mine {mine:?}\n  want {want:?}",
                        sql.split_whitespace().collect::<Vec<&str>>().join(" ")
                    );
                }
            }
        }
    }
    score
}

/// What the engine answers for one case, written as `execsql` writes an
/// answer: every value of every row of every statement, in order, as
/// one list.
fn answer(writer: &mut Writer, sql: &str) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for statement in statements(sql) {
        let text = statement.trim();
        if text.is_empty() {
            continue;
        }
        if reads(text) {
            let bytes = writer.written();
            let database = Database::open(&bytes).ok()?;
            let answered = match database.query(text.as_bytes()) {
                Ok(answered) => answered,
                Err(error) => {
                    refused(&format!("{} {error:?}", first_words(text)));
                    return None;
                }
            };
            for row in &answered.rows {
                for value in row {
                    out.push(listed(value));
                }
            }
        } else {
            let rows = match writer.run(text.as_bytes()) {
                Ok(rows) => rows,
                Err(error) => {
                    refused(&format!("{} {error:?}", first_words(text)));
                    return None;
                }
            };
            for row in &rows {
                for value in row {
                    out.push(listed(value));
                }
            }
        }
    }
    Some(out)
}

/// The elements of a TCL list, which is what `do_execsql_test` writes
/// its answer as: words parted by space, with a word that holds a space
/// written inside braces.
pub(crate) fn elements(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut held = String::new();
    let mut depth: usize = 0;
    let mut started = false;
    for character in text.chars() {
        match character {
            '{' => {
                if depth > 0 {
                    held.push(character);
                }
                depth = depth.saturating_add(1);
                started = true;
            }
            '}' if depth > 0 => {
                depth = depth.saturating_sub(1);
                if depth > 0 {
                    held.push(character);
                }
            }
            character if character.is_whitespace() && depth == 0 => {
                if started {
                    out.push(core::mem::take(&mut held));
                    started = false;
                }
            }
            character => {
                held.push(character);
                started = true;
            }
        }
    }
    if started {
        out.push(held);
    }
    out
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

/// Whether the statement answers rows rather than changing them.
///
/// A `PRAGMA` that sets something changes the file; one that sets
/// nothing answers what the file holds.
fn reads(sql: &str) -> bool {
    let word = sql.split_whitespace().next().unwrap_or("");
    word.eq_ignore_ascii_case("select")
        || word.eq_ignore_ascii_case("values")
        || word.eq_ignore_ascii_case("with")
        || (word.eq_ignore_ascii_case("pragma") && !sql.contains('='))
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
                ';' => {
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

/// One value as an element of a TCL list: nothing is the empty element,
/// and a real keeps the digits SQLite prints it with.
fn listed(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Int(number) => number.to_string(),
        Value::Real(number) => {
            String::from_utf8_lossy(&db_sqlite::fp::text(*number, db_sqlite::fp::DIGITS))
                .into_owned()
        }
        Value::Text(bytes) | Value::Blob(bytes) => String::from_utf8_lossy(bytes).into_owned(),
    }
}
