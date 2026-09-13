// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Realm intrinsic object graph.
//!
//! One Realm owns the well-known intrinsics of ECMA-262 clause 9.3 that the
//! Register backend allocates against: the ordinary prototype chain roots and
//! the Error hierarchy of 20.5 the VM throws from. Every intrinsic is held in
//! a permanent heap root, so a minor collection forwards it instead of
//! reclaiming it.

use super::{
    heap::{GenerationalHeap, HeapError, Root},
    shape::PropertyFlags,
    value::{ObjectRef, VALUE_NULL, Value},
};

/// Native error type of ECMA-262 20.5.5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeErrorKind {
    /// `EvalError`, 20.5.5.1.
    EvalError,
    /// `RangeError`, 20.5.5.2.
    RangeError,
    /// `ReferenceError`, 20.5.5.3.
    ReferenceError,
    /// `SyntaxError`, 20.5.5.4.
    SyntaxError,
    /// `TypeError`, 20.5.5.5.
    TypeError,
    /// `URIError`, 20.5.5.6.
    UriError,
}

/// Whether an implemented intrinsic of `%Object.prototype%` has this name.
#[must_use]
pub fn object_prototype_intrinsic(name: &[u16]) -> Option<Intrinsic> {
    Intrinsic::ALL
        .into_iter()
        .find(|intrinsic| intrinsic.name().encode_utf16().eq(name.iter().copied()))
}

/// A native function of the standard library.
///
/// The identifier travels in the object's `NativeFunction` kind, so a call
/// dispatches on it without a lookup table in the heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intrinsic {
    /// `Object.prototype.hasOwnProperty` (20.1.3.2).
    ObjectPrototypeHasOwnProperty,
    /// `Object.prototype.isPrototypeOf` (20.1.3.3).
    ObjectPrototypeIsPrototypeOf,
    /// `Object.prototype.propertyIsEnumerable` (20.1.3.4).
    ObjectPrototypePropertyIsEnumerable,
    /// `Object.prototype.toString` (20.1.3.6).
    ObjectPrototypeToString,
}

impl Intrinsic {
    /// Every intrinsic, in the order the Realm allocates them.
    pub const ALL: [Self; 4] = [
        Self::ObjectPrototypeHasOwnProperty,
        Self::ObjectPrototypeIsPrototypeOf,
        Self::ObjectPrototypePropertyIsEnumerable,
        Self::ObjectPrototypeToString,
    ];

    /// The identifier carried by the function object.
    #[must_use]
    pub const fn id(self) -> u32 {
        match self {
            Self::ObjectPrototypeHasOwnProperty => 0,
            Self::ObjectPrototypeIsPrototypeOf => 1,
            Self::ObjectPrototypePropertyIsEnumerable => 2,
            Self::ObjectPrototypeToString => 3,
        }
    }

    /// Index of this intrinsic in [`Self::ALL`].
    const fn index(self) -> usize {
        match self {
            Self::ObjectPrototypeHasOwnProperty => 0,
            Self::ObjectPrototypeIsPrototypeOf => 1,
            Self::ObjectPrototypePropertyIsEnumerable => 2,
            Self::ObjectPrototypeToString => 3,
        }
    }

    /// The intrinsic one identifier denotes.
    #[must_use]
    pub const fn from_id(id: u32) -> Option<Self> {
        match id {
            0 => Some(Self::ObjectPrototypeHasOwnProperty),
            1 => Some(Self::ObjectPrototypeIsPrototypeOf),
            2 => Some(Self::ObjectPrototypePropertyIsEnumerable),
            3 => Some(Self::ObjectPrototypeToString),
            _ => None,
        }
    }

    /// The `name` property of the function object (17).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ObjectPrototypeHasOwnProperty => "hasOwnProperty",
            Self::ObjectPrototypeIsPrototypeOf => "isPrototypeOf",
            Self::ObjectPrototypePropertyIsEnumerable => "propertyIsEnumerable",
            Self::ObjectPrototypeToString => "toString",
        }
    }

    /// The `length` property of the function object (17).
    #[must_use]
    pub const fn length(self) -> u32 {
        match self {
            Self::ObjectPrototypeHasOwnProperty
            | Self::ObjectPrototypeIsPrototypeOf
            | Self::ObjectPrototypePropertyIsEnumerable => 1,
            Self::ObjectPrototypeToString => 0,
        }
    }
}

/// The property names `%Object.prototype%` owns: 20.1.3 and, for the four
/// accessor helpers and `__proto__`, B.2.2.
///
/// A compiler that types a property read from an object's own layout has to
/// consult this list, because a name on it is resolved on the Prototype Chain
/// and is not undefined. The Realm materializes these as intrinsics one at a
/// time; the list is the contract either way.
pub const OBJECT_PROTOTYPE_PROPERTIES: [&str; 12] = [
    "constructor",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toLocaleString",
    "toString",
    "valueOf",
    "__proto__",
    "__defineGetter__",
    "__defineSetter__",
    "__lookupGetter__",
    "__lookupSetter__",
];

/// The property names `%Array.prototype%` owns (23.1.3), excluding the two
/// Symbol keys it also carries.
///
/// It serves the same purpose as [`OBJECT_PROTOTYPE_PROPERTIES`]: an Array read
/// of one of these names is answered by the Prototype Chain.
pub const ARRAY_PROTOTYPE_PROPERTIES: [&str; 39] = [
    "at",
    "concat",
    "constructor",
    "copyWithin",
    "entries",
    "every",
    "fill",
    "filter",
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "flat",
    "flatMap",
    "forEach",
    "includes",
    "indexOf",
    "join",
    "keys",
    "lastIndexOf",
    "map",
    "pop",
    "push",
    "reduce",
    "reduceRight",
    "reverse",
    "shift",
    "slice",
    "some",
    "sort",
    "splice",
    "toLocaleString",
    "toReversed",
    "toSorted",
    "toSpliced",
    "toString",
    "unshift",
    "values",
    "with",
];

/// Whether `%Array.prototype%` or `%Object.prototype%` owns a property of this
/// name, which an Array resolves on its Prototype Chain.
#[must_use]
pub fn array_prototype_owns(name: &[u16]) -> bool {
    object_prototype_owns(name)
        || ARRAY_PROTOTYPE_PROPERTIES
            .into_iter()
            .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

/// Whether `%Object.prototype%` owns a property of this name.
#[must_use]
pub fn object_prototype_owns(name: &[u16]) -> bool {
    OBJECT_PROTOTYPE_PROPERTIES
        .into_iter()
        .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

/// Number of native error types of 20.5.5.
pub const NATIVE_ERROR_COUNT: usize = 6;

impl NativeErrorKind {
    /// Every native error type, in the order of 20.5.5.
    pub const ALL: [Self; NATIVE_ERROR_COUNT] = [
        Self::EvalError,
        Self::RangeError,
        Self::ReferenceError,
        Self::SyntaxError,
        Self::TypeError,
        Self::UriError,
    ];

    /// The constructor name, which is also the prototype's `name` (20.5.6.3.3).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::EvalError => "EvalError",
            Self::RangeError => "RangeError",
            Self::ReferenceError => "ReferenceError",
            Self::SyntaxError => "SyntaxError",
            Self::TypeError => "TypeError",
            Self::UriError => "URIError",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::EvalError => 0,
            Self::RangeError => 1,
            Self::ReferenceError => 2,
            Self::SyntaxError => 3,
            Self::TypeError => 4,
            Self::UriError => 5,
        }
    }
}

/// Attributes of a built-in data property: writable, non-enumerable,
/// configurable (7.3.7 and the clause 17 default).
#[must_use]
pub const fn builtin_data() -> PropertyFlags {
    PropertyFlags {
        writable: true,
        enumerable: false,
        configurable: true,
        is_accessor: false,
    }
}

/// Attributes of a built-in function's `name` and `length` (10.2.8, 10.2.9):
/// not writable, not enumerable, configurable.
#[must_use]
pub const fn builtin_metadata() -> PropertyFlags {
    PropertyFlags {
        writable: false,
        enumerable: false,
        configurable: true,
        is_accessor: false,
    }
}

/// Well-known intrinsics of one Realm.
pub struct Realm {
    object_prototype: Root,
    function_prototype: Root,
    array_prototype: Root,
    error_prototype: Root,
    native_error_prototypes: [Root; NATIVE_ERROR_COUNT],
    intrinsics: [Root; Intrinsic::ALL.len()],
}

impl Realm {
    /// Builds the intrinsic object graph in `heap`.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] when an intrinsic cannot be allocated, rooted or
    /// given its initial properties.
    pub fn new(heap: &mut GenerationalHeap) -> Result<Self, HeapError> {
        let root_shape = heap.shapes.root_shape();

        // 20.1.3: %Object.prototype% has a null [[Prototype]].
        let object_prototype = heap.allocate_immortal_object(root_shape, VALUE_NULL)?;
        let object_prototype = heap.push_root(Value::from_object(object_prototype))?;
        let ordinary = Self::rooted(heap, object_prototype)?;

        // 20.2.3: %Function.prototype% is a built-in function whose
        // [[Prototype]] is %Object.prototype%. It is modelled as an ordinary
        // object until callable intrinsics exist.
        let function_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let function_prototype = heap.push_root(Value::from_object(function_prototype))?;

        // 23.1.3: %Array.prototype% is an Array exotic object.
        let array_prototype = heap.allocate_immortal_array(ordinary, 0)?;
        let array_prototype = heap.push_root(Value::from_object(array_prototype))?;

        // 20.5.3: %Error.prototype% is an ordinary object with "message" and
        // "name", not an Error instance.
        let error_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let error_prototype = heap.push_root(Value::from_object(error_prototype))?;
        Self::define_error_prototype(heap, error_prototype, "Error")?;
        let error_parent = Self::rooted(heap, error_prototype)?;

        // 20.5.6.3: each NativeError prototype inherits from %Error.prototype%.
        let mut native_error_prototypes = [error_prototype; NATIVE_ERROR_COUNT];
        for kind in NativeErrorKind::ALL {
            let prototype = heap.allocate_immortal_object(root_shape, error_parent)?;
            let prototype = heap.push_root(Value::from_object(prototype))?;
            Self::define_error_prototype(heap, prototype, kind.name())?;
            *native_error_prototypes
                .get_mut(kind.index())
                .ok_or(HeapError::InvalidReference)? = prototype;
        }

        let mut intrinsics = [object_prototype; Intrinsic::ALL.len()];
        let function_parent = Self::rooted(heap, function_prototype)?;
        for intrinsic in Intrinsic::ALL {
            let function =
                heap.allocate_immortal_native(function_parent, intrinsic.id(), intrinsic.length())?;
            let length_key = intern(heap, "length")?;
            let length = Value::from_smi(i32::try_from(intrinsic.length()).unwrap_or(i32::MAX));
            heap.define_own_named(function, length_key, length, builtin_metadata())?;
            let name_key = intern(heap, "name")?;
            let name = heap.strings.allocate_str(intrinsic.name())?;
            heap.define_own_named(
                function,
                name_key,
                Value::from_string(name),
                builtin_metadata(),
            )?;
            *intrinsics
                .get_mut(intrinsic.index())
                .ok_or(HeapError::InvalidReference)? =
                heap.push_root(Value::from_object(function))?;
            let holder = Self::rooted(heap, object_prototype)?
                .as_object()
                .ok_or(HeapError::InvalidReference)?;
            let key = heap.strings.intern(intrinsic.name())?;
            heap.define_own_named(holder, key, Value::from_object(function), builtin_data())?;
        }

        Ok(Self {
            object_prototype,
            function_prototype,
            array_prototype,
            error_prototype,
            native_error_prototypes,
            intrinsics,
        })
    }

    /// The function object of one native intrinsic.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn intrinsic(
        &self,
        heap: &GenerationalHeap,
        intrinsic: Intrinsic,
    ) -> Result<Value, HeapError> {
        let root = *self
            .intrinsics
            .get(intrinsic.index())
            .ok_or(HeapError::InvalidReference)?;
        Self::rooted(heap, root)
    }

    /// %Object.prototype%.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn object_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.object_prototype)
    }

    /// %Function.prototype%.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn function_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.function_prototype)
    }

    /// %Array.prototype%.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn array_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.array_prototype)
    }

    /// %Error.prototype%.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn error_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.error_prototype)
    }

    /// The prototype of one native error type (20.5.6.3).
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn native_error_prototype(
        &self,
        heap: &GenerationalHeap,
        kind: NativeErrorKind,
    ) -> Result<Value, HeapError> {
        let root = *self
            .native_error_prototypes
            .get(kind.index())
            .ok_or(HeapError::InvalidReference)?;
        Self::rooted(heap, root)
    }

    /// Allocates an ordinary object with %Object.prototype% (10.1.12).
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] when the Nursery is full or a root is stale.
    pub fn ordinary_object(&self, heap: &mut GenerationalHeap) -> Result<ObjectRef, HeapError> {
        let prototype = self.object_prototype(heap)?;
        let shape = heap.shapes.root_shape();
        heap.allocate_object(shape, prototype)
    }

    /// Allocates an Array with %Array.prototype% (23.1.3).
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] when the Nursery is full or a root is stale.
    pub fn array(&self, heap: &mut GenerationalHeap, length: u32) -> Result<ObjectRef, HeapError> {
        let prototype = self.array_prototype(heap)?;
        let array = heap.allocate_array(length)?;
        heap.set_object_prototype(array, prototype)?;
        Ok(array)
    }

    /// Creates a native error instance as 20.5.6.1.1 does for a defined
    /// `message`: an ordinary object with the type's prototype and a
    /// non-enumerable own `message`.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] when the Nursery is full, a root is stale, or
    /// the message cannot be interned.
    pub fn create_native_error(
        &self,
        heap: &mut GenerationalHeap,
        kind: NativeErrorKind,
        message: &str,
    ) -> Result<ObjectRef, HeapError> {
        let prototype = self.native_error_prototype(heap, kind)?;
        let shape = heap.shapes.root_shape();
        let error = heap.allocate_object(shape, prototype)?;
        heap.set_object_kind(error, super::object::ObjectKind::Error)?;
        if !message.is_empty() {
            let name = intern(heap, "message")?;
            let text = heap.strings.allocate_str(message)?;
            heap.define_own_named(error, name, Value::from_string(text), builtin_data())?;
        }
        Ok(error)
    }

    fn define_error_prototype(
        heap: &mut GenerationalHeap,
        prototype: Root,
        name: &str,
    ) -> Result<(), HeapError> {
        let object = Self::rooted(heap, prototype)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        // 20.5.3.2 / 20.5.6.3.2: "message" is the empty String.
        let message_key = intern(heap, "message")?;
        let empty = heap.strings.allocate_str("")?;
        heap.define_own_named(
            object,
            message_key,
            Value::from_string(empty),
            builtin_data(),
        )?;
        // 20.5.3.3 / 20.5.6.3.3: "name" is the constructor name.
        let name_key = intern(heap, "name")?;
        let name_value = heap.strings.allocate_str(name)?;
        heap.define_own_named(
            object,
            name_key,
            Value::from_string(name_value),
            builtin_data(),
        )?;
        Ok(())
    }

    fn rooted(heap: &GenerationalHeap, root: Root) -> Result<Value, HeapError> {
        heap.root_value(root).ok_or(HeapError::InvalidReference)
    }
}

fn intern(heap: &mut GenerationalHeap, name: &str) -> Result<super::value::StringRef, HeapError> {
    Ok(heap.strings.intern(name)?)
}

#[cfg(test)]
mod tests {
    use super::{NativeErrorKind, Realm, builtin_data};
    use crate::engine::{agent::Agent, heap::GenerationalHeap, value::Value};

    fn text(heap: &GenerationalHeap, value: Value) -> alloc::string::String {
        heap.strings.to_rust_string(value).unwrap()
    }

    #[test]
    fn intrinsic_prototype_chain_matches_the_standard_object_graph() {
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();

        // 20.1.3: %Object.prototype% has a null [[Prototype]].
        let object_prototype = realm.object_prototype(&heap).unwrap();
        assert!(
            heap.get_object(object_prototype.as_object().unwrap())
                .unwrap()
                .prototype
                .is_null()
        );

        // 20.2.3 and 23.1.3 both inherit from %Object.prototype%.
        for intrinsic in [
            realm.function_prototype(&heap).unwrap(),
            realm.array_prototype(&heap).unwrap(),
            realm.error_prototype(&heap).unwrap(),
        ] {
            assert_eq!(
                heap.get_object(intrinsic.as_object().unwrap())
                    .unwrap()
                    .prototype,
                object_prototype
            );
        }

        // 20.5.6.3: every NativeError prototype inherits from %Error.prototype%.
        let error_prototype = realm.error_prototype(&heap).unwrap();
        for kind in NativeErrorKind::ALL {
            let prototype = realm.native_error_prototype(&heap, kind).unwrap();
            assert_eq!(
                heap.get_object(prototype.as_object().unwrap())
                    .unwrap()
                    .prototype,
                error_prototype
            );
        }
    }

    #[test]
    fn error_prototypes_carry_the_specified_name_and_empty_message() {
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let name_key = heap.strings.intern("name").unwrap();
        let message_key = heap.strings.intern("message").unwrap();

        let mut expected = alloc::vec![(realm.error_prototype(&heap).unwrap(), "Error")];
        for kind in NativeErrorKind::ALL {
            expected.push((
                realm.native_error_prototype(&heap, kind).unwrap(),
                kind.name(),
            ));
        }
        for (prototype, name) in expected {
            let reference = prototype.as_object().unwrap();
            let found = heap.lookup_named(reference, name_key).unwrap().unwrap();
            assert_eq!(found.holder_depth, 0);
            assert_eq!(text(&heap, found.value), name);
            let message = heap.lookup_named(reference, message_key).unwrap().unwrap();
            assert_eq!(text(&heap, message.value), "");
            let shape = heap.get_object(reference).unwrap().shape_id;
            assert_eq!(
                heap.shapes.lookup(shape, name_key).unwrap().flags,
                builtin_data()
            );
        }
    }

    #[test]
    fn native_error_instances_inherit_name_and_own_a_non_enumerable_message() {
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let error = realm
            .create_native_error(&mut heap, NativeErrorKind::TypeError, "not a function")
            .unwrap();

        let name_key = heap.strings.intern("name").unwrap();
        let inherited = heap.lookup_named(error, name_key).unwrap().unwrap();
        assert_eq!(inherited.holder_depth, 1);
        assert_eq!(text(&heap, inherited.value), "TypeError");

        // 20.5.6.1.1 step 3: "message" is an own non-enumerable property.
        let message_key = heap.strings.intern("message").unwrap();
        let own = heap.lookup_named(error, message_key).unwrap().unwrap();
        assert_eq!(own.holder_depth, 0);
        assert_eq!(text(&heap, own.value), "not a function");
        let shape = heap.get_object(error).unwrap().shape_id;
        assert!(
            !heap
                .shapes
                .lookup(shape, message_key)
                .unwrap()
                .flags
                .enumerable
        );

        // An undefined message defines no own property, so "message" resolves
        // to the empty String on the type's own prototype (20.5.6.3.2).
        let bare = realm
            .create_native_error(&mut heap, NativeErrorKind::RangeError, "")
            .unwrap();
        let inherited_message = heap.lookup_named(bare, message_key).unwrap().unwrap();
        assert_eq!(inherited_message.holder_depth, 1);
        assert_eq!(text(&heap, inherited_message.value), "");
    }

    #[test]
    fn intrinsics_are_immortal_and_survive_collection_without_being_copied() {
        let mut agent = Agent::new().unwrap();
        let before = agent.realm.object_prototype(&agent.heap).unwrap();
        assert!(before.as_object().unwrap().is_old());
        agent.heap.scavenge().unwrap();
        let after = agent.realm.object_prototype(&agent.heap).unwrap();
        assert_eq!(before, after);

        // A fresh ordinary object is young and reaches the immortal prototype.
        let object = agent.realm.ordinary_object(&mut agent.heap).unwrap();
        assert!(object.is_young());
        assert_eq!(agent.heap.get_object(object).unwrap().prototype, after);
    }
}
