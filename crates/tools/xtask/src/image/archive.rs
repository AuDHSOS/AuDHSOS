// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The archive of the boot image: the programs the root task starts.
//!
//! It is written with the same code that reads it, `user_loader::tar`, so
//! that what the xtask produces and what the root task walks cannot drift
//! apart. The check on it is the reader: an archive this module writes is
//! read back before it is handed on.

use user_loader::tar::{Builder, archive_len};

use crate::error::Error;

/// The programs of the archive, in the order the start table names them.
///
/// The order does not matter to the reader, which searches by name, and it
/// is kept because a dump of the image is easier to read that way.
pub(crate) const PROGRAMS: [&str; 11] = [
    "server-memory",
    "server-name",
    "server-console",
    "server-display",
    "server-input",
    "app-hello",
    "app-checks",
    "app-paint",
    "app-input",
    "app-canvas",
    "app-faulter",
];

/// Writes a ustar archive holding `files`.
///
/// # Errors
///
/// [`Error::Usage`] when a name does not fit the header fields, and when
/// the archive that was written does not read back — which would be a
/// disagreement between the writer and the reader of one crate.
pub(crate) fn build(files: &[(&str, Vec<u8>)]) -> Result<Vec<u8>, Error> {
    let borrowed: Vec<(&[u8], &[u8])> = files
        .iter()
        .map(|(name, data)| (name.as_bytes(), data.as_slice()))
        .collect();
    let mut bytes = vec![0u8; archive_len(&borrowed)];
    let written = {
        let mut builder = Builder::new(&mut bytes);
        for (name, data) in &borrowed {
            builder
                .file(name, data)
                .map_err(|error| Error::Usage(format!("the archive: {error}")))?;
        }
        builder
            .finish()
            .map_err(|error| Error::Usage(format!("the archive: {error}")))?
    };
    bytes.truncate(written);
    check(&bytes, files)?;
    Ok(bytes)
}

/// Reads the archive back and insists it holds what it was given.
fn check(bytes: &[u8], files: &[(&str, Vec<u8>)]) -> Result<(), Error> {
    let archive = user_loader::tar::Archive::new(bytes);
    for (name, data) in files {
        let found = archive
            .find(name.as_bytes())
            .map_err(|error| Error::Usage(format!("the archive does not read: {error}")))?;
        let entry = found
            .ok_or_else(|| Error::Usage(format!("the archive was written without `{name}`")))?;
        if entry.data != data.as_slice() {
            return Err(Error::Usage(format!(
                "`{name}` reads back as {} bytes, not {}",
                entry.data.len(),
                data.len()
            )));
        }
    }
    Ok(())
}
