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
    context::ContextRef,
    elements::ElementsRef,
    shape::ShapeId,
    value::{VALUE_UNDEFINED, Value},
};
use alloc::vec::Vec;

/// Number of in-object property slots allocated inline with the object header.
pub const IN_OBJECT_SLOT_COUNT: usize = 2;

/// `[[ArrayLikeIterationKind]]` of 23.1.5.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrayIterationKind {
    /// 23.1.3.17: the index of each element.
    Key,
    /// 23.1.3.38: each element.
    Value,
    /// 23.1.3.4: the index and the element, in an Array of two.
    KeyAndValue,
}

/// Exotic object specializations.
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectKind {
    /// Ordinary ECMAScript object.
    Ordinary,
    /// ECMAScript Array with magic `length`.
    Array {
        /// Observable Array length property.
        length: u32,
        /// `[[Writable]]` of that property, which 10.4.2.4 can clear and
        /// nothing can set again.
        writable: bool,
    },
    /// Bytecode-compiled callable function.
    Function {
        /// The code unit this function was compiled with. A call resolves
        /// `code_id` against that unit, not against whichever Script happens to
        /// be running.
        unit: u32,
        /// Bytecode function index.
        code_id: u32,
        /// Heap context for captured variables.
        context: Option<ContextRef>,
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
    /// Error instance, the `[[ErrorData]]` slot of 20.5.4.
    Error,
    /// Array Iterator instance, the slots of 23.1.5.3.
    ArrayIterator {
        /// `[[IteratedArrayLike]]`, undefined once the iteration is done.
        target: Value,
        /// `[[ArrayLikeNextIndex]]`.
        index: u32,
        /// `[[ArrayLikeIterationKind]]`.
        kind: ArrayIterationKind,
    },
    /// The state of one call of a method of 23.1.3 that calls back into the
    /// Script (23.1.3.5, .8, .11, .12, .15, .21, .24, .25).
    ///
    /// Each element is a call, and the engine leaves the method to make it, so
    /// what the walk has reached cannot live in the frame of the caller. It
    /// lives here, where the collector traces it like any other object, and
    /// the callback takes its arguments from here too: the operation has no
    /// frame whose registers could hold them.
    ArrayIteration {
        /// The intrinsic this state belongs to.
        intrinsic: u32,
        /// `O` of the clause, which is also the third argument of every
        /// callback.
        target: Value,
        /// The callback, and the `this` value it is called with.
        callback: Value,
        /// `thisArg` of the clause.
        receiver: Value,
        /// The Array a copying method fills, or the accumulator of 23.1.3.24.
        output: Value,
        /// The element the callback that is in flight was given.
        element: Value,
        /// Its index, which is the second argument of the callback.
        element_index: u32,
        /// The index the next callback is for.
        index: u32,
        /// Where the walk stops, taken once as 23.1.3 takes it.
        length: u32,
        /// Whether an accumulator has been taken yet (23.1.3.24 step 6).
        started: bool,
        /// Whether the call in flight is the getter of an accessor element
        /// rather than the callback of the clause.
        getter: bool,
        /// Which value the walk is still converting before it begins: none,
        /// the `length` 7.1.20 reads, or the index 7.1.5 makes of the
        /// argument a scan starts from.
        converting: u8,
        /// Which method 7.1.1 has already asked for the value in flight: 0
        /// before any, 1 after the getter, 2 after `valueOf`, 3 after
        /// `toString`. The object being converted waits in `element`.
        convert_step: u8,
    },
    /// The `%Reflect%` namespace object of 28.1, which is ordinary in every
    /// way but one: a name it should own and this Realm has not built is a
    /// gap rather than the undefined an ordinary object answers.
    Reflect,
    /// The `%JSON%` namespace object of 25.5, which is ordinary in every way
    /// but one: a name it should own and this Realm has not built is a gap
    /// rather than the undefined an ordinary object answers.
    Json,
    /// The `%Math%` namespace object of 21.3, which is ordinary in every way
    /// but one: a name it should own and this Realm has not built is a gap
    /// rather than the undefined an ordinary object answers.
    Math,
    /// The `[[Get]]` and `[[Set]]` of an accessor property, 6.1.7.1.
    ///
    /// A Shape says which of its properties are accessors; the slot of one
    /// holds this pair rather than a value. Neither field is reachable from a
    /// Script, so the pair is not an object a Script can name.
    Accessor {
        /// `[[Get]]`, undefined when the property has no getter.
        get: Value,
        /// `[[Set]]`, undefined when the property has no setter.
        set: Value,
    },
    /// `RegExp` instance, the slots of 22.2.7: the pattern it was compiled
    /// from, named by the unit that holds it and its index there.
    RegExp {
        /// The code unit whose `regex_constants` hold the pattern.
        unit: u32,
        /// Index of the pattern in that unit.
        index: u32,
    },
    /// Bound function exotic object, the slots of 10.4.1.
    BoundFunction {
        /// `[[BoundTargetFunction]]`.
        target: Value,
        /// `[[BoundThis]]`.
        receiver: Value,
    },
}

impl ObjectKind {
    /// Every `Value` this kind holds.
    ///
    /// The collector follows what it can see, so a kind that holds a
    /// reference says so here and nowhere else: a new one that forgets to
    /// would leave a reference behind a scavenge.
    #[must_use]
    pub const fn values(&self) -> [Option<Value>; 5] {
        match self {
            Self::StringWrapper(value) | Self::ArrayIterator { target: value, .. } => {
                [Some(*value), None, None, None, None]
            }
            Self::BoundFunction { target, receiver }
            | Self::Accessor {
                get: target,
                set: receiver,
            } => [Some(*target), Some(*receiver), None, None, None],
            Self::ArrayIteration {
                target,
                callback,
                receiver,
                output,
                element,
                ..
            } => [
                Some(*target),
                Some(*callback),
                Some(*receiver),
                Some(*output),
                Some(*element),
            ],
            _ => [None, None, None, None, None],
        }
    }

    /// Every `Value` this kind holds, to be forwarded by a collection.
    pub const fn values_mut(&mut self) -> [Option<&mut Value>; 5] {
        match self {
            Self::StringWrapper(value) | Self::ArrayIterator { target: value, .. } => {
                [Some(value), None, None, None, None]
            }
            Self::BoundFunction { target, receiver }
            | Self::Accessor {
                get: target,
                set: receiver,
            } => [Some(target), Some(receiver), None, None, None],
            Self::ArrayIteration {
                target,
                callback,
                receiver,
                output,
                element,
                ..
            } => [
                Some(target),
                Some(callback),
                Some(receiver),
                Some(output),
                Some(element),
            ],
            _ => [None, None, None, None, None],
        }
    }
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
            kind: ObjectKind::Array {
                length,
                writable: true,
            },
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
