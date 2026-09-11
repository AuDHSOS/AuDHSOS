// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "Elements store indices and lattice transitions"
)]

//! Elements backing store for array indices and indexed properties.
//!
//! Elements live in a dedicated, type-specialized backing store rather than
//! string-keyed property maps. Storage follows a monotone specialization lattice:
//! `PackedSmi` -> `PackedDouble` -> `PackedValues` -> Holey -> Dictionary.

use super::value::Value;
use alloc::{collections::BTreeMap, vec::Vec};

/// Reference to an elements backing store in the arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElementsRef(u32);

const OLD_GENERATION_BIT: u32 = 1 << 31;
const YOUNG_GENERATION_SHIFT: u32 = 11;
const YOUNG_INDEX_MASK: u32 = (1 << YOUNG_GENERATION_SHIFT) - 1;
const OLD_GENERATION_SHIFT: u32 = 23;
const OLD_INDEX_MASK: u32 = (1 << OLD_GENERATION_SHIFT) - 1;

impl ElementsRef {
    /// Creates a reference into the active Nursery semispace.
    #[must_use]
    pub(crate) const fn young(index: u32, generation: u32) -> Self {
        Self((index & YOUNG_INDEX_MASK) | (generation << YOUNG_GENERATION_SHIFT))
    }

    /// Creates a reference into the Old Generation.
    #[must_use]
    pub(crate) const fn old(index: u32, generation: u8) -> Self {
        Self(
            OLD_GENERATION_BIT
                | (index & OLD_INDEX_MASK)
                | ((generation as u32) << OLD_GENERATION_SHIFT),
        )
    }

    /// Returns `true` when this reference addresses the Old Generation.
    #[must_use]
    pub const fn is_old(self) -> bool {
        self.0 & OLD_GENERATION_BIT != 0
    }

    /// Returns `true` when this reference addresses the Nursery.
    #[must_use]
    pub const fn is_young(self) -> bool {
        !self.is_old()
    }

    /// Returns the generation-local backing-store index.
    #[must_use]
    pub const fn index(self) -> u32 {
        if self.is_old() {
            self.0 & OLD_INDEX_MASK
        } else {
            self.0 & YOUNG_INDEX_MASK
        }
    }

    /// Returns the generation used to reject stale reused references.
    #[must_use]
    pub const fn generation(self) -> u32 {
        if self.is_old() {
            (self.0 & !OLD_GENERATION_BIT) >> OLD_GENERATION_SHIFT
        } else {
            self.0 >> YOUNG_GENERATION_SHIFT
        }
    }
}

/// Type specialization lattice for array elements.
#[derive(Clone, Debug, PartialEq)]
pub enum ElementsKind {
    /// Compact array of 32-bit signed integers.
    PackedSmi(Vec<i32>),
    /// Compact array of binary64 floating-point numbers.
    PackedDouble(Vec<f64>),
    /// Generic packed array of 64-bit NaN-boxed values.
    PackedValues(Vec<Value>),
    /// Generic array with possible holes (missing indices).
    Holey(Vec<Option<Value>>),
    /// Sparse dictionary for huge or irregular indices.
    Dictionary(BTreeMap<u32, Value>),
}

impl ElementsKind {
    /// Creates an empty packed Smi elements store.
    #[must_use]
    pub fn new_packed_smi(capacity: usize) -> Self {
        Self::PackedSmi(Vec::with_capacity(capacity))
    }

    /// Returns the length (number of elements or highest mapped index).
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::PackedSmi(v) => v.len(),
            Self::PackedDouble(v) => v.len(),
            Self::PackedValues(v) => v.len(),
            Self::Holey(v) => v.len(),
            Self::Dictionary(m) => m
                .keys()
                .next_back()
                .map_or(0, |&k| (k as usize).saturating_add(1)),
        }
    }

    /// Returns `true` if the elements store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads the element at index `idx`.
    #[must_use]
    pub fn get(&self, idx: u32) -> Option<Value> {
        let index = idx as usize;
        match self {
            Self::PackedSmi(v) => v.get(index).copied().map(Value::from_smi),
            Self::PackedDouble(v) => v.get(index).copied().map(Value::from_f64),
            Self::PackedValues(v) => v.get(index).copied(),
            Self::Holey(v) => v.get(index).copied().flatten(),
            Self::Dictionary(m) => m.get(&idx).copied(),
        }
    }

    /// Pushes a new element, transitioning the lattice if necessary.
    pub fn push(&mut self, val: Value) {
        let next_idx = self.len();
        #[expect(clippy::as_conversions, reason = "array length fits u32 in JS range")]
        self.set(next_idx as u32, val);
    }

    /// Writes `val` at `idx`, transitioning to more general lattice types as required.
    #[expect(
        clippy::too_many_lines,
        clippy::comparison_chain,
        reason = "lattice transition state machine"
    )]
    pub fn set(&mut self, idx: u32, val: Value) {
        let index = idx as usize;
        // Transition check
        match self {
            Self::PackedSmi(vec) => {
                if let Some(smi) = val.as_smi() {
                    if index == vec.len() {
                        vec.push(smi);
                        return;
                    } else if index < vec.len() {
                        if let Some(slot) = vec.get_mut(index) {
                            *slot = smi;
                        }
                        return;
                    }
                }
                // Cannot stay in PackedSmi; promote to PackedDouble or PackedValues
                if let Some(double) = val.as_f64() {
                    let mut new_doubles: Vec<f64> = vec.iter().map(|&i| f64::from(i)).collect();
                    if index == new_doubles.len() {
                        new_doubles.push(double);
                    } else if index < new_doubles.len() {
                        if let Some(slot) = new_doubles.get_mut(index) {
                            *slot = double;
                        }
                    } else {
                        // Hole creation -> Holey
                        let mut holey: Vec<Option<Value>> = new_doubles
                            .into_iter()
                            .map(|d| Some(Value::from_f64(d)))
                            .collect();
                        holey.resize(index, None);
                        holey.push(Some(val));
                        *self = Self::Holey(holey);
                        return;
                    }
                    *self = Self::PackedDouble(new_doubles);
                } else {
                    // General value
                    let mut new_vals: Vec<Value> =
                        vec.iter().map(|&i| Value::from_smi(i)).collect();
                    if index == new_vals.len() {
                        new_vals.push(val);
                    } else if index < new_vals.len() {
                        if let Some(slot) = new_vals.get_mut(index) {
                            *slot = val;
                        }
                    } else {
                        let mut holey: Vec<Option<Value>> =
                            new_vals.into_iter().map(Some).collect();
                        holey.resize(index, None);
                        holey.push(Some(val));
                        *self = Self::Holey(holey);
                        return;
                    }
                    *self = Self::PackedValues(new_vals);
                }
            }
            Self::PackedDouble(vec) => {
                if let Some(double) = val.as_f64() {
                    if index == vec.len() {
                        vec.push(double);
                        return;
                    } else if index < vec.len() {
                        if let Some(slot) = vec.get_mut(index) {
                            *slot = double;
                        }
                        return;
                    }
                }
                // Promote to PackedValues or Holey
                let mut new_vals: Vec<Value> = vec.iter().map(|&d| Value::from_f64(d)).collect();
                if index == new_vals.len() {
                    new_vals.push(val);
                } else if index < new_vals.len() {
                    if let Some(slot) = new_vals.get_mut(index) {
                        *slot = val;
                    }
                } else {
                    let mut holey: Vec<Option<Value>> = new_vals.into_iter().map(Some).collect();
                    holey.resize(index, None);
                    holey.push(Some(val));
                    *self = Self::Holey(holey);
                    return;
                }
                *self = Self::PackedValues(new_vals);
            }
            Self::PackedValues(vec) => {
                if index == vec.len() {
                    vec.push(val);
                } else if index < vec.len() {
                    if let Some(slot) = vec.get_mut(index) {
                        *slot = val;
                    }
                } else {
                    // Sparse write -> Holey or Dictionary
                    if index.saturating_sub(vec.len()) > 1024 {
                        let mut dict = BTreeMap::new();
                        for (i, &v) in vec.iter().enumerate() {
                            #[expect(clippy::as_conversions, reason = "index fits u32")]
                            dict.insert(i as u32, v);
                        }
                        dict.insert(idx, val);
                        *self = Self::Dictionary(dict);
                    } else {
                        let mut holey: Vec<Option<Value>> = vec.iter().copied().map(Some).collect();
                        holey.resize(index, None);
                        holey.push(Some(val));
                        *self = Self::Holey(holey);
                    }
                }
            }
            Self::Holey(vec) => {
                if index == vec.len() {
                    vec.push(Some(val));
                } else if index < vec.len() {
                    if let Some(slot) = vec.get_mut(index) {
                        *slot = Some(val);
                    }
                } else {
                    vec.resize(index, None);
                    vec.push(Some(val));
                }
            }
            Self::Dictionary(dict) => {
                dict.insert(idx, val);
            }
        }
    }

    /// Deletes the element at `idx` (turning packed arrays into Holey).
    pub fn delete(&mut self, idx: u32) -> bool {
        let index = idx as usize;
        if index >= self.len() {
            return true;
        }
        match self {
            Self::PackedSmi(vec) => {
                let mut holey: Vec<Option<Value>> =
                    vec.iter().map(|&i| Some(Value::from_smi(i))).collect();
                if let Some(slot) = holey.get_mut(index) {
                    *slot = None;
                }
                *self = Self::Holey(holey);
            }
            Self::PackedDouble(vec) => {
                let mut holey: Vec<Option<Value>> =
                    vec.iter().map(|&d| Some(Value::from_f64(d))).collect();
                if let Some(slot) = holey.get_mut(index) {
                    *slot = None;
                }
                *self = Self::Holey(holey);
            }
            Self::PackedValues(vec) => {
                let mut holey: Vec<Option<Value>> = vec.iter().copied().map(Some).collect();
                if let Some(slot) = holey.get_mut(index) {
                    *slot = None;
                }
                *self = Self::Holey(holey);
            }
            Self::Holey(vec) => {
                if let Some(slot) = vec.get_mut(index) {
                    *slot = None;
                }
            }
            Self::Dictionary(dict) => {
                dict.remove(&idx);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::value::VALUE_UNDEFINED;
    use super::*;

    #[test]
    fn packed_smi_fast_indexing_and_promotion() {
        let mut elements = ElementsKind::new_packed_smi(4);
        elements.push(Value::from_smi(10));
        elements.push(Value::from_smi(20));
        assert_eq!(elements.len(), 2);
        assert!(matches!(elements, ElementsKind::PackedSmi(_)));
        assert_eq!(elements.get(0), Some(Value::from_smi(10)));
        assert_eq!(elements.get(1), Some(Value::from_smi(20)));

        // Pushing a double promotes to PackedDouble
        elements.push(Value::from_f64(30.5));
        assert_eq!(elements.len(), 3);
        assert!(matches!(elements, ElementsKind::PackedDouble(_)));
        assert_eq!(elements.get(0), Some(Value::from_f64(10.0)));
        assert_eq!(elements.get(2), Some(Value::from_f64(30.5)));

        // Pushing an object or boolean promotes to PackedValues
        elements.push(VALUE_UNDEFINED);
        assert_eq!(elements.len(), 4);
        assert!(matches!(elements, ElementsKind::PackedValues(_)));
        assert_eq!(elements.get(3), Some(VALUE_UNDEFINED));

        // Deleting element 1 creates a hole -> Holey
        elements.delete(1);
        assert!(matches!(elements, ElementsKind::Holey(_)));
        assert_eq!(elements.get(1), None);
        assert_eq!(elements.get(0), Some(Value::from_f64(10.0)));
    }

    #[test]
    fn elements_references_preserve_space_index_and_generation() {
        let young = ElementsRef::young(1023, 0xF_FFFF);
        assert!(young.is_young());
        assert_eq!(young.index(), 1023);
        assert_eq!(young.generation(), 0xF_FFFF);

        let old = ElementsRef::old(0x7F_FFFF, 0xFF);
        assert!(old.is_old());
        assert_eq!(old.index(), 0x7F_FFFF);
        assert_eq!(old.generation(), 0xFF);
        assert_ne!(young, old);
    }

    #[test]
    fn dense_element_updates_preserve_or_widen_their_representation() {
        let mut smi = ElementsKind::new_packed_smi(4);
        assert!(smi.is_empty());
        smi.push(Value::from_smi(1));
        smi.set(0, Value::from_smi(2));
        assert_eq!(smi.get(0), Some(Value::from_smi(2)));

        let mut smi_to_double = smi.clone();
        smi_to_double.set(0, Value::from_f64(2.5));
        assert!(matches!(smi_to_double, ElementsKind::PackedDouble(_)));
        smi_to_double.push(Value::from_f64(3.5));
        smi_to_double.set(1, Value::from_f64(4.5));
        assert_eq!(smi_to_double.get(1), Some(Value::from_f64(4.5)));

        let mut smi_to_values = smi.clone();
        smi_to_values.set(0, VALUE_UNDEFINED);
        assert!(matches!(smi_to_values, ElementsKind::PackedValues(_)));
        smi_to_values.push(Value::from_bool(true));
        smi_to_values.set(1, Value::from_bool(false));
        assert_eq!(smi_to_values.get(1), Some(Value::from_bool(false)));

        let mut double_to_values = smi_to_double;
        double_to_values.set(0, VALUE_UNDEFINED);
        assert!(matches!(double_to_values, ElementsKind::PackedValues(_)));
        assert_eq!(double_to_values.get(0), Some(VALUE_UNDEFINED));
    }

    #[test]
    fn sparse_writes_follow_the_holey_and_dictionary_lattice() {
        let mut smi_double_gap = ElementsKind::new_packed_smi(1);
        smi_double_gap.push(Value::from_smi(1));
        smi_double_gap.set(3, Value::from_f64(4.5));
        assert!(matches!(smi_double_gap, ElementsKind::Holey(_)));
        assert_eq!(smi_double_gap.get(1), None);

        let mut smi_value_gap = ElementsKind::new_packed_smi(1);
        smi_value_gap.push(Value::from_smi(1));
        smi_value_gap.set(3, VALUE_UNDEFINED);
        assert!(matches!(smi_value_gap, ElementsKind::Holey(_)));

        let mut double_gap = ElementsKind::PackedDouble(vec![1.0]);
        double_gap.set(3, Value::from_f64(4.0));
        assert!(matches!(double_gap, ElementsKind::Holey(_)));

        let mut values = ElementsKind::PackedValues(vec![Value::from_smi(1)]);
        values.set(1, Value::from_smi(2));
        values.set(0, Value::from_smi(3));
        values.set(4, Value::from_smi(5));
        assert!(matches!(values, ElementsKind::Holey(_)));

        values.set(5, Value::from_smi(6));
        values.set(2, Value::from_smi(3));
        values.set(8, Value::from_smi(9));
        assert_eq!(values.get(2), Some(Value::from_smi(3)));
        assert_eq!(values.get(7), None);

        let mut dictionary = ElementsKind::PackedValues(vec![Value::from_smi(1)]);
        dictionary.set(2048, Value::from_smi(2));
        assert!(matches!(dictionary, ElementsKind::Dictionary(_)));
        assert_eq!(dictionary.len(), 2049);
        assert_eq!(dictionary.get(2048), Some(Value::from_smi(2)));
        dictionary.set(2048, Value::from_smi(3));
        assert_eq!(dictionary.get(2048), Some(Value::from_smi(3)));
    }

    #[test]
    fn deletion_covers_every_elements_representation() {
        let mut smi = ElementsKind::PackedSmi(vec![1, 2]);
        assert!(smi.delete(0));
        assert!(matches!(smi, ElementsKind::Holey(_)));
        assert_eq!(smi.get(0), None);

        let mut double = ElementsKind::PackedDouble(vec![1.0, 2.0]);
        assert!(double.delete(1));
        assert!(matches!(double, ElementsKind::Holey(_)));
        assert_eq!(double.get(1), None);

        let mut holey = ElementsKind::Holey(vec![Some(Value::from_smi(1))]);
        assert!(holey.delete(0));
        assert_eq!(holey.get(0), None);
        assert!(holey.delete(8));

        let mut dictionary = BTreeMap::new();
        dictionary.insert(4, Value::from_smi(4));
        let mut dictionary = ElementsKind::Dictionary(dictionary);
        assert!(dictionary.delete(4));
        assert!(dictionary.is_empty());
    }
}
