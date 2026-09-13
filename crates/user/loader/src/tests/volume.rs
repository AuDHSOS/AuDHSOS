// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::volume`.

use crate::volume::{NAME_LEN, STEM, file_name};

/// The name `program` lies under, as text.
fn spelled(program: &[u8]) -> String {
    let (name, len) = file_name(program).expect("a program has a name");
    String::from_utf8(name.get(..len).unwrap().to_vec()).expect("the name is text")
}

#[test]
fn a_name_loses_what_an_eight_three_entry_has_no_room_for() {
    assert_eq!(spelled(b"app-canvas"), "APPCANVA.ELF");
    assert_eq!(spelled(b"app-hello"), "APPHELLO.ELF");
    assert_eq!(spelled(b"server-display"), "SERVERDI.ELF");
    assert_eq!(spelled(b"server-fs"), "SERVERFS.ELF");
}

#[test]
fn a_name_shorter_than_the_stem_keeps_every_character_it_has() {
    assert_eq!(spelled(b"a"), "A.ELF");
    assert_eq!(spelled(b"init"), "INIT.ELF");
}

#[test]
fn a_stem_is_never_longer_than_the_eight_characters_an_entry_holds() {
    let (_, len) = file_name(b"a-very-long-program-name").unwrap();
    assert_eq!(len, NAME_LEN);
    assert_eq!(spelled(b"a-very-long-program-name").len(), STEM + 4);
}

#[test]
fn a_name_with_no_letter_and_no_digit_names_no_file() {
    assert_eq!(file_name(b"---"), None);
    assert_eq!(file_name(b""), None);
}

#[test]
fn every_program_of_this_system_has_a_name_of_its_own() {
    let programs = [
        "server-memory",
        "server-name",
        "server-console",
        "server-fs",
        "server-display",
        "server-input",
        "app-hello",
        "app-checks",
        "app-paint",
        "app-input",
        "app-canvas",
        "app-lspci",
        "app-files",
        "app-faulter",
    ];
    let mut names: Vec<String> = programs
        .iter()
        .map(|program| spelled(program.as_bytes()))
        .collect();
    names.sort();
    let count = names.len();
    names.dedup();
    assert_eq!(names.len(), count, "two programs share one file name");
}
