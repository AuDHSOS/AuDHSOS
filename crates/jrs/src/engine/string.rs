// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::as_conversions,
    reason = "String indices and verified Latin-1 code units use lossless conversions"
)]

//! Dual-encoding, collectable String subsystem.
//!
//! Strings are stored as Latin-1 or UTF-16 flats, bounded-depth ropes, or
//! zero-copy slices. Interned identifier/property atoms are permanent arena
//! roots; other entries are reclaimed by the engine's major collector.

use super::value::{StringRef, Value};
use alloc::{collections::BTreeMap, collections::BTreeSet, string::String, vec::Vec};

/// Maximum depth retained by a lazy Cons/Sliced string graph.
pub const MAX_ROPE_DEPTH: u16 = 32;

/// String allocation or reference failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringError {
    /// A generation-checked string reference is stale or invalid.
    InvalidReference,
    /// The arena exhausted its 24-bit index space.
    ReferenceSpaceExhausted,
    /// The requested slice lies outside the parent string.
    InvalidSlice,
}

/// Internal representation of a heap string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StringKind {
    /// Latin-1 encoded characters (1 byte per code unit).
    Latin1(Vec<u8>),
    /// UTF-16 encoded characters (2 bytes per code unit, preserving surrogates).
    Utf16(Vec<u16>),
    /// Concatenation of two strings, evaluated lazily until flattening.
    Cons {
        /// Left child string.
        left: Value,
        /// Right child string.
        right: Value,
    },
    /// Substring view into a parent string without copying.
    Sliced {
        /// Parent string.
        parent: Value,
        /// Offset in UTF-16 code units.
        offset: usize,
    },
}

/// Heap entry for a string.
#[derive(Clone, Debug)]
pub struct StringRecord {
    /// String storage kind.
    pub kind: StringKind,
    /// Cached length in UTF-16 code units.
    pub len: usize,
    /// Maximum edge depth below this record.
    pub depth: u16,
    /// Whether this string is a permanent interned atom.
    pub is_interned: bool,
}

struct Entry {
    generation: u8,
    record: Option<StringRecord>,
}

/// Statistics for one string-arena mark-sweep collection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StringCollectionStats {
    /// Reachable or interned records retained by marking.
    pub marked: usize,
    /// Unreachable records reclaimed by sweeping.
    pub reclaimed: usize,
}

/// String arena and permanent atom table.
#[derive(Default)]
pub struct StringArena {
    records: Vec<Entry>,
    free: Vec<usize>,
    intern_table: BTreeMap<Vec<u16>, StringRef>,
}

impl StringArena {
    /// Creates a new empty string arena.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            records: Vec::new(),
            free: Vec::new(),
            intern_table: BTreeMap::new(),
        }
    }

    /// Allocates a Latin-1 flat string.
    ///
    /// # Errors
    ///
    /// Returns [`StringError::ReferenceSpaceExhausted`] when no reusable or
    /// representable arena slot remains.
    pub fn allocate_latin1(&mut self, bytes: Vec<u8>) -> Result<StringRef, StringError> {
        let len = bytes.len();
        self.allocate_record(StringRecord {
            kind: StringKind::Latin1(bytes),
            len,
            depth: 0,
            is_interned: false,
        })
    }

    /// Allocates a UTF-16 flat string.
    ///
    /// # Errors
    ///
    /// Returns [`StringError::ReferenceSpaceExhausted`] when no reusable or
    /// representable arena slot remains.
    pub fn allocate_utf16(&mut self, units: Vec<u16>) -> Result<StringRef, StringError> {
        let len = units.len();
        self.allocate_record(StringRecord {
            kind: StringKind::Utf16(units),
            len,
            depth: 0,
            is_interned: false,
        })
    }

    /// Allocates a UTF-8 slice using Latin-1 or UTF-16 storage as appropriate.
    ///
    /// # Errors
    ///
    /// Returns [`StringError::ReferenceSpaceExhausted`] when no arena slot remains.
    pub fn allocate_str(&mut self, string: &str) -> Result<StringRef, StringError> {
        if string.chars().all(|character| u32::from(character) <= 0xFF) {
            let bytes: Vec<u8> = string.chars().map(|character| character as u8).collect();
            self.allocate_latin1(bytes)
        } else {
            self.allocate_utf16(string.encode_utf16().collect())
        }
    }

    /// Interns a permanent identifier/property atom.
    ///
    /// # Errors
    ///
    /// Returns [`StringError::ReferenceSpaceExhausted`] when no arena slot remains.
    pub fn intern(&mut self, string: &str) -> Result<StringRef, StringError> {
        let units: Vec<u16> = string.encode_utf16().collect();
        if let Some(&existing) = self.intern_table.get(&units) {
            return Ok(existing);
        }
        let reference = self.allocate_str(string)?;
        self.record_mut(reference)?.is_interned = true;
        self.intern_table.insert(units, reference);
        Ok(reference)
    }

    /// Allocates a lazy concatenation, flattening inputs when depth is bounded out.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid child references or exhausted arena space.
    pub fn allocate_cons(
        &mut self,
        mut left: Value,
        mut right: Value,
    ) -> Result<StringRef, StringError> {
        let left_len = self.length_of(left).ok_or(StringError::InvalidReference)?;
        let right_len = self.length_of(right).ok_or(StringError::InvalidReference)?;
        let mut depth = self
            .depth_of(left)?
            .max(self.depth_of(right)?)
            .saturating_add(1);
        if depth > MAX_ROPE_DEPTH {
            left = self.flatten_value(left)?;
            right = self.flatten_value(right)?;
            depth = 1;
        }
        self.allocate_record(StringRecord {
            kind: StringKind::Cons { left, right },
            len: left_len.saturating_add(right_len),
            depth,
            is_interned: false,
        })
    }

    /// Allocates a zero-copy substring view.
    ///
    /// Empty/full slices remain valid records so callers retain ordinary heap
    /// string identity while sharing the parent storage.
    ///
    /// # Errors
    ///
    /// Returns [`StringError::InvalidSlice`] for an out-of-bounds range, or a
    /// reference/allocation error from the arena.
    pub fn allocate_slice(
        &mut self,
        mut parent: Value,
        offset: usize,
        len: usize,
    ) -> Result<StringRef, StringError> {
        let parent_len = self
            .length_of(parent)
            .ok_or(StringError::InvalidReference)?;
        if offset > parent_len || len > parent_len.saturating_sub(offset) {
            return Err(StringError::InvalidSlice);
        }
        let mut depth = self.depth_of(parent)?.saturating_add(1);
        if depth > MAX_ROPE_DEPTH {
            parent = self.flatten_value(parent)?;
            depth = 1;
        }
        self.allocate_record(StringRecord {
            kind: StringKind::Sliced { parent, offset },
            len,
            depth,
            is_interned: false,
        })
    }

    /// Returns the length in UTF-16 code units of a string value.
    #[must_use]
    pub fn length_of(&self, value: Value) -> Option<usize> {
        if let Some(reference) = value.as_heap_string() {
            self.record(reference).map(|record| record.len)
        } else if value.is_sso_string() {
            Some(((value.0 >> 40) & 0x0F) as usize)
        } else {
            None
        }
    }

    /// Retrieves the UTF-16 code unit at `index`.
    #[must_use]
    pub fn char_code_at(&self, value: Value, index: usize) -> Option<u16> {
        if value.is_sso_string() {
            let mut buffer = [0u8; 5];
            let len = value.as_sso_string(&mut buffer)?;
            return buffer
                .get(index)
                .copied()
                .filter(|_| index < len)
                .map(u16::from);
        }
        let record = self.record(value.as_heap_string()?)?;
        if index >= record.len {
            return None;
        }
        match &record.kind {
            StringKind::Latin1(bytes) => bytes.get(index).copied().map(u16::from),
            StringKind::Utf16(units) => units.get(index).copied(),
            StringKind::Cons { left, right } => {
                let left_len = self.length_of(*left)?;
                if index < left_len {
                    self.char_code_at(*left, index)
                } else {
                    self.char_code_at(*right, index.saturating_sub(left_len))
                }
            }
            StringKind::Sliced { parent, offset } => {
                self.char_code_at(*parent, offset.saturating_add(index))
            }
        }
    }

    /// Materializes a Rust string, replacing unpaired UTF-16 surrogates only at
    /// this diagnostic boundary.
    #[must_use]
    pub fn to_rust_string(&self, value: Value) -> Option<String> {
        let units = self.to_utf16(value)?;
        Some(String::from_utf16_lossy(&units))
    }

    /// Compares two string values by UTF-16 code units without flattening.
    ///
    /// # Errors
    ///
    /// Returns [`StringError::InvalidReference`] when either value is not a
    /// string or contains a stale arena reference.
    pub fn equals(&self, left: Value, right: Value) -> Result<bool, StringError> {
        let left_len = self.length_of(left).ok_or(StringError::InvalidReference)?;
        let right_len = self.length_of(right).ok_or(StringError::InvalidReference)?;
        if left_len != right_len {
            return Ok(false);
        }
        for index in 0..left_len {
            if self.char_code_at(left, index) != self.char_code_at(right, index) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Materializes a string value as UTF-16 code units.
    #[must_use]
    pub fn to_utf16(&self, value: Value) -> Option<Vec<u16>> {
        let len = self.length_of(value)?;
        let mut units = Vec::with_capacity(len);
        for index in 0..len {
            units.push(self.char_code_at(value, index)?);
        }
        Some(units)
    }

    /// Flattens a Cons/Sliced record in place while preserving its identity.
    ///
    /// # Errors
    ///
    /// Returns [`StringError::InvalidReference`] for a stale record or child.
    pub fn flatten(&mut self, reference: StringRef) -> Result<(), StringError> {
        let value = Value::from_string(reference);
        let kind = &self
            .record(reference)
            .ok_or(StringError::InvalidReference)?
            .kind;
        if matches!(kind, StringKind::Latin1(_) | StringKind::Utf16(_)) {
            return Ok(());
        }
        let units = self.to_utf16(value).ok_or(StringError::InvalidReference)?;
        let record = self.record_mut(reference)?;
        record.depth = 0;
        if units.iter().all(|unit| *unit <= 0xFF) {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "every UTF-16 unit was verified to fit Latin-1"
            )]
            let bytes = units.into_iter().map(|unit| unit as u8).collect();
            record.kind = StringKind::Latin1(bytes);
        } else {
            record.kind = StringKind::Utf16(units);
        }
        Ok(())
    }

    /// Reclaims every non-interned record unreachable from `roots`.
    ///
    /// # Errors
    ///
    /// Returns [`StringError::InvalidReference`] when a root or rope edge is stale.
    pub fn collect(&mut self, roots: &[Value]) -> Result<StringCollectionStats, StringError> {
        let mut marked = BTreeSet::new();
        let mut work = Vec::new();
        for reference in self.intern_table.values() {
            work.push(*reference);
        }
        for value in roots {
            if let Some(reference) = value.as_heap_string() {
                work.push(reference);
            }
        }
        while let Some(reference) = work.pop() {
            if !marked.insert(reference) {
                continue;
            }
            match &self
                .record(reference)
                .ok_or(StringError::InvalidReference)?
                .kind
            {
                StringKind::Cons { left, right } => {
                    if let Some(reference) = left.as_heap_string() {
                        work.push(reference);
                    }
                    if let Some(reference) = right.as_heap_string() {
                        work.push(reference);
                    }
                }
                StringKind::Sliced { parent, .. } => {
                    if let Some(reference) = parent.as_heap_string() {
                        work.push(reference);
                    }
                }
                StringKind::Latin1(_) | StringKind::Utf16(_) => {}
            }
        }

        let mut stats = StringCollectionStats {
            marked: marked.len(),
            reclaimed: 0,
        };
        for (index, entry) in self.records.iter_mut().enumerate() {
            let reference = StringRef::from_parts(string_index(index)?, entry.generation);
            if entry.record.is_some() && !marked.contains(&reference) {
                entry.record = None;
                stats.reclaimed = stats.reclaimed.saturating_add(1);
                if let Some(generation) = entry.generation.checked_add(1) {
                    entry.generation = generation;
                    self.free.push(index);
                }
            }
        }
        Ok(stats)
    }

    fn allocate_record(&mut self, record: StringRecord) -> Result<StringRef, StringError> {
        if let Some(index) = self.free.pop() {
            let entry = self
                .records
                .get_mut(index)
                .ok_or(StringError::InvalidReference)?;
            entry.record = Some(record);
            return Ok(StringRef::from_parts(
                string_index(index)?,
                entry.generation,
            ));
        }
        let index = string_index(self.records.len())?;
        self.records.push(Entry {
            generation: 0,
            record: Some(record),
        });
        Ok(StringRef::from_parts(index, 0))
    }

    fn record(&self, reference: StringRef) -> Option<&StringRecord> {
        let entry = self.records.get(reference.index() as usize)?;
        (entry.generation == reference.generation())
            .then_some(entry.record.as_ref())
            .flatten()
    }

    fn record_mut(&mut self, reference: StringRef) -> Result<&mut StringRecord, StringError> {
        let entry = self
            .records
            .get_mut(reference.index() as usize)
            .ok_or(StringError::InvalidReference)?;
        if entry.generation != reference.generation() {
            return Err(StringError::InvalidReference);
        }
        entry.record.as_mut().ok_or(StringError::InvalidReference)
    }

    fn depth_of(&self, value: Value) -> Result<u16, StringError> {
        if let Some(reference) = value.as_heap_string() {
            self.record(reference)
                .map(|record| record.depth)
                .ok_or(StringError::InvalidReference)
        } else if value.is_sso_string() {
            Ok(0)
        } else {
            Err(StringError::InvalidReference)
        }
    }

    fn flatten_value(&mut self, value: Value) -> Result<Value, StringError> {
        if let Some(reference) = value.as_heap_string() {
            self.flatten(reference)?;
        }
        Ok(value)
    }
}

fn string_index(index: usize) -> Result<u32, StringError> {
    u32::try_from(index)
        .ok()
        .filter(|index| *index < (1 << 24))
        .ok_or(StringError::ReferenceSpaceExhausted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::value::VALUE_NULL;

    #[test]
    fn dual_encoding_cons_slices_and_flattening_preserve_code_units() {
        let mut arena = StringArena::new();
        let hello = arena.allocate_str("hello").unwrap();
        let world = arena.allocate_str(" 世界").unwrap();
        let cons = arena
            .allocate_cons(Value::from_string(hello), Value::from_string(world))
            .unwrap();
        let cons_value = Value::from_string(cons);

        assert_eq!(arena.length_of(cons_value), Some(8));
        assert_eq!(arena.char_code_at(cons_value, 0), Some(u16::from(b'h')));
        assert_eq!(
            arena.to_rust_string(cons_value).as_deref(),
            Some("hello 世界")
        );

        let slice = arena.allocate_slice(cons_value, 6, 2).unwrap();
        assert_eq!(
            arena.to_rust_string(Value::from_string(slice)).as_deref(),
            Some("世界")
        );
        arena.flatten(cons).unwrap();
        assert!(matches!(
            arena.record(cons).unwrap().kind,
            StringKind::Utf16(_)
        ));
        assert_eq!(
            arena.to_rust_string(cons_value).as_deref(),
            Some("hello 世界")
        );
    }

    #[test]
    fn rope_depth_is_bounded_by_flattening_children() {
        let mut arena = StringArena::new();
        let unit = Value::from_string(arena.allocate_str("x").unwrap());
        let mut rope = unit;
        for _ in 0..=MAX_ROPE_DEPTH {
            rope = Value::from_string(arena.allocate_cons(rope, unit).unwrap());
        }
        let record = arena.record(rope.as_heap_string().unwrap()).unwrap();
        assert!(record.depth <= MAX_ROPE_DEPTH);
        assert_eq!(arena.length_of(rope), Some(usize::from(MAX_ROPE_DEPTH) + 2));
    }

    #[test]
    fn collection_traces_ropes_keeps_atoms_and_rejects_stale_references() {
        let mut arena = StringArena::new();
        let atom = arena.intern("name").unwrap();
        let left = arena.allocate_str("left").unwrap();
        let right = arena.allocate_str("right").unwrap();
        let garbage = arena.allocate_str("garbage").unwrap();
        let cons = arena
            .allocate_cons(Value::from_string(left), Value::from_string(right))
            .unwrap();

        let stats = arena.collect(&[Value::from_string(cons)]).unwrap();
        assert_eq!(stats.marked, 4);
        assert_eq!(stats.reclaimed, 1);
        assert_eq!(
            arena.to_rust_string(Value::from_string(cons)).as_deref(),
            Some("leftright")
        );
        assert_eq!(
            arena.to_rust_string(Value::from_string(atom)).as_deref(),
            Some("name")
        );
        assert_eq!(arena.to_rust_string(Value::from_string(garbage)), None);

        let replacement = arena.allocate_str("replacement").unwrap();
        assert_eq!(replacement.index(), garbage.index());
        assert_ne!(replacement.generation(), garbage.generation());
        assert_eq!(arena.to_rust_string(Value::from_string(garbage)), None);
    }

    #[test]
    fn invalid_string_operations_fail_without_aliasing() {
        let mut arena = StringArena::new();
        let string = arena.allocate_str("abc").unwrap();
        let value = Value::from_string(string);
        assert_eq!(
            arena.allocate_slice(value, 2, 2),
            Err(StringError::InvalidSlice)
        );
        assert_eq!(arena.length_of(VALUE_NULL), None);
        assert_eq!(arena.char_code_at(value, 3), None);
        assert_eq!(arena.to_rust_string(VALUE_NULL), None);
        assert_eq!(arena.intern("abc").unwrap(), arena.intern("abc").unwrap());
    }
}
