// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What gets converted, and where it lands.
//!
//! The output is not a copy of the source tree. A reader looking for the
//! testing strategy should not have to know that it lives beside the
//! roadmap in `docs/`, and a reader looking for what `net-ip` does should
//! not have to remember that its README is three directories down. So the
//! documents are filed by what they are:
//!
//! ```text
//! target/pdf/
//! ├── index.pdf     everything below, with a link to each
//! ├── project/      the files at the root of the repository
//! ├── handbook/     docs/, the numbered chapters in their order
//! ├── crates/       one file per workspace crate, named after the package
//! ├── tools/        one file per tool that is not a workspace crate
//! ├── rfc/          the requests for comments the network stack answers to
//! └── other/        anything Markdown that none of the above claimed
//! ```
//!
//! The last one matters: a rule that silently drops a document it has no
//! category for is a rule that loses documents. Whatever is not claimed by
//! a category is still converted, under a name made from its path.

use std::path::{Path, PathBuf};

use crate::error::Error;

/// What kind of source a document is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Markdown, laid out as prose.
    Markdown,
    /// The plain text of an RFC, laid out as the fixed-pitch pages it is.
    Rfc,
}

/// One document to convert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Source {
    /// Where it is read from.
    pub(crate) path: PathBuf,
    /// Where it is written, relative to the output directory.
    pub(crate) target: PathBuf,
    /// How it is laid out.
    pub(crate) kind: Kind,
    /// What the document is called, for the title and the running head.
    pub(crate) title: String,
    /// The path shown under the title, relative to the repository root.
    pub(crate) origin: String,
    /// The size of the source in bytes, which is how the jobs are ordered.
    pub(crate) size: u64,
}

/// The directories a converted document can land in.
pub(crate) const SECTIONS: [&str; 6] = ["project", "handbook", "crates", "tools", "rfc", "other"];

/// Finds every document under `root`.
pub(crate) fn collect(root: &Path) -> Result<Vec<Source>, Error> {
    let mut sources = Vec::new();
    project(root, &mut sources)?;
    handbook(root, &mut sources)?;
    rfcs(root, &mut sources)?;
    crates(root, &mut sources)?;
    tools(root, &mut sources)?;
    other(root, &mut sources)?;
    sources.sort_by(|a, b| a.target.cmp(&b.target));
    sources.dedup_by(|a, b| a.path == b.path);
    Ok(sources)
}

/// The Markdown files at the root of the repository.
fn project(root: &Path, out: &mut Vec<Source>) -> Result<(), Error> {
    for path in list(root, "md")? {
        let stem = stem(&path);
        let target = PathBuf::from("project").join(format!("{}.pdf", stem.to_lowercase()));
        out.push(markdown(root, &path, target)?);
    }
    Ok(())
}

/// The handbook: the numbered chapters of `docs/`, with its README first.
fn handbook(root: &Path, out: &mut Vec<Source>) -> Result<(), Error> {
    let docs = root.join("docs");
    if !docs.is_dir() {
        return Ok(());
    }
    for path in list(&docs, "md")? {
        let stem = stem(&path);
        let name = if stem.eq_ignore_ascii_case("README") {
            "00-contents".to_owned()
        } else {
            stem
        };
        let target = PathBuf::from("handbook").join(format!("{name}.pdf"));
        out.push(markdown(root, &path, target)?);
    }
    Ok(())
}

/// The requests for comments, and the README that indexes them.
fn rfcs(root: &Path, out: &mut Vec<Source>) -> Result<(), Error> {
    let dir = root.join("docs").join("rfc");
    if !dir.is_dir() {
        return Ok(());
    }
    for path in list(&dir, "txt")? {
        let stem = stem(&path);
        let target = PathBuf::from("rfc").join(format!("{stem}.pdf"));
        let size = size_of(&path);
        out.push(Source {
            origin: relative(root, &path),
            title: title_of_rfc(&stem),
            path,
            target,
            kind: Kind::Rfc,
            size,
        });
    }
    for path in list(&dir, "md")? {
        let stem = stem(&path);
        let name = if stem.eq_ignore_ascii_case("README") {
            "00-index".to_owned()
        } else {
            stem
        };
        out.push(markdown(
            root,
            &path,
            PathBuf::from("rfc").join(format!("{name}.pdf")),
        )?);
    }
    Ok(())
}

/// One document per workspace crate, named after the package rather than
/// after the directory: `crates/net/ip/README.md` becomes `net-ip.pdf`,
/// which is the name the rest of the repository calls it by.
fn crates(root: &Path, out: &mut Vec<Source>) -> Result<(), Error> {
    let mut readmes = Vec::new();
    walk(&root.join("crates"), &mut readmes)?;
    for path in readmes {
        if !is_readme(&path) {
            continue;
        }
        let Some(dir) = path.parent() else { continue };
        let name = package_name(dir).unwrap_or_else(|| directory_name(dir));
        let target = PathBuf::from("crates").join(format!("{name}.pdf"));
        out.push(markdown(root, &path, target)?);
    }
    Ok(())
}

/// The tools that live outside the workspace.
fn tools(root: &Path, out: &mut Vec<Source>) -> Result<(), Error> {
    let mut readmes = Vec::new();
    walk(&root.join("tools"), &mut readmes)?;
    for path in readmes {
        if !is_readme(&path) {
            continue;
        }
        let Some(dir) = path.parent() else { continue };
        let name = package_name(dir).unwrap_or_else(|| directory_name(dir));
        let target = PathBuf::from("tools").join(format!("{name}.pdf"));
        out.push(markdown(root, &path, target)?);
    }
    Ok(())
}

/// Everything else that is Markdown, so that nothing is lost by having no
/// category. The name is the path with its separators turned into dashes,
/// which keeps two files of the same name apart.
fn other(root: &Path, out: &mut Vec<Source>) -> Result<(), Error> {
    let mut found = Vec::new();
    walk(root, &mut found)?;
    let claimed: Vec<PathBuf> = out.iter().map(|source| source.path.clone()).collect();
    for path in found {
        if claimed.contains(&path) {
            continue;
        }
        let name = relative(root, &path)
            .trim_end_matches(".md")
            .replace(['/', ' '], "-")
            .to_lowercase();
        let target = PathBuf::from("other").join(format!("{name}.pdf"));
        out.push(markdown(root, &path, target)?);
    }
    Ok(())
}

/// A Markdown source, with its title read from its first heading.
fn markdown(root: &Path, path: &Path, target: PathBuf) -> Result<Source, Error> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::path("reading", path, e))?;
    let title = first_heading(&text).unwrap_or_else(|| stem(path));
    Ok(Source {
        origin: relative(root, path),
        path: path.to_path_buf(),
        target,
        kind: Kind::Markdown,
        title,
        size: u64::try_from(text.len()).unwrap_or(0),
    })
}

/// The text of the first heading of a document.
fn first_heading(text: &str) -> Option<String> {
    doc_markdown::parse(text)
        .first()
        .and_then(doc_markdown::Block::heading_text)
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty())
}

/// The files with `extension` directly inside `dir`, in path order.
fn list(dir: &Path, extension: &str) -> Result<Vec<PathBuf>, Error> {
    let mut found = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(found),
        Err(error) => return Err(Error::path("reading", dir, error)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| Error::path("reading", dir, e))?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == extension) {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}

/// Every Markdown file below `dir`, skipping what a build or an agent
/// wrote and what is only kept to be read.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Error> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Error::path("reading", dir, error)),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::path("reading", dir, e))?;
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if !SKIPPED.iter().any(|skip| name == *skip) {
                walk(&path, out)?;
            }
        } else if path.extension().is_some_and(|ext| ext == "md") {
            found.push(path);
        }
    }
    found.sort();
    out.append(&mut found);
    Ok(())
}

/// Directories no document is read from.
const SKIPPED: [&str; 5] = ["target", ".git", "research", ".claude", "node_modules"];

/// Whether a path is the README of the directory it is in.
fn is_readme(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("README.md"))
}

/// The `name` of the `[package]` section of the manifest in `dir`.
fn package_name(dir: &Path) -> Option<String> {
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package
            && let Some(rest) = line.strip_prefix("name")
            && let Some(value) = rest.trim_start().strip_prefix('=')
        {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

/// The name of a directory.
fn directory_name(dir: &Path) -> String {
    dir.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unnamed")
        .to_owned()
}

/// The file name of a path without its extension.
fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("unnamed")
        .to_owned()
}

/// A path relative to the root, with `/` as the separator.
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The size of a file, or zero when it cannot be read.
fn size_of(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |meta| meta.len())
}

/// `rfc791` becomes `RFC 791`.
fn title_of_rfc(stem: &str) -> String {
    match stem.strip_prefix("rfc") {
        Some(number) if number.chars().all(|c| c.is_ascii_digit()) => format!("RFC {number}"),
        _ => stem.to_owned(),
    }
}
