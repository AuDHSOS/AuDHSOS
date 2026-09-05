// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod cursor;
pub mod error;
pub mod functions;
pub mod line;

pub use error::SymbolError;
pub use functions::{Function, Functions};
pub use line::{LineProgram, Row, Strings};

use audhsos_elf::sections::{Sections, sections};

/// Where an address is in the source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Location<'a> {
    /// The function the address falls in, if the symbol table names one.
    pub function: Option<&'a str>,
    /// The file, if the line program names one.
    pub file: Option<&'a str>,
    /// The directory of that file, if the line program names one.
    pub directory: Option<&'a str>,
    /// The line, or zero.
    pub line: u32,
    /// The column, or zero.
    pub column: u32,
}

impl Location<'_> {
    /// `true` if nothing about the address could be said.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.function.is_none() && self.file.is_none()
    }
}

/// The symbols of one ELF file.
#[derive(Clone, Copy, Debug)]
pub struct Symbols<'a> {
    functions: Functions<'a>,
    lines: LineProgram<'a>,
}

impl<'a> Symbols<'a> {
    /// Reads the symbol table and the line program of `bytes`.
    ///
    /// A file without a symbol table or without a line program is not an
    /// error: what is missing is reported as `None` at every lookup.
    ///
    /// # Errors
    ///
    /// [`SymbolError::Elf`] if the file is not an ELF object this crate
    /// can read.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, SymbolError> {
        let sections = sections(bytes)?;
        Ok(Self::from_sections(&sections))
    }

    /// The symbols of a file whose sections the caller already read.
    #[must_use]
    pub fn from_sections(sections: &Sections<'a>) -> Self {
        let strings = Strings {
            debug_str: sections.content_by_name(".debug_str").unwrap_or(&[]),
            debug_line_str: sections.content_by_name(".debug_line_str").unwrap_or(&[]),
        };
        Symbols {
            functions: Functions::new(sections),
            lines: LineProgram::new(
                sections.content_by_name(".debug_line").unwrap_or(&[]),
                strings,
            ),
        }
    }

    /// The functions of the file.
    #[must_use]
    pub const fn functions(&self) -> &Functions<'a> {
        &self.functions
    }

    /// The line program of the file.
    #[must_use]
    pub const fn lines(&self) -> &LineProgram<'a> {
        &self.lines
    }

    /// Where `address` is in the source.
    ///
    /// # Errors
    ///
    /// The errors of [`LineProgram::row_for`]. The function is reported
    /// even when the line program cannot be read, because a name alone is
    /// worth more than nothing.
    pub fn resolve(&self, address: u64) -> Result<Location<'a>, SymbolError> {
        let function = self.functions.at(address).map(|found| found.name);
        let row = self.lines.row_for(address)?;
        Ok(Location {
            function,
            file: row.and_then(|row| row.file),
            directory: row.and_then(|row| row.directory),
            line: row.map_or(0, |row| row.line),
            column: row.map_or(0, |row| row.column),
        })
    }

    /// Where `address` is in the source, with the name alone when the line
    /// program cannot be read.
    #[must_use]
    pub fn resolve_or_name(&self, address: u64) -> Location<'a> {
        self.resolve(address).unwrap_or(Location {
            function: self.functions.at(address).map(|found| found.name),
            ..Location::default()
        })
    }
}

#[cfg(test)]
mod tests;
