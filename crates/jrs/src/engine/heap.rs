// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::as_conversions,
    reason = "generation-local heap indices are checked before conversion"
)]

//! Generational Memory Management, semispace scavenging and precise roots.
//!
//! Objects and elements are bump-allocated in the active Nursery semispace.
//! A minor collection copies the complete reachable Young graph into a fresh
//! semispace and promotes entries that survived two collections. Old-to-Young
//! stores pass through central APIs which maintain Remembered Sets.

use super::{
    elements::{ElementsKind, ElementsRef},
    object::{JSObject, ObjectKind},
    shape::{ShapeId, ShapeTable},
    string::StringArena,
    value::{ObjectRef, VALUE_NULL, Value},
};
use alloc::{collections::BTreeMap, collections::BTreeSet, vec::Vec};

/// Default capacity of the Nursery in number of objects.
pub const NURSERY_OBJECT_CAPACITY: usize = 1024;

/// Number of survived minor collections before promotion.
pub const PROMOTION_AGE: u8 = 2;

/// Allocation or reference failure in the generational heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeapError {
    /// The Nursery must be collected before another object can be allocated.
    NurseryFull,
    /// A generation-tagged reference does not address a live entry.
    InvalidReference,
    /// A generation-local index cannot be represented in the 31-bit payload.
    ReferenceSpaceExhausted,
}

#[derive(Clone)]
struct YoungObject {
    value: JSObject,
    age: u8,
}

#[derive(Clone)]
struct YoungElements {
    value: ElementsKind,
    age: u8,
}

/// Active Nursery semispace where Young entries are bump-allocated.
struct Nursery {
    objects: Vec<YoungObject>,
    elements: Vec<YoungElements>,
    object_capacity: usize,
}

impl Nursery {
    fn new(capacity: usize) -> Self {
        Self {
            objects: Vec::with_capacity(capacity),
            elements: Vec::with_capacity(capacity),
            object_capacity: capacity,
        }
    }

    const fn is_full(&self) -> bool {
        self.objects.len() >= self.object_capacity
    }
}

/// Long-lived Old Generation heap space.
#[derive(Default)]
struct OldGeneration {
    objects: Vec<Option<JSObject>>,
    elements: Vec<Option<ElementsKind>>,
    free_objects: Vec<usize>,
    free_elements: Vec<usize>,
}

impl OldGeneration {
    fn allocate_object(&mut self, object: JSObject) -> Result<ObjectRef, HeapError> {
        let index = if let Some(index) = self.free_objects.pop() {
            let slot = self
                .objects
                .get_mut(index)
                .ok_or(HeapError::InvalidReference)?;
            *slot = Some(object);
            index
        } else {
            let index = self.objects.len();
            self.objects.push(Some(object));
            index
        };
        generation_index(index).map(ObjectRef::old)
    }

    fn allocate_elements(&mut self, elements: ElementsKind) -> Result<ElementsRef, HeapError> {
        let index = if let Some(index) = self.free_elements.pop() {
            let slot = self
                .elements
                .get_mut(index)
                .ok_or(HeapError::InvalidReference)?;
            *slot = Some(elements);
            index
        } else {
            let index = self.elements.len();
            self.elements.push(Some(elements));
            index
        };
        generation_index(index).map(ElementsRef::old)
    }
}

/// Stable index into the heap's native root stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Root(u32);

/// Statistics for one completed minor collection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScavengeStats {
    /// Young objects copied to the next Nursery semispace.
    pub copied_objects: usize,
    /// Young objects promoted to the Old Generation.
    pub promoted_objects: usize,
    /// Young elements stores copied to the next Nursery semispace.
    pub copied_elements: usize,
    /// Young elements stores promoted to the Old Generation.
    pub promoted_elements: usize,
}

/// Generational execution heap.
pub struct GenerationalHeap {
    /// Shared Shape transition table.
    pub shapes: ShapeTable,
    /// Shared String arena and interning table.
    pub strings: StringArena,
    nursery: Nursery,
    old_gen: OldGeneration,
    roots: Vec<Value>,
    scope_markers: Vec<usize>,
    remembered_objects: BTreeSet<u32>,
    remembered_elements: BTreeSet<u32>,
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
        Self::with_nursery_capacity(NURSERY_OBJECT_CAPACITY)
    }

    /// Creates a heap with a specified Nursery object capacity.
    ///
    /// A small capacity is useful for deterministic GC stress tests.
    #[must_use]
    pub fn with_nursery_capacity(capacity: usize) -> Self {
        Self {
            shapes: ShapeTable::new(),
            strings: StringArena::new(),
            nursery: Nursery::new(capacity.max(1)),
            old_gen: OldGeneration::default(),
            roots: Vec::with_capacity(128),
            scope_markers: Vec::with_capacity(16),
            remembered_objects: BTreeSet::new(),
            remembered_elements: BTreeSet::new(),
        }
    }

    /// Returns whether a minor collection is required before object allocation.
    #[must_use]
    pub const fn nursery_is_full(&self) -> bool {
        self.nursery.is_full()
    }

    /// Bump-allocates a new object in the Nursery.
    ///
    /// The caller must expose all VM roots and run [`Self::scavenge_with_roots`]
    /// when this returns [`HeapError::NurseryFull`]. Collection is never hidden
    /// inside allocation because the heap cannot discover interpreter registers.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::NurseryFull`] when a Safe Point is required, or
    /// [`HeapError::ReferenceSpaceExhausted`] when no tagged index remains.
    pub fn allocate_object(
        &mut self,
        shape_id: ShapeId,
        prototype: Value,
    ) -> Result<ObjectRef, HeapError> {
        if self.nursery.is_full() {
            return Err(HeapError::NurseryFull);
        }
        let index = generation_index(self.nursery.objects.len())?;
        self.nursery.objects.push(YoungObject {
            value: JSObject::new(shape_id, prototype),
            age: 0,
        });
        Ok(ObjectRef::young(index))
    }

    /// Allocates an Array and its dedicated elements store in the Nursery.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::NurseryFull`] when a Safe Point is required, or
    /// [`HeapError::ReferenceSpaceExhausted`] when no tagged index remains.
    pub fn allocate_array(&mut self, length: u32) -> Result<ObjectRef, HeapError> {
        if self.nursery.is_full() {
            return Err(HeapError::NurseryFull);
        }
        let elements_index = generation_index(self.nursery.elements.len())?;
        self.nursery.elements.push(YoungElements {
            value: ElementsKind::new_packed_smi(16.min(length as usize)),
            age: 0,
        });
        let elements = ElementsRef::young(elements_index);

        let object_index = generation_index(self.nursery.objects.len())?;
        self.nursery.objects.push(YoungObject {
            value: JSObject::new_array(self.shapes.root_shape(), VALUE_NULL, elements, length),
            age: 0,
        });
        Ok(ObjectRef::young(object_index))
    }

    /// Reads an immutable object reference from its tagged generation.
    #[must_use]
    pub fn get_object(&self, reference: ObjectRef) -> Option<&JSObject> {
        let index = reference.index() as usize;
        if reference.is_old() {
            self.old_gen.objects.get(index)?.as_ref()
        } else {
            self.nursery.objects.get(index).map(|entry| &entry.value)
        }
    }

    /// Reads an immutable elements store from its tagged generation.
    #[must_use]
    pub fn get_elements(&self, reference: ElementsRef) -> Option<&ElementsKind> {
        let index = reference.index() as usize;
        if reference.is_old() {
            self.old_gen.elements.get(index)?.as_ref()
        } else {
            self.nursery.elements.get(index).map(|entry| &entry.value)
        }
    }

    /// Updates an object's Shape without exposing a mutable heap borrow.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale or invalid object reference.
    pub fn set_object_shape(
        &mut self,
        reference: ObjectRef,
        shape_id: ShapeId,
    ) -> Result<(), HeapError> {
        self.object_mut(reference)?.shape_id = shape_id;
        Ok(())
    }

    /// Stores an object prototype and records an Old-to-Young edge.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale or invalid object reference.
    pub fn set_object_prototype(
        &mut self,
        reference: ObjectRef,
        prototype: Value,
    ) -> Result<(), HeapError> {
        self.remember_object_store(reference, prototype);
        self.object_mut(reference)?.prototype = prototype;
        Ok(())
    }

    /// Stores a named-property slot and records an Old-to-Young edge.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale or invalid object reference.
    pub fn set_object_slot(
        &mut self,
        reference: ObjectRef,
        slot: u32,
        value: Value,
    ) -> Result<(), HeapError> {
        self.remember_object_store(reference, value);
        self.object_mut(reference)?.set_slot(slot, value);
        Ok(())
    }

    /// Writes an indexed element and records an Old-to-Young edge.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale or invalid elements reference.
    pub fn set_element(
        &mut self,
        reference: ElementsRef,
        index: u32,
        value: Value,
    ) -> Result<(), HeapError> {
        self.remember_elements_store(reference, value);
        self.elements_mut(reference)?.set(index, value);
        Ok(())
    }

    /// Appends an indexed element and records an Old-to-Young edge.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale or invalid elements reference.
    pub fn push_element(&mut self, reference: ElementsRef, value: Value) -> Result<(), HeapError> {
        self.remember_elements_store(reference, value);
        self.elements_mut(reference)?.push(value);
        Ok(())
    }

    /// Deletes an indexed element.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale or invalid elements reference.
    pub fn delete_element(
        &mut self,
        reference: ElementsRef,
        index: u32,
    ) -> Result<bool, HeapError> {
        Ok(self.elements_mut(reference)?.delete(index))
    }

    /// Pushes a GC root and returns a stable index for reading its forwarded value.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::ReferenceSpaceExhausted`] when no root index remains.
    pub fn push_root(&mut self, value: Value) -> Result<Root, HeapError> {
        let index = generation_index(self.roots.len())?;
        self.roots.push(value);
        Ok(Root(index))
    }

    /// Reads the current value of a root after any number of minor collections.
    #[must_use]
    pub fn root_value(&self, root: Root) -> Option<Value> {
        self.roots.get(root.0 as usize).copied()
    }

    /// Starts a new native `HandleScope`.
    pub fn enter_scope(&mut self) {
        self.scope_markers.push(self.roots.len());
    }

    /// Exits the current `HandleScope` and releases all roots created within it.
    pub fn exit_scope(&mut self) {
        if let Some(mark) = self.scope_markers.pop() {
            self.roots.truncate(mark);
        }
    }

    /// Runs a minor collection using native roots only.
    ///
    /// # Errors
    ///
    /// Returns an error if a reachable reference is invalid or no target-space
    /// index can be represented.
    pub fn scavenge(&mut self) -> Result<ScavengeStats, HeapError> {
        let mut no_external_roots = [];
        let mut no_accumulator = VALUE_NULL;
        self.scavenge_with_roots(&mut no_external_roots, &mut no_accumulator)
    }

    /// Runs a minor collection with an interpreter register slice and accumulator.
    ///
    /// Every forwarded object reference in `registers`, `accumulator`, and the
    /// native root stack is rewritten before the old semispace is discarded.
    ///
    /// # Errors
    ///
    /// Returns an error if a reachable reference is invalid or no target-space
    /// index can be represented.
    pub fn scavenge_with_roots(
        &mut self,
        registers: &mut [Value],
        accumulator: &mut Value,
    ) -> Result<ScavengeStats, HeapError> {
        let capacity = self.nursery.object_capacity;
        let from = core::mem::replace(&mut self.nursery, Nursery::new(capacity));
        let remembered_objects = core::mem::take(&mut self.remembered_objects);
        let remembered_elements = core::mem::take(&mut self.remembered_elements);
        let mut evacuator = Evacuator::new(from, &mut self.old_gen, &mut self.nursery);

        for root in &mut self.roots {
            evacuator.evacuate_value(root)?;
        }
        for value in registers {
            evacuator.evacuate_value(value)?;
        }
        evacuator.evacuate_value(accumulator)?;

        for index in remembered_objects {
            evacuator.enqueue_old_object(index);
        }
        for index in remembered_elements {
            evacuator.enqueue_old_elements(index);
        }
        evacuator.drain()?;

        let stats = evacuator.stats;
        drop(evacuator);
        self.rebuild_remembered_sets();
        Ok(stats)
    }

    fn object_mut(&mut self, reference: ObjectRef) -> Result<&mut JSObject, HeapError> {
        let index = reference.index() as usize;
        if reference.is_old() {
            self.old_gen
                .objects
                .get_mut(index)
                .and_then(Option::as_mut)
                .ok_or(HeapError::InvalidReference)
        } else {
            self.nursery
                .objects
                .get_mut(index)
                .map(|entry| &mut entry.value)
                .ok_or(HeapError::InvalidReference)
        }
    }

    fn elements_mut(&mut self, reference: ElementsRef) -> Result<&mut ElementsKind, HeapError> {
        let index = reference.index() as usize;
        if reference.is_old() {
            self.old_gen
                .elements
                .get_mut(index)
                .and_then(Option::as_mut)
                .ok_or(HeapError::InvalidReference)
        } else {
            self.nursery
                .elements
                .get_mut(index)
                .map(|entry| &mut entry.value)
                .ok_or(HeapError::InvalidReference)
        }
    }

    fn remember_object_store(&mut self, owner: ObjectRef, value: Value) {
        if owner.is_old() && value.as_object().is_some_and(ObjectRef::is_young) {
            self.remembered_objects.insert(owner.index());
        }
    }

    fn remember_elements_store(&mut self, owner: ElementsRef, value: Value) {
        if owner.is_old() && value.as_object().is_some_and(ObjectRef::is_young) {
            self.remembered_elements.insert(owner.index());
        }
    }

    fn rebuild_remembered_sets(&mut self) {
        for (index, object) in self.old_gen.objects.iter().enumerate() {
            if object.as_ref().is_some_and(object_contains_young)
                && let Ok(index) = generation_index(index)
            {
                self.remembered_objects.insert(index);
            }
        }
        for (index, elements) in self.old_gen.elements.iter().enumerate() {
            if elements.as_ref().is_some_and(elements_contain_young)
                && let Ok(index) = generation_index(index)
            {
                self.remembered_elements.insert(index);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Work {
    Object(ObjectRef),
    Elements(ElementsRef),
}

struct Evacuator<'heap> {
    from: Nursery,
    old: &'heap mut OldGeneration,
    to: &'heap mut Nursery,
    object_forwarding: BTreeMap<u32, ObjectRef>,
    elements_forwarding: BTreeMap<u32, ElementsRef>,
    work: Vec<Work>,
    stats: ScavengeStats,
}

impl<'heap> Evacuator<'heap> {
    fn new(from: Nursery, old: &'heap mut OldGeneration, to: &'heap mut Nursery) -> Self {
        Self {
            from,
            old,
            to,
            object_forwarding: BTreeMap::new(),
            elements_forwarding: BTreeMap::new(),
            work: Vec::new(),
            stats: ScavengeStats::default(),
        }
    }

    fn evacuate_value(&mut self, value: &mut Value) -> Result<(), HeapError> {
        let Some(reference) = value.as_object() else {
            return Ok(());
        };
        *value = Value::from_object(self.evacuate_object(reference)?);
        Ok(())
    }

    fn evacuate_object(&mut self, reference: ObjectRef) -> Result<ObjectRef, HeapError> {
        if reference.is_old() {
            return Ok(reference);
        }
        if let Some(forwarded) = self.object_forwarding.get(&reference.index()) {
            return Ok(*forwarded);
        }
        let entry = self
            .from
            .objects
            .get(reference.index() as usize)
            .cloned()
            .ok_or(HeapError::InvalidReference)?;
        let age = entry.age.saturating_add(1);
        let forwarded = if age >= PROMOTION_AGE {
            self.stats.promoted_objects = self.stats.promoted_objects.saturating_add(1);
            self.old.allocate_object(entry.value)?
        } else {
            let index = generation_index(self.to.objects.len())?;
            self.to.objects.push(YoungObject {
                value: entry.value,
                age,
            });
            self.stats.copied_objects = self.stats.copied_objects.saturating_add(1);
            ObjectRef::young(index)
        };
        self.object_forwarding.insert(reference.index(), forwarded);
        self.work.push(Work::Object(forwarded));
        Ok(forwarded)
    }

    fn evacuate_elements(&mut self, reference: ElementsRef) -> Result<ElementsRef, HeapError> {
        if reference.is_old() {
            return Ok(reference);
        }
        if let Some(forwarded) = self.elements_forwarding.get(&reference.index()) {
            return Ok(*forwarded);
        }
        let entry = self
            .from
            .elements
            .get(reference.index() as usize)
            .cloned()
            .ok_or(HeapError::InvalidReference)?;
        let age = entry.age.saturating_add(1);
        let forwarded = if age >= PROMOTION_AGE {
            self.stats.promoted_elements = self.stats.promoted_elements.saturating_add(1);
            self.old.allocate_elements(entry.value)?
        } else {
            let index = generation_index(self.to.elements.len())?;
            self.to.elements.push(YoungElements {
                value: entry.value,
                age,
            });
            self.stats.copied_elements = self.stats.copied_elements.saturating_add(1);
            ElementsRef::young(index)
        };
        self.elements_forwarding
            .insert(reference.index(), forwarded);
        self.work.push(Work::Elements(forwarded));
        Ok(forwarded)
    }

    fn enqueue_old_object(&mut self, index: u32) {
        self.work.push(Work::Object(ObjectRef::old(index)));
    }

    fn enqueue_old_elements(&mut self, index: u32) {
        self.work.push(Work::Elements(ElementsRef::old(index)));
    }

    fn drain(&mut self) -> Result<(), HeapError> {
        while let Some(work) = self.work.pop() {
            match work {
                Work::Object(reference) => self.scan_object(reference)?,
                Work::Elements(reference) => self.scan_elements(reference)?,
            }
        }
        Ok(())
    }

    fn scan_object(&mut self, reference: ObjectRef) -> Result<(), HeapError> {
        let mut object = self.object(reference)?.clone();
        self.evacuate_value(&mut object.prototype)?;
        for value in &mut object.in_object_slots {
            self.evacuate_value(value)?;
        }
        if let Some(slots) = &mut object.out_of_line_slots {
            for value in slots {
                self.evacuate_value(value)?;
            }
        }
        if let ObjectKind::StringWrapper(value) = &mut object.kind {
            self.evacuate_value(value)?;
        }
        if let Some(elements) = &mut object.elements {
            *elements = self.evacuate_elements(*elements)?;
        }
        *self.object_mut(reference)? = object;
        Ok(())
    }

    fn scan_elements(&mut self, reference: ElementsRef) -> Result<(), HeapError> {
        let mut elements = self.elements(reference)?.clone();
        visit_elements_values_mut(&mut elements, |value| self.evacuate_value(value))?;
        *self.elements_mut(reference)? = elements;
        Ok(())
    }

    fn object(&self, reference: ObjectRef) -> Result<&JSObject, HeapError> {
        let index = reference.index() as usize;
        if reference.is_old() {
            self.old
                .objects
                .get(index)
                .and_then(Option::as_ref)
                .ok_or(HeapError::InvalidReference)
        } else {
            self.to
                .objects
                .get(index)
                .map(|entry| &entry.value)
                .ok_or(HeapError::InvalidReference)
        }
    }

    fn object_mut(&mut self, reference: ObjectRef) -> Result<&mut JSObject, HeapError> {
        let index = reference.index() as usize;
        if reference.is_old() {
            self.old
                .objects
                .get_mut(index)
                .and_then(Option::as_mut)
                .ok_or(HeapError::InvalidReference)
        } else {
            self.to
                .objects
                .get_mut(index)
                .map(|entry| &mut entry.value)
                .ok_or(HeapError::InvalidReference)
        }
    }

    fn elements(&self, reference: ElementsRef) -> Result<&ElementsKind, HeapError> {
        let index = reference.index() as usize;
        if reference.is_old() {
            self.old
                .elements
                .get(index)
                .and_then(Option::as_ref)
                .ok_or(HeapError::InvalidReference)
        } else {
            self.to
                .elements
                .get(index)
                .map(|entry| &entry.value)
                .ok_or(HeapError::InvalidReference)
        }
    }

    fn elements_mut(&mut self, reference: ElementsRef) -> Result<&mut ElementsKind, HeapError> {
        let index = reference.index() as usize;
        if reference.is_old() {
            self.old
                .elements
                .get_mut(index)
                .and_then(Option::as_mut)
                .ok_or(HeapError::InvalidReference)
        } else {
            self.to
                .elements
                .get_mut(index)
                .map(|entry| &mut entry.value)
                .ok_or(HeapError::InvalidReference)
        }
    }
}

fn generation_index(index: usize) -> Result<u32, HeapError> {
    u32::try_from(index)
        .ok()
        .filter(|index| *index < (1 << 31))
        .ok_or(HeapError::ReferenceSpaceExhausted)
}

fn value_is_young(value: Value) -> bool {
    value.as_object().is_some_and(ObjectRef::is_young)
}

fn object_contains_young(object: &JSObject) -> bool {
    value_is_young(object.prototype)
        || object.in_object_slots.iter().copied().any(value_is_young)
        || object
            .out_of_line_slots
            .as_ref()
            .is_some_and(|slots| slots.iter().copied().any(value_is_young))
        || matches!(&object.kind, ObjectKind::StringWrapper(value) if value_is_young(*value))
        || object.elements.is_some_and(ElementsRef::is_young)
}

fn elements_contain_young(elements: &ElementsKind) -> bool {
    match elements {
        ElementsKind::PackedValues(values) => values.iter().copied().any(value_is_young),
        ElementsKind::Holey(values) => values.iter().flatten().copied().any(value_is_young),
        ElementsKind::Dictionary(values) => values.values().copied().any(value_is_young),
        ElementsKind::PackedSmi(_) | ElementsKind::PackedDouble(_) => false,
    }
}

fn visit_elements_values_mut(
    elements: &mut ElementsKind,
    mut visitor: impl FnMut(&mut Value) -> Result<(), HeapError>,
) -> Result<(), HeapError> {
    match elements {
        ElementsKind::PackedValues(values) => {
            for value in values {
                visitor(value)?;
            }
        }
        ElementsKind::Holey(values) => {
            for value in values.iter_mut().flatten() {
                visitor(value)?;
            }
        }
        ElementsKind::Dictionary(values) => {
            for value in values.values_mut() {
                visitor(value)?;
            }
        }
        ElementsKind::PackedSmi(_) | ElementsKind::PackedDouble(_) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rooted_object(heap: &GenerationalHeap, root: Root) -> ObjectRef {
        heap.root_value(root)
            .and_then(Value::as_object)
            .expect("rooted object")
    }

    #[test]
    fn scavenge_copies_transitive_cycles_and_drops_unreachable_objects() {
        let mut heap = GenerationalHeap::with_nursery_capacity(4);
        let shape = heap.shapes.root_shape();
        heap.enter_scope();
        let first = heap.allocate_object(shape, VALUE_NULL).unwrap();
        let second = heap.allocate_object(shape, VALUE_NULL).unwrap();
        let unreachable = heap.allocate_object(shape, VALUE_NULL).unwrap();
        heap.set_object_slot(first, 0, Value::from_object(second))
            .unwrap();
        heap.set_object_slot(second, 0, Value::from_object(first))
            .unwrap();
        let root = heap.push_root(Value::from_object(first)).unwrap();

        let stats = heap.scavenge().unwrap();

        assert_eq!(stats.copied_objects, 2);
        assert!(heap.get_object(unreachable).is_none());
        let forwarded_first = rooted_object(&heap, root);
        let forwarded_second = heap
            .get_object(forwarded_first)
            .and_then(|object| object.get_slot(0))
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(
            heap.get_object(forwarded_second)
                .and_then(|object| object.get_slot(0)),
            Some(Value::from_object(forwarded_first))
        );
    }

    #[test]
    fn second_survival_promotes_object_and_elements() {
        let mut heap = GenerationalHeap::with_nursery_capacity(2);
        heap.enter_scope();
        let array = heap.allocate_array(0).unwrap();
        let root = heap.push_root(Value::from_object(array)).unwrap();

        let first = heap.scavenge().unwrap();
        assert_eq!(first.copied_objects, 1);
        assert_eq!(first.copied_elements, 1);
        assert!(rooted_object(&heap, root).is_young());

        let second = heap.scavenge().unwrap();
        assert_eq!(second.promoted_objects, 1);
        assert_eq!(second.promoted_elements, 1);
        let promoted = rooted_object(&heap, root);
        assert!(promoted.is_old());
        assert!(
            heap.get_object(promoted)
                .and_then(|object| object.elements)
                .is_some_and(ElementsRef::is_old)
        );
    }

    #[test]
    fn write_barrier_preserves_old_to_young_object_edge() {
        let mut heap = GenerationalHeap::with_nursery_capacity(3);
        let shape = heap.shapes.root_shape();
        heap.enter_scope();
        let parent = heap.allocate_object(shape, VALUE_NULL).unwrap();
        let root = heap.push_root(Value::from_object(parent)).unwrap();
        heap.scavenge().unwrap();
        heap.scavenge().unwrap();
        let parent = rooted_object(&heap, root);
        assert!(parent.is_old());

        let child = heap.allocate_object(shape, VALUE_NULL).unwrap();
        heap.set_object_slot(parent, 0, Value::from_object(child))
            .unwrap();
        heap.scavenge().unwrap();

        let child = heap
            .get_object(parent)
            .and_then(|object| object.get_slot(0))
            .and_then(Value::as_object)
            .unwrap();
        assert!(child.is_young());
        assert!(heap.get_object(child).is_some());
    }

    #[test]
    fn write_barrier_preserves_old_elements_to_young_object_edge() {
        let mut heap = GenerationalHeap::with_nursery_capacity(3);
        heap.enter_scope();
        let array = heap.allocate_array(0).unwrap();
        let root = heap.push_root(Value::from_object(array)).unwrap();
        heap.scavenge().unwrap();
        heap.scavenge().unwrap();
        let array = rooted_object(&heap, root);
        let elements = heap.get_object(array).unwrap().elements.unwrap();
        assert!(elements.is_old());

        let child = heap
            .allocate_object(heap.shapes.root_shape(), VALUE_NULL)
            .unwrap();
        heap.push_element(elements, Value::from_object(child))
            .unwrap();
        heap.scavenge().unwrap();

        let child = heap
            .get_elements(elements)
            .and_then(|store| store.get(0))
            .and_then(Value::as_object)
            .unwrap();
        assert!(child.is_young());
        assert!(heap.get_object(child).is_some());
    }

    #[test]
    fn external_register_roots_are_forwarded() {
        let mut heap = GenerationalHeap::with_nursery_capacity(2);
        let object = heap
            .allocate_object(heap.shapes.root_shape(), VALUE_NULL)
            .unwrap();
        let mut registers = [Value::from_object(object)];
        let mut accumulator = VALUE_NULL;

        heap.scavenge_with_roots(&mut registers, &mut accumulator)
            .unwrap();

        let forwarded = registers[0].as_object().unwrap();
        assert!(heap.get_object(forwarded).is_some());
    }
}
