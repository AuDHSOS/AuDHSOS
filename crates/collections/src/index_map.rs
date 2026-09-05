// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A map of fixed capacity, kept sorted by key.
//!
//! The entries live in one array in key order, so a lookup is a binary
//! search and iteration is sorted without a second thought. Insertion and
//! removal move the entries above the place, which is the trade a fixed
//! map makes: a hash map would need a hasher and a policy for collisions,
//! and neither is worth it for the tens of entries a kernel table holds.

use core::cmp::Ordering;

use crate::error::CollectionError;

/// A map from `K` to `V` with at most `N` entries.
#[derive(Debug)]
pub struct IndexMap<K, V, const N: usize> {
    /// The entries, sorted by key. Those below `len` are `Some`.
    slots: [Option<(K, V)>; N],
    /// How many entries there are.
    len: usize,
}

impl<K: Ord, V, const N: usize> IndexMap<K, V, N> {
    /// The number of entries this type holds.
    pub const CAPACITY: usize = N;

    /// An empty map.
    #[must_use]
    pub const fn new() -> IndexMap<K, V, N> {
        IndexMap {
            slots: [const { None }; N],
            len: 0,
        }
    }

    /// How many entries this map holds.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// How many entries are in it.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether it holds nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether one more entry would not fit.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.len >= N
    }

    /// Stores `value` under `key` and answers what stood there.
    ///
    /// A key that is already in the map replaces its value and does not
    /// grow the map, so a caller can insert in a loop without counting.
    ///
    /// # Errors
    ///
    /// [`CollectionError::Full`] when the key is new and the map is at its
    /// capacity.
    pub fn insert(&mut self, key: K, value: V) -> Result<Option<V>, CollectionError> {
        match self.search(&key) {
            Ok(at) => {
                let slot = self.slots.get_mut(at).ok_or(CollectionError::Full)?;
                let previous = slot.take().map(|(_, old)| old);
                *slot = Some((key, value));
                Ok(previous)
            }
            Err(at) => {
                if self.is_full() {
                    return Err(CollectionError::Full);
                }
                let mut position = self.len;
                while position > at {
                    let below = position.wrapping_sub(1);
                    let moved = self.slots.get_mut(below).and_then(Option::take);
                    let slot = self.slots.get_mut(position).ok_or(CollectionError::Full)?;
                    *slot = moved;
                    position = below;
                }
                let slot = self.slots.get_mut(at).ok_or(CollectionError::Full)?;
                *slot = Some((key, value));
                self.len = self.len.wrapping_add(1);
                Ok(None)
            }
        }
    }

    /// The value stored under `key`.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<&V> {
        let at = self.search(key).ok()?;
        self.slots.get(at)?.as_ref().map(|(_, value)| value)
    }

    /// The value stored under `key`, to change.
    #[must_use]
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let at = self.search(key).ok()?;
        self.slots.get_mut(at)?.as_mut().map(|(_, value)| value)
    }

    /// Whether `key` is in the map.
    #[must_use]
    pub fn contains_key(&self, key: &K) -> bool {
        self.search(key).is_ok()
    }

    /// Removes `key` and answers the value that stood under it.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        let at = self.search(key).ok()?;
        let removed = self.slots.get_mut(at)?.take().map(|(_, value)| value);
        let mut position = at;
        loop {
            let above = position.wrapping_add(1);
            if above >= self.len {
                break;
            }
            let moved = self.slots.get_mut(above).and_then(Option::take);
            let slot = self.slots.get_mut(position)?;
            *slot = moved;
            position = above;
        }
        self.len = self.len.wrapping_sub(1);
        removed
    }

    /// Removes every entry.
    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            *slot = None;
        }
        self.len = 0;
    }

    /// The entries in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.slots
            .iter()
            .take(self.len)
            .filter_map(Option::as_ref)
            .map(|(key, value)| (key, value))
    }

    /// The keys in order.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(key, _)| key)
    }

    /// The values in the order of their keys.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, value)| value)
    }

    /// Where `key` is, or where it would go. `Ok` is a hit, `Err` is the
    /// place an insertion belongs.
    fn search(&self, key: &K) -> Result<usize, usize> {
        let mut low = 0usize;
        let mut high = self.len;
        while low < high {
            let middle = low.wrapping_add(high.wrapping_sub(low).wrapping_div(2));
            let Some(Some((stored, _))) = self.slots.get(middle) else {
                return Err(low);
            };
            match stored.cmp(key) {
                Ordering::Less => low = middle.wrapping_add(1),
                Ordering::Greater => high = middle,
                Ordering::Equal => return Ok(middle),
            }
        }
        Err(low)
    }
}

impl<K: Ord, V, const N: usize> Default for IndexMap<K, V, N> {
    fn default() -> IndexMap<K, V, N> {
        IndexMap::new()
    }
}
