// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The symbol table: which function an address falls in.
//!
//! Invariants: a function this module reports covers the address that was
//! asked for, and the narrowest one does, so that a symbol that encloses
//! another does not hide it.

use audhsos_elf::sections::{SHT_SYMTAB, Sections, string_at};

/// Number of bytes of one symbol table entry.
pub const SYM_LEN: usize = 24;

/// The type bits of the info byte say the symbol is a function.
const STT_FUNC: u8 = 2;

/// The mask that keeps the type bits of the info byte.
const TYPE_MASK: u8 = 0x0F;

/// A function of the symbol table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Function<'a> {
    /// The name as the string table spells it.
    pub name: &'a str,
    /// The address the function starts at.
    pub start: u64,
    /// The number of bytes it occupies.
    pub size: u64,
}

impl Function<'_> {
    /// `true` if `address` lies in the function. A function of size zero
    /// covers its first byte, which is what a hand-written entry point
    /// with no size looks like.
    #[must_use]
    pub const fn contains(&self, address: u64) -> bool {
        if address < self.start {
            return false;
        }
        match address.checked_sub(self.start) {
            Some(offset) => offset < self.size || (self.size == 0 && offset == 0),
            None => false,
        }
    }
}

/// The functions of a file, read out of its symbol table.
#[derive(Clone, Copy, Debug)]
pub struct Functions<'a> {
    entries: &'a [u8],
    names: &'a [u8],
}

impl<'a> Functions<'a> {
    /// The functions of `sections`, or an empty set if the file carries no
    /// symbol table.
    #[must_use]
    pub fn new(sections: &Sections<'a>) -> Self {
        let Some(table) = sections.iter().find(|section| section.kind == SHT_SYMTAB) else {
            return Functions::empty();
        };
        let Some(entries) = sections.content(table) else {
            return Functions::empty();
        };
        let names = usize::try_from(table.link)
            .ok()
            .and_then(|index| sections.get(index))
            .and_then(|strings| sections.content(strings))
            .unwrap_or(&[]);
        Functions { entries, names }
    }

    /// A set with no function in it.
    #[must_use]
    pub const fn empty() -> Self {
        Functions {
            entries: &[],
            names: &[],
        }
    }

    /// The number of entries the table holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len().wrapping_div(SYM_LEN)
    }

    /// `true` if the file carries no symbol table.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The function in slot `index`, if that slot names one.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<Function<'a>> {
        let start = index.checked_mul(SYM_LEN)?;
        let entry = self.entries.get(start..start.checked_add(SYM_LEN)?)?;
        let info = *entry.get(4)?;
        if info & TYPE_MASK != STT_FUNC {
            return None;
        }
        let name = u32::from_le_bytes([
            *entry.first()?,
            *entry.get(1)?,
            *entry.get(2)?,
            *entry.get(3)?,
        ]);
        Some(Function {
            name: string_at(self.names, name)?,
            start: le_u64(entry.get(8..16)?)?,
            size: le_u64(entry.get(16..24)?)?,
        })
    }

    /// Every function of the table, in table order.
    pub fn iter(&self) -> impl Iterator<Item = Function<'a>> + '_ {
        (0..self.len()).filter_map(|index| self.get(index))
    }

    /// The narrowest function that covers `address`.
    #[must_use]
    pub fn at(&self, address: u64) -> Option<Function<'a>> {
        self.iter()
            .filter(|function| function.contains(address))
            .min_by_key(|function| function.size)
    }
}

/// The little-endian `u64` in `slice`.
fn le_u64(slice: &[u8]) -> Option<u64> {
    let mut value = [0u8; 8];
    if slice.len() != value.len() {
        return None;
    }
    for (slot, byte) in value.iter_mut().zip(slice) {
        *slot = *byte;
    }
    Some(u64::from_le_bytes(value))
}
