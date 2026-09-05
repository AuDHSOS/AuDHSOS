// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the whole crate: a file with a symbol table and a line
//! program, and the property of the catalog 6.6.53 that no input causes a
//! panic and every lookup either lands inside the file or reports nothing.

use test_support::generators::{bytes, range, vec};
use test_support::property::check;

use crate::{Location, SymbolError, Symbols};

use super::build::{
    LineUnit, Part, Symbol, elf, end_sequence, set_address, special, symbol_table, uleb,
};

/// Where the function and the rows of the test file sit.
const BASE: u64 = 0x1000;

/// A file with two functions, a line program over the first, and the
/// string sections a version 5 table would want.
fn file(version: u16) -> Vec<u8> {
    let mut program = set_address(BASE);
    program.push(special(0, 4)); // line 5
    program.push(special(0x10, 3)); // line 8
    program.push(2);
    program.extend_from_slice(&uleb(0x10));
    program.extend_from_slice(&end_sequence());
    let unit = LineUnit {
        version,
        directory: "/src/kernel",
        file: "boot.rs",
        program,
        extra_opcodes: 0,
    }
    .bytes();
    let mut parts = symbol_table(&[
        Symbol::function("kernel_entry", BASE, 0x20),
        Symbol::function("on_trap", 0x2000, 0x10),
    ]);
    parts.push(Part::bits(".debug_line", unit));
    parts.push(Part::bits(".text", vec![0x90; 32]));
    elf(parts)
}

#[test]
fn an_address_resolves_to_a_function_a_file_and_a_line() {
    for version in [4u16, 5] {
        let bytes = file(version);
        let symbols = Symbols::parse(&bytes).expect("the file parses");
        let found = symbols.resolve(BASE + 0x14).expect("a lookup");
        assert_eq!(
            found,
            Location {
                function: Some("kernel_entry"),
                file: Some("boot.rs"),
                directory: Some("/src/kernel"),
                line: 8,
                column: 0,
            },
            "version {version}"
        );
        assert!(!found.is_empty());
    }
}

#[test]
fn a_function_without_a_line_program_row_still_gets_its_name() {
    let bytes = file(4);
    let symbols = Symbols::parse(&bytes).expect("the file parses");
    let found = symbols.resolve(0x2004).expect("a lookup");
    assert_eq!(found.function, Some("on_trap"));
    assert_eq!(found.file, None);
    assert_eq!(found.line, 0);
    assert!(!found.is_empty());
}

#[test]
fn an_address_in_nothing_reports_nothing() {
    let bytes = file(4);
    let symbols = Symbols::parse(&bytes).expect("the file parses");
    let found = symbols.resolve(0x5000).expect("a lookup");
    assert!(found.is_empty());
    assert_eq!(found, Location::default());
}

#[test]
fn a_file_without_debug_sections_still_names_functions() {
    let bytes = elf(symbol_table(&[Symbol::function("only", BASE, 0x10)]));
    let symbols = Symbols::parse(&bytes).expect("the file parses");
    assert!(symbols.lines().is_empty());
    assert!(!symbols.functions().is_empty());
    let found = symbols.resolve(BASE).expect("a lookup");
    assert_eq!(found.function, Some("only"));
    assert_eq!(found.file, None);
}

#[test]
fn a_file_that_is_not_an_elf_object_is_refused() {
    assert!(matches!(
        Symbols::parse(&[0u8; 8]),
        Err(SymbolError::Elf(_))
    ));
    assert!(matches!(
        Symbols::parse(&[0u8; 128]),
        Err(SymbolError::Elf(_))
    ));
    let error = Symbols::parse(&[0u8; 64]).expect_err("not an ELF object");
    assert!(!format!("{error}").is_empty());
    assert!(!format!("{error:?}").is_empty());
}

#[test]
fn a_broken_line_program_still_yields_the_function_name() {
    let mut bytes = file(4);
    // Break the version of the line program, which the unit holds right
    // after its four-byte length.
    let at = {
        let sections = audhsos_elf::sections::sections(&bytes).expect("the sections");
        let section = sections.by_name(".debug_line").expect("the line program");
        usize::try_from(section.offset).expect("an offset") + 4
    };
    if let Some(slot) = bytes.get_mut(at) {
        *slot = 99;
    }
    let symbols = Symbols::parse(&bytes).expect("the file parses");
    let found = symbols.resolve_or_name(BASE + 4);
    assert_eq!(found.function, Some("kernel_entry"));
    assert_eq!(found.file, None);
}

#[test]
fn every_error_of_the_crate_reads_as_a_sentence() {
    let errors = [
        SymbolError::Truncated,
        SymbolError::Overflow,
        SymbolError::LineVersion(7),
        SymbolError::LineRange,
        SymbolError::UnknownForm(0x99),
    ];
    for error in errors {
        assert!(!format!("{error}").is_empty(), "{error:?}");
    }
}

#[test]
fn property_no_file_makes_a_lookup_panic() {
    let generator = vec(bytes(0..=200), 0..=4);
    check(
        "no file makes a lookup panic",
        &generator,
        |chunks: &Vec<Vec<u8>>| {
            let mut file = Vec::new();
            for chunk in chunks {
                file.extend_from_slice(chunk);
            }
            // Whatever the bytes are, the crate answers or fails; it never
            // panics and never reports a place it did not read.
            if let Ok(symbols) = Symbols::parse(&file) {
                for address in [0u64, 1, BASE, u64::MAX] {
                    let _ = symbols.resolve(address);
                    let _ = symbols.resolve_or_name(address);
                }
            }
            Ok(())
        },
    );
}

#[test]
fn property_a_lookup_lands_in_the_function_it_names() {
    let generator = range(0u64..=0x40);
    check(
        "a lookup lands in the function it names",
        &generator,
        |offset: &u64| {
            let bytes = file(4);
            let symbols = Symbols::parse(&bytes).map_err(|error| format!("{error}"))?;
            let address = BASE.wrapping_add(*offset);
            let found = symbols
                .resolve(address)
                .map_err(|error| format!("{error}"))?;
            let named = symbols.functions().at(address);
            match (found.function, named) {
                (Some(name), Some(function)) if function.name == name => {
                    if function.contains(address) {
                        Ok(())
                    } else {
                        Err(format!("{name} does not cover {address:#x}"))
                    }
                }
                (None, None) => Ok(()),
                _ => Err(format!("{address:#x} is named inconsistently")),
            }
        },
    );
}
