// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Where a program lies on the volume, and what it is called there.
//!
//! The archive names a program `app-canvas`; a FAT32 directory entry holds
//! eight characters of a name and three of an extension (D-09), so the
//! same program is the file `APPCANVA.ELF`. The rule is here because two
//! programs have to agree on it: the tool that writes the volume and the
//! root task that reads it.
//!
//! The rule: take the letters and the digits of the name, in order, upper
//! case, at most eight of them, and give the file the extension `ELF`. A
//! hyphen is dropped because 8.3 has no room for it and no meaning for it.
//!
//! Invariant: a name this module answers is one `fs-fat` accepts, so a
//! caller that got a name never has to check it again.

/// The directory the programs of this system lie in on the volume.
pub const DIRECTORY: [&str; 2] = ["AUDHSOS", "BIN"];

/// The extension every program file carries.
pub const EXTENSION: &str = "ELF";

/// Characters of the stem of an 8.3 name.
pub const STEM: usize = 8;

/// Bytes of a name as a client of the file protocol writes it: the stem,
/// the dot, and the extension.
pub const NAME_LEN: usize = STEM + 1 + 3;

/// The file `program` lies in, as `STEM.ELF`, and how many bytes of the
/// answer are the name.
///
/// Answers `None` for a name with no letter and no digit in it, which is
/// no program of this system.
#[must_use]
pub fn file_name(program: &[u8]) -> Option<([u8; NAME_LEN], usize)> {
    let mut name = [0u8; NAME_LEN];
    let mut len = 0usize;
    for byte in program.iter().copied().filter(|byte| kept(*byte)) {
        if len >= STEM {
            break;
        }
        if let Some(slot) = name.get_mut(len) {
            *slot = byte.to_ascii_uppercase();
            len = len.saturating_add(1);
        }
    }
    if len == 0 {
        return None;
    }
    for byte in b".".iter().chain(EXTENSION.as_bytes()) {
        if let Some(slot) = name.get_mut(len) {
            *slot = *byte;
            len = len.saturating_add(1);
        }
    }
    Some((name, len))
}

/// Whether a byte of a program name stands in the file name.
const fn kept(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}
