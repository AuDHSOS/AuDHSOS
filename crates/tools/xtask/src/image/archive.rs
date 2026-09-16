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

/// The programs of the archive: the boot set, and nothing else.
///
/// The root task cannot read a file before the file system server runs,
/// and that server is itself a file, so the programs that must start
/// before a file is readable come from somewhere that needs no file
/// system server. That somewhere is this archive. Every other program is
/// on the volume ([`ON_THE_VOLUME`]).
///
/// The order does not matter to the reader, which searches by name, and it
/// is kept because a dump of the image is easier to read that way.
pub(crate) const PROGRAMS: [&str; 4] = [
    "server-memory",
    "server-name",
    "server-console",
    "server-fs",
];

/// The programs that lie on the volume, in the order the start table names
/// them.
///
/// Each is written under `AUDHSOS/BIN/` as `user_loader::volume::file_name`
/// spells it.
pub(crate) const ON_THE_VOLUME: [&str; 16] = [
    "server-display",
    "server-input",
    "app-hello",
    "app-checks",
    "app-paint",
    "app-input",
    "app-canvas",
    "app-lspci",
    "server-desk",
    "server-net",
    "app-net",
    "app-ssh",
    "app-tls",
    "app-files",
    "app-shell",
    "app-faulter",
];

/// The programs a desktop image carries: the servers the compositor needs,
/// the compositor, and the shell that runs in a window of it.
///
/// The root task starts the programs of its own table whatever the volume
/// holds, and reports the ones it does not find; a machine built this way
/// therefore boots in seconds rather than minutes, because the programs it
/// leaves out are the megabytes of the network stack and of the
/// demonstrations, read off the volume two kibibytes at a time (D-92).
pub(crate) const DESKTOP: [&str; 5] = [
    "server-display",
    "server-input",
    "server-desk",
    "server-net",
    "app-shell",
];

/// The programs a desktop image starts at boot: the servers, the
/// compositor, and the shell that opens the one window of it.
pub(crate) const DESKTOP_START: [&str; 5] = [
    "server-display",
    "server-input",
    "server-net",
    "server-desk",
    "app-shell",
];

/// Where the list of what to start at boot lies on the volume.
pub(crate) const START_LIST_PATH: &str = "AUDHSOS/START.TXT";

/// The bytes of that list: one name per line.
pub(crate) fn start_list(names: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for name in names {
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

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
