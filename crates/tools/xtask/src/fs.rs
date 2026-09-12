// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! File system helpers.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::policy::EXCLUDED_DIRECTORIES;

/// Every regular file below `root`, skipping the directories
/// [`EXCLUDED_DIRECTORIES`] names, in path order.
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
            if !EXCLUDED_DIRECTORIES.iter().any(|skip| name == *skip) {
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

/// Creates `path` as `len` zero bytes when nothing is there, leaves what
/// is there as it stands, and answers whether it wrote a file. A failure
/// leaves nothing behind.
///
/// The file is sparse where the file system has sparse files, so a disk
/// nothing has written to costs no blocks.
pub(crate) fn create_sparse(path: &Path, len: u64) -> Result<bool, Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|source| Error::io(format!("creating {}", parent.display()), source))?;
    }
    // The file is claimed rather than asked after: a check that answers
    // whether it is there says nothing about the moment after it, and the
    // create that would follow such a check truncates what it finds.
    let file = match fs::File::create_new(path) {
        Ok(file) => file,
        Err(source) if source.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(source) => return Err(Error::io(format!("creating {}", path.display()), source)),
    };
    file.set_len(len).map_err(|source| {
        // What is left is a file of no length, which the next run would
        // take for the disk it kept.
        let _ = fs::remove_file(path);
        Error::io(format!("sizing {}", path.display()), source)
    })?;
    Ok(true)
}

/// The extension of a path as a string, or an empty string.
pub(crate) fn extension(path: &Path) -> &str {
    path.extension().and_then(|e| e.to_str()).unwrap_or("")
}

/// The file name of a path as a string, or an empty string.
pub(crate) fn file_name(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}
