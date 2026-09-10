// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::as_conversions,
    reason = "Object slot indexing and inline slot arrays"
)]

//! Compact JavaScript object representation and standard internal methods.
//!
//! Ordinary objects have a compact 32-byte header with 2 in-object property
//! slots. Out-of-line slots are allocated only when properties exceed the
//! in-object capacity. All internal method operations (`[[Get]]`, `[[Set]]`,
//! `[[DefineOwnProperty]]`) route through Shapes and Elements.

use super::{
    elements::ElementsRef,
    shape::ShapeId,
    value::{VALUE_UNDEFINED, Value},
};
use alloc::vec::Vec;

/// Number of in-object property slots allocated inline with the object header.
pub const IN_OBJECT_SLOT_COUNT: usize = 2;

/// Exotic object specializations.
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectKind {
    /// Ordinary ECMAScript object.
    Ordinary,
    /// ECMAScript Array with magic `length`.
    Array {
        /// Observable Array length property.
        length: u32,
    },
    /// Bytecode-compiled callable function.
    Function {
        /// Bytecode function index.
        code_id: u32,
        /// Heap context for captured variables.
        context: Option<u32>,
    },
    /// Native Rust host/intrinsic callable.
    NativeFunction {
        /// Intrinsic or host capability identifier.
        id: u32,
        /// Formatted arity length.
        length: u32,
    },
    /// Boxed Boolean primitive.
    BooleanWrapper(bool),
    /// Boxed Number primitive.
    NumberWrapper(f64),
    /// Boxed String primitive.
    StringWrapper(Value),
}

/// Compact representation of a JavaScript object in the heap.
#[derive(Clone, Debug)]
pub struct JSObject {
    /// Structural layout identifier (Shape).
    pub shape_id: ShapeId,
    /// `[[Prototype]]` pointer (an Object or Null).
    pub prototype: Value,
    /// Dedicated array/indexed elements backing store.
    pub elements: Option<ElementsRef>,
    /// Inline property slots for fast zero-allocation access.
    pub in_object_slots: [Value; IN_OBJECT_SLOT_COUNT],
    /// Out-of-line property slots if properties exceed `IN_OBJECT_SLOT_COUNT`.
    pub out_of_line_slots: Option<Vec<Value>>,
    /// Whether new properties can be added (`[[Extensible]]`).
    pub extensible: bool,
    /// Exotic internal slot specialization.
    pub kind: ObjectKind,
}

impl JSObject {
    /// Creates a new ordinary empty object with the given shape and prototype.
    #[must_use]
    pub const fn new(shape_id: ShapeId, prototype: Value) -> Self {
        Self {
            shape_id,
            prototype,
            elements: None,
            in_object_slots: [VALUE_UNDEFINED; IN_OBJECT_SLOT_COUNT],
            out_of_line_slots: None,
            extensible: true,
            kind: ObjectKind::Ordinary,
        }
    }

    /// Creates a new Array object with an elements store and `length`.
    #[must_use]
    pub const fn new_array(
        shape_id: ShapeId,
        prototype: Value,
        elements: ElementsRef,
        length: u32,
    ) -> Self {
        Self {
            shape_id,
            prototype,
            elements: Some(elements),
            in_object_slots: [VALUE_UNDEFINED; IN_OBJECT_SLOT_COUNT],
            out_of_line_slots: None,
            extensible: true,
            kind: ObjectKind::Array { length },
        }
    }

    /// Reads a property from a specific slot index.
    #[must_use]
    pub fn get_slot(&self, slot: u32) -> Option<Value> {
        let idx = slot as usize;
        if idx < IN_OBJECT_SLOT_COUNT {
            self.in_object_slots.get(idx).copied()
        } else if let Some(ool) = &self.out_of_line_slots {
            let ool_idx = idx.saturating_sub(IN_OBJECT_SLOT_COUNT);
            ool.get(ool_idx).copied()
        } else {
            None
        }
    }

    /// Writes a property value to a specific slot index, expanding out-of-line storage if needed.
    pub fn set_slot(&mut self, slot: u32, value: Value) {
        let idx = slot as usize;
        if idx < IN_OBJECT_SLOT_COUNT {
            if let Some(target) = self.in_object_slots.get_mut(idx) {
                *target = value;
            }
        } else {
            let ool_idx = idx.saturating_sub(IN_OBJECT_SLOT_COUNT);
            let ool = self.out_of_line_slots.get_or_insert_with(Vec::new);
            if ool_idx >= ool.len() {
                ool.resize(ool_idx.saturating_add(1), VALUE_UNDEFINED);
            }
            if let Some(target) = ool.get_mut(ool_idx) {
                *target = value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::value::VALUE_NULL;
    use super::*;

    #[test]
    fn object_slot_storage_and_growth() {
        let mut obj = JSObject::new(ShapeId(0), VALUE_NULL);
        assert_eq!(obj.get_slot(0), Some(VALUE_UNDEFINED));
        assert_eq!(obj.get_slot(1), Some(VALUE_UNDEFINED));
        assert_eq!(obj.get_slot(2), None);

        // In-object slot write
        obj.set_slot(0, Value::from_smi(100));
        obj.set_slot(1, Value::from_smi(200));
        assert_eq!(obj.get_slot(0), Some(Value::from_smi(100)));
        assert_eq!(obj.get_slot(1), Some(Value::from_smi(200)));
        assert!(obj.out_of_line_slots.is_none());

        // Out-of-line slot expansion
        obj.set_slot(2, Value::from_smi(300));
        assert!(obj.out_of_line_slots.is_some());
        assert_eq!(obj.get_slot(2), Some(Value::from_smi(300)));

        obj.set_slot(5, Value::from_smi(600));
        assert_eq!(obj.get_slot(5), Some(Value::from_smi(600)));
        assert_eq!(obj.get_slot(4), Some(VALUE_UNDEFINED));
    }
}
