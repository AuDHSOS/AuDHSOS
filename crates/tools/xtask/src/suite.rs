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

/// One case: the statements and the answer the file writes for them.
pub(crate) struct Case {
    /// What the file calls it.
    pub(crate) name: String,
    /// The statements, as written.
    pub(crate) sql: String,
    /// The answer, as the elements of the list `execsql` would have
    /// answered.
    pub(crate) want: Vec<String>,
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
pub(crate) fn cases(text: &str) -> Vec<Case> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("do_execsql_test") {
        let after = rest
            .get(at.saturating_add("do_execsql_test".len())..)
            .unwrap_or("");
        rest = after;
        let Some((name, after)) = word(after) else {
            continue;
        };
        let Some((sql, after)) = braced(after) else {
            continue;
        };
        let Some((want, after)) = braced(after) else {
            continue;
        };
        rest = after;
        if [sql, want]
            .iter()
            .any(|text| text.contains('$') || text.contains('['))
        {
            continue;
        }
        out.push(Case {
            name: name.to_owned(),
            sql: sql.to_owned(),
            want: elements(want),
        });
    }
    out
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
fn score(file: &str, cases: &[Case]) -> Score {
    let mut score = Score::default();
    let Ok(mut writer) = Writer::new(4096, 0, Encoding::Utf8) else {
        return score;
    };
    let show = SHOW.load(std::sync::atomic::Ordering::Relaxed);
    let mut stopped = false;
    for case in cases {
        // A case the engine refused leaves the database short of what
        // the cases after it read, so the rest of the file is refused
        // with it rather than counted wrong.
        if stopped {
            score.refused = score.refused.saturating_add(1);
            continue;
        }
        match answer(&mut writer, &case.sql) {
            None => {
                score.refused = score.refused.saturating_add(1);
                stopped = true;
            }
            Some(ref mine) if *mine == case.want => {
                score.passed = score.passed.saturating_add(1);
            }
            Some(mine) => {
                score.failed = score.failed.saturating_add(1);
                if show {
                    crate::out::note!(
                        "{file} {}\n  sql  {}\n  mine {mine:?}\n  want {:?}",
                        case.name,
                        case.sql.split_whitespace().collect::<Vec<&str>>().join(" "),
                        case.want
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
            let answered = database.query(text.as_bytes()).ok()?;
            for row in &answered.rows {
                for value in row {
                    out.push(listed(value));
                }
            }
        } else {
            writer.run(text.as_bytes()).ok()?;
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

/// Whether the statement answers rows rather than changing them.
fn reads(sql: &str) -> bool {
    let word = sql.split_whitespace().next().unwrap_or("");
    word.eq_ignore_ascii_case("select")
        || word.eq_ignore_ascii_case("values")
        || word.eq_ignore_ascii_case("with")
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
