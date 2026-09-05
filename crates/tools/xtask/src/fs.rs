// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! File system helpers.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Error;

/// Directories never descended into.
const SKIPPED_DIRECTORIES: &[&str] = &["target", ".git"];

/// Every regular file below `root`, skipping build output and version
/// control, in path order.
pub(crate) fn walk_files(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    walk_into(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk_into(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), Error> {
    let entries = fs::read_dir(dir)
        .map_err(|source| Error::io(format!("reading {}", dir.display()), source))?;
    for entry in entries {
        let entry =
            entry.map_err(|source| Error::io(format!("reading {}", dir.display()), source))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| Error::io(format!("inspecting {}", path.display()), source))?;
        if file_type.is_dir() {
            let name = entry.file_name();
            if !SKIPPED_DIRECTORIES.iter().any(|skip| name == *skip) {
                walk_into(&path, files)?;
            }
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

/// Reads a text file.
pub(crate) fn read(path: &Path) -> Result<String, Error> {
    fs::read_to_string(path)
        .map_err(|source| Error::io(format!("reading {}", path.display()), source))
}

/// Reads a binary file.
pub(crate) fn read_bytes(path: &Path) -> Result<Vec<u8>, Error> {
    fs::read(path).map_err(|source| Error::io(format!("reading {}", path.display()), source))
}

/// Writes a binary file, creating the directory it lives in.
pub(crate) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|source| Error::io(format!("creating {}", parent.display()), source))?;
    }
    fs::write(path, bytes)
        .map_err(|source| Error::io(format!("writing {}", path.display()), source))
}

/// The extension of a path as a string, or an empty string.
pub(crate) fn extension(path: &Path) -> &str {
    path.extension().and_then(|e| e.to_str()).unwrap_or("")
}

/// The file name of a path as a string, or an empty string.
pub(crate) fn file_name(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}
