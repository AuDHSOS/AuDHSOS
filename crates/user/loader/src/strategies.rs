// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Generators of archives for property tests and for the seed corpus of
//! the fuzz target. Available behind the feature `test-strategies` and in
//! this crate's own tests.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use test_support::generators::{BoxGen, Generator, bytes, one_of, pair, vec as gen_vec};

use crate::tar::{Builder, Kind, archive_len};

/// A file of an archive: a name and its bytes.
pub type File = (Vec<u8>, Vec<u8>);

/// A name out of a small alphabet, so that the same names come up again and
/// the reader's comparison is exercised rather than only its parser.
#[must_use]
pub fn any_name() -> BoxGen<Vec<u8>> {
    one_of(vec![
        b"server-memory".to_vec(),
        b"server-name".to_vec(),
        b"server-console".to_vec(),
        b"app-hello".to_vec(),
        b"a".to_vec(),
        b"boot/programs/server-console".to_vec(),
    ])
    .boxed()
}

/// A file with a name and up to `len` bytes of content.
#[must_use]
pub fn any_file(len: usize) -> BoxGen<File> {
    pair(any_name(), bytes(0..=len)).boxed()
}

/// An archive of up to four files.
#[must_use]
pub fn any_archive() -> BoxGen<Vec<u8>> {
    gen_vec(any_file(600), 0..=4)
        .map(|files| write(&files))
        .boxed()
}

/// The bytes of an archive holding `files`.
#[must_use]
pub fn write(files: &[File]) -> Vec<u8> {
    let borrowed: Vec<(&[u8], &[u8])> = files
        .iter()
        .map(|(name, data)| (name.as_slice(), data.as_slice()))
        .collect();
    let mut bytes = vec![0u8; archive_len(&borrowed)];
    let written = {
        let mut builder = Builder::new(&mut bytes);
        // The buffer was sized by `archive_len` for exactly these files,
        // so no entry can fail to fit.
        for (name, data) in &borrowed {
            let _written = builder.entry(name, Kind::File, data);
        }
        builder.finish().unwrap_or(0)
    };
    bytes.truncate(written);
    bytes
}
