// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "String arena indices and code unit manipulation"
)]

//! Dual-encoding String subsystem.
//!
//! Strings are stored either in Latin-1 (1 byte per character for ASCII/ISO-8859-1)
//! or UTF-16 (2 bytes per character). Concatenation creates O(1) `Cons` strings,
//! and substrings create zero-copy `Sliced` strings. Interned strings enable O(1)
//! pointer-like comparisons for property names.

use super::value::{StringRef, Value};
use alloc::{collections::BTreeMap, string::String, vec::Vec};

/// Internal representation of a heap string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StringKind {
    /// Latin-1 encoded characters (1 byte per code unit).
    Latin1(Vec<u8>),
    /// UTF-16 encoded characters (2 bytes per code unit, preserving surrogates).
    Utf16(Vec<u16>),
    /// Concatenation of two strings (evaluated lazily upon index access or flattening).
    Cons {
        /// Left child string.
        left: Value,
        /// Right child string.
        right: Value,
        /// Total length in UTF-16 code units.
        len: usize,
    },
    /// Substring view into a parent string without copying.
    Sliced {
        /// Parent string.
        parent: Value,
        /// Offset in code units.
        offset: usize,
        /// Substring length in code units.
        len: usize,
    },
}

/// Heap entry for a string.
#[derive(Clone, Debug)]
pub struct StringRecord {
    /// String storage kind.
    pub kind: StringKind,
    /// Cached total length in code units.
    pub len: usize,
    /// Whether this string is an interned identifier.
    pub is_interned: bool,
}

/// String arena and interning table.
#[derive(Default)]
pub struct StringArena {
    records: Vec<StringRecord>,
    intern_table: BTreeMap<Vec<u16>, StringRef>,
}

impl StringArena {
    /// Creates a new empty string arena.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            records: Vec::new(),
            intern_table: BTreeMap::new(),
        }
    }

    /// Allocates a Latin-1 flat string.
    pub fn allocate_latin1(&mut self, bytes: Vec<u8>) -> StringRef {
        let len = bytes.len();
        let id = self.records.len();
        self.records.push(StringRecord {
            kind: StringKind::Latin1(bytes),
            len,
            is_interned: false,
        });
        #[expect(
            clippy::as_conversions,
            reason = "record count bounded by arena limits"
        )]
        StringRef(id as u32)
    }

    /// Allocates a UTF-16 flat string.
    pub fn allocate_utf16(&mut self, units: Vec<u16>) -> StringRef {
        let len = units.len();
        let id = self.records.len();
        self.records.push(StringRecord {
            kind: StringKind::Utf16(units),
            len,
            is_interned: false,
        });
        #[expect(
            clippy::as_conversions,
            reason = "record count bounded by arena limits"
        )]
        StringRef(id as u32)
    }

    /// Allocates from a UTF-8 string slice, choosing Latin-1 or UTF-16 automatically.
    pub fn allocate_str(&mut self, s: &str) -> StringRef {
        let is_latin1 = s.chars().all(|c| u32::from(c) <= 0xFF);
        if is_latin1 {
            #[expect(clippy::as_conversions, reason = "all code points are <= 0xFF")]
            let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
            self.allocate_latin1(bytes)
        } else {
            let units: Vec<u16> = s.encode_utf16().collect();
            self.allocate_utf16(units)
        }
    }

    /// Interns a string for O(1) identifier and property lookup.
    pub fn intern(&mut self, s: &str) -> StringRef {
        let units: Vec<u16> = s.encode_utf16().collect();
        if let Some(&existing) = self.intern_table.get(&units) {
            return existing;
        }
        let sref = self.allocate_str(s);
        if let Some(record) = self.records.get_mut(sref.0 as usize) {
            record.is_interned = true;
        }
        self.intern_table.insert(units, sref);
        sref
    }

    /// Allocates an O(1) concatenation of two strings (`left + right`).
    pub fn allocate_cons(&mut self, left: Value, right: Value) -> StringRef {
        let left_len = self.length_of(left);
        let right_len = self.length_of(right);
        let total_len = left_len.saturating_add(right_len);
        let id = self.records.len();
        self.records.push(StringRecord {
            kind: StringKind::Cons {
                left,
                right,
                len: total_len,
            },
            len: total_len,
            is_interned: false,
        });
        #[expect(
            clippy::as_conversions,
            reason = "record count bounded by arena limits"
        )]
        StringRef(id as u32)
    }

    /// Returns the length in code units of a string value.
    #[must_use]
    pub fn length_of(&self, value: Value) -> usize {
        if let Some(sref) = value.as_heap_string() {
            self.records.get(sref.0 as usize).map_or(0, |r| r.len)
        } else if value.is_sso_string() {
            #[expect(clippy::as_conversions, reason = "sso length fits in usize")]
            let len = ((value.0 >> 40) & 0x0F) as usize;
            len
        } else {
            0
        }
    }

    /// Retrieves the UTF-16 code unit at index `idx`.
    pub fn char_code_at(&self, value: Value, idx: usize) -> Option<u16> {
        if value.is_sso_string() {
            let mut buf = [0u8; 5];
            let len = value.as_sso_string(&mut buf)?;
            if idx < len {
                return buf.get(idx).copied().map(u16::from);
            }
            return None;
        }
        let sref = value.as_heap_string()?;
        let record = self.records.get(sref.0 as usize)?;
        if idx >= record.len {
            return None;
        }
        match &record.kind {
            StringKind::Latin1(bytes) => bytes.get(idx).copied().map(u16::from),
            StringKind::Utf16(units) => units.get(idx).copied(),
            StringKind::Cons { left, right, .. } => {
                let left_len = self.length_of(*left);
                if idx < left_len {
                    self.char_code_at(*left, idx)
                } else {
                    let right_idx = idx.saturating_sub(left_len);
                    self.char_code_at(*right, right_idx)
                }
            }
            StringKind::Sliced {
                parent,
                offset,
                len,
            } => {
                if idx < *len {
                    let parent_idx = offset.saturating_add(idx);
                    self.char_code_at(*parent, parent_idx)
                } else {
                    None
                }
            }
        }
    }

    /// Materializes a flat UTF-8 string for inspection or printing.
    #[must_use]
    pub fn to_rust_string(&self, value: Value) -> String {
        if value.is_sso_string() {
            let mut buf = [0u8; 5];
            let len = value.as_sso_string(&mut buf).unwrap_or(0);
            #[expect(clippy::indexing_slicing, reason = "slice within bounded len")]
            return String::from_utf8_lossy(&buf[..len]).into_owned();
        }
        let Some(sref) = value.as_heap_string() else {
            return String::new();
        };
        let Some(record) = self.records.get(sref.0 as usize) else {
            return String::new();
        };
        match &record.kind {
            StringKind::Latin1(bytes) => bytes.iter().map(|&b| char::from(b)).collect(),
            StringKind::Utf16(units) => String::from_utf16_lossy(units),
            StringKind::Cons { left, right, .. } => {
                let mut s = self.to_rust_string(*left);
                s.push_str(&self.to_rust_string(*right));
                s
            }
            StringKind::Sliced {
                parent,
                offset,
                len,
            } => {
                let parent_str = self.to_rust_string(*parent);
                parent_str.chars().skip(*offset).take(*len).collect()
            }
        }
    }

    /// Flattens a `Cons` string into a flat Latin-1 or UTF-16 string in place.
    pub fn flatten(&mut self, sref: StringRef) {
        let Some(record) = self.records.get(sref.0 as usize) else {
            return;
        };
        if !matches!(record.kind, StringKind::Cons { .. }) {
            return;
        }
        let len = record.len;
        let mut units = Vec::with_capacity(len);
        let val = Value::from_string(sref);
        for i in 0..len {
            if let Some(ch) = self.char_code_at(val, i) {
                units.push(ch);
            }
        }
        let is_latin1 = units.iter().all(|&u| u <= 0xFF);
        if let Some(rec) = self.records.get_mut(sref.0 as usize) {
            if is_latin1 {
                #[expect(clippy::as_conversions, reason = "every unit is verified <= 0xFF")]
                let bytes: Vec<u8> = units.into_iter().map(|u| u as u8).collect();
                rec.kind = StringKind::Latin1(bytes);
            } else {
                rec.kind = StringKind::Utf16(units);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dual_encoding_and_cons_concatenation() {
        let mut arena = StringArena::new();
        let hello = arena.allocate_str("hello");
        let world = arena.allocate_str(" world!");

        let hello_val = Value::from_string(hello);
        let world_val = Value::from_string(world);

        assert_eq!(arena.length_of(hello_val), 5);
        assert_eq!(arena.length_of(world_val), 7);

        let cons = arena.allocate_cons(hello_val, world_val);
        let cons_val = Value::from_string(cons);
        assert_eq!(arena.length_of(cons_val), 12);

        assert_eq!(arena.char_code_at(cons_val, 0), Some(u16::from(b'h')));
        assert_eq!(arena.char_code_at(cons_val, 5), Some(u16::from(b' ')));
        assert_eq!(arena.char_code_at(cons_val, 11), Some(u16::from(b'!')));
        assert_eq!(arena.char_code_at(cons_val, 12), None);

        assert_eq!(arena.to_rust_string(cons_val), "hello world!");

        arena.flatten(cons);
        assert_eq!(arena.to_rust_string(cons_val), "hello world!");
    }

    #[test]
    fn string_interning() {
        let mut arena = StringArena::new();
        let a = arena.intern("foo");
        let b = arena.intern("foo");
        let c = arena.intern("bar");

        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
