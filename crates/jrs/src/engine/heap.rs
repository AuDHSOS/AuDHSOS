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
//! stores pass through central APIs which maintain Remembered Sets. A precise
//! major mark-sweep collector reclaims unreachable promoted graphs.

use super::{
    elements::{ElementsKind, ElementsRef},
    object::{JSObject, ObjectKind},
    shape::{ShapeId, ShapeTable},
    string::{StringArena, StringError},
    value::{ObjectRef, StringRef, VALUE_NULL, VALUE_UNDEFINED, Value},
};
use alloc::{collections::BTreeMap, collections::BTreeSet, vec::Vec};

/// Default capacity of the Nursery in number of objects.
pub const NURSERY_OBJECT_CAPACITY: usize = 1024;

/// Maximum number of entries representable in one Nursery semispace.
pub const MAX_NURSERY_ENTRIES: usize = 1 << 11;

/// Number of survived minor collections before promotion.
pub const PROMOTION_AGE: u8 = 2;

/// Allocation or reference failure in the generational heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeapError {
    /// The Nursery must be collected before another object can be allocated.
    NurseryFull,
    /// A generation-tagged reference does not address a live entry.
    InvalidReference,
    /// An object's `[[Prototype]]` must be another object or null.
    InvalidPrototype,
    /// A `[[Prototype]]` update would create or preserve a cycle.
    PrototypeCycle,
    /// A generation-local index cannot be represented in the handle payload.
    ReferenceSpaceExhausted,
    /// String arena operation failed.
    String(StringError),
}

impl From<StringError> for HeapError {
    fn from(error: StringError) -> Self {
        Self::String(error)
    }
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
    generation: u32,
}

impl Nursery {
    fn new(capacity: usize, generation: u32) -> Self {
        Self {
            objects: Vec::with_capacity(capacity),
            elements: Vec::with_capacity(capacity),
            object_capacity: capacity,
            generation,
        }
    }

    const fn is_full(&self) -> bool {
        self.objects.len() >= self.object_capacity
    }
}

struct OldEntry<T> {
    generation: u8,
    value: Option<T>,
}

/// Long-lived Old Generation heap space.
#[derive(Default)]
struct OldGeneration {
    objects: Vec<OldEntry<JSObject>>,
    elements: Vec<OldEntry<ElementsKind>>,
    free_objects: Vec<usize>,
    free_elements: Vec<usize>,
}

impl OldGeneration {
    fn allocate_object(&mut self, object: JSObject) -> Result<ObjectRef, HeapError> {
        let (index, generation) = if let Some(index) = self.free_objects.pop() {
            let entry = self
                .objects
                .get_mut(index)
                .ok_or(HeapError::InvalidReference)?;
            entry.value = Some(object);
            (index, entry.generation)
        } else {
            let index = self.objects.len();
            self.objects.push(OldEntry {
                generation: 0,
                value: Some(object),
            });
            (index, 0)
        };
        generation_index(index).map(|index| ObjectRef::old(index, generation))
    }

    fn allocate_elements(&mut self, elements: ElementsKind) -> Result<ElementsRef, HeapError> {
        let (index, generation) = if let Some(index) = self.free_elements.pop() {
            let entry = self
                .elements
                .get_mut(index)
                .ok_or(HeapError::InvalidReference)?;
            entry.value = Some(elements);
            (index, entry.generation)
        } else {
            let index = self.elements.len();
            self.elements.push(OldEntry {
                generation: 0,
                value: Some(elements),
            });
            (index, 0)
        };
        generation_index(index).map(|index| ElementsRef::old(index, generation))
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

/// Statistics for one completed Old Generation mark-sweep collection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MajorCollectionStats {
    /// Reachable Old Generation objects retained by marking.
    pub marked_objects: usize,
    /// Unreachable Old Generation objects reclaimed by sweeping.
    pub reclaimed_objects: usize,
    /// Reachable Old Generation elements stores retained by marking.
    pub marked_elements: usize,
    /// Unreachable Old Generation elements stores reclaimed by sweeping.
    pub reclaimed_elements: usize,
    /// Reachable or interned heap strings retained by marking.
    pub marked_strings: usize,
    /// Unreachable heap strings reclaimed by sweeping.
    pub reclaimed_strings: usize,
}

/// Resolved named-property location and value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamedProperty {
    /// Shape observed on the original receiver.
    pub receiver_shape: ShapeId,
    /// Number of Prototype Chain edges from receiver to holder.
    pub holder_depth: u16,
    /// Property slot in the holder.
    pub slot: u32,
    /// Property value at lookup time.
    pub value: Value,
    /// Prototype-validity epoch at lookup time.
    pub prototype_epoch: Option<u64>,
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
    remembered_objects: BTreeSet<ObjectRef>,
    remembered_elements: BTreeSet<ElementsRef>,
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
            nursery: Nursery::new(capacity.clamp(1, MAX_NURSERY_ENTRIES), 0),
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
        let index = young_index(self.nursery.objects.len())?;
        self.nursery.objects.push(YoungObject {
            value: JSObject::new(shape_id, prototype),
            age: 0,
        });
        Ok(ObjectRef::young(index, self.nursery.generation))
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
        let elements_index = young_index(self.nursery.elements.len())?;
        self.nursery.elements.push(YoungElements {
            value: ElementsKind::new_packed_smi(16.min(length as usize)),
            age: 0,
        });
        let elements = ElementsRef::young(elements_index, self.nursery.generation);

        let object_index = young_index(self.nursery.objects.len())?;
        self.nursery.objects.push(YoungObject {
            value: JSObject::new_array(self.shapes.root_shape(), VALUE_NULL, elements, length),
            age: 0,
        });
        Ok(ObjectRef::young(object_index, self.nursery.generation))
    }

    /// Reads an immutable object reference from its tagged generation.
    #[must_use]
    pub fn get_object(&self, reference: ObjectRef) -> Option<&JSObject> {
        let index = reference.index() as usize;
        if reference.is_old() {
            let entry = self.old_gen.objects.get(index)?;
            (u32::from(entry.generation) == reference.generation())
                .then_some(entry.value.as_ref())
                .flatten()
        } else {
            (self.nursery.generation == reference.generation())
                .then(|| self.nursery.objects.get(index).map(|entry| &entry.value))
                .flatten()
        }
    }

    /// Reads an immutable elements store from its tagged generation.
    #[must_use]
    pub fn get_elements(&self, reference: ElementsRef) -> Option<&ElementsKind> {
        let index = reference.index() as usize;
        if reference.is_old() {
            let entry = self.old_gen.elements.get(index)?;
            (u32::from(entry.generation) == reference.generation())
                .then_some(entry.value.as_ref())
                .flatten()
        } else {
            (self.nursery.generation == reference.generation())
                .then(|| self.nursery.elements.get(index).map(|entry| &entry.value))
                .flatten()
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
        self.shapes.invalidate_prototypes();
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
        if !prototype.is_null() && !prototype.is_object() {
            return Err(HeapError::InvalidPrototype);
        }
        self.get_object(reference)
            .ok_or(HeapError::InvalidReference)?;
        if let Some(mut current) = prototype.as_object() {
            let mut visited = BTreeSet::new();
            loop {
                if current == reference || !visited.insert(current) {
                    return Err(HeapError::PrototypeCycle);
                }
                let next = self
                    .get_object(current)
                    .ok_or(HeapError::InvalidReference)?
                    .prototype;
                if next.is_null() {
                    break;
                }
                current = next.as_object().ok_or(HeapError::InvalidPrototype)?;
            }
        }
        self.remember_object_store(reference, prototype);
        self.object_mut(reference)?.prototype = prototype;
        self.shapes.invalidate_prototypes();
        Ok(())
    }

    /// Resolves a named property through the Prototype Chain.
    ///
    /// # Errors
    ///
    /// Returns an error for stale references, malformed Prototype values, a
    /// cycle, or a chain deeper than the cache representation.
    pub fn lookup_named(
        &self,
        receiver: ObjectRef,
        name: StringRef,
    ) -> Result<Option<NamedProperty>, HeapError> {
        let receiver_shape = self
            .get_object(receiver)
            .ok_or(HeapError::InvalidReference)?
            .shape_id;
        let mut current = receiver;
        let mut depth = 0u16;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current) {
                return Err(HeapError::PrototypeCycle);
            }
            let object = self
                .get_object(current)
                .ok_or(HeapError::InvalidReference)?;
            if let Some(location) = self.shapes.lookup(object.shape_id, name) {
                return Ok(Some(NamedProperty {
                    receiver_shape,
                    holder_depth: depth,
                    slot: location.slot_offset,
                    value: object
                        .get_slot(location.slot_offset)
                        .unwrap_or(VALUE_UNDEFINED),
                    prototype_epoch: self.shapes.prototype_epoch(),
                }));
            }
            if object.prototype.is_null() {
                return Ok(None);
            }
            current = object
                .prototype
                .as_object()
                .ok_or(HeapError::InvalidPrototype)?;
            depth = depth
                .checked_add(1)
                .ok_or(HeapError::ReferenceSpaceExhausted)?;
        }
    }

    /// Loads a previously resolved named-property cache case.
    ///
    /// A mismatching receiver Shape or Prototype epoch is a normal cache miss.
    /// Invalid references along an otherwise matching chain are heap errors.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale object or malformed Prototype value.
    pub fn load_cached_named(
        &self,
        receiver: ObjectRef,
        receiver_shape: ShapeId,
        holder_depth: u16,
        slot: u32,
        prototype_epoch: u64,
    ) -> Result<Option<Value>, HeapError> {
        let object = self
            .get_object(receiver)
            .ok_or(HeapError::InvalidReference)?;
        if object.shape_id != receiver_shape
            || self.shapes.prototype_epoch() != Some(prototype_epoch)
        {
            return Ok(None);
        }
        let mut current = receiver;
        for _ in 0..holder_depth {
            let prototype = self
                .get_object(current)
                .ok_or(HeapError::InvalidReference)?
                .prototype;
            current = prototype.as_object().ok_or(HeapError::InvalidPrototype)?;
        }
        Ok(self
            .get_object(current)
            .and_then(|holder| holder.get_slot(slot)))
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
        let next_generation = self
            .nursery
            .generation
            .checked_add(1)
            .filter(|generation| *generation < (1 << 20))
            .ok_or(HeapError::ReferenceSpaceExhausted)?;
        let from = core::mem::replace(&mut self.nursery, Nursery::new(capacity, next_generation));
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

    /// Runs a precise Old Generation mark-sweep collection using native roots.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] if the reachable graph contains
    /// an invalid generation-tagged reference.
    pub fn collect_old(&mut self) -> Result<MajorCollectionStats, HeapError> {
        self.collect_old_with_roots(&[], VALUE_NULL)
    }

    /// Runs a precise Old Generation mark-sweep collection with VM roots.
    ///
    /// The marker traverses reachable Young entries as bridges into the Old
    /// Generation, but only Old entries are swept. Old references are stable,
    /// so register and root values do not require forwarding updates.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] if the reachable graph contains
    /// an invalid generation-tagged reference.
    pub fn collect_old_with_roots(
        &mut self,
        registers: &[Value],
        accumulator: Value,
    ) -> Result<MajorCollectionStats, HeapError> {
        let (marked_objects, marked_elements, marked_strings) = {
            let mut marker = MajorMarker::new(&self.nursery, &self.old_gen);
            for value in self.roots.iter().chain(registers) {
                marker.mark_value(*value);
            }
            marker.mark_value(accumulator);
            marker.drain()?;
            (
                marker.marked_objects,
                marker.marked_elements,
                marker.marked_strings,
            )
        };

        let mut stats = MajorCollectionStats {
            marked_objects: marked_objects.len(),
            marked_elements: marked_elements.len(),
            ..MajorCollectionStats::default()
        };
        for (index, entry) in self.old_gen.objects.iter_mut().enumerate() {
            let index_u32 = generation_index(index)?;
            if entry.value.is_some() && !marked_objects.contains(&index_u32) {
                entry.value = None;
                stats.reclaimed_objects = stats.reclaimed_objects.saturating_add(1);
                if let Some(generation) = entry.generation.checked_add(1) {
                    entry.generation = generation;
                    self.old_gen.free_objects.push(index);
                }
            }
        }
        for (index, entry) in self.old_gen.elements.iter_mut().enumerate() {
            let index_u32 = generation_index(index)?;
            if entry.value.is_some() && !marked_elements.contains(&index_u32) {
                entry.value = None;
                stats.reclaimed_elements = stats.reclaimed_elements.saturating_add(1);
                if let Some(generation) = entry.generation.checked_add(1) {
                    entry.generation = generation;
                    self.old_gen.free_elements.push(index);
                }
            }
        }
        let string_roots: Vec<_> = marked_strings
            .iter()
            .copied()
            .map(Value::from_string)
            .collect();
        let string_stats = self.strings.collect(&string_roots)?;
        stats.marked_strings = string_stats.marked;
        stats.reclaimed_strings = string_stats.reclaimed;
        self.rebuild_remembered_sets();
        Ok(stats)
    }

    fn object_mut(&mut self, reference: ObjectRef) -> Result<&mut JSObject, HeapError> {
        let index = reference.index() as usize;
        if reference.is_old() {
            let entry = self
                .old_gen
                .objects
                .get_mut(index)
                .ok_or(HeapError::InvalidReference)?;
            if u32::from(entry.generation) != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
            entry.value.as_mut().ok_or(HeapError::InvalidReference)
        } else {
            if self.nursery.generation != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
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
            let entry = self
                .old_gen
                .elements
                .get_mut(index)
                .ok_or(HeapError::InvalidReference)?;
            if u32::from(entry.generation) != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
            entry.value.as_mut().ok_or(HeapError::InvalidReference)
        } else {
            if self.nursery.generation != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
            self.nursery
                .elements
                .get_mut(index)
                .map(|entry| &mut entry.value)
                .ok_or(HeapError::InvalidReference)
        }
    }

    fn remember_object_store(&mut self, owner: ObjectRef, value: Value) {
        if owner.is_old() && value.as_object().is_some_and(ObjectRef::is_young) {
            self.remembered_objects.insert(owner);
        }
    }

    fn remember_elements_store(&mut self, owner: ElementsRef, value: Value) {
        if owner.is_old() && value.as_object().is_some_and(ObjectRef::is_young) {
            self.remembered_elements.insert(owner);
        }
    }

    fn rebuild_remembered_sets(&mut self) {
        self.remembered_objects.clear();
        self.remembered_elements.clear();
        for (index, object) in self.old_gen.objects.iter().enumerate() {
            if object.value.as_ref().is_some_and(object_contains_young)
                && let Ok(index) = generation_index(index)
            {
                self.remembered_objects
                    .insert(ObjectRef::old(index, object.generation));
            }
        }
        for (index, elements) in self.old_gen.elements.iter().enumerate() {
            if elements.value.as_ref().is_some_and(elements_contain_young)
                && let Ok(index) = generation_index(index)
            {
                self.remembered_elements
                    .insert(ElementsRef::old(index, elements.generation));
            }
        }
    }
}

struct MajorMarker<'heap> {
    nursery: &'heap Nursery,
    old: &'heap OldGeneration,
    marked_objects: BTreeSet<u32>,
    marked_elements: BTreeSet<u32>,
    marked_strings: BTreeSet<StringRef>,
    visited_young_objects: BTreeSet<u32>,
    visited_young_elements: BTreeSet<u32>,
    work: Vec<Work>,
}

impl<'heap> MajorMarker<'heap> {
    const fn new(nursery: &'heap Nursery, old: &'heap OldGeneration) -> Self {
        Self {
            nursery,
            old,
            marked_objects: BTreeSet::new(),
            marked_elements: BTreeSet::new(),
            marked_strings: BTreeSet::new(),
            visited_young_objects: BTreeSet::new(),
            visited_young_elements: BTreeSet::new(),
            work: Vec::new(),
        }
    }

    fn mark_value(&mut self, value: Value) {
        if let Some(reference) = value.as_object() {
            self.work.push(Work::Object(reference));
        }
        if let Some(reference) = value.as_heap_string() {
            self.marked_strings.insert(reference);
        }
    }

    fn drain(&mut self) -> Result<(), HeapError> {
        while let Some(work) = self.work.pop() {
            match work {
                Work::Object(reference) => self.mark_object(reference)?,
                Work::Elements(reference) => self.mark_elements(reference)?,
                Work::String(reference) => {
                    self.marked_strings.insert(reference);
                }
            }
        }
        Ok(())
    }

    fn mark_object(&mut self, reference: ObjectRef) -> Result<(), HeapError> {
        if reference.is_old() {
            let entry = self
                .old
                .objects
                .get(reference.index() as usize)
                .ok_or(HeapError::InvalidReference)?;
            if u32::from(entry.generation) != reference.generation() || entry.value.is_none() {
                return Err(HeapError::InvalidReference);
            }
        } else if self.nursery.generation != reference.generation()
            || self
                .nursery
                .objects
                .get(reference.index() as usize)
                .is_none()
        {
            return Err(HeapError::InvalidReference);
        }
        let first_visit = if reference.is_old() {
            self.marked_objects.insert(reference.index())
        } else {
            self.visited_young_objects.insert(reference.index())
        };
        if !first_visit {
            return Ok(());
        }
        let object = if reference.is_old() {
            let entry = self
                .old
                .objects
                .get(reference.index() as usize)
                .ok_or(HeapError::InvalidReference)?;
            if u32::from(entry.generation) != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
            entry.value.as_ref().ok_or(HeapError::InvalidReference)?
        } else {
            if self.nursery.generation != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
            &self
                .nursery
                .objects
                .get(reference.index() as usize)
                .ok_or(HeapError::InvalidReference)?
                .value
        };
        trace_object_work(object, &mut self.work);
        Ok(())
    }

    fn mark_elements(&mut self, reference: ElementsRef) -> Result<(), HeapError> {
        if reference.is_old() {
            let entry = self
                .old
                .elements
                .get(reference.index() as usize)
                .ok_or(HeapError::InvalidReference)?;
            if u32::from(entry.generation) != reference.generation() || entry.value.is_none() {
                return Err(HeapError::InvalidReference);
            }
        } else if self.nursery.generation != reference.generation()
            || self
                .nursery
                .elements
                .get(reference.index() as usize)
                .is_none()
        {
            return Err(HeapError::InvalidReference);
        }
        let first_visit = if reference.is_old() {
            self.marked_elements.insert(reference.index())
        } else {
            self.visited_young_elements.insert(reference.index())
        };
        if !first_visit {
            return Ok(());
        }
        let elements = if reference.is_old() {
            let entry = self
                .old
                .elements
                .get(reference.index() as usize)
                .ok_or(HeapError::InvalidReference)?;
            if u32::from(entry.generation) != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
            entry.value.as_ref().ok_or(HeapError::InvalidReference)
        } else {
            if self.nursery.generation != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
            self.nursery
                .elements
                .get(reference.index() as usize)
                .map(|entry| &entry.value)
                .ok_or(HeapError::InvalidReference)
        }?;
        trace_elements_work(elements, &mut self.work);
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Work {
    Object(ObjectRef),
    Elements(ElementsRef),
    String(StringRef),
}

struct Evacuator<'heap> {
    from: Nursery,
    old: &'heap mut OldGeneration,
    to: &'heap mut Nursery,
    object_forwarding: BTreeMap<ObjectRef, ObjectRef>,
    elements_forwarding: BTreeMap<ElementsRef, ElementsRef>,
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
            self.object(reference)?;
            return Ok(reference);
        }
        if reference.generation() != self.from.generation {
            return Err(HeapError::InvalidReference);
        }
        if let Some(forwarded) = self.object_forwarding.get(&reference) {
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
            ObjectRef::young(index, self.to.generation)
        };
        self.object_forwarding.insert(reference, forwarded);
        self.work.push(Work::Object(forwarded));
        Ok(forwarded)
    }

    fn evacuate_elements(&mut self, reference: ElementsRef) -> Result<ElementsRef, HeapError> {
        if reference.is_old() {
            self.elements(reference)?;
            return Ok(reference);
        }
        if reference.generation() != self.from.generation {
            return Err(HeapError::InvalidReference);
        }
        if let Some(forwarded) = self.elements_forwarding.get(&reference) {
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
            ElementsRef::young(index, self.to.generation)
        };
        self.elements_forwarding.insert(reference, forwarded);
        self.work.push(Work::Elements(forwarded));
        Ok(forwarded)
    }

    fn enqueue_old_object(&mut self, reference: ObjectRef) {
        self.work.push(Work::Object(reference));
    }

    fn enqueue_old_elements(&mut self, reference: ElementsRef) {
        self.work.push(Work::Elements(reference));
    }

    fn drain(&mut self) -> Result<(), HeapError> {
        while let Some(work) = self.work.pop() {
            match work {
                Work::Object(reference) => self.scan_object(reference)?,
                Work::Elements(reference) => self.scan_elements(reference)?,
                Work::String(_) => {}
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
            let entry = self
                .old
                .objects
                .get(index)
                .ok_or(HeapError::InvalidReference)?;
            if u32::from(entry.generation) != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
            entry.value.as_ref().ok_or(HeapError::InvalidReference)
        } else {
            if self.to.generation != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
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
            let entry = self
                .old
                .objects
                .get_mut(index)
                .ok_or(HeapError::InvalidReference)?;
            if u32::from(entry.generation) != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
            entry.value.as_mut().ok_or(HeapError::InvalidReference)
        } else {
            if self.to.generation != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
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
            let entry = self
                .old
                .elements
                .get(index)
                .ok_or(HeapError::InvalidReference)?;
            if u32::from(entry.generation) != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
            entry.value.as_ref().ok_or(HeapError::InvalidReference)
        } else {
            if self.to.generation != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
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
            let entry = self
                .old
                .elements
                .get_mut(index)
                .ok_or(HeapError::InvalidReference)?;
            if u32::from(entry.generation) != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
            entry.value.as_mut().ok_or(HeapError::InvalidReference)
        } else {
            if self.to.generation != reference.generation() {
                return Err(HeapError::InvalidReference);
            }
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
        .filter(|index| *index < (1 << 23))
        .ok_or(HeapError::ReferenceSpaceExhausted)
}

fn young_index(index: usize) -> Result<u32, HeapError> {
    u32::try_from(index)
        .ok()
        .filter(|index| *index < (1 << 11))
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

fn trace_object_work(object: &JSObject, work: &mut Vec<Work>) {
    push_value_work(work, object.prototype);
    for value in object.in_object_slots {
        push_value_work(work, value);
    }
    if let Some(slots) = &object.out_of_line_slots {
        for value in slots {
            push_value_work(work, *value);
        }
    }
    if let ObjectKind::StringWrapper(value) = &object.kind {
        push_value_work(work, *value);
    }
    if let Some(elements) = object.elements {
        work.push(Work::Elements(elements));
    }
}

fn trace_elements_work(elements: &ElementsKind, work: &mut Vec<Work>) {
    match elements {
        ElementsKind::PackedValues(values) => {
            for value in values {
                push_value_work(work, *value);
            }
        }
        ElementsKind::Holey(values) => {
            for value in values.iter().flatten() {
                push_value_work(work, *value);
            }
        }
        ElementsKind::Dictionary(values) => {
            for value in values.values() {
                push_value_work(work, *value);
            }
        }
        ElementsKind::PackedSmi(_) | ElementsKind::PackedDouble(_) => {}
    }
}

fn push_value_work(work: &mut Vec<Work>, value: Value) {
    if let Some(reference) = value.as_object() {
        work.push(Work::Object(reference));
    }
    if let Some(reference) = value.as_heap_string() {
        work.push(Work::String(reference));
    }
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
        assert!(heap.get_object(object).is_none());
    }

    #[test]
    fn stale_young_references_are_rejected_by_all_store_paths() {
        let mut heap = GenerationalHeap::with_nursery_capacity(2);
        let object = heap
            .allocate_object(heap.shapes.root_shape(), VALUE_NULL)
            .unwrap();
        let array = heap.allocate_array(0).unwrap();
        let elements = heap.get_object(array).unwrap().elements.unwrap();
        heap.scavenge().unwrap();

        assert!(heap.get_object(object).is_none());
        assert!(heap.get_elements(elements).is_none());
        assert_eq!(
            heap.set_object_shape(object, heap.shapes.root_shape()),
            Err(HeapError::InvalidReference)
        );
        assert_eq!(
            heap.set_object_prototype(object, VALUE_NULL),
            Err(HeapError::InvalidReference)
        );
        assert_eq!(
            heap.set_object_slot(object, 0, VALUE_NULL),
            Err(HeapError::InvalidReference)
        );
        assert_eq!(
            heap.set_element(elements, 0, VALUE_NULL),
            Err(HeapError::InvalidReference)
        );
        assert_eq!(
            heap.push_element(elements, VALUE_NULL),
            Err(HeapError::InvalidReference)
        );
        assert_eq!(
            heap.delete_element(elements, 0),
            Err(HeapError::InvalidReference)
        );
    }

    #[test]
    fn major_collection_reclaims_unreachable_promoted_cycles() {
        let mut heap = GenerationalHeap::with_nursery_capacity(4);
        let shape = heap.shapes.root_shape();
        heap.enter_scope();
        let first = heap.allocate_object(shape, VALUE_NULL).unwrap();
        let second = heap.allocate_object(shape, VALUE_NULL).unwrap();
        heap.set_object_slot(first, 0, Value::from_object(second))
            .unwrap();
        heap.set_object_slot(second, 0, Value::from_object(first))
            .unwrap();
        let root = heap.push_root(Value::from_object(first)).unwrap();
        heap.scavenge().unwrap();
        heap.scavenge().unwrap();
        let promoted_first = rooted_object(&heap, root);
        let promoted_second = heap
            .get_object(promoted_first)
            .and_then(|object| object.get_slot(0))
            .and_then(Value::as_object)
            .unwrap();
        heap.exit_scope();

        let stats = heap.collect_old().unwrap();

        assert_eq!(stats.reclaimed_objects, 2);
        assert!(heap.get_object(promoted_first).is_none());
        assert!(heap.get_object(promoted_second).is_none());
    }

    #[test]
    fn major_collection_traces_young_bridges_into_old_generation() {
        let mut heap = GenerationalHeap::with_nursery_capacity(4);
        let shape = heap.shapes.root_shape();
        heap.enter_scope();
        heap.enter_scope();
        let target = heap.allocate_object(shape, VALUE_NULL).unwrap();
        let target_root = heap.push_root(Value::from_object(target)).unwrap();
        heap.scavenge().unwrap();
        heap.scavenge().unwrap();
        let target = rooted_object(&heap, target_root);
        assert!(target.is_old());
        heap.exit_scope();

        let bridge = heap.allocate_object(shape, VALUE_NULL).unwrap();
        heap.set_object_slot(bridge, 0, Value::from_object(target))
            .unwrap();
        heap.push_root(Value::from_object(bridge)).unwrap();

        let stats = heap.collect_old().unwrap();

        assert_eq!(stats.marked_objects, 1);
        assert_eq!(stats.reclaimed_objects, 0);
        assert!(heap.get_object(target).is_some());
    }

    #[test]
    fn major_collection_does_not_trace_unreachable_young_garbage() {
        let mut heap = GenerationalHeap::with_nursery_capacity(4);
        let shape = heap.shapes.root_shape();
        heap.enter_scope();
        let target = heap.allocate_object(shape, VALUE_NULL).unwrap();
        heap.push_root(Value::from_object(target)).unwrap();
        heap.scavenge().unwrap();
        heap.scavenge().unwrap();
        let target = heap.root_value(Root(0)).unwrap().as_object().unwrap();
        heap.exit_scope();

        let garbage = heap.allocate_object(shape, VALUE_NULL).unwrap();
        heap.set_object_slot(garbage, 0, Value::from_object(target))
            .unwrap();
        let stats = heap.collect_old().unwrap();

        assert_eq!(stats.reclaimed_objects, 1);
        assert!(heap.get_object(target).is_none());
    }

    #[test]
    fn major_collection_marks_elements_and_reuses_swept_storage() {
        let mut heap = GenerationalHeap::with_nursery_capacity(3);
        heap.enter_scope();
        let array = heap.allocate_array(0).unwrap();
        let root = heap.push_root(Value::from_object(array)).unwrap();
        heap.scavenge().unwrap();
        heap.scavenge().unwrap();
        let old_array = rooted_object(&heap, root);
        let old_elements = heap.get_object(old_array).unwrap().elements.unwrap();

        let retained = heap.collect_old().unwrap();
        assert_eq!(retained.marked_objects, 1);
        assert_eq!(retained.marked_elements, 1);
        heap.exit_scope();
        let reclaimed = heap.collect_old().unwrap();
        assert_eq!(reclaimed.reclaimed_objects, 1);
        assert_eq!(reclaimed.reclaimed_elements, 1);

        heap.enter_scope();
        let replacement = heap.allocate_array(0).unwrap();
        heap.push_root(Value::from_object(replacement)).unwrap();
        heap.scavenge().unwrap();
        heap.scavenge().unwrap();
        let replacement = heap.root_value(Root(0)).unwrap().as_object().unwrap();
        let replacement_elements = heap.get_object(replacement).unwrap().elements.unwrap();
        assert_eq!(replacement.index(), old_array.index());
        assert_ne!(replacement.generation(), old_array.generation());
        assert_eq!(replacement_elements.index(), old_elements.index());
        assert_ne!(replacement_elements.generation(), old_elements.generation());
        assert!(heap.get_object(old_array).is_none());
        assert!(heap.get_elements(old_elements).is_none());
        assert_eq!(
            heap.set_object_slot(old_array, 0, VALUE_NULL),
            Err(HeapError::InvalidReference)
        );
        assert_eq!(
            heap.push_element(old_elements, VALUE_NULL),
            Err(HeapError::InvalidReference)
        );
    }

    #[test]
    fn nursery_capacity_is_clamped_to_the_handle_index_space() {
        let minimum = GenerationalHeap::with_nursery_capacity(0);
        assert_eq!(minimum.nursery.object_capacity, 1);
        let maximum = GenerationalHeap::with_nursery_capacity(usize::MAX);
        assert_eq!(maximum.nursery.object_capacity, MAX_NURSERY_ENTRIES);
    }

    #[test]
    fn major_collection_traces_strings_through_objects_and_ropes() {
        let mut heap = GenerationalHeap::with_nursery_capacity(2);
        heap.enter_scope();
        let left = heap.strings.allocate_str("left").unwrap();
        let right = heap.strings.allocate_str("right").unwrap();
        let garbage = heap.strings.allocate_str("garbage").unwrap();
        let rope = heap
            .strings
            .allocate_cons(Value::from_string(left), Value::from_string(right))
            .unwrap();
        let object = heap
            .allocate_object(heap.shapes.root_shape(), VALUE_NULL)
            .unwrap();
        heap.set_object_slot(object, 0, Value::from_string(rope))
            .unwrap();
        heap.push_root(Value::from_object(object)).unwrap();

        let stats = heap.collect_old().unwrap();

        assert_eq!(stats.marked_strings, 3);
        assert_eq!(stats.reclaimed_strings, 1);
        assert_eq!(
            heap.strings
                .to_rust_string(Value::from_string(rope))
                .as_deref(),
            Some("leftright")
        );
        assert_eq!(
            heap.strings.to_rust_string(Value::from_string(garbage)),
            None
        );
    }
}
