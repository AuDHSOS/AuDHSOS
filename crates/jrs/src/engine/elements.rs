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
pub struct ElementsRef(pub u32);

const OLD_GENERATION_BIT: u32 = 1 << 31;

impl ElementsRef {
    /// Creates a reference into the active Nursery semispace.
    #[must_use]
    pub const fn young(index: u32) -> Self {
        Self(index & !OLD_GENERATION_BIT)
    }

    /// Creates a reference into the Old Generation.
    #[must_use]
    pub const fn old(index: u32) -> Self {
        Self(OLD_GENERATION_BIT | (index & !OLD_GENERATION_BIT))
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
        self.0 & !OLD_GENERATION_BIT
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
}
