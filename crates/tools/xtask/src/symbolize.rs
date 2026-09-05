// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Turning the addresses a kernel prints over the serial line into places
//! in the source.
//!
//! Invariant: a report is a comment on the output, never a replacement for
//! it: an address that resolves to nothing is left alone, and a file that
//! carries no symbols costs the run nothing.

use std::collections::BTreeSet;
use std::path::Path;

use audhsos_abi::layout::KERNEL_SPACE_START;
use audhsos_symbols::Symbols;

use crate::error::Error;
use crate::fs;

/// The prefix a hexadecimal address carries in the kernel's output.
const PREFIX: &str = "0x";

/// The shortest address this looks at, in hexadecimal digits. A kernel
/// address is sixteen digits; anything shorter is an error code or a
/// vector, not a place in the code.
const DIGITS: usize = 9;

/// Every address in the kernel half that `output` names, in order and
/// without repetition.
#[must_use]
pub(crate) fn addresses_in(output: &str) -> Vec<u64> {
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for (index, _) in output.match_indices(PREFIX) {
        let Some(rest) = output.get(index.saturating_add(PREFIX.len())..) else {
            continue;
        };
        let end = rest
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(rest.len());
        let digits = match rest.get(..end) {
            Some(digits) if digits.len() >= DIGITS => digits,
            _ => continue,
        };
        let Ok(address) = u64::from_str_radix(digits, 16) else {
            continue;
        };
        if address >= KERNEL_SPACE_START && seen.insert(address) {
            found.push(address);
        }
    }
    found
}

/// One address and what the file says about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// The address as the kernel printed it.
    pub(crate) address: u64,
    /// The function it falls in.
    pub(crate) function: Option<String>,
    /// The file and the line, as `dir/file:line`.
    pub(crate) place: Option<String>,
}

impl std::fmt::Display for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#018x}", self.address)?;
        match &self.function {
            Some(name) => write!(f, " {name}")?,
            None => f.write_str(" <unknown>")?,
        }
        if let Some(place) = &self.place {
            write!(f, " at {place}")?;
        }
        Ok(())
    }
}

/// What `symbols` says about every address of `addresses`.
#[must_use]
pub(crate) fn resolve_all(symbols: &Symbols<'_>, addresses: &[u64]) -> Vec<Resolved> {
    addresses
        .iter()
        .map(|address| {
            let found = symbols.resolve_or_name(*address);
            Resolved {
                address: *address,
                function: found.function.map(str::to_owned),
                place: place_of(found.directory, found.file, found.line),
            }
        })
        .filter(|resolved| resolved.function.is_some() || resolved.place.is_some())
        .collect()
}

/// The file and the line as one string, when the line program named them.
fn place_of(directory: Option<&str>, file: Option<&str>, line: u32) -> Option<String> {
    let file = file?;
    match directory {
        Some(directory) if !file.starts_with('/') => Some(format!("{directory}/{file}:{line}")),
        _ => Some(format!("{file}:{line}")),
    }
}

/// Resolves every kernel address of `output` against the ELF file at
/// `image` and writes what it found on the standard error output.
///
/// A file that cannot be read, or carries no symbols, produces nothing:
/// the report is a comment on a run that already failed and must not fail
/// it a second time.
pub(crate) fn report(image: &Path, output: &str) {
    let addresses = addresses_in(output);
    if addresses.is_empty() {
        return;
    }
    let Ok(bytes) = fs::read_bytes(image) else {
        return;
    };
    let Ok(symbols) = Symbols::parse(&bytes) else {
        return;
    };
    let resolved = resolve_all(&symbols, &addresses);
    if resolved.is_empty() {
        return;
    }
    eprintln!("--- addresses of {} ---", image.display());
    for line in resolved {
        eprintln!("  {line}");
    }
}

/// The `symbolize` subcommand: an ELF file and the addresses to look up.
///
/// # Errors
///
/// [`Error::Usage`] without a file or without an address, and if an
/// address is not a hexadecimal number; the errors of reading the file.
pub(crate) fn command(options: &[String]) -> Result<(), Error> {
    let image = options
        .first()
        .ok_or_else(|| Error::Usage("symbolize needs the path of an ELF file".to_owned()))?;
    let rest = options.get(1..).unwrap_or(&[]);
    if rest.is_empty() {
        return Err(Error::Usage(
            "symbolize needs at least one address".to_owned(),
        ));
    }
    let bytes = fs::read_bytes(Path::new(image))?;
    let symbols =
        Symbols::parse(&bytes).map_err(|error| Error::Usage(format!("{image}: {error}")))?;
    let mut addresses = Vec::new();
    for text in rest {
        let digits = text.trim_start_matches("0x");
        let address = u64::from_str_radix(digits, 16)
            .map_err(|_| Error::Usage(format!("`{text}` is not a hexadecimal address")))?;
        addresses.push(address);
    }
    for address in addresses {
        let found = symbols.resolve_or_name(address);
        println!(
            "{}",
            Resolved {
                address,
                function: found.function.map(str::to_owned),
                place: place_of(found.directory, found.file, found.line),
            }
        );
    }
    Ok(())
}
