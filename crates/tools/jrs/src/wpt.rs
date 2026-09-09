// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Original WPT testharness scripts in a shell realm. No replacement assertions,
//! ignored failures, expected-failure masks or browser compatibility claims.

use doc_html::token::{self, Token};
use jrs::{Error, Host, Limits, Realm, Value};
use std::{
    cell::Cell,
    io::Write,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

const CALLBACKS: &str = "\n;(function(){const automatic={explicit_done:true};setup(automatic);add_start_callback(function(properties){print('__jrs_wpt_start__',properties!==automatic&&!!(properties.single_test||properties.explicit_done));});add_result_callback(function(t){print('__jrs_wpt_result__',t.status,t.name,t.message);});add_completion_callback(function(tests,status){print('__jrs_wpt_complete__',status.status,tests.length,status.message);});})();\n";
const COMPLETE: &str = "\n;done();\n";

pub(super) fn run(
    mut args: impl Iterator<Item = String>,
    output: &mut impl Write,
) -> Result<(), String> {
    let root = PathBuf::from(
        args.next()
            .ok_or("--wpt requires ROOT and at least one test file")?,
    )
    .canonicalize()
    .map_err(|e| e.to_string())?;
    let files: Vec<_> = args.collect();
    if files.is_empty() {
        return Err("--wpt requires at least one test file".to_owned());
    }
    let limits = Limits {
        source_bytes: 4_194_304,
        tokens: 1_048_576,
        fuel: 100_000_000,
        ..Limits::default()
    };
    let harness = read(&root, "resources/testharness.js", limits.source_bytes)?;
    let mut failed = 0usize;
    for file in &files {
        let result = (|| {
            let test = scripts(&root, file, limits.source_bytes)?;
            let mut host = Report::default();
            let completed = host.completed.clone();
            let manual = host.manual.clone();
            let started = host.started.clone();
            let execution = (|| {
                let mut realm = Realm::new(limits, &mut host)?;
                realm.install_queue_microtask()?;
                realm.install_events()?;
                realm.install_timers()?;
                realm.evaluate(&harness)?;
                realm.evaluate(CALLBACKS)?;
                for source in test.sources {
                    realm.evaluate(&source)?;
                }
                // Single-file tests use done() as their assertion completion,
                // not merely the end-of-loading signal. Never complete them here.
                drive_timers(&mut realm, &completed, &started, &manual)?;
                Ok::<_, Error>(Value::Undefined)
            })();
            for line in &host.lines {
                writeln!(output, "{file}: {line}").map_err(|e| e.to_string())?;
            }
            execution.map_err(|e| e.to_string())?;
            host.validate()
        })();
        match result {
            Ok(count) => {
                writeln!(output, "{file}: PASS ({count} subtests)").map_err(|e| e.to_string())?;
            }
            Err(error) => {
                failed = failed.saturating_add(1);
                writeln!(output, "{file}: FAIL: {error}").map_err(|e| e.to_string())?;
            }
        }
    }
    writeln!(
        output,
        "WPT shell: {} files, {failed} failed; standalone events/microtasks, no DOM tree",
        files.len()
    )
    .map_err(|e| e.to_string())?;
    if failed == 0 {
        Ok(())
    } else {
        Err(format!("{failed} WPT shell files failed"))
    }
}

fn drive_timers(
    realm: &mut Realm<'_>,
    completed: &Cell<bool>,
    started: &Cell<bool>,
    manual: &Cell<bool>,
) -> Result<(), Error> {
    let start = Instant::now();
    let mut load_signaled = false;
    while !completed.get() {
        // The first test may itself be created in a later timer. Before the
        // start callback, setup properties are not yet observable to this host.
        if started.get() && !manual.get() && !load_signaled {
            realm.evaluate(COMPLETE)?;
            load_signaled = true;
            if completed.get() {
                break;
            }
        }
        if start.elapsed() >= Duration::from_secs(30) {
            return Err(Error::Limit {
                resource: "WPT timer wall time",
            });
        }
        let Some(wait) = realm.timer_wait()? else {
            break;
        };
        if wait > 0 {
            // Stay responsive and recheck time; no busy polling or fabricated clock.
            let wait = u64::try_from(wait.min(50_000_000)).unwrap_or(50_000_000);
            std::thread::sleep(Duration::from_nanos(wait));
        }
        realm.run_timer()?;
    }
    Ok(())
}

fn path(root: &Path, name: &str) -> Result<PathBuf, String> {
    let path = root
        .join(name.trim_start_matches('/'))
        .canonicalize()
        .map_err(|e| format!("{name}: {e}"))?;
    if !path.starts_with(root) {
        return Err("WPT resource escapes its root".to_owned());
    }
    Ok(path)
}
fn read(root: &Path, name: &str, limit: usize) -> Result<String, String> {
    let file = std::fs::File::open(path(root, name)?).map_err(|e| e.to_string())?;
    super::read_source(file, limit)
}
#[derive(Default)]
struct Scripts {
    sources: Vec<String>,
    bytes: usize,
}

fn scripts(root: &Path, file: &str, limit: usize) -> Result<Scripts, String> {
    let source = read(root, file, limit)?;
    let parent = Path::new(file).parent().unwrap_or(Path::new(""));
    if Path::new(file)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("js"))
    {
        let mut result = Scripts::default();
        for line in source.lines() {
            if let Some(resource) = line.trim().strip_prefix("// META: script=") {
                append_resource(&mut result, root, parent, resource, limit)?;
            }
            if line.contains("META:") && line.contains("variant=") {
                return Err("WPT variants require a URL-aware host".to_owned());
            }
        }
        append(&mut result, &source, limit)?;
        return Ok(result);
    }
    let mut result = Scripts::default();
    let mut inline = false;
    for token in token::tokens(&source) {
        match token {
            Token::Start {
                name, attributes, ..
            } if name == "script" => {
                if let Some((_, value)) = attributes.iter().find(|(key, _)| key == "type") {
                    match value.trim().to_ascii_lowercase().as_str() {
                        "" | "text/javascript" | "application/javascript" => {}
                        "module" => return Err("module script is not supported".to_owned()),
                        _ => {
                            inline = false;
                            continue;
                        }
                    }
                }
                if attributes
                    .iter()
                    .any(|(key, _)| matches!(key.as_str(), "async" | "defer"))
                {
                    return Err("async/defer script requires browser scheduling".to_owned());
                }
                if let Some((_, resource)) = attributes.iter().find(|(key, _)| key == "src") {
                    append_resource(&mut result, root, parent, resource, limit)?;
                    inline = false;
                } else {
                    inline = true;
                }
            }
            Token::End { name } if name == "script" => inline = false,
            Token::Text { text } if inline => {
                append(&mut result, &text, limit)?;
            }
            _ => {}
        }
    }
    if result.sources.is_empty() {
        return Err("no executable test scripts".to_owned());
    }
    Ok(result)
}
fn append_resource(
    out: &mut Scripts,
    root: &Path,
    parent: &Path,
    resource: &str,
    limit: usize,
) -> Result<(), String> {
    if matches!(
        resource,
        "/resources/testharness.js" | "/resources/testharnessreport.js"
    ) {
        return Ok(());
    }
    if resource.contains(':') || resource.contains('?') || resource.contains('#') {
        return Err(format!("unsupported WPT resource URL: {resource}"));
    }
    let name = if resource.starts_with('/') {
        resource.to_owned()
    } else {
        parent.join(resource).to_string_lossy().into_owned()
    };
    let source = read(root, &name, limit)?;
    append(out, &source, limit)
}

fn append(out: &mut Scripts, source: &str, limit: usize) -> Result<(), String> {
    if out
        .bytes
        .checked_add(source.len())
        .is_none_or(|n| n > limit)
    {
        return Err("combined WPT source limit exceeded".to_owned());
    }
    out.bytes = out.bytes.saturating_add(source.len());
    out.sources.push(source.to_owned());
    Ok(())
}

struct Report {
    completed: Rc<Cell<bool>>,
    manual: Rc<Cell<bool>>,
    started: Rc<Cell<bool>>,
    start: std::time::Instant,
    results: usize,
    failures: usize,
    completions: usize,
    total: usize,
    harness_ok: bool,
    lines: Vec<String>,
}
impl Default for Report {
    fn default() -> Self {
        Self {
            completed: Rc::new(Cell::new(false)),
            manual: Rc::new(Cell::new(false)),
            started: Rc::new(Cell::new(false)),
            start: std::time::Instant::now(),
            results: 0,
            failures: 0,
            completions: 0,
            total: 0,
            harness_ok: false,
            lines: Vec::new(),
        }
    }
}
impl Host for Report {
    fn timer_now(&mut self) -> Result<u128, Error> {
        Ok(self.start.elapsed().as_nanos())
    }
    fn event_timestamp(&mut self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }
    fn print(&mut self, args: &[Value]) -> Result<(), Error> {
        let marker = args.first().map(ToString::to_string).unwrap_or_default();
        if marker == "__jrs_wpt_start__" {
            if args.len() != 2 || !matches!(args.get(1), Some(Value::Boolean(_))) {
                return Err(Error::Host);
            }
            self.manual.set(args.get(1) == Some(&Value::Boolean(true)));
            self.started.set(true);
        } else if marker == "__jrs_wpt_result__" {
            if args.len() != 4 {
                return Err(Error::Host);
            }
            self.results = self.results.saturating_add(1);
            let pass = args.get(1) == Some(&Value::Number(0.0));
            if !pass {
                self.failures = self.failures.saturating_add(1);
            }
            self.lines.push(format!(
                "{} {}: {}",
                if pass { "PASS" } else { "FAIL" },
                args.get(2).map(ToString::to_string).unwrap_or_default(),
                args.get(3).map(ToString::to_string).unwrap_or_default()
            ));
        } else if marker == "__jrs_wpt_complete__" {
            if args.len() != 4 {
                return Err(Error::Host);
            }
            self.completions = self.completions.saturating_add(1);
            self.completed.set(true);
            self.harness_ok = args.get(1) == Some(&Value::Number(0.0));
            self.total = args
                .get(2)
                .and_then(|v| v.to_string().parse().ok())
                .ok_or(Error::Host)?;
            if !self.harness_ok {
                self.lines.push(format!(
                    "harness failure: {}",
                    args.get(3).map(ToString::to_string).unwrap_or_default()
                ));
            }
        } else {
            self.lines.push(
                args.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        Ok(())
    }
}
impl Report {
    fn validate(&self) -> Result<usize, String> {
        if self.completions != 1 {
            return Err(format!(
                "expected one completion callback, received {}",
                self.completions
            ));
        }
        if self.results == 0 || self.total != self.results {
            return Err(format!(
                "missing results: received {}, harness total {}",
                self.results, self.total
            ));
        }
        if !self.harness_ok || self.failures != 0 {
            return Err(format!(
                "{} failed subtests or harness error",
                self.failures
            ));
        }
        Ok(self.results)
    }
}

#[cfg(test)]
#[path = "tests/wpt.rs"]
mod tests;
