// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Original Test262 inputs, explicit variants and fail-closed diagnostic accounting.
//! Unsupported host/module operations and unproven parse negatives cannot pass.
mod metadata;
use jrs::{Backend, Error, Host, Limits, Realm, Script, Value, compile_script};
use metadata::{Metadata, Variant};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
    rc::Rc,
};

#[derive(Clone, Default)]
struct Report(Rc<RefCell<ReportState>>);
#[derive(Default)]
struct ReportState {
    completions: usize,
    failure: bool,
    unsupported: Option<String>,
}
impl Host for Report {
    fn print(&mut self, args: &[Value]) -> Result<(), Error> {
        let text = args.first().map(ToString::to_string).unwrap_or_default();
        let mut r = self.0.borrow_mut();
        if text == "Test262:AsyncTestComplete" {
            r.completions = r.completions.saturating_add(1);
        } else if text.starts_with("Test262:AsyncTestFailure:") {
            r.failure = true;
        }
        Ok(())
    }
    fn call(&mut self, id: u32, _: &Value, _: &[Value]) -> Result<Value, Error> {
        let feature = match id {
            0 => "$262.createRealm",
            2 => "$262.detachArrayBuffer",
            3 => "$262.AbstractModuleSource",
            _ => "$262.agent",
        };
        self.0.borrow_mut().unsupported = Some(feature.to_owned());
        Err(Error::Host)
    }
}
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Pass,
    Fail(String),
    Unsupported(String),
}
impl Outcome {
    const fn status(&self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail(_) => "FAIL",
            Self::Unsupported(_) => "UNSUPPORTED",
        }
    }
    fn detail(&self) -> &str {
        match self {
            Self::Pass => "",
            Self::Fail(s) | Self::Unsupported(s) => s,
        }
    }
}
#[derive(Default)]
struct Counts {
    files: usize,
    variants: usize,
    passed: usize,
    failed: usize,
    unsupported: usize,
    fixtures: usize,
}
struct Runner {
    root: PathBuf,
    limits: Limits,
    backend: Backend,
    harness: BTreeMap<String, Result<Rc<Script>, String>>,
    harness_bytes: usize,
}

fn parse_negative_outcome(compiled: Result<Script, Error>, expected: &str) -> Outcome {
    match compiled {
        Ok(_) => Outcome::Fail("parse-negative test compiled successfully".into()),
        Err(Error::Syntax { .. }) if expected == "SyntaxError" => Outcome::Pass,
        Err(Error::Syntax { .. }) => {
            Outcome::Fail("parse phase produced SyntaxError of the wrong expected type".into())
        }
        Err(Error::Unsupported { feature }) => Outcome::Unsupported(format!("compile: {feature}")),
        Err(Error::UnverifiedSyntax { .. }) => {
            Outcome::Unsupported("parse rejection is not yet verified".into())
        }
        Err(error) => Outcome::Fail(format!("compile: {error}")),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "sequential per-file reporting keeps every selected variant accounted for"
)]
pub(super) fn run(
    mut args: impl Iterator<Item = String>,
    limits: Limits,
    backend: Backend,
    output: &mut impl Write,
) -> Result<(), String> {
    let root = PathBuf::from(
        args.next()
            .ok_or("--test262 requires ROOT [--all | TEST_PATH...] [--summary]")?,
    )
    .canonicalize()
    .map_err(|e| e.to_string())?;
    if !root.join("harness/assert.js").is_file() || !root.join("test").is_dir() {
        return Err("invalid Test262 root".into());
    }
    let mut paths = Vec::new();
    let mut all = false;
    let mut summary = false;
    let mut jobs = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--all" => all = true,
            "--summary" => summary = true,
            "--jobs" => {
                let value = args.next().ok_or("--jobs requires a count")?;
                let count: usize = value.parse().map_err(|_| format!("bad --jobs {value}"))?;
                if count == 0 {
                    return Err("--jobs must be at least one".into());
                }
                jobs = Some(count);
            }
            _ if arg.starts_with('-') => return Err(format!("unknown Test262 option {arg}")),
            _ => paths.push(arg),
        }
    }
    if all && !paths.is_empty() {
        return Err("--all cannot be combined with selections".into());
    }
    if all {
        paths.push("test".into());
    }
    if paths.is_empty() {
        return Err("select Test262 files/directories or --all".into());
    }
    let mut files = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let test_root = root
        .join("test")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    for path in paths {
        let path = contained(&root, &path)?;
        if !path.starts_with(&test_root) {
            return Err("selection must be inside Test262 test/".into());
        }
        discover(&path, &test_root, &mut visited, &mut files, 0)?;
    }
    if files.is_empty() {
        return Err("no JavaScript test inputs selected".into());
    }
    // Every file gets a realm of its own, so files share nothing and are run
    // on as many threads as the machine has. The answers are merged in the
    // order the files were discovered, so the report does not depend on which
    // thread finished first.
    let files: Vec<PathBuf> = files.into_iter().collect();
    let jobs = jobs
        .or_else(|| std::thread::available_parallelism().ok().map(Into::into))
        .unwrap_or(1)
        .min(files.len().max(1));
    let judged = judge_in_parallel(&files, &root, limits, backend, jobs);
    let mut counts = Counts::default();
    for (label, judgement) in judged {
        match judgement {
            Judgement::Fixture => {
                counts.fixtures = counts.fixtures.saturating_add(1);
                continue;
            }
            Judgement::Metadata(error) => {
                counts.files = counts.files.saturating_add(1);
                record(
                    output,
                    &mut counts,
                    &label,
                    "metadata",
                    &Outcome::Fail(error),
                    summary,
                )?;
            }
            Judgement::Variants(outcomes) => {
                counts.files = counts.files.saturating_add(1);
                for (variant, outcome) in outcomes {
                    record(output, &mut counts, &label, variant, &outcome, summary)?;
                }
            }
        }
        if summary && counts.files % 1000 == 0 {
            writeln!(
                output,
                "Test262 progress: {} files / {} variants; {} passed, {} failed, {} unsupported",
                counts.files, counts.variants, counts.passed, counts.failed, counts.unsupported
            )
            .map_err(|e| e.to_string())?;
            output.flush().map_err(|e| e.to_string())?;
        }
    }
    writeln!(output,"Test262: {} files, {} variants: {} passed, {} failed, {} unsupported; {} fixtures not standalone",counts.files,counts.variants,counts.passed,counts.failed,counts.unsupported,counts.fixtures).map_err(|e|e.to_string())?;
    if counts.variants == 0 || counts.failed != 0 || counts.unsupported != 0 {
        Err("Test262 acceptance incomplete".into())
    } else {
        Ok(())
    }
}
/// What one file answered, before any of it is counted or printed.
enum Judgement {
    /// A `_FIXTURE` file, which is not a test of its own.
    Fixture,
    /// The file could not be read, or its metadata could not be parsed.
    Metadata(String),
    /// One outcome per variant the metadata names.
    Variants(Vec<(&'static str, Outcome)>),
}

/// Runs every file, spreading them over `jobs` threads.
///
/// A worker takes every `jobs`-th file rather than a block of them, because
/// neighbouring files are alike in cost and a block of cheap ones would finish
/// while a block of expensive ones was still running. Each worker keeps its
/// own harness cache: a compiled Script is reference-counted and belongs to
/// the thread that made it.
fn judge_in_parallel(
    files: &[PathBuf],
    root: &Path,
    limits: Limits,
    backend: Backend,
    jobs: usize,
) -> Vec<(String, Judgement)> {
    let mut answers: Vec<Option<(String, Judgement)>> = (0..files.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..jobs)
            .map(|worker| {
                scope.spawn(move || {
                    let mut runner = Runner {
                        root: root.to_path_buf(),
                        limits,
                        backend,
                        harness: BTreeMap::new(),
                        harness_bytes: 0,
                    };
                    files
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| index.checked_rem(jobs) == Some(worker))
                        .map(|(index, file)| (index, judge_one(&mut runner, file, limits)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for handle in handles {
            let Ok(part) = handle.join() else {
                continue;
            };
            for (index, answer) in part {
                if let Some(slot) = answers.get_mut(index) {
                    *slot = Some(answer);
                }
            }
        }
    });
    answers.into_iter().flatten().collect()
}

/// Reads one file, parses its metadata and runs every variant it names.
fn judge_one(runner: &mut Runner, file: &Path, limits: Limits) -> (String, Judgement) {
    let label = file
        .strip_prefix(&runner.root)
        .unwrap_or(file)
        .to_string_lossy()
        .into_owned();
    if file
        .file_name()
        .is_some_and(|name| name.to_string_lossy().contains("_FIXTURE"))
    {
        return (label, Judgement::Fixture);
    }
    let source = read(file, limits.source_bytes);
    let meta = source
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|text| metadata::parse(text));
    match (source, meta) {
        (Ok(source), Ok(meta)) => {
            let outcomes = meta
                .variants()
                .into_iter()
                .map(|variant| (variant.name(), runner.execute(&source, &meta, variant)))
                .collect();
            (label, Judgement::Variants(outcomes))
        }
        (_, Err(error)) | (Err(error), _) => (label, Judgement::Metadata(error)),
    }
}

fn record(
    output: &mut impl Write,
    c: &mut Counts,
    file: &str,
    variant: &str,
    outcome: &Outcome,
    summary: bool,
) -> Result<(), String> {
    c.variants = c.variants.saturating_add(1);
    match outcome {
        Outcome::Pass => c.passed = c.passed.saturating_add(1),
        Outcome::Fail(_) => c.failed = c.failed.saturating_add(1),
        Outcome::Unsupported(_) => c.unsupported = c.unsupported.saturating_add(1),
    }
    if !summary {
        writeln!(
            output,
            "{file} [{variant}]: {} {}",
            outcome.status(),
            outcome.detail().replace(['\n', '\r'], " ")
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn contained(root: &Path, name: &str) -> Result<PathBuf, String> {
    if Path::new(name).is_absolute() {
        return Err("absolute Test262 path is not allowed".into());
    }
    let path = root
        .join(name)
        .canonicalize()
        .map_err(|e| format!("{name}: {e}"))?;
    if !path.starts_with(root) {
        return Err("Test262 input escapes its root".into());
    }
    Ok(path)
}
fn discover(
    path: &Path,
    root: &Path,
    visited: &mut BTreeSet<PathBuf>,
    files: &mut BTreeSet<PathBuf>,
    depth: usize,
) -> Result<(), String> {
    if depth > 64 || visited.len().saturating_add(files.len()) >= 200_000 {
        return Err("Test262 discovery limit".into());
    }
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(root) {
        return Err("Test262 symlink escapes test root".into());
    }
    if path.is_dir() {
        if !visited.insert(path.clone()) {
            return Ok(());
        }
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            discover(
                &entry.map_err(|e| e.to_string())?.path(),
                root,
                visited,
                files,
                depth.saturating_add(1),
            )?;
        }
    } else if path.extension().is_some_and(|s| s == "js") {
        files.insert(path);
    }
    Ok(())
}
fn read(path: &Path, limit: usize) -> Result<String, String> {
    super::read_source(std::fs::File::open(path).map_err(|e| e.to_string())?, limit)
}

impl Runner {
    fn harness(&mut self, name: &str) -> Result<Rc<Script>, String> {
        if let Some(result) = self.harness.get(name) {
            return result.clone();
        }
        let result = (|| {
            let root = self
                .root
                .join("harness")
                .canonicalize()
                .map_err(|e| e.to_string())?;
            let source = read(&contained(&root, name)?, self.limits.source_bytes)?;
            self.harness_bytes = self
                .harness_bytes
                .checked_add(source.len())
                .filter(|n| *n <= 16_777_216)
                .ok_or("harness cache source limit")?;
            compile_script(&source, self.limits)
                .map(Rc::new)
                .map_err(|e| e.to_string())
        })();
        if self.harness.len() >= 1024 {
            return Err("harness cache entry limit".into());
        }
        self.harness.insert(name.to_owned(), result.clone());
        result
    }
    fn execute(&mut self, source: &str, meta: &Metadata, variant: Variant) -> Outcome {
        if variant == Variant::Module {
            return Outcome::Unsupported("module parsing/resolution is not implemented".into());
        }
        if meta.flag("CanBlockIsTrue") {
            return Outcome::Unsupported("blocking agents are not implemented".into());
        }
        if meta
            .negative
            .as_ref()
            .is_some_and(|(phase, _)| phase == "resolution")
        {
            return Outcome::Fail("resolution negative requires a module".into());
        }
        let source = if variant == Variant::Strict {
            format!("\"use strict\";\n{source}")
        } else {
            source.to_owned()
        };
        let compiled = compile_script(&source, self.limits);
        if let Some((phase, expected)) = &meta.negative
            && phase == "parse"
        {
            return parse_negative_outcome(compiled, expected);
        }
        let script = match compiled {
            Ok(s) => s,
            Err(Error::Unsupported { feature }) => {
                return Outcome::Unsupported(format!("compile: {feature}"));
            }
            Err(e) => return Outcome::Fail(format!("compile: {e}")),
        };
        let harness = match self.includes(meta, variant) {
            Ok(h) => h,
            Err(e) => return Outcome::Fail(e),
        };
        let mut report = Report::default();
        let state = report.0.clone();
        let mut realm = match Realm::with_backend(self.limits, &mut report, self.backend) {
            Ok(r) => r,
            Err(e) => return Outcome::Fail(format!("realm: {e}")),
        };
        if let Err(e) = install_host(&mut realm) {
            return Outcome::Fail(format!("host setup: {e}"));
        }
        for script in harness {
            if let Err(e) = realm.run_compiled(&script) {
                return Outcome::Fail(format!("harness execution: {}", describe(&mut realm, &e)));
            }
        }
        // A test passes or fails by what it throws, never by its completion
        // value, so the value never has to reach this runner.
        let result = realm.run_compiled(&script);
        if let Some(feature) = &state.borrow().unsupported {
            return Outcome::Unsupported(feature.clone());
        }
        if let Err(Error::Unsupported { feature }) = &result {
            return Outcome::Unsupported((*feature).into());
        }
        let outcome = match (&meta.negative, result) {
            (None, Ok(())) => Outcome::Pass,
            (None, Err(e)) => Outcome::Fail(format!("runtime: {}", describe(&mut realm, &e))),
            (Some((phase, expected)), Err(e)) if phase == "runtime" => {
                if error_name(&mut realm, &e).as_deref() == Some(expected) {
                    Outcome::Pass
                } else {
                    Outcome::Fail(format!(
                        "runtime expected {expected}, received {}",
                        describe(&mut realm, &e)
                    ))
                }
            }
            (Some(_), Ok(())) => Outcome::Fail("negative test completed without throwing".into()),
            (_, Err(e)) => Outcome::Fail(format!("unexpected phase: {e}")),
        };
        if outcome != Outcome::Pass {
            return outcome;
        }
        let report = state.borrow();
        if let Some(feature) = &report.unsupported {
            return Outcome::Unsupported(feature.clone());
        }
        if report.failure {
            return Outcome::Fail("async failure marker".into());
        }
        if meta.flag("async") && report.completions != 1 {
            return Outcome::Fail(format!(
                "async completion count {}, expected 1 after job checkpoint",
                report.completions
            ));
        }
        Outcome::Pass
    }
    fn includes(&mut self, meta: &Metadata, variant: Variant) -> Result<Vec<Rc<Script>>, String> {
        if variant == Variant::Raw {
            return Ok(Vec::new());
        }
        let mut names = vec!["assert.js", "sta.js"];
        if meta.flag("async") {
            names.push("doneprintHandle.js");
        }
        names.extend(meta.includes.iter().map(String::as_str));
        names
            .into_iter()
            .map(|name| {
                self.harness(name)
                    .map_err(|e| format!("harness {name}: {e}"))
            })
            .collect()
    }
}
fn error_name(realm: &mut Realm<'_>, error: &Error) -> Option<String> {
    match error {
        Error::Syntax { .. } | Error::UnverifiedSyntax { .. } => Some("SyntaxError".into()),
        Error::Type { .. } => Some("TypeError".into()),
        Error::Reference { .. } => Some("ReferenceError".into()),
        Error::Range { .. } => Some("RangeError".into()),
        Error::Thrown { value } if matches!(value, Value::Object(_) | Value::Function(_)) => {
            let ctor = realm.get(value, &Value::string("constructor")).ok()?;
            let name = realm.get(&ctor, &Value::string("name")).ok()?;
            if let Value::String(_) = name {
                Some(name.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}
fn describe(realm: &mut Realm<'_>, error: &Error) -> String {
    if let Error::Thrown { value } = error
        && let Ok(message) = realm.get(value, &Value::string("message"))
        && matches!(message, Value::String(_))
    {
        return format!(
            "{}: {message}",
            error_name(realm, error).unwrap_or_else(|| "throw".into())
        );
    }
    error.to_string()
}
fn property(realm: &mut Realm<'_>, object: &Value, name: &str, value: &Value) -> Result<(), Error> {
    let desc = realm.object()?;
    for (key, value) in [
        ("value", value.clone()),
        ("writable", Value::Boolean(true)),
        ("configurable", Value::Boolean(true)),
        ("enumerable", Value::Boolean(false)),
    ] {
        realm.set(&desc, &Value::string(key), &value)?;
    }
    realm.define_property(object, &Value::string(name), &desc)?;
    realm.release(&desc)
}
fn install_host(realm: &mut Realm<'_>) -> Result<(), Error> {
    let global = realm.get_global("globalThis")?;
    let host = realm.object()?;
    property(realm, &global, "$262", &host)?;
    property(realm, &host, "global", &global)?;
    let print = realm.string_print_function()?;
    property(realm, &global, "print", &print)?;
    let gc = realm.gc_function()?;
    property(realm, &host, "gc", &gc)?;
    let eval = realm.eval_script_function()?;
    property(realm, &host, "evalScript", &eval)?;
    for (id, name) in [
        (0, "createRealm"),
        (2, "detachArrayBuffer"),
        (3, "AbstractModuleSource"),
    ] {
        let f = realm.host_function(id, name, u32::from(id != 0))?;
        property(realm, &host, name, &f)?;
    }
    let agent = realm.object()?;
    property(realm, &host, "agent", &agent)?;
    for name in ["start", "broadcast", "getReport", "sleep", "monotonicNow"] {
        let f = realm.host_function(4, name, 0)?;
        property(realm, &agent, name, &f)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/test262.rs"]
mod tests;
