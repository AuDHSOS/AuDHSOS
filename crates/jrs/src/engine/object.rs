// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    reason = "Object slot indexing, inline slot arrays and the element codec of 25.1.3"
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
    value::{SymbolRef, VALUE_UNDEFINED, Value},
};
use alloc::vec::Vec;

/// Number of in-object property slots allocated inline with the object header.
pub const IN_OBJECT_SLOT_COUNT: usize = 2;

/// The compiled pattern of a `RegExp` instance (22.2.4.1).
///
/// Two instances hold the same pattern only where they hold the same `Rc`,
/// which is what the equality compares; the automaton itself has none.
#[derive(Clone, Debug)]
pub struct PatternRef(pub alloc::rc::Rc<crate::regexp::RegExp>);

impl PartialEq for PatternRef {
    fn eq(&self, other: &Self) -> bool {
        alloc::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}

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
        /// `[[HomeObject]]` of 10.2, which 13.3.7 reads the Prototype of;
        /// undefined for a function that is no method. An arrow takes the one
        /// of the function it was made in, because 15.3.4 gives it no
        /// `super` of its own.
        home: Value,
    },
    /// Native Rust host/intrinsic callable.
    NativeFunction {
        /// Intrinsic or host capability identifier.
        id: u32,
        /// Formatted arity length.
        length: u32,
        /// What a closure of the specification carries besides its code: the
        /// `[[Promise]]` and `[[AlreadyResolved]]` of 27.2.1.3.1, held as the
        /// Array the pair of resolving functions shares. Undefined for an
        /// intrinsic that closes over nothing.
        state: Value,
    },
    /// Boxed Boolean primitive.
    BooleanWrapper(bool),
    /// Boxed Number primitive.
    NumberWrapper(f64),
    /// Boxed String primitive.
    StringWrapper(Value),
    /// Boxed Symbol primitive, the `[[SymbolData]]` slot of 20.4.3.
    SymbolWrapper(SymbolRef),
    /// The arguments object of 10.4.4, which carries `[[ParameterMap]]` and so
    /// takes the builtin tag `Arguments` of 20.1.3.6.
    Arguments {
        /// The context 10.4.4.7 maps the indices onto. 10.4.4.6 builds the
        /// object of a strict function or of a parameter list that is not
        /// simple, which maps nothing and holds none.
        context: Option<ContextRef>,
        /// The slot index 0 maps to; index `i` maps to `slot + i`.
        slot: u16,
        /// One bit per index that is still mapped, index 0 in bit 0. 10.4.4.1
        /// and 10.4.4.2 clear a bit, which leaves the value in the Shape.
        mapped: u64,
    },
    /// Error instance, the `[[ErrorData]]` slot of 20.5.4.
    Error,
    /// The iterator of 24.1.5.1 and 24.2.5.1, which walks the entries of the
    /// collection it was made from.
    CollectionIterator {
        /// The collection, undefined once the walk reached the end.
        target: Value,
        /// The position of the key slot the next step reads.
        index: u32,
        /// Which of the three 24.1.5.1 answers.
        kind: ArrayIterationKind,
    },
    /// The `[[MapData]]` of 24.1.4 and the `[[SetData]]` of 24.2.4.
    Collection {
        /// The entries in the order 24.1.1.1 added them, as the Array that
        /// holds a key and a value for each of them. 24.1.3.3 writes the
        /// `empty` of the clause over a deleted key, so the position an
        /// iterator of 24.1.5 stands at does not move.
        entries: Value,
        /// Whether this is the Set of 24.2, whose entry answers its key as its
        /// value as well.
        set: bool,
    },
    /// The `[[ArrayBufferData]]` of 25.1.5, whose bytes are the block 25.1.3.1
    /// created; `None` is the detached block of 25.1.3.4.
    ArrayBuffer(Option<alloc::vec::Vec<u8>>),
    /// Boxed `BigInt` of 6.1.6.2, which 7.1.18 makes of one.
    BigIntWrapper(super::value::BigIntRef),
    /// The block of 25.2, which 25.2.3.1 allocates and no clause detaches.
    SharedArrayBuffer {
        /// `[[ArrayBufferData]]`.
        bytes: Vec<u8>,
        /// `[[ArrayBufferMaxByteLength]]`, absent for a block 25.2.3.1 made
        /// without one, which never grows.
        max: Option<u32>,
    },
    /// The namespace object of 25.4.
    Atomics,
    /// The `$262` of the conformance suite, which the embedding asks for and
    /// which no clause of the specification names.
    Host262,
    /// The `[[TypedArrayName]]` and the rest of 23.2.5: the block the array
    /// looks into, where, and which element kind 23.2.5.1 gave it.
    TypedArray {
        /// `[[ViewedArrayBuffer]]`.
        buffer: Value,
        /// `[[ByteOffset]]`.
        offset: u32,
        /// `[[ArrayLength]]`, in elements and not in bytes.
        length: u32,
        /// The row of table 71 the array stands in.
        kind: u8,
    },
    /// The `[[DataView]]` of 25.3.5: the block it looks into, and where.
    DataView {
        /// `[[ViewedArrayBuffer]]`.
        buffer: Value,
        /// `[[ByteOffset]]`.
        offset: u32,
        /// `[[ByteLength]]`.
        length: u32,
    },
    /// The `[[DateValue]]` of 21.4.4: the time value, or `NaN` for the Date
    /// 21.4.1.1 calls invalid.
    Date(f64),
    /// The `[[IsRawJSON]]` of the rawJSON proposal, whose `rawJSON` property
    /// holds the text 25.5.2 writes out verbatim.
    RawJson,
    /// The `[[WeakMapData]]` of 24.3.4 and the `[[WeakSetData]]` of 24.4.4.
    ///
    /// The pairs are held here rather than in an Array, because the collector
    /// must not follow a key: an entry lives only as long as something else
    /// holds its key, which is what `weak_entries` and `weak_entries_mut` name
    /// and `values` deliberately does not.
    WeakCollection {
        /// Each entry as its key and its value, in the order 24.3.3.3 added
        /// them; the Set of 24.4 answers its key as its value as well.
        entries: alloc::vec::Vec<(Value, Value)>,
        /// Whether this is the Set of 24.4.
        set: bool,
    },
    /// Array Iterator instance, the slots of 23.1.5.3.
    ArrayIterator {
        /// `[[IteratedArrayLike]]`, undefined once the iteration is done.
        target: Value,
        /// `[[ArrayLikeNextIndex]]`.
        index: u32,
        /// `[[ArrayLikeIterationKind]]`.
        kind: ArrayIterationKind,
    },
    /// The iterator 22.1.5 gives a String: the text it walks and where it
    /// stands in it, in code units.
    StringIterator {
        /// `[[IteratedString]]`, undefined once the iteration is done.
        target: Value,
        /// `[[StringNextIndex]]`.
        index: u32,
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
        /// Which part of 23.1.2 the walk stands in: the elements, the
        /// construct of the receiver, or the `length` of the answer.
        phase: u8,
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
        /// The pattern 22.2.4.1 compiled, which a literal shares with the unit
        /// it was compiled into and 22.2.3.1 makes at run time.
        pattern: PatternRef,
    },
    /// Promise instance, the slots of 27.2.6.
    Promise {
        /// `[[PromiseState]]`: 0 pending, 1 fulfilled, 2 rejected.
        state: u8,
        /// `[[PromiseIsHandled]]`.
        handled: bool,
        /// `[[PromiseResult]]`, undefined while the promise is pending.
        value: Value,
        /// `[[PromiseFulfillReactions]]`, as the Array of reaction records,
        /// undefined once the promise is settled.
        fulfill: Value,
        /// `[[PromiseRejectReactions]]`, as the Array of reaction records,
        /// undefined once the promise is settled.
        reject: Value,
    },
    /// The suspended body of an async function, which 27.7.5.3 leaves behind
    /// when it awaits and 27.7.5.2 resumes.
    ///
    /// A frame of this engine is a window of the register stack, so a body
    /// that leaves before it ends puts that window in the heap and takes it
    /// back where it stopped.
    Continuation {
        /// The unit the body was compiled with.
        unit: u32,
        /// Its index in that unit.
        code_id: u32,
        /// The offset the body continues at.
        pc: u32,
        /// How many parameter and local bindings the frame holds.
        bindings: u16,
        /// The registers of the frame, as the Array they were copied into.
        registers: Value,
        /// The capability 27.7.5.2 answers the caller with.
        capability: Value,
        /// The lexical context the frame ran in.
        context: Option<ContextRef>,
    },
    /// A Generator of 27.5, which holds the frame its body suspended in and
    /// what 27.5.1.2 reads as its `[[GeneratorState]]`.
    Generator {
        /// The continuation of the body, or undefined where it has none left.
        continuation: Value,
        /// `[[GeneratorState]]` of 27.5.1: 0 before the body ran, 1 where it
        /// suspended at a `yield`, 2 while it runs and 3 once it is done.
        state: u8,
    },
    /// An `AsyncGenerator` of 27.6, which holds the frame its body suspended
    /// in beside the requests of 27.6.3.1 that wait for it.
    AsyncGenerator {
        /// The continuation of the body, or undefined where it has none left.
        continuation: Value,
        /// The queue of 27.6.3.1 as an Array: each request is the capability
        /// of 27.2.1.1 it answers through and the value its call was given.
        queue: Value,
        /// Where the front of the queue stands, which a request that was
        /// answered moves.
        head: u32,
        /// `[[AsyncGeneratorState]]` of 27.6.1: 0 before the body ran, 1 where
        /// it suspended at a `yield`, 2 while it runs and 3 once it is done.
        state: u8,
    },
    /// The object 27.1.4.1 wraps a sync iterator in, whose
    /// `[[SyncIteratorRecord]]` is the iterator and its `next`.
    AsyncFromSyncIterator {
        /// `[[Iterator]]` of the record.
        iterator: Value,
        /// `[[NextMethod]]` of the record, read once by 7.4.2 step 3.
        next: Value,
    },
    /// Bound function exotic object, the slots of 10.4.1.
    BoundFunction {
        /// `[[BoundTargetFunction]]`.
        target: Value,
        /// `[[BoundThis]]`.
        receiver: Value,
        /// `[[BoundArguments]]`, as the Array holding them, or undefined
        /// where 20.2.3.2 was given none.
        arguments: Value,
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
            Self::StringWrapper(value)
            | Self::ArrayIterator { target: value, .. }
            | Self::StringIterator { target: value, .. }
            | Self::CollectionIterator { target: value, .. }
            | Self::NativeFunction { state: value, .. }
            | Self::Collection { entries: value, .. }
            | Self::DataView { buffer: value, .. }
            | Self::TypedArray { buffer: value, .. }
            | Self::Function { home: value, .. } => [Some(*value), None, None, None, None],
            Self::Promise {
                value,
                fulfill,
                reject,
                ..
            } => [Some(*value), Some(*fulfill), Some(*reject), None, None],
            Self::Continuation {
                registers,
                capability,
                ..
            } => [Some(*registers), Some(*capability), None, None, None],
            Self::Generator { continuation, .. } => [Some(*continuation), None, None, None, None],
            Self::AsyncGenerator {
                continuation,
                queue,
                ..
            } => [Some(*continuation), Some(*queue), None, None, None],
            Self::AsyncFromSyncIterator { iterator, next } => {
                [Some(*iterator), Some(*next), None, None, None]
            }
            Self::BoundFunction {
                target,
                receiver,
                arguments,
            } => [Some(*target), Some(*receiver), Some(*arguments), None, None],
            Self::Accessor {
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

    /// The lexical context this kind holds.
    ///
    /// The collector reaches a context through its own work list, so a kind
    /// that holds one says so here and nowhere else.
    #[must_use]
    pub const fn context(&self) -> Option<ContextRef> {
        match self {
            Self::Function { context, .. }
            | Self::Continuation { context, .. }
            | Self::Arguments { context, .. } => *context,
            _ => None,
        }
    }

    /// The same context, to be forwarded by a collection.
    pub const fn context_mut(&mut self) -> Option<&mut ContextRef> {
        match self {
            Self::Function { context, .. }
            | Self::Continuation { context, .. }
            | Self::Arguments { context, .. } => context.as_mut(),
            _ => None,
        }
    }

    /// The entries of a weak collection, which no collector follows without
    /// first finding the key alive elsewhere.
    #[must_use]
    pub const fn weak_entries(&self) -> Option<&alloc::vec::Vec<(Value, Value)>> {
        match self {
            Self::WeakCollection { entries, .. } => Some(entries),
            _ => None,
        }
    }

    /// The entries of a weak collection, for the clause that writes one and
    /// for the collector that prunes a dead key.
    pub const fn weak_entries_mut(&mut self) -> Option<&mut alloc::vec::Vec<(Value, Value)>> {
        match self {
            Self::WeakCollection { entries, .. } => Some(entries),
            _ => None,
        }
    }

    /// Every `Value` this kind holds, to be forwarded by a collection.
    pub const fn values_mut(&mut self) -> [Option<&mut Value>; 5] {
        match self {
            Self::StringWrapper(value)
            | Self::ArrayIterator { target: value, .. }
            | Self::StringIterator { target: value, .. }
            | Self::CollectionIterator { target: value, .. }
            | Self::NativeFunction { state: value, .. }
            | Self::Collection { entries: value, .. }
            | Self::DataView { buffer: value, .. }
            | Self::TypedArray { buffer: value, .. }
            | Self::Function { home: value, .. } => [Some(value), None, None, None, None],
            Self::Promise {
                value,
                fulfill,
                reject,
                ..
            } => [Some(value), Some(fulfill), Some(reject), None, None],
            Self::Continuation {
                registers,
                capability,
                ..
            } => [Some(registers), Some(capability), None, None, None],
            Self::Generator { continuation, .. } => [Some(continuation), None, None, None, None],
            Self::AsyncGenerator {
                continuation,
                queue,
                ..
            } => [Some(continuation), Some(queue), None, None, None],
            Self::AsyncFromSyncIterator { iterator, next } => {
                [Some(iterator), Some(next), None, None, None]
            }
            Self::BoundFunction {
                target,
                receiver,
                arguments,
            } => [Some(target), Some(receiver), Some(arguments), None, None],
            Self::Accessor {
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

/// `GetValueFromBuffer` of 25.1.3.10 for the kinds 25.3.4 names.
#[expect(
    clippy::cast_precision_loss,
    reason = "every width below 2^53 is a binary64 exactly"
)]
pub(crate) fn read_element(raw: [u8; 8], size: usize, kind: u8, little: bool) -> f64 {
    let mut value = 0u64;
    for offset in 0..size {
        let at = if little { size - 1 - offset } else { offset };
        value = (value << 8) | u64::from(*raw.get(at).unwrap_or(&0));
    }
    match kind {
        5 => from_binary16(u16::try_from(value & 0xFFFF).unwrap_or(0)),
        2 => f64::from(f32::from_bits(
            u32::try_from(value & 0xFFFF_FFFF).unwrap_or(0),
        )),
        3 => f64::from_bits(value),
        1 => value as f64,
        _ => {
            let bits = size.saturating_mul(8);
            let sign = 1u64 << (bits.saturating_sub(1));
            if value & sign == 0 {
                value as f64
            } else {
                -(((!value).wrapping_add(1) & (sign | (sign - 1))) as f64)
            }
        }
    }
}

/// `SetValueInBuffer` of 25.1.3.12 for the same kinds.
#[expect(
    clippy::cast_possible_truncation,
    reason = "7.1.6 and 7.1.7 wrap on purpose, and 6.1.6.1.20 rounds to the nearer binary32"
)]
pub(crate) fn write_element(value: f64, size: usize, kind: u8, little: bool) -> [u8; 8] {
    let bits = match kind {
        5 => u64::from(to_binary16(value)),
        2 => u64::from((value as f32).to_bits()),
        3 => value.to_bits(),
        _ => {
            let wrapped = crate::value::number_uint32(value);
            if size == 8 {
                u64::from(wrapped)
            } else {
                u64::from(wrapped) & ((1u64 << (size.saturating_mul(8))) - 1)
            }
        }
    };
    let mut out = [0u8; 8];
    for offset in 0..size {
        let shift = if little { offset } else { size - 1 - offset };
        if let Some(slot) = out.get_mut(offset) {
            *slot = ((bits >> (shift.saturating_mul(8))) & 0xFF) as u8;
        }
    }
    out
}

/// The bytes one element of a row of table 71 takes, how 25.1.3.10 reads it
/// back, and whether 7.1.11 clamps what a write gives it.
pub(crate) const fn element_form(kind: u8) -> (usize, u8, bool) {
    // The second field is the one `read_element` and `write_element` name: 0
    // signed, 1 unsigned, 2 a binary32, 3 a binary64, 5 a binary16, and 6 and
    // 7 the two rows a `BigInt` holds.
    match kind {
        0 => (1, 0, false),
        1 => (1, 1, false),
        2 => (1, 1, true),
        3 => (2, 0, false),
        4 => (2, 1, false),
        5 => (4, 0, false),
        6 => (4, 1, false),
        7 => (4, 2, false),
        9 => (2, 5, false),
        10 => (8, 6, false),
        11 => (8, 7, false),
        _ => (8, 3, false),
    }
}

/// Whether the row of table 71 holds a `BigInt` rather than a Number.
pub(crate) const fn holds_a_bigint(kind: u8) -> bool {
    matches!(kind, 10 | 11)
}

#[cfg(test)]
mod binary16_tests {
    use super::{from_binary16, to_binary16};

    #[test]
    fn the_binary16_of_table_71_rounds_to_the_nearer_value_and_ties_to_even() {
        for (value, bits) in [
            (0.0, 0x0000u16),
            (-0.0, 0x8000),
            (1.0, 0x3C00),
            (-2.0, 0xC000),
            (1.5, 0x3E00),
            (65504.0, 0x7BFF),
            (65536.0, 0x7C00),
            (f64::INFINITY, 0x7C00),
            (f64::NEG_INFINITY, 0xFC00),
            // The smallest normal and the smallest subnormal.
            (6.103_515_625e-5, 0x0400),
            (5.960_464_477_539_063e-8, 0x0001),
            // Halfway between two binary16 values, which ties to the even one.
            (2049.0, 0x6800),
            (2051.0, 0x6802),
        ] {
            assert_eq!(to_binary16(value), bits, "{value}");
        }
        for bits in [0x0000u16, 0x3C00, 0xC000, 0x7BFF, 0x0400, 0x0001, 0x6802] {
            assert_eq!(to_binary16(from_binary16(bits)), bits, "{bits:#06x}");
        }
        assert!(from_binary16(0x7E00).is_nan());
        assert_eq!(to_binary16(f64::NAN), 0x7E00);
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

/// Two raised to this power, for an exponent a binary64 holds.
const fn power_of_two(exponent: i32) -> f64 {
    let biased = exponent.saturating_add(1023);
    if biased <= 0 {
        return 0.0;
    }
    f64::from_bits((biased.unsigned_abs() as u64) << 52)
}

/// The Number a binary16 of table 71 denotes.
pub(crate) fn from_binary16(bits: u16) -> f64 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1F);
    let fraction = f64::from(bits & 0x03FF);
    if exponent == 0x1F {
        return if fraction == 0.0 {
            sign * f64::INFINITY
        } else {
            f64::NAN
        };
    }
    if exponent == 0 {
        return sign * fraction * power_of_two(-24);
    }
    sign * (1.0 + fraction / 1024.0) * power_of_two(exponent - 15)
}

/// The binary16 nearest the Number, with a tie to the even one, as 25.1.3.12
/// rounds every other width.
pub(crate) fn to_binary16(value: f64) -> u16 {
    if value.is_nan() {
        return 0x7E00;
    }
    let bits = value.to_bits();
    let sign = ((bits >> 48) & 0x8000) as u16;
    if value.is_infinite() {
        return sign | 0x7C00;
    }
    let exponent = ((bits >> 52) & 0x7FF) as i32;
    let fraction = bits & 0x000F_FFFF_FFFF_FFFF;
    if exponent == 0 && fraction == 0 {
        return sign;
    }
    // The significand carries the implicit one of a normal binary64.
    let (significand, unbiased) = if exponent == 0 {
        (fraction, -1022)
    } else {
        (fraction | (1u64 << 52), exponent - 1023)
    };
    let mut half_exponent = unbiased + 15;
    // A binary16 keeps eleven significant bits, so a binary64 drops 42 of its
    // 53, and a subnormal one drops as many more as its exponent is below one.
    let mut shift = 42i32;
    if half_exponent < 1 {
        shift += 1 - half_exponent;
        half_exponent = 0;
        if shift > 53 {
            return sign;
        }
    }
    let shift = shift.unsigned_abs();
    let dropped = significand & ((1u64 << shift) - 1);
    let mut kept = significand >> shift;
    let half = 1u64 << (shift - 1);
    if dropped > half || (dropped == half && kept & 1 == 1) {
        kept += 1;
    }
    // A subnormal that rounds up into the normal range carries its leading bit
    // into the exponent field, which needs no step of its own.
    if half_exponent == 0 {
        return sign | u16::try_from(kept).unwrap_or(0);
    }
    if kept & (1 << 11) != 0 {
        kept >>= 1;
        half_exponent += 1;
    }
    if half_exponent >= 31 {
        return sign | 0x7C00;
    }
    sign | (u16::try_from(half_exponent).unwrap_or(0) << 10)
        | (u16::try_from(kept).unwrap_or(0) & 0x03FF)
}
