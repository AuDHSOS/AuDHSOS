// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The trust anchors of the image: from the files of a directory to the
//! table the boot volume carries (D-42, D-147).
//!
//! The directory is an operator's input. Every file in it is one root
//! certificate, written as DER or as PEM with the label `CERTIFICATE`;
//! `audhsos-x509::anchors` writes the table and the program of the image
//! reads it with the same code, so the two cannot disagree about its
//! shape.
//!
//! A checkout with no such directory builds an image with a table of no
//! anchors rather than failing: an image that reaches no HTTPS server is
//! a usable image, and a build that stops because nobody dropped a
//! certificate in is not.

use std::path::{Path, PathBuf};

use audhsos_encoding::pem;
use audhsos_x509::anchors;

use crate::error::Error;
use crate::fs;

/// Where the anchors of a checkout lie.
pub(crate) const ANCHOR_DIR: &str = "anchors";

/// The path the table takes on the boot volume.
pub(crate) const VOLUME_PATH: &str = "AUDHSOS/ANCHORS.BIN";

/// The label a certificate carries in PEM, RFC 7468, section 5.
const LABEL: &str = "CERTIFICATE";

/// The longest body line a PEM certificate of this directory may have.
/// RFC 7468 wraps at 64; a file some other tool wrapped more narrowly
/// reads all the same.
const WRAP: usize = 64;

/// One anchor of the directory: where it came from and what it holds.
pub(crate) struct Anchor {
    /// The file, relative to the checkout.
    pub(crate) name: String,
    /// The certificate, as DER.
    pub(crate) der: Vec<u8>,
}

/// The anchors of `root`, in the order of their file names.
///
/// # Errors
///
/// [`Error::Io`] for a file that cannot be read, and [`Error::Parse`] for
/// one that is neither DER nor PEM with a certificate in it.
pub(crate) fn of(root: &Path) -> Result<Vec<Anchor>, Error> {
    let directory = root.join(ANCHOR_DIR);
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = fs::walk_files(&directory)?
        .into_iter()
        .filter(|path| matches!(fs::extension(path), "der" | "pem" | "crt" | "cer"))
        .collect();
    paths.sort();

    let mut anchors = Vec::new();
    for path in paths {
        let bytes = fs::read_bytes(&path)?;
        let name = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        anchors.push(Anchor {
            der: der_of(&bytes, &name)?,
            name,
        });
    }
    Ok(anchors)
}

/// The table of `anchors`, as the bytes the volume carries.
///
/// # Errors
///
/// [`Error::Parse`] for a list `audhsos-x509` refuses to write: more
/// certificates than a `u16` counts, or one of no bytes.
pub(crate) fn table(anchors: &[Anchor]) -> Result<Vec<u8>, Error> {
    let certificates: Vec<&[u8]> = anchors.iter().map(|anchor| anchor.der.as_slice()).collect();
    let len = anchors::table_len(&certificates)
        .map_err(|error| Error::Parse(format!("the anchors make no table: {error}")))?;
    let mut bytes = vec![0u8; len];
    let written = anchors::write(&certificates, &mut bytes)
        .map_err(|error| Error::Parse(format!("the anchor table would not be written: {error}")))?;
    bytes.truncate(written);
    Ok(bytes)
}

/// The DER of one file, whichever of the two forms it is written in.
fn der_of(bytes: &[u8], name: &str) -> Result<Vec<u8>, Error> {
    if bytes.first() == Some(&0x30) {
        return Ok(bytes.to_vec());
    }
    let wrap = std::num::NonZeroUsize::new(WRAP)
        .ok_or_else(|| Error::Parse("a wrap of zero".to_owned()))?;
    let mut decoded = vec![0u8; bytes.len()];
    let block = pem::decode_wrapped(bytes, &mut decoded, wrap)
        .map_err(|error| Error::Parse(format!("{name} is neither DER nor PEM: {error}")))?;
    if block.label != LABEL {
        return Err(Error::Parse(format!(
            "{name} is a PEM block labelled `{}` and not `{LABEL}`",
            block.label
        )));
    }
    Ok(block.bytes.to_vec())
}
