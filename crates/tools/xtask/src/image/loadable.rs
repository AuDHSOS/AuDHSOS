// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The part of a program the machine reads: the ELF header, the program
//! header table, and the file bytes of every segment.
//!
//! What the linker writes behind those is the symbol table, the section
//! headers and the debug information, which no loader of this system
//! opens: `audhsos-elf` reads the header and the program headers and
//! ignores `e_shoff` altogether. In a build without optimization that tail
//! is about ninety-five per cent of the file, and every byte of it is read
//! off the volume two kibibytes per message (D-92) before a program starts.
//!
//! So the image writer cuts each program there. What it wrote is read back
//! with the same reader the root task uses, and the regions of the two have
//! to agree; a cut that lost something is a build that fails rather than a
//! machine that faults.

use user_loader::{Region, plan};

use crate::error::Error;

/// Where the program header table of `bytes` ends, which is the one part
/// the loader reads that lies outside every segment.
///
/// The three fields are read out of the header by their offsets of the
/// ELF64 specification: `e_phoff` at 32, `e_phentsize` at 54, `e_phnum` at
/// 56.
fn table_end(bytes: &[u8]) -> Option<u64> {
    let word = |at: usize, len: usize| -> Option<u64> {
        let mut value = 0u64;
        for (index, byte) in bytes.get(at..at.checked_add(len)?)?.iter().enumerate() {
            value |= u64::from(*byte).checked_shl(u32::try_from(index).ok()?.checked_mul(8)?)?;
        }
        Some(value)
    };
    let phoff = word(32, 8)?;
    let phentsize = word(54, 2)?;
    let phnum = word(56, 2)?;
    phoff.checked_add(phentsize.checked_mul(phnum)?)
}

/// The first byte of `bytes` the loader does not read.
fn end_of_what_is_read(bytes: &[u8], regions: &[Region]) -> Option<u64> {
    let mut end = table_end(bytes)?;
    for region in regions {
        let last = region.file_offset.checked_add(region.file_size)?;
        end = end.max(last);
    }
    Some(end)
}

/// `bytes` without the part no loader of this system reads.
///
/// # Errors
///
/// [`Error::Usage`] when the file is no program this system loads, when
/// the end of what is read is not a length of this machine, and when the
/// bytes that were kept do not read back as the same regions.
pub(crate) fn trim(name: &str, bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let before = plan(bytes).map_err(|error| {
        Error::Usage(format!("`{name}` is no program this system loads: {error}"))
    })?;
    let regions: Vec<Region> = before.regions().collect();
    let end = end_of_what_is_read(bytes, &regions)
        .and_then(|end| usize::try_from(end).ok())
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| Error::Usage(format!("`{name}` names bytes it does not hold")))?;
    let kept = bytes.get(..end).unwrap_or(bytes).to_vec();
    let after = plan(&kept).map_err(|error| {
        Error::Usage(format!(
            "`{name}` does not read back after the cut at {end}: {error}"
        ))
    })?;
    let again: Vec<Region> = after.regions().collect();
    if again != regions || after.entry != before.entry {
        return Err(Error::Usage(format!(
            "`{name}` reads back as another program after the cut at {end}"
        )));
    }
    Ok(kept)
}
