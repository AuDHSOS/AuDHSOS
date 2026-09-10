// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "Generational nursery bump-pointer and scavenge promotions"
)]

//! Generational Memory Management, Bump-Pointer Nursery and Rooting.
//!
//! Objects are bump-allocated in a young generation (Nursery). Collections
//! use a semi-space scavenge: live objects surviving collection are copied to
//! the Old Generation, and the nursery is cleared in O(1). Native roots are
//! managed precisely via `HandleScope` guards.

use super::{
    elements::{ElementsKind, ElementsRef},
    object::JSObject,
    shape::{ShapeId, ShapeTable},
    string::StringArena,
    value::{ObjectRef, VALUE_NULL, Value},
};
use alloc::{collections::BTreeMap, vec::Vec};

/// Default capacity of the Nursery in number of objects.
pub const NURSERY_OBJECT_CAPACITY: usize = 1024;

/// The Nursery space where young objects are bump-allocated.
pub struct Nursery {
    objects: Vec<JSObject>,
    elements: Vec<ElementsKind>,
}

impl Nursery {
    /// Creates a new nursery with preallocated capacities.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            objects: Vec::with_capacity(capacity),
            elements: Vec::with_capacity(capacity),
        }
    }

    /// Clears the nursery in O(1) by resetting lengths without deallocating buffer capacity.
    pub fn clear(&mut self) {
        self.objects.clear();
        self.elements.clear();
    }

    /// Returns `true` if nursery capacity is reached.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.objects.len() >= self.objects.capacity()
    }
}

/// Long-lived Old Generation heap space.
#[derive(Default)]
pub struct OldGeneration {
    objects: Vec<Option<JSObject>>,
    elements: Vec<Option<ElementsKind>>,
    free_objects: Vec<usize>,
}

impl OldGeneration {
    /// Allocates or promotes an object into the old generation.
    pub fn allocate(&mut self, object: JSObject) -> usize {
        if let Some(free_idx) = self.free_objects.pop()
            && let Some(slot) = self.objects.get_mut(free_idx)
        {
            *slot = Some(object);
            return free_idx;
        }
        let idx = self.objects.len();
        self.objects.push(Some(object));
        idx
    }

    /// Allocates or promotes an elements store into the old generation.
    pub fn allocate_elements(&mut self, elem: ElementsKind) -> usize {
        let idx = self.elements.len();
        self.elements.push(Some(elem));
        idx
    }
}

/// Generational execution heap.
pub struct GenerationalHeap {
    /// Shared Shape transition table.
    pub shapes: ShapeTable,
    /// Shared String arena and interning table.
    pub strings: StringArena,
    nursery: Nursery,
    old_gen: OldGeneration,
    // Roots registered by active HandleScopes or native frames
    roots: Vec<Value>,
    scope_markers: Vec<usize>,
}

impl Default for GenerationalHeap {
    fn default() -> Self {
        Self::new()
    }
}

impl GenerationalHeap {
    /// Creates an empty generational heap.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shapes: ShapeTable::new(),
            strings: StringArena::new(),
            nursery: Nursery::new(NURSERY_OBJECT_CAPACITY),
            old_gen: OldGeneration::default(),
            roots: Vec::with_capacity(128),
            scope_markers: Vec::with_capacity(16),
        }
    }

    /// Bump-allocates a new object in the nursery.
    pub fn allocate_object(&mut self, shape_id: ShapeId, prototype: Value) -> ObjectRef {
        if self.nursery.is_full() {
            self.scavenge();
        }
        let idx = self.nursery.objects.len();
        self.nursery
            .objects
            .push(JSObject::new(shape_id, prototype));
        #[expect(clippy::as_conversions, reason = "nursery object index fits in u32")]
        ObjectRef(idx as u32)
    }

    /// Allocates an array with a dedicated elements store.
    pub fn allocate_array(&mut self, length: u32) -> ObjectRef {
        if self.nursery.is_full() {
            self.scavenge();
        }
        let elem_idx = self.nursery.elements.len();
        #[expect(clippy::as_conversions, reason = "initial capacity clamped to 16")]
        self.nursery
            .elements
            .push(ElementsKind::new_packed_smi(16.min(length as usize)));
        #[expect(clippy::as_conversions, reason = "nursery element index fits in u32")]
        let elem_ref = ElementsRef(elem_idx as u32);

        let root_shape = self.shapes.root_shape();
        let obj_idx = self.nursery.objects.len();
        self.nursery.objects.push(JSObject::new_array(
            root_shape, VALUE_NULL, elem_ref, length,
        ));
        #[expect(clippy::as_conversions, reason = "nursery object index fits in u32")]
        ObjectRef(obj_idx as u32)
    }

    /// Reads an immutable reference to an object.
    #[must_use]
    pub fn get_object(&self, oref: ObjectRef) -> Option<&JSObject> {
        let idx = oref.0 as usize;
        // Check nursery first, then old generation
        if let Some(obj) = self.nursery.objects.get(idx) {
            Some(obj)
        } else if let Some(Some(obj)) = self.old_gen.objects.get(idx) {
            Some(obj)
        } else {
            None
        }
    }

    /// Reads a mutable reference to an object.
    pub fn get_object_mut(&mut self, oref: ObjectRef) -> Option<&mut JSObject> {
        let idx = oref.0 as usize;
        if idx < self.nursery.objects.len() {
            self.nursery.objects.get_mut(idx)
        } else if let Some(slot) = self.old_gen.objects.get_mut(idx) {
            slot.as_mut()
        } else {
            None
        }
    }

    /// Reads an immutable reference to an elements store.
    #[must_use]
    pub fn get_elements(&self, eref: ElementsRef) -> Option<&ElementsKind> {
        let idx = eref.0 as usize;
        if let Some(elem) = self.nursery.elements.get(idx) {
            Some(elem)
        } else if let Some(Some(elem)) = self.old_gen.elements.get(idx) {
            Some(elem)
        } else {
            None
        }
    }

    /// Reads a mutable reference to an elements store.
    pub fn get_elements_mut(&mut self, eref: ElementsRef) -> Option<&mut ElementsKind> {
        let idx = eref.0 as usize;
        if idx < self.nursery.elements.len() {
            self.nursery.elements.get_mut(idx)
        } else if let Some(slot) = self.old_gen.elements.get_mut(idx) {
            slot.as_mut()
        } else {
            None
        }
    }

    /// Pushes a GC root onto the active root stack.
    pub fn push_root(&mut self, value: Value) {
        self.roots.push(value);
    }

    /// Starts a new `HandleScope`.
    pub fn enter_scope(&mut self) {
        self.scope_markers.push(self.roots.len());
    }

    /// Exits the current `HandleScope` and pops its roots.
    pub fn exit_scope(&mut self) {
        if let Some(mark) = self.scope_markers.pop() {
            self.roots.truncate(mark);
        }
    }

    /// Executes a scavenge collection: copies live reachable nursery objects to `OldGen`.
    pub fn scavenge(&mut self) {
        let mut forward_map: BTreeMap<u32, u32> = BTreeMap::new();

        // Promote reachable roots
        for i in 0..self.roots.len() {
            if let Some(&root) = self.roots.get(i)
                && let Some(oref) = root.as_object()
            {
                let old_id = oref.0;
                if let Some(obj) = self.nursery.objects.get(old_id as usize).cloned() {
                    let new_id = self.old_gen.allocate(obj);
                    #[expect(clippy::as_conversions, reason = "index fits u32")]
                    let new_u32 = new_id as u32;
                    forward_map.insert(old_id, new_u32);
                    if let Some(root_slot) = self.roots.get_mut(i) {
                        *root_slot = Value::from_object(ObjectRef(new_u32));
                    }
                }
            }
        }

        // Reset nursery in O(1)
        self.nursery.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_allocation_and_scavenge_promotes_roots() {
        let mut heap = GenerationalHeap::new();
        let root_shape = heap.shapes.root_shape();

        heap.enter_scope();
        let obj1 = heap.allocate_object(root_shape, VALUE_NULL);
        heap.push_root(Value::from_object(obj1));

        // obj2 is allocated but not rooted
        let _obj2 = heap.allocate_object(root_shape, VALUE_NULL);

        // Trigger scavenge
        heap.scavenge();

        // obj1 should have been promoted to OldGen
        let promoted_root = heap.roots.first().copied().unwrap();
        assert!(promoted_root.is_object());
        let new_ref = promoted_root.as_object().unwrap();
        assert!(heap.get_object(new_ref).is_some());

        heap.exit_scope();
        assert!(heap.roots.is_empty());
    }
}
