// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every Markdown document, every RFC, and every HTML standard of this
//! repository, as PDF.
//!
//! The work is one document at a time and nothing is shared between two of
//! them, so the tool reads, parses, lays out, and writes each document on
//! whichever thread picked it up. The only ordering is at the ends: the
//! jobs are started with the largest source first, so that the last worker
//! is not still on RFC 1122 when the others have run out of work, and the
//! results are reported in the order the documents were found rather than
//! the order they happened to finish.

#![forbid(unsafe_code)]

mod error;
mod index;
mod layout;
mod links;
mod pool;
mod rfc;
mod sources;
mod text;
mod theme;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use crate::error::Error;
use crate::links::Links;
use crate::sources::{Kind, Source};

/// What the tool does when asked.
const USAGE: &str = "\
usage: docpdf [options]

Converts every Markdown document, every RFC, and every HTML standard of
the repository into a PDF, several at a time, and writes an index beside
them.

options:
  --root <dir>     the repository to read (default: the one this was built
                   from, or the working directory)
  --out <dir>      where to write (default: <root>/target/pdf)
  --jobs <n>       how many documents to convert at a time (default: the
                   parallelism of the machine)
  --only <text>    convert only the documents whose output path contains
                   this text, for instance `--only rfc/`
  --list           say what would be converted and write nothing
  --compress       deflate the content streams, which makes the files
                   about a third of the size and no longer readable as
                   the operators they are built from
  --quiet          report only the summary line
  --help           this text
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::Usage(message)) => {
            eprintln!("{message}\n{USAGE}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// What one conversion produced.
struct Outcome {
    /// Where it was written, relative to the output directory.
    target: PathBuf,
    /// How many pages it has.
    pages: usize,
    /// How large the file is.
    bytes: usize,
    /// What went wrong, if anything did.
    failure: Option<String>,
}

/// The command line, once read.
struct Options {
    /// The repository.
    root: PathBuf,
    /// Where to write.
    out: PathBuf,
    /// How many documents at a time.
    jobs: usize,
    /// Convert only what matches.
    only: Option<String>,
    /// Say what would happen and stop.
    list: bool,
    /// Deflate the content streams.
    compress: bool,
    /// Report only the summary.
    quiet: bool,
}

fn run() -> Result<(), Error> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some(options) = parse(&arguments)? else {
        print!("{USAGE}");
        return Ok(());
    };
    let mut sources = sources::collect(&options.root)?;
    if let Some(only) = &options.only {
        sources.retain(|source| source.target.to_string_lossy().contains(only.as_str()));
    }
    if sources.is_empty() {
        return Err(Error::Usage(format!(
            "no document to convert under {}",
            options.root.display()
        )));
    }
    if options.list {
        for source in &sources {
            println!("{}  <-  {}", source.target.display(), source.origin);
        }
        println!("{} document(s)", sources.len());
        return Ok(());
    }

    let links = Links::new(&sources);
    // Longest first: the worker that draws RFC 1122 starts at the same
    // moment as the one that draws a two-page README, instead of picking
    // it up last while the others stand idle.
    let mut queue: Vec<&Source> = sources.iter().collect();
    queue.sort_by_key(|source| std::cmp::Reverse(source.size));
    let jobs = options.jobs.min(queue.len()).max(1);

    let started = Instant::now();
    let mut outcomes = pool::map(&queue, jobs, |source| {
        convert(source, &links, &options.out, options.compress)
    });
    let elapsed = started.elapsed();

    outcomes.sort_by(|a, b| a.target.cmp(&b.target));
    let mut failures = Vec::new();
    let mut pages = 0usize;
    let mut bytes = 0usize;
    for outcome in &outcomes {
        if let Some(failure) = &outcome.failure {
            failures.push(failure.clone());
            continue;
        }
        pages = pages.saturating_add(outcome.pages);
        bytes = bytes.saturating_add(outcome.bytes);
        if !options.quiet {
            println!(
                "{:<44} {:>5} page(s) {:>9}",
                outcome.target.display(),
                outcome.pages,
                size(outcome.bytes)
            );
        }
    }

    let written = outcomes.len().saturating_sub(failures.len());
    if written > 0 {
        let (index, count) =
            index::render(&sources, &outcomes_by_target(&outcomes), options.compress);
        let path = options.out.join("index.pdf");
        write(&path, &index)?;
        pages = pages.saturating_add(count);
        bytes = bytes.saturating_add(index.len());
        if !options.quiet {
            println!(
                "{:<44} {:>5} page(s) {:>9}",
                "index.pdf",
                count,
                size(index.len())
            );
        }
    }
    println!(
        "{written} document(s), {pages} page(s), {} in {:.2}s on {jobs} thread(s)",
        size(bytes),
        elapsed.as_secs_f64()
    );
    if failures.is_empty() {
        Ok(())
    } else {
        Err(Error::Documents(failures))
    }
}

/// Converts one document and writes it.
fn convert(source: &Source, links: &Links, out: &Path, compress: bool) -> Outcome {
    let path = out.join(&source.target);
    let mut outcome = Outcome {
        target: source.target.clone(),
        pages: 0,
        bytes: 0,
        failure: None,
    };
    let text = match std::fs::read_to_string(&source.path) {
        Ok(text) => text,
        Err(error) => {
            outcome.failure = Some(format!("{}: {error}", source.origin));
            return outcome;
        }
    };
    let (bytes, pages) = match source.kind {
        Kind::Markdown | Kind::Html => {
            let blocks = if source.kind == Kind::Html {
                doc_html::parse(&text)
            } else {
                doc_markdown::parse(&text)
            };
            let mut layout = layout::Layout::new(source, links, compress);
            layout.document(&blocks);
            let pages = layout.pages();
            (layout.finish(), pages)
        }
        Kind::Rfc => rfc::render(source, &text, compress),
    };
    if let Err(error) = write(&path, &bytes) {
        outcome.failure = Some(error.to_string());
        return outcome;
    }
    outcome.pages = pages;
    outcome.bytes = bytes.len();
    outcome
}

/// Writes a file, creating the directory it lives in.
fn write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::path("creating", parent, e))?;
    }
    std::fs::write(path, bytes).map_err(|e| Error::path("writing", path, e))
}

/// The page count of every document that was written, by target path.
fn outcomes_by_target(outcomes: &[Outcome]) -> Vec<(PathBuf, usize)> {
    outcomes
        .iter()
        .filter(|outcome| outcome.failure.is_none())
        .map(|outcome| (outcome.target.clone(), outcome.pages))
        .collect()
}

/// A byte count a person can read.
fn size(bytes: usize) -> String {
    let bytes = u64::try_from(bytes).unwrap_or(0);
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let kilobytes = bytes.wrapping_div(1024);
    if kilobytes < 1024 {
        return format!("{kilobytes} KiB");
    }
    let whole = kilobytes.wrapping_div(1024);
    let fraction = kilobytes
        .wrapping_rem(1024)
        .saturating_mul(10)
        .wrapping_div(1024);
    format!("{whole}.{fraction} MiB")
}

/// Reads the command line. `None` means the usage was asked for.
fn parse(arguments: &[String]) -> Result<Option<Options>, Error> {
    let mut options = Options {
        root: default_root()?,
        out: PathBuf::new(),
        jobs: pool::parallelism(),
        only: None,
        list: false,
        compress: false,
        quiet: false,
    };
    let mut out = None;
    let mut at = 0;
    while let Some(argument) = arguments.get(at) {
        let value = || {
            arguments
                .get(at.saturating_add(1))
                .cloned()
                .ok_or_else(|| Error::Usage(format!("`{argument}` needs a value")))
        };
        match argument.as_str() {
            "--help" | "-h" | "help" => return Ok(None),
            "--root" => {
                options.root = PathBuf::from(value()?);
                at = at.saturating_add(2);
            }
            "--out" => {
                out = Some(PathBuf::from(value()?));
                at = at.saturating_add(2);
            }
            "--jobs" => {
                let text = value()?;
                options.jobs = text
                    .parse()
                    .ok()
                    .filter(|count| *count > 0)
                    .ok_or_else(|| Error::Usage(format!("`{text}` is not a thread count")))?;
                at = at.saturating_add(2);
            }
            "--only" => {
                options.only = Some(value()?);
                at = at.saturating_add(2);
            }
            "--list" => {
                options.list = true;
                at = at.saturating_add(1);
            }
            "--compress" => {
                options.compress = true;
                at = at.saturating_add(1);
            }
            "--quiet" => {
                options.quiet = true;
                at = at.saturating_add(1);
            }
            other => return Err(Error::Usage(format!("unknown option `{other}`"))),
        }
    }
    options.out = out.unwrap_or_else(|| options.root.join("target").join("pdf"));
    Ok(Some(options))
}

/// The repository to read: the one this binary was built from, or the
/// directory the tool was started in.
fn default_root() -> Result<PathBuf, Error> {
    if let Some(root) = std::env::var_os("AUDHSOS_ROOT") {
        return Ok(PathBuf::from(root));
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(root) = manifest.ancestors().nth(3)
        && root.join("Cargo.toml").is_file()
    {
        return Ok(root.to_path_buf());
    }
    std::env::current_dir().map_err(|source| Error::io("reading the working directory", source))
}
