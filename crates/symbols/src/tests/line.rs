// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::line`, covering the line program items of the catalog
//! 6.6.53.

use crate::error::SymbolError;
use crate::line::{LineProgram, Strings};

use super::build::{LineUnit, end_sequence, set_address, sleb, special, special_with_base, uleb};

/// Where the rows of the test program start.
const BASE: u64 = 0x1000;

/// A program with two rows and an end of sequence above them.
fn program() -> Vec<u8> {
    let mut program = set_address(BASE);
    program.push(special(0, 4)); // 0x1000, line 5
    program.push(special(0x10, 2)); // 0x1010, line 7
    program.push(2); // DW_LNS_advance_pc
    program.extend_from_slice(&uleb(0x10)); // to 0x1020
    program.extend_from_slice(&end_sequence());
    program
}

/// A unit of the given version over the program above.
fn unit(version: u16) -> Vec<u8> {
    LineUnit {
        version,
        directory: "/src",
        file: "main.rs",
        program: program(),
        extra_opcodes: 0,
    }
    .bytes()
}

fn lines(bytes: &[u8]) -> LineProgram<'_> {
    LineProgram::new(bytes, Strings::default())
}

#[test]
fn a_version_four_program_answers_for_the_addresses_of_its_rows() {
    let bytes = unit(4);
    let program = lines(&bytes);
    assert!(!program.is_empty());
    for (address, line) in [
        (BASE, 5),
        (BASE + 0xF, 5),
        (BASE + 0x10, 7),
        (BASE + 0x1F, 7),
    ] {
        let row = program.row_for(address).unwrap().expect("a row");
        assert_eq!(row.line, line, "at {address:#x}");
        assert_eq!(row.file, Some("main.rs"));
        assert_eq!(row.directory, Some("/src"));
    }
}

#[test]
fn a_version_five_program_answers_the_same_way() {
    let bytes = unit(5);
    let program = lines(&bytes);
    for (address, line) in [(BASE, 5), (BASE + 0x10, 7)] {
        let row = program.row_for(address).unwrap().expect("a row");
        assert_eq!(row.line, line, "at {address:#x}");
        assert_eq!(row.file, Some("main.rs"));
        assert_eq!(row.directory, Some("/src"));
    }
}

#[test]
fn an_address_before_the_first_row_and_one_at_the_end_report_nothing() {
    for version in [4u16, 5] {
        let bytes = unit(version);
        let program = lines(&bytes);
        assert_eq!(program.row_for(BASE - 1).unwrap(), None, "{version}");
        assert_eq!(program.row_for(BASE + 0x20).unwrap(), None, "{version}");
        assert_eq!(program.row_for(u64::MAX).unwrap(), None, "{version}");
    }
}

#[test]
fn the_standard_opcodes_move_the_machine() {
    let mut program = set_address(BASE);
    program.push(3); // DW_LNS_advance_line
    program.extend_from_slice(&sleb(41)); // line 42
    program.push(5); // DW_LNS_set_column
    program.extend_from_slice(&uleb(7));
    program.push(6); // DW_LNS_negate_stmt
    program.push(7); // DW_LNS_set_basic_block
    program.push(10); // DW_LNS_set_prologue_end
    program.push(11); // DW_LNS_set_epilogue_begin
    program.push(12); // DW_LNS_set_isa
    program.extend_from_slice(&uleb(1));
    program.push(1); // DW_LNS_copy emits the row
    program.push(9); // DW_LNS_fixed_advance_pc
    program.extend_from_slice(&0x20u16.to_le_bytes());
    program.push(3); // DW_LNS_advance_line
    program.extend_from_slice(&sleb(-11)); // line 31
    program.push(1); // DW_LNS_copy
    program.push(8); // DW_LNS_const_add_pc, which advances by 17 here
    program.extend_from_slice(&end_sequence());
    let bytes = LineUnit {
        version: 4,
        directory: "/src",
        file: "main.rs",
        program,
        extra_opcodes: 0,
    }
    .bytes();
    let lines = lines(&bytes);
    let first = lines.row_for(BASE).unwrap().expect("the first row");
    assert_eq!((first.line, first.column), (42, 7));
    let second = lines.row_for(BASE + 0x20).unwrap().expect("the second row");
    assert_eq!(second.line, 31);
    assert_eq!(lines.row_for(BASE + 0x31).unwrap(), None, "past the end");
}

#[test]
fn a_discriminator_and_an_unknown_extended_opcode_are_skipped() {
    let mut program = set_address(BASE);
    program.extend_from_slice(&[0, 2, 4, 3]); // DW_LNE_set_discriminator 3
    program.extend_from_slice(&[0, 3, 0x80, 1, 2]); // an unknown extended opcode
    program.extend_from_slice(&[0, 0]); // an extended opcode of no length
    program.push(special(0, 4));
    program.extend_from_slice(&[0, 5, 3, b'x', 0, 0, 0]); // DW_LNE_define_file
    program.push(2);
    program.extend_from_slice(&uleb(0x10));
    program.extend_from_slice(&end_sequence());
    let bytes = LineUnit {
        version: 4,
        directory: "/src",
        file: "main.rs",
        program,
        extra_opcodes: 0,
    }
    .bytes();
    let row = lines(&bytes).row_for(BASE).unwrap().expect("a row");
    assert_eq!(row.line, 5);
}

#[test]
fn an_unknown_standard_opcode_is_skipped_by_its_declared_length() {
    let mut program = set_address(BASE);
    program.push(13); // the first opcode beyond the twelve
    program.extend_from_slice(&uleb(99)); // its one operand
    program.push(special_with_base(0, 4, 14));
    program.push(2);
    program.extend_from_slice(&uleb(0x10));
    program.extend_from_slice(&end_sequence());
    let bytes = LineUnit {
        version: 4,
        directory: "/src",
        file: "main.rs",
        program,
        extra_opcodes: 1,
    }
    .bytes();
    let row = lines(&bytes).row_for(BASE).unwrap().expect("a row");
    assert_eq!(row.line, 5);
}

#[test]
fn two_units_are_walked_until_one_answers() {
    let mut bytes = unit(4);
    let mut second = LineUnit {
        version: 5,
        directory: "/other",
        file: "second.rs",
        program: {
            let mut program = set_address(0x9000);
            program.push(special(0, 0)); // line 1
            program.push(2);
            program.extend_from_slice(&uleb(0x10));
            program.extend_from_slice(&end_sequence());
            program
        },
        extra_opcodes: 0,
    }
    .bytes();
    bytes.append(&mut second);
    let program = lines(&bytes);
    assert_eq!(program.row_for(BASE).unwrap().map(|row| row.line), Some(5));
    let far = program.row_for(0x9008).unwrap().expect("the second unit");
    assert_eq!((far.file, far.line), (Some("second.rs"), 1));
}

#[test]
fn a_version_this_crate_does_not_read_is_refused() {
    let bytes = unit(3);
    assert_eq!(
        lines(&bytes).row_for(BASE),
        Err(SymbolError::LineVersion(3))
    );
    let bytes = unit(6);
    assert_eq!(
        lines(&bytes).row_for(BASE),
        Err(SymbolError::LineVersion(6))
    );
}

#[test]
fn a_truncated_program_is_refused_without_a_panic() {
    let whole = unit(4);
    for cut in 1..whole.len() {
        let bytes = whole.get(..cut).expect("a prefix");
        let outcome = lines(bytes).row_for(BASE);
        assert!(
            matches!(outcome, Err(_) | Ok(None)),
            "a prefix of {cut} bytes neither failed nor reported nothing"
        );
    }
}

#[test]
fn a_line_range_of_zero_is_refused() {
    let mut bytes = unit(4);
    // The line range sits after the unit length, the version, the header
    // length, the minimum instruction length, the maximum operations, the
    // default statement flag, and the line base.
    let at = 4 + 2 + 4 + 1 + 1 + 1 + 1;
    if let Some(slot) = bytes.get_mut(at) {
        *slot = 0;
    }
    assert_eq!(lines(&bytes).row_for(BASE), Err(SymbolError::LineRange));
}

#[test]
fn an_empty_section_reports_nothing() {
    let program = LineProgram::new(&[], Strings::default());
    assert!(program.is_empty());
    assert_eq!(program.row_for(BASE).unwrap(), None);
}

#[test]
fn a_file_number_no_table_entry_matches_reports_no_file() {
    let mut program = set_address(BASE);
    program.push(4); // DW_LNS_set_file
    program.extend_from_slice(&uleb(99));
    program.push(special(0, 4));
    program.push(2);
    program.extend_from_slice(&uleb(0x10));
    program.extend_from_slice(&end_sequence());
    for version in [4u16, 5] {
        let bytes = LineUnit {
            version,
            directory: "/src",
            file: "main.rs",
            program: program.clone(),
            extra_opcodes: 0,
        }
        .bytes();
        let row = lines(&bytes).row_for(BASE).unwrap().expect("a row");
        assert_eq!(row.file, None, "version {version}");
        assert_eq!(row.line, 5);
    }
}

use super::build::{RawUnit, header_prelude, one_row, table_v5};

/// A version 5 unit whose two tables use the given formats and entries.
///
/// The file register of the machine starts at one in both versions, so a
/// table with one entry gets it twice: entry zero is the primary source
/// file of the unit and entry one is what the rows name, which is the
/// shape a compiler emits.
fn v5_unit(
    directories: (Vec<(u64, u64)>, Vec<Vec<u8>>),
    files: (Vec<(u64, u64)>, Vec<Vec<u8>>),
    wide: bool,
) -> Vec<u8> {
    let (formats, mut entries) = files;
    if entries.len() == 1 {
        entries.push(entries.first().cloned().unwrap_or_default());
    }
    let mut rest = header_prelude(5);
    rest.extend_from_slice(&table_v5(&directories.0, &directories.1));
    rest.extend_from_slice(&table_v5(&formats, &entries));
    RawUnit {
        version: 5,
        wide,
        rest,
        program: one_row(BASE, 4),
    }
    .bytes()
}

/// A string table with one string at offset one.
const STRINGS: &[u8] = b"\0/from/the/section\0name.rs\0";

#[test]
fn a_version_five_table_reads_its_paths_out_of_the_string_sections() {
    // The directory comes from `.debug_line_str`, the file from
    // `.debug_str`, which is what a real unit mixes.
    let bytes = v5_unit(
        (vec![(1, 0x1F)], vec![1u32.to_le_bytes().to_vec()]),
        (
            vec![(1, 0x0E), (2, 0x0F)],
            vec![{
                let mut entry = 19u32.to_le_bytes().to_vec();
                entry.extend_from_slice(&uleb(0));
                entry
            }],
        ),
        false,
    );
    let program = LineProgram::new(
        &bytes,
        Strings {
            debug_str: STRINGS,
            debug_line_str: STRINGS,
        },
    );
    let row = program.row_for(BASE).unwrap().expect("a row");
    assert_eq!(row.file, Some("name.rs"));
    assert_eq!(row.directory, Some("/from/the/section"));
}

#[test]
fn the_sixty_four_bit_form_of_a_unit_reads_the_same_way() {
    let bytes = v5_unit(
        (vec![(1, 0x08)], vec![b"/wide\0".to_vec()]),
        (
            vec![(1, 0x08), (2, 0x0F)],
            vec![{
                let mut entry = b"wide.rs\0".to_vec();
                entry.extend_from_slice(&uleb(0));
                entry
            }],
        ),
        true,
    );
    let row = lines(&bytes).row_for(BASE).unwrap().expect("a row");
    assert_eq!((row.file, row.directory), (Some("wide.rs"), Some("/wide")));
}

#[test]
fn every_fixed_width_form_of_a_file_entry_is_read_or_skipped() {
    // A file entry that carries its directory index in each of the widths
    // the standard allows, plus the two forms this crate only skips.
    for (form, encoded) in [
        (0x0Bu64, vec![0u8]),
        (0x05, 0u16.to_le_bytes().to_vec()),
        (0x06, 0u32.to_le_bytes().to_vec()),
        (0x07, 0u64.to_le_bytes().to_vec()),
    ] {
        let mut entry = b"file.rs\0".to_vec();
        entry.extend_from_slice(&encoded);
        entry.extend_from_slice(&[0u8; 16]); // DW_FORM_data16
        entry.extend_from_slice(&uleb(2)); // DW_FORM_block of two bytes
        entry.extend_from_slice(&[0xAA, 0xBB]);
        let bytes = v5_unit(
            (vec![(1, 0x08)], vec![b"/dir\0".to_vec()]),
            (
                vec![(1, 0x08), (2, form), (5, 0x1E), (6, 0x09)],
                vec![entry],
            ),
            false,
        );
        let row = lines(&bytes).row_for(BASE).unwrap().expect("a row");
        assert_eq!(row.file, Some("file.rs"), "form {form:#x}");
        assert_eq!(row.directory, Some("/dir"), "form {form:#x}");
    }
}

#[test]
fn a_form_this_crate_does_not_read_is_refused() {
    let bytes = v5_unit(
        (vec![(1, 0x08)], vec![b"/dir\0".to_vec()]),
        (vec![(1, 0x99)], vec![b"file.rs\0".to_vec()]),
        false,
    );
    assert_eq!(
        lines(&bytes).row_for(BASE),
        Err(SymbolError::UnknownForm(0x99))
    );
}

#[test]
fn a_table_with_more_formats_than_the_fixed_capacity_is_refused() {
    let pairs: Vec<(u64, u64)> = (0..9).map(|index| (index, 0x0Bu64)).collect();
    let entry = vec![0u8; 9];
    let bytes = v5_unit(
        (vec![(1, 0x08)], vec![b"/dir\0".to_vec()]),
        (pairs, vec![entry]),
        false,
    );
    assert_eq!(lines(&bytes).row_for(BASE), Err(SymbolError::Overflow));
}

#[test]
fn a_directory_index_no_entry_matches_reports_no_directory() {
    let mut entry = b"file.rs\0".to_vec();
    entry.extend_from_slice(&uleb(7));
    let bytes = v5_unit(
        (vec![(1, 0x08)], vec![b"/dir\0".to_vec()]),
        (vec![(1, 0x08), (2, 0x0F)], vec![entry]),
        false,
    );
    let row = lines(&bytes).row_for(BASE).unwrap().expect("a row");
    assert_eq!(row.file, Some("file.rs"));
    assert_eq!(row.directory, None);
}

#[test]
fn a_version_four_directory_index_beyond_the_table_reports_none() {
    let mut rest = header_prelude(4);
    rest.extend_from_slice(b"/only\0\0"); // one directory, then the end
    rest.extend_from_slice(b"file.rs\0");
    rest.extend_from_slice(&uleb(9)); // a directory that is not there
    rest.extend_from_slice(&uleb(0));
    rest.extend_from_slice(&uleb(0));
    rest.push(0); // end of the files
    let bytes = RawUnit {
        version: 4,
        wide: false,
        rest,
        program: one_row(BASE, 4),
    }
    .bytes();
    let row = lines(&bytes).row_for(BASE).unwrap().expect("a row");
    assert_eq!((row.file, row.directory), (Some("file.rs"), None));
}

#[test]
fn a_version_four_file_in_the_compilation_directory_reports_no_directory() {
    let mut rest = header_prelude(4);
    rest.push(0); // no directories at all
    rest.extend_from_slice(b"file.rs\0");
    rest.extend_from_slice(&uleb(0)); // the directory of the compilation
    rest.extend_from_slice(&uleb(0));
    rest.extend_from_slice(&uleb(0));
    rest.push(0);
    let bytes = RawUnit {
        version: 4,
        wide: false,
        rest,
        program: one_row(BASE, 4),
    }
    .bytes();
    let row = lines(&bytes).row_for(BASE).unwrap().expect("a row");
    assert_eq!((row.file, row.directory), (Some("file.rs"), None));
}

#[test]
fn a_string_offset_beyond_its_section_reports_no_path() {
    let bytes = v5_unit(
        (vec![(1, 0x08)], vec![b"/dir\0".to_vec()]),
        (
            vec![(1, 0x0E), (2, 0x0F)],
            vec![{
                let mut entry = 9999u32.to_le_bytes().to_vec();
                entry.extend_from_slice(&uleb(0));
                entry
            }],
        ),
        false,
    );
    let program = LineProgram::new(
        &bytes,
        Strings {
            debug_str: STRINGS,
            debug_line_str: &[],
        },
    );
    let row = program.row_for(BASE).unwrap().expect("a row");
    assert_eq!(row.file, None);
}
