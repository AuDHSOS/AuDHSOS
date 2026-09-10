// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "Shape indices are bounded and fit in u32/usize"
)]

//! Hidden Classes (Shapes) and Transition Trees.
//!
//! An object's layout is factored out into a shared descriptor (Shape).
//! Property lookup is an offset calculation, not a hash/tree probe.
//! Objects with identical property insertion sequences share the same Shape.

use super::value::StringRef;
use alloc::{collections::BTreeMap, vec::Vec};

/// Unique identifier of a Shape in the `ShapeTable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShapeId(pub u32);

/// ECMAScript property attribute flags.
#[expect(
    clippy::struct_excessive_bools,
    reason = "ECMAScript property attributes: writable, enumerable, configurable, accessor"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PropertyFlags {
    /// Property is writable.
    pub writable: bool,
    /// Property is enumerable.
    pub enumerable: bool,
    /// Property is configurable.
    pub configurable: bool,
    /// Property is an accessor pair (getter/setter).
    pub is_accessor: bool,
}

impl PropertyFlags {
    /// Standard ordinary data property attributes (writable, enumerable, configurable).
    #[must_use]
    pub const fn ordinary_data() -> Self {
        Self {
            writable: true,
            enumerable: true,
            configurable: true,
            is_accessor: false,
        }
    }
}

/// Descriptor for a single property inside a Shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyLocation {
    /// Offset in the object's in-object or out-of-line slot array.
    pub slot_offset: u32,
    /// Attribute flags.
    pub flags: PropertyFlags,
}

/// Single node in the Shape transition graph.
#[derive(Clone, Debug)]
pub struct Shape {
    /// Parent shape from which this shape transitioned.
    pub parent: Option<ShapeId>,
    /// Property name introduced in this transition.
    pub property_name: Option<StringRef>,
    /// Property attributes.
    pub flags: PropertyFlags,
    /// Slot index assigned to this property.
    pub slot_offset: u32,
    /// Total number of own properties described by this shape.
    pub property_count: u32,
    /// Outgoing transitions: property name -> next shape.
    pub transitions: BTreeMap<StringRef, ShapeId>,
    /// Validity cell counter for prototype invalidation.
    pub validity_epoch: u32,
}

/// Shape storage, transition tree and validity manager.
pub struct ShapeTable {
    shapes: Vec<Shape>,
    root_shape: ShapeId,
    global_validity_epoch: u32,
}

impl Default for ShapeTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ShapeTable {
    /// Creates a new Shape table with a unique empty root shape.
    #[must_use]
    pub fn new() -> Self {
        let mut shapes = Vec::with_capacity(64);
        shapes.push(Shape {
            parent: None,
            property_name: None,
            flags: PropertyFlags::ordinary_data(),
            slot_offset: 0,
            property_count: 0,
            transitions: BTreeMap::new(),
            validity_epoch: 0,
        });
        Self {
            shapes,
            root_shape: ShapeId(0),
            global_validity_epoch: 0,
        }
    }

    /// Returns the root shape for newly created empty objects `{}`.
    #[must_use]
    pub const fn root_shape(&self) -> ShapeId {
        self.root_shape
    }

    /// Transitions `current` shape by adding `name` with `flags`.
    /// Returns the resulting `ShapeId` and the allocated `slot_offset`.
    pub fn transition(
        &mut self,
        current: ShapeId,
        name: StringRef,
        flags: PropertyFlags,
    ) -> (ShapeId, u32) {
        if let Some(shape) = self.shapes.get(current.0 as usize)
            && let Some(&existing) = shape.transitions.get(&name)
            && let Some(target) = self.shapes.get(existing.0 as usize)
            && target.flags == flags
        {
            return (existing, target.slot_offset);
        }

        let (slot_offset, property_count) =
            self.shapes.get(current.0 as usize).map_or((0, 0), |s| {
                (s.property_count, s.property_count.saturating_add(1))
            });

        let new_id = self.shapes.len();
        #[expect(clippy::as_conversions, reason = "shape count fits in u32")]
        let new_shape_id = ShapeId(new_id as u32);

        self.shapes.push(Shape {
            parent: Some(current),
            property_name: Some(name),
            flags,
            slot_offset,
            property_count,
            transitions: BTreeMap::new(),
            validity_epoch: self.global_validity_epoch,
        });

        if let Some(parent_shape) = self.shapes.get_mut(current.0 as usize) {
            parent_shape.transitions.insert(name, new_shape_id);
        }

        (new_shape_id, slot_offset)
    }

    /// Searches for `name` along the shape transition chain.
    #[must_use]
    pub fn lookup(&self, shape_id: ShapeId, name: StringRef) -> Option<PropertyLocation> {
        let mut curr = shape_id;
        loop {
            let shape = self.shapes.get(curr.0 as usize)?;
            if shape.property_name == Some(name) {
                return Some(PropertyLocation {
                    slot_offset: shape.slot_offset,
                    flags: shape.flags,
                });
            }
            curr = shape.parent?;
        }
    }

    /// Returns the property count for a given shape.
    #[must_use]
    pub fn property_count(&self, shape_id: ShapeId) -> u32 {
        self.shapes
            .get(shape_id.0 as usize)
            .map_or(0, |s| s.property_count)
    }

    /// Invalidates all cached prototype assumptions by incrementing the epoch.
    pub const fn invalidate_prototypes(&mut self) {
        self.global_validity_epoch = self.global_validity_epoch.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_transition_and_reuse() {
        let mut table = ShapeTable::new();
        let root = table.root_shape();

        let prop_x = StringRef(1);
        let prop_y = StringRef(2);

        let (shape1, offset1) = table.transition(root, prop_x, PropertyFlags::ordinary_data());
        assert_eq!(offset1, 0);
        assert_ne!(shape1, root);

        let (shape2, offset2) = table.transition(shape1, prop_y, PropertyFlags::ordinary_data());
        assert_eq!(offset2, 1);
        assert_ne!(shape2, shape1);

        // Another object transitioning along the same path arrives at the exact same shapes
        let (shape1_again, _) = table.transition(root, prop_x, PropertyFlags::ordinary_data());
        assert_eq!(shape1_again, shape1);

        let (shape2_again, _) =
            table.transition(shape1_again, prop_y, PropertyFlags::ordinary_data());
        assert_eq!(shape2_again, shape2);

        // Lookups along shape chain
        let loc_x = table.lookup(shape2, prop_x).unwrap();
        assert_eq!(loc_x.slot_offset, 0);

        let loc_y = table.lookup(shape2, prop_y).unwrap();
        assert_eq!(loc_y.slot_offset, 1);

        let loc_z = table.lookup(shape2, StringRef(3));
        assert!(loc_z.is_none());
    }
}
