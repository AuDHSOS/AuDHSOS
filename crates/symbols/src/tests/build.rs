// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Building the bytes the tests read: an ELF file with the sections a
//! symbolizer wants, and line programs of both versions.

/// Number of bytes of the ELF file header.
const EHDR_LEN: usize = 64;

/// Number of bytes of one section header entry.
const SHDR_LEN: usize = 64;

/// The section holds a symbol table.
const SHT_SYMTAB: u32 = 2;

/// The section holds a string table.
const SHT_STRTAB: u32 = 3;

/// The section holds bytes of its own.
const SHT_PROGBITS: u32 = 1;

/// One section of a file under construction.
pub(super) struct Part {
    /// The name the section table gives it.
    pub(super) name: &'static str,
    /// What the section holds.
    pub(super) kind: u32,
    /// The bytes of it.
    pub(super) content: Vec<u8>,
    /// The section this one refers to, by name.
    pub(super) link: &'static str,
    /// Number of bytes of one entry.
    pub(super) entry_size: u64,
}

impl Part {
    /// A section of raw bytes.
    pub(super) fn bits(name: &'static str, content: Vec<u8>) -> Self {
        Part {
            name,
            kind: SHT_PROGBITS,
            content,
            link: "",
            entry_size: 0,
        }
    }
}

/// One entry of a symbol table under construction.
pub(super) struct Symbol {
    /// The name of the function.
    pub(super) name: &'static str,
    /// The address it starts at.
    pub(super) start: u64,
    /// The number of bytes it occupies.
    pub(super) size: u64,
    /// The type bits of the info byte.
    pub(super) kind: u8,
}

impl Symbol {
    /// A function of the given extent.
    pub(super) fn function(name: &'static str, start: u64, size: u64) -> Self {
        Symbol {
            name,
            start,
            size,
            kind: 2,
        }
    }
}

/// The symbol table and its string table, as two sections.
pub(super) fn symbol_table(symbols: &[Symbol]) -> Vec<Part> {
    let mut strings = vec![0u8];
    let mut entries = vec![0u8; 24];
    for symbol in symbols {
        let offset = u32::try_from(strings.len()).unwrap_or(0);
        strings.extend_from_slice(symbol.name.as_bytes());
        strings.push(0);
        entries.extend_from_slice(&offset.to_le_bytes());
        entries.push(symbol.kind);
        entries.push(0);
        entries.extend_from_slice(&1u16.to_le_bytes());
        entries.extend_from_slice(&symbol.start.to_le_bytes());
        entries.extend_from_slice(&symbol.size.to_le_bytes());
    }
    vec![
        Part {
            name: ".symtab",
            kind: SHT_SYMTAB,
            content: entries,
            link: ".strtab",
            entry_size: 24,
        },
        Part {
            name: ".strtab",
            kind: SHT_STRTAB,
            content: strings,
            link: "",
            entry_size: 0,
        },
    ]
}

/// An ELF64 file that holds `parts` and nothing else.
pub(super) fn elf(parts: Vec<Part>) -> Vec<u8> {
    let mut names = vec![0u8];
    let mut name_offsets = Vec::new();
    for part in &parts {
        name_offsets.push(u32::try_from(names.len()).unwrap_or(0));
        names.extend_from_slice(part.name.as_bytes());
        names.push(0);
    }
    let shstrtab = u32::try_from(names.len()).unwrap_or(0);
    names.extend_from_slice(b".shstrtab\0");

    let count = parts.len() + 2;
    let mut file = vec![0u8; EHDR_LEN];
    let mut offsets = Vec::new();
    for part in &parts {
        offsets.push(file.len());
        file.extend_from_slice(&part.content);
    }
    let names_offset = file.len();
    file.extend_from_slice(&names);
    while file.len() % 8 != 0 {
        file.push(0);
    }
    let table = file.len();

    file.extend_from_slice(&[0u8; SHDR_LEN]);
    for (index, part) in parts.iter().enumerate() {
        let link = parts
            .iter()
            .position(|other| other.name == part.link)
            .map_or(0u32, |position| u32::try_from(position + 1).unwrap_or(0));
        file.extend_from_slice(&section(
            name_offsets.get(index).copied().unwrap_or(0),
            part.kind,
            offsets.get(index).copied().unwrap_or(0),
            part.content.len(),
            link,
            part.entry_size,
        ));
    }
    file.extend_from_slice(&section(
        shstrtab,
        SHT_STRTAB,
        names_offset,
        names.len(),
        0,
        0,
    ));

    write_header(&mut file, table, count);
    file
}

/// One section header entry.
fn section(
    name: u32,
    kind: u32,
    offset: usize,
    size: usize,
    link: u32,
    entry_size: u64,
) -> Vec<u8> {
    let mut entry = Vec::with_capacity(SHDR_LEN);
    entry.extend_from_slice(&name.to_le_bytes());
    entry.extend_from_slice(&kind.to_le_bytes());
    entry.extend_from_slice(&0u64.to_le_bytes());
    entry.extend_from_slice(&0u64.to_le_bytes());
    entry.extend_from_slice(&(offset as u64).to_le_bytes());
    entry.extend_from_slice(&(size as u64).to_le_bytes());
    entry.extend_from_slice(&link.to_le_bytes());
    entry.extend_from_slice(&0u32.to_le_bytes());
    entry.extend_from_slice(&1u64.to_le_bytes());
    entry.extend_from_slice(&entry_size.to_le_bytes());
    entry
}

/// Fills in the file header of a file whose sections are in place.
fn write_header(file: &mut [u8], table: usize, count: usize) {
    let header = &mut file[..EHDR_LEN];
    header[..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    header[4] = 2;
    header[5] = 1;
    header[6] = 1;
    header[16..18].copy_from_slice(&2u16.to_le_bytes());
    header[18..20].copy_from_slice(&0x3Eu16.to_le_bytes());
    header[20..24].copy_from_slice(&1u32.to_le_bytes());
    header[0x28..0x30].copy_from_slice(&(table as u64).to_le_bytes());
    header[0x34..0x36].copy_from_slice(&(EHDR_LEN as u16).to_le_bytes());
    header[0x3A..0x3C].copy_from_slice(&(SHDR_LEN as u16).to_le_bytes());
    header[0x3C..0x3E].copy_from_slice(&(count as u16).to_le_bytes());
    header[0x3E..0x40].copy_from_slice(&((count - 1) as u16).to_le_bytes());
}

/// A number in the unsigned variable-length encoding.
pub(super) fn uleb(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let byte = u8::try_from(value & 0x7F).unwrap_or(0);
        value >>= 7;
        if value == 0 {
            bytes.push(byte);
            return bytes;
        }
        bytes.push(byte | 0x80);
    }
}

/// A number in the signed variable-length encoding.
pub(super) fn sleb(mut value: i64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let byte = u8::try_from(value & 0x7F).unwrap_or(0);
        value >>= 7;
        let sign = byte & 0x40 != 0;
        if (value == 0 && !sign) || (value == -1 && sign) {
            bytes.push(byte);
            return bytes;
        }
        bytes.push(byte | 0x80);
    }
}

/// A version 4 line program unit over one file in one directory.
pub(super) struct LineUnit {
    /// The version of the unit.
    pub(super) version: u16,
    /// The directory the file lies in.
    pub(super) directory: &'static str,
    /// The file the rows name.
    pub(super) file: &'static str,
    /// The instructions of the program.
    pub(super) program: Vec<u8>,
    /// Standard opcodes beyond the twelve this crate knows, each taking
    /// one operand, so that the skipping of an unknown one is exercised.
    pub(super) extra_opcodes: u8,
}

impl LineUnit {
    /// The bytes of the unit, with its lengths filled in.
    pub(super) fn bytes(&self) -> Vec<u8> {
        let mut header = Vec::new();
        header.extend_from_slice(&self.version.to_le_bytes());
        if self.version >= 5 {
            header.push(8);
            header.push(0);
        }
        let mut rest = Vec::new();
        rest.push(1); // minimum_instruction_length
        if self.version >= 4 {
            rest.push(1); // maximum_operations_per_instruction
        }
        rest.push(1); // default_is_stmt
        rest.push(0xFB_u8); // line_base of -5
        rest.push(14); // line_range
        rest.push(13u8.saturating_add(self.extra_opcodes)); // opcode_base
        rest.extend_from_slice(&[0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1]);
        for _ in 0..self.extra_opcodes {
            rest.push(1);
        }
        if self.version >= 5 {
            rest.push(1); // one directory format pair
            rest.extend_from_slice(&uleb(1)); // DW_LNCT_path
            rest.extend_from_slice(&uleb(0x08)); // DW_FORM_string
            rest.extend_from_slice(&uleb(1)); // one directory
            rest.extend_from_slice(self.directory.as_bytes());
            rest.push(0);
            rest.push(2); // two file format pairs
            rest.extend_from_slice(&uleb(1)); // DW_LNCT_path
            rest.extend_from_slice(&uleb(0x08)); // DW_FORM_string
            rest.extend_from_slice(&uleb(2)); // DW_LNCT_directory_index
            rest.extend_from_slice(&uleb(0x0F)); // DW_FORM_udata
            rest.extend_from_slice(&uleb(2)); // two files
            rest.extend_from_slice(b"<primary>\0");
            rest.extend_from_slice(&uleb(0));
            rest.extend_from_slice(self.file.as_bytes());
            rest.push(0);
            rest.extend_from_slice(&uleb(0));
        } else {
            rest.extend_from_slice(self.directory.as_bytes());
            rest.push(0);
            rest.push(0); // end of the directories
            rest.extend_from_slice(self.file.as_bytes());
            rest.push(0);
            rest.extend_from_slice(&uleb(1)); // directory one
            rest.extend_from_slice(&uleb(0)); // mtime
            rest.extend_from_slice(&uleb(0)); // length
            rest.push(0); // end of the files
        }
        let header_length = u32::try_from(rest.len()).unwrap_or(0);
        header.extend_from_slice(&header_length.to_le_bytes());
        header.extend_from_slice(&rest);
        header.extend_from_slice(&self.program);
        let unit_length = u32::try_from(header.len()).unwrap_or(0);
        let mut unit = unit_length.to_le_bytes().to_vec();
        unit.extend_from_slice(&header);
        unit
    }
}

/// `DW_LNE_set_address` for `address`.
pub(super) fn set_address(address: u64) -> Vec<u8> {
    let mut bytes = vec![0u8];
    bytes.extend_from_slice(&uleb(9));
    bytes.push(2);
    bytes.extend_from_slice(&address.to_le_bytes());
    bytes
}

/// `DW_LNE_end_sequence`.
pub(super) fn end_sequence() -> Vec<u8> {
    vec![0, 1, 1]
}

/// The special opcode that advances by `address` bytes and `line` lines,
/// for the header this module builds.
pub(super) fn special(address: u64, line: i64) -> u8 {
    special_with_base(address, line, 13)
}

/// The same, for a header with a different opcode base.
pub(super) fn special_with_base(address: u64, line: i64, base: u8) -> u8 {
    let adjusted = (line - (-5)) + 14 * i64::try_from(address).unwrap_or(0);
    u8::try_from(adjusted + i64::from(base)).unwrap_or(255)
}

/// A line program unit assembled by hand: everything from the minimum
/// instruction length to the end of the file table goes into `rest`, and
/// this fills in the two lengths around it.
pub(super) struct RawUnit {
    /// The version of the unit.
    pub(super) version: u16,
    /// `true` for the 64-bit form of the offsets.
    pub(super) wide: bool,
    /// The header from the minimum instruction length onward.
    pub(super) rest: Vec<u8>,
    /// The instructions of the program.
    pub(super) program: Vec<u8>,
}

impl RawUnit {
    /// The bytes of the unit.
    pub(super) fn bytes(&self) -> Vec<u8> {
        let mut body = self.version.to_le_bytes().to_vec();
        if self.version >= 5 {
            body.push(8); // address_size
            body.push(0); // segment_selector_size
        }
        if self.wide {
            body.extend_from_slice(&(self.rest.len() as u64).to_le_bytes());
        } else {
            body.extend_from_slice(&(self.rest.len() as u32).to_le_bytes());
        }
        body.extend_from_slice(&self.rest);
        body.extend_from_slice(&self.program);
        let mut unit = Vec::new();
        if self.wide {
            unit.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
            unit.extend_from_slice(&(body.len() as u64).to_le_bytes());
        } else {
            unit.extend_from_slice(&(body.len() as u32).to_le_bytes());
        }
        unit.extend_from_slice(&body);
        unit
    }
}

/// The fixed part of a header: the six bytes and the twelve lengths that
/// every unit this module builds shares.
pub(super) fn header_prelude(version: u16) -> Vec<u8> {
    let mut rest = vec![1u8]; // minimum_instruction_length
    if version >= 4 {
        rest.push(1); // maximum_operations_per_instruction
    }
    rest.push(1); // default_is_stmt
    rest.push(0xFB); // line_base of -5
    rest.push(14); // line_range
    rest.push(13); // opcode_base
    rest.extend_from_slice(&[0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1]);
    rest
}

/// A version 5 table of one entry, whose format the caller chooses.
pub(super) fn table_v5(pairs: &[(u64, u64)], entries: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![u8::try_from(pairs.len()).unwrap_or(0)];
    for (kind, form) in pairs {
        bytes.extend_from_slice(&uleb(*kind));
        bytes.extend_from_slice(&uleb(*form));
    }
    bytes.extend_from_slice(&uleb(entries.len() as u64));
    for entry in entries {
        bytes.extend_from_slice(entry);
    }
    bytes
}

/// A program of one row at `address` on line `line + 1`, ending above it.
pub(super) fn one_row(address: u64, line: i64) -> Vec<u8> {
    let mut program = set_address(address);
    program.push(special(0, line));
    program.push(2);
    program.extend_from_slice(&uleb(0x10));
    program.extend_from_slice(&end_sequence());
    program
}
