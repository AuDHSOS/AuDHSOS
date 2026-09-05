// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A builder that assembles ELF images byte by byte and generators built
//! on it. Available behind the feature `test-strategies` and in this
//! crate's own tests, so that the tests and the fuzz corpus share one
//! description of a well-formed file.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use test_support::generators::{BoxGen, Generator, bool, pair, range, vec as gen_vec};

use crate::image::{EHDR_LEN, PHDR_LEN};

/// The virtual address the default image is linked at.
pub const DEFAULT_VADDR: u64 = 0xFFFF_FFFF_8000_1000;

/// One entry of the program header table, as the builder writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgramHeader {
    /// `p_type`; `1` is `PT_LOAD`.
    pub kind: u32,
    /// `p_flags`; bit 0 execute, bit 1 write, bit 2 read.
    pub flags: u32,
    /// `p_offset`.
    pub offset: u64,
    /// `p_vaddr`.
    pub vaddr: u64,
    /// `p_filesz`.
    pub file_size: u64,
    /// `p_memsz`.
    pub mem_size: u64,
    /// `p_align`.
    pub align: u64,
}

impl ProgramHeader {
    /// A readable and executable segment of one page.
    #[must_use]
    pub const fn code(offset: u64, vaddr: u64, len: u64) -> Self {
        ProgramHeader {
            kind: 1,
            flags: 5,
            offset,
            vaddr,
            file_size: len,
            mem_size: len,
            align: 0x1000,
        }
    }

    /// A readable and writable segment.
    #[must_use]
    pub const fn data(offset: u64, vaddr: u64, file_size: u64, mem_size: u64) -> Self {
        ProgramHeader {
            kind: 1,
            flags: 6,
            offset,
            vaddr,
            file_size,
            mem_size,
            align: 0x1000,
        }
    }

    fn to_bytes(self) -> [u8; PHDR_LEN] {
        let mut bytes = [0u8; PHDR_LEN];
        let words: [u32; PHDR_LEN / 4] = [
            self.kind,
            self.flags,
            low(self.offset),
            high(self.offset),
            low(self.vaddr),
            high(self.vaddr),
            low(self.vaddr),
            high(self.vaddr),
            low(self.file_size),
            high(self.file_size),
            low(self.mem_size),
            high(self.mem_size),
            low(self.align),
            high(self.align),
        ];
        let (chunks, _rest) = bytes.as_chunks_mut::<4>();
        for (chunk, word) in chunks.iter_mut().zip(words) {
            *chunk = word.to_le_bytes();
        }
        bytes
    }
}

/// Every field a test may want to bend, so that the parser sees values a
/// linker never writes.
#[derive(Clone, Debug)]
pub struct ElfBuilder {
    /// The first four bytes.
    pub magic: [u8; 4],
    /// `EI_CLASS`; `2` is 64-bit.
    pub class: u8,
    /// `EI_DATA`; `1` is little-endian.
    pub data: u8,
    /// `e_type`; `2` is `ET_EXEC`.
    pub kind: u16,
    /// `e_machine`; `0x3E` is `x86-64`.
    pub machine: u16,
    /// `e_entry`.
    pub entry: u64,
    /// `e_ehsize`.
    pub header_size: u16,
    /// `e_phentsize`.
    pub entry_size: u16,
    /// `e_phoff`; `None` puts the table right behind the header.
    pub table_offset: Option<u64>,
    /// `e_phnum`; `None` uses the number of program headers.
    pub table_count: Option<u16>,
    /// The program header table.
    pub headers: Vec<ProgramHeader>,
    /// Length of the file; `None` computes the smallest length that holds
    /// the table and every segment.
    pub file_len: Option<usize>,
}

impl Default for ElfBuilder {
    fn default() -> Self {
        ElfBuilder {
            magic: [0x7F, b'E', b'L', b'F'],
            class: 2,
            data: 1,
            kind: 2,
            machine: 0x3E,
            entry: DEFAULT_VADDR,
            header_size: 64,
            entry_size: 56,
            table_offset: None,
            table_count: None,
            headers: vec![ProgramHeader::code(0x1000, DEFAULT_VADDR, 0x1000)],
            file_len: None,
        }
    }
}

impl ElfBuilder {
    /// A well-formed image with one executable segment.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The same image with `headers` instead of the default segment.
    #[must_use]
    pub fn with_headers(mut self, headers: Vec<ProgramHeader>) -> Self {
        self.headers = headers;
        self
    }

    /// The offset the program header table is written at.
    #[must_use]
    pub fn offset_of_table(&self) -> u64 {
        self.table_offset
            .unwrap_or_else(|| u64::try_from(EHDR_LEN).unwrap_or(0))
    }

    /// The bytes of the image.
    #[must_use]
    pub fn build(&self) -> Vec<u8> {
        let table_offset = self.offset_of_table();
        let count = self
            .table_count
            .unwrap_or_else(|| u16::try_from(self.headers.len()).unwrap_or(u16::MAX));
        let table_end = table_offset
            .saturating_add(u64::from(count).saturating_mul(u64::from(self.entry_size)));
        let content_end = self
            .headers
            .iter()
            .map(|header| header.offset.saturating_add(header.file_size))
            .max()
            .unwrap_or(0);
        let len = self.file_len.unwrap_or_else(|| {
            usize::try_from(
                table_end
                    .max(content_end)
                    .max(u64::try_from(EHDR_LEN).unwrap_or(0)),
            )
            .unwrap_or(usize::MAX)
        });
        let mut bytes = vec![0u8; len];
        write_at(&mut bytes, 0, &self.magic);
        write_at(&mut bytes, 4, &[self.class, self.data, 1, 0]);
        write_at(&mut bytes, 16, &self.kind.to_le_bytes());
        write_at(&mut bytes, 18, &self.machine.to_le_bytes());
        write_at(&mut bytes, 20, &1u32.to_le_bytes());
        write_at(&mut bytes, 24, &self.entry.to_le_bytes());
        write_at(&mut bytes, 32, &table_offset.to_le_bytes());
        write_at(&mut bytes, 52, &self.header_size.to_le_bytes());
        write_at(&mut bytes, 54, &self.entry_size.to_le_bytes());
        write_at(&mut bytes, 56, &count.to_le_bytes());
        for (index, header) in self.headers.iter().enumerate() {
            let offset = usize::try_from(table_offset)
                .unwrap_or(usize::MAX)
                .saturating_add(index.saturating_mul(usize::from(self.entry_size)));
            write_at(&mut bytes, offset, &header.to_bytes());
        }
        bytes
    }
}

fn write_at(bytes: &mut [u8], offset: usize, value: &[u8]) {
    if let Some(slot) = offset
        .checked_add(value.len())
        .and_then(|end| bytes.get_mut(offset..end))
    {
        slot.copy_from_slice(value);
    }
}

const fn low(value: u64) -> u32 {
    #[expect(clippy::as_conversions, reason = "narrowing a value masked to 32 bits")]
    {
        (value & 0xFFFF_FFFF) as u32
    }
}

const fn high(value: u64) -> u32 {
    #[expect(
        clippy::as_conversions,
        reason = "narrowing the upper 32 bits of a u64"
    )]
    {
        (value >> 32) as u32
    }
}

/// A well-formed image, sometimes with a second, writable segment.
#[must_use]
pub fn any_elf_image() -> BoxGen<Vec<u8>> {
    pair(bool(), range(0u64..=4))
        .map(|(with_data, pages)| {
            let mut builder = ElfBuilder::new();
            if with_data {
                let len = pages.saturating_mul(0x1000);
                builder.headers.push(ProgramHeader::data(
                    0x2000,
                    DEFAULT_VADDR.saturating_add(0x2000),
                    len,
                    len.saturating_add(0x1000),
                ));
            }
            builder.build()
        })
        .boxed()
}

/// A well-formed image with up to eight bytes replaced and sometimes
/// truncated, so that most cases are near-valid.
#[must_use]
pub fn any_elf_bytes() -> BoxGen<Vec<u8>> {
    let mutations = gen_vec(pair(range(0usize..=255), range(0u8..=u8::MAX)), 0..=8);
    pair(any_elf_image(), pair(mutations, range(0usize..=2)))
        .map(|(mut bytes, (mutations, truncate))| {
            for (index, value) in mutations {
                if let Some(slot) = bytes.get_mut(index) {
                    *slot = value;
                }
            }
            match truncate {
                1 => bytes.truncate(EHDR_LEN.saturating_sub(1)),
                2 => bytes.truncate(EHDR_LEN.saturating_add(PHDR_LEN / 2)),
                _ => {}
            }
            bytes
        })
        .boxed()
}
