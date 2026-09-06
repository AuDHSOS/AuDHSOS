// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The ustar reader against arbitrary bytes: no input may panic, no input
//! may loop, and every entry the reader hands out must lie inside the bytes
//! it was given and carry a name that stays inside the archive.

use user_loader::tar::{Archive, TarError};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let archive = Archive::new(bytes);
    let base = bytes.as_ptr() as usize;
    let end = base.wrapping_add(bytes.len());
    for entry in archive.entries() {
        let Ok(entry) = entry else {
            // The walk stops at the first entry that does not read, so an
            // error is the last item and there is nothing after it.
            break;
        };
        let start = entry.data.as_ptr() as usize;
        assert!(
            start >= base && start.wrapping_add(entry.data.len()) <= end,
            "an entry points outside the archive"
        );
        let name = entry.path.as_bytes();
        assert!(!name.is_empty(), "an entry with no name");
        assert!(name.first() != Some(&b'/'), "an absolute name was accepted");
        assert!(
            !name.split(|byte| *byte == b'/').any(|part| part == b".."),
            "a name with a parent component was accepted"
        );
    }
    // What the search finds is what was asked for, and a search that fails
    // fails for the same reason the walk does.
    //
    // The other direction does not hold and must not be asserted: a search
    // stops at its match, so it can succeed over an archive whose later
    // headers are broken. The fuzzer found exactly that, and the input is
    // in the corpus as `find-stops-before-a-broken-header`.
    let walked: Result<(), TarError> = archive.entries().try_fold((), |(), entry| entry.map(|_| ()));
    match archive.find(b"server-console") {
        Ok(Some(entry)) => assert_eq!(
            entry.path.as_bytes(),
            b"server-console",
            "the search found something else"
        ),
        Ok(None) => assert!(walked.is_ok(), "a search that found nothing over a broken archive"),
        Err(error) => assert_eq!(
            Err(error),
            walked,
            "the search and the walk disagree about why the archive is broken"
        ),
    }
});
