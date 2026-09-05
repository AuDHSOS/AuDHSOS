// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::functions`, covering the symbol table items of the
//! catalog 6.6.53.

use audhsos_elf::sections::sections;

use crate::functions::Functions;

use super::build::{Part, Symbol, elf, symbol_table};

/// A file whose symbol table holds two functions and one other symbol.
fn file() -> Vec<u8> {
    let mut parts = symbol_table(&[
        Symbol::function("start", 0x1000, 0x40),
        Symbol::function("handler", 0x2000, 0x10),
        Symbol {
            name: "a_variable",
            start: 0x1010,
            size: 8,
            kind: 1,
        },
    ]);
    parts.push(Part::bits(".text", vec![0x90; 16]));
    elf(parts)
}

#[test]
fn an_address_inside_a_function_names_it() {
    let bytes = file();
    let sections = sections(&bytes).unwrap();
    let functions = Functions::new(&sections);
    assert_eq!(functions.at(0x1020).map(|f| f.name), Some("start"));
    assert_eq!(functions.at(0x2004).map(|f| f.name), Some("handler"));
}

#[test]
fn the_first_and_the_last_byte_belong_to_the_function_and_the_next_does_not() {
    let bytes = file();
    let sections = sections(&bytes).unwrap();
    let functions = Functions::new(&sections);
    assert_eq!(functions.at(0x1000).map(|f| f.name), Some("start"));
    assert_eq!(functions.at(0x103F).map(|f| f.name), Some("start"));
    assert_eq!(functions.at(0x1040).map(|f| f.name), None);
}

#[test]
fn an_address_in_no_function_names_none() {
    let bytes = file();
    let sections = sections(&bytes).unwrap();
    let functions = Functions::new(&sections);
    assert_eq!(functions.at(0), None);
    assert_eq!(functions.at(0x1800), None);
    assert_eq!(functions.at(u64::MAX), None);
}

#[test]
fn a_symbol_that_is_not_a_function_is_not_reported() {
    let bytes = file();
    let sections = sections(&bytes).unwrap();
    let functions = Functions::new(&sections);
    assert_eq!(functions.at(0x1014).map(|f| f.name), Some("start"));
    assert!(functions.iter().all(|f| f.name != "a_variable"));
}

#[test]
fn the_narrowest_function_wins_when_one_encloses_another() {
    let parts = symbol_table(&[
        Symbol::function("outer", 0x1000, 0x100),
        Symbol::function("inner", 0x1040, 0x10),
    ]);
    let bytes = elf(parts);
    let sections = sections(&bytes).unwrap();
    let functions = Functions::new(&sections);
    assert_eq!(functions.at(0x1044).map(|f| f.name), Some("inner"));
    assert_eq!(functions.at(0x1080).map(|f| f.name), Some("outer"));
}

#[test]
fn a_function_of_no_size_covers_its_first_byte_alone() {
    let bytes = elf(symbol_table(&[Symbol::function("entry", 0x3000, 0)]));
    let sections = sections(&bytes).unwrap();
    let functions = Functions::new(&sections);
    assert_eq!(functions.at(0x3000).map(|f| f.name), Some("entry"));
    assert_eq!(functions.at(0x3001), None);
}

#[test]
fn a_file_with_no_symbol_table_reports_nothing() {
    let bytes = elf(vec![Part::bits(".text", vec![0x90; 8])]);
    let sections = sections(&bytes).unwrap();
    let functions = Functions::new(&sections);
    assert!(functions.is_empty());
    assert_eq!(functions.len(), 0);
    assert_eq!(functions.at(0x1000), None);
    assert!(Functions::empty().is_empty());
}

#[test]
fn the_table_reports_what_it_holds() {
    let bytes = file();
    let sections = sections(&bytes).unwrap();
    let functions = Functions::new(&sections);
    assert_eq!(functions.len(), 4, "the empty first entry counts");
    assert_eq!(functions.iter().count(), 2);
    assert_eq!(functions.get(9999), None);
    let start = functions.at(0x1000).unwrap();
    assert_eq!((start.start, start.size), (0x1000, 0x40));
    assert!(!format!("{start:?}").is_empty());
}
