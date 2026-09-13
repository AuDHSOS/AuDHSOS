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
    value::{PropertyKey, SymbolRef},
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

/// The implemented intrinsic of one holder that has this name.
#[must_use]
pub fn holder_intrinsic(holder: IntrinsicHolder, name: &[u16]) -> Option<Intrinsic> {
    Intrinsic::ALL.into_iter().find(|intrinsic| {
        intrinsic.holder() == holder && intrinsic.name().encode_utf16().eq(name.iter().copied())
    })
}

/// Whether an implemented intrinsic of `%Object.prototype%` has this name.
#[must_use]
pub fn object_prototype_intrinsic(name: &[u16]) -> Option<Intrinsic> {
    holder_intrinsic(IntrinsicHolder::ObjectPrototype, name)
}

/// A well-known Symbol of table 1 in 6.1.5.1.
///
/// Every Symbol the engine has is one of these: a user Symbol needs the
/// `Symbol` constructor, which does not exist yet, so the set is closed and
/// each one is its own `SymbolRef`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WellKnownSymbol {
    /// `@@asyncIterator`.
    AsyncIterator,
    /// `@@hasInstance`.
    HasInstance,
    /// `@@isConcatSpreadable`.
    IsConcatSpreadable,
    /// `@@iterator`.
    Iterator,
    /// `@@match`.
    Match,
    /// `@@matchAll`.
    MatchAll,
    /// `@@replace`.
    Replace,
    /// `@@search`.
    Search,
    /// `@@species`.
    Species,
    /// `@@split`.
    Split,
    /// `@@toPrimitive`.
    ToPrimitive,
    /// `@@toStringTag`.
    ToStringTag,
    /// `@@unscopables`.
    Unscopables,
}

impl WellKnownSymbol {
    /// Every well-known Symbol, in the order of table 1.
    pub const ALL: [Self; 13] = [
        Self::AsyncIterator,
        Self::HasInstance,
        Self::IsConcatSpreadable,
        Self::Iterator,
        Self::Match,
        Self::MatchAll,
        Self::Replace,
        Self::Search,
        Self::Species,
        Self::Split,
        Self::ToPrimitive,
        Self::ToStringTag,
        Self::Unscopables,
    ];

    /// The `[[Description]]` of this Symbol.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::AsyncIterator => "Symbol.asyncIterator",
            Self::HasInstance => "Symbol.hasInstance",
            Self::IsConcatSpreadable => "Symbol.isConcatSpreadable",
            Self::Iterator => "Symbol.iterator",
            Self::Match => "Symbol.match",
            Self::MatchAll => "Symbol.matchAll",
            Self::Replace => "Symbol.replace",
            Self::Search => "Symbol.search",
            Self::Species => "Symbol.species",
            Self::Split => "Symbol.split",
            Self::ToPrimitive => "Symbol.toPrimitive",
            Self::ToStringTag => "Symbol.toStringTag",
            Self::Unscopables => "Symbol.unscopables",
        }
    }

    /// The reference that identifies this Symbol.
    #[must_use]
    pub const fn reference(self) -> SymbolRef {
        SymbolRef(self.index())
    }

    /// The key this Symbol is when it names a property.
    #[must_use]
    pub const fn key(self) -> PropertyKey {
        PropertyKey::Symbol(self.reference())
    }

    /// The well-known Symbol one reference denotes.
    #[must_use]
    pub const fn from_reference(reference: SymbolRef) -> Option<Self> {
        match reference.0 {
            0 => Some(Self::AsyncIterator),
            1 => Some(Self::HasInstance),
            2 => Some(Self::IsConcatSpreadable),
            3 => Some(Self::Iterator),
            4 => Some(Self::Match),
            5 => Some(Self::MatchAll),
            6 => Some(Self::Replace),
            7 => Some(Self::Search),
            8 => Some(Self::Species),
            9 => Some(Self::Split),
            10 => Some(Self::ToPrimitive),
            11 => Some(Self::ToStringTag),
            12 => Some(Self::Unscopables),
            _ => None,
        }
    }

    const fn index(self) -> u32 {
        match self {
            Self::AsyncIterator => 0,
            Self::HasInstance => 1,
            Self::IsConcatSpreadable => 2,
            Self::Iterator => 3,
            Self::Match => 4,
            Self::MatchAll => 5,
            Self::Replace => 6,
            Self::Search => 7,
            Self::Species => 8,
            Self::Split => 9,
            Self::ToPrimitive => 10,
            Self::ToStringTag => 11,
            Self::Unscopables => 12,
        }
    }
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
    /// `String.prototype.charAt` (22.1.3.1).
    StringPrototypeCharAt,
    /// `String.prototype.charCodeAt` (22.1.3.2).
    StringPrototypeCharCodeAt,
    /// `String.prototype.indexOf` (22.1.3.9).
    StringPrototypeIndexOf,
    /// `String.prototype.at` (22.1.3.1).
    StringPrototypeAt,
    /// `String.prototype.concat` (22.1.3.5).
    StringPrototypeConcat,
    /// `String.prototype.endsWith` (22.1.3.7).
    StringPrototypeEndsWith,
    /// `String.prototype.includes` (22.1.3.8).
    StringPrototypeIncludes,
    /// `String.prototype.lastIndexOf` (22.1.3.10).
    StringPrototypeLastIndexOf,
    /// `String.prototype.repeat` (22.1.3.17).
    StringPrototypeRepeat,
    /// `String.prototype.slice` (22.1.3.22).
    StringPrototypeSlice,
    /// `String.prototype.startsWith` (22.1.3.24).
    StringPrototypeStartsWith,
    /// `String.prototype.substring` (22.1.3.25).
    StringPrototypeSubstring,
    /// `String.prototype.codePointAt` (22.1.3.4).
    StringPrototypeCodePointAt,
    /// `String.prototype.padEnd` (22.1.3.15).
    StringPrototypePadEnd,
    /// `String.prototype.padStart` (22.1.3.16).
    StringPrototypePadStart,
    /// `String.prototype.trim` (22.1.3.32).
    StringPrototypeTrim,
    /// `String.prototype.trimEnd` (22.1.3.33).
    StringPrototypeTrimEnd,
    /// `String.prototype.trimStart` (22.1.3.34).
    StringPrototypeTrimStart,
}

/// The intrinsic object a native function is installed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntrinsicHolder {
    /// `%Object.prototype%`.
    ObjectPrototype,
    /// `%String.prototype%`.
    StringPrototype,
}

impl Intrinsic {
    /// Every intrinsic, in the order the Realm allocates them.
    pub const ALL: [Self; 22] = [
        Self::ObjectPrototypeHasOwnProperty,
        Self::ObjectPrototypeIsPrototypeOf,
        Self::ObjectPrototypePropertyIsEnumerable,
        Self::ObjectPrototypeToString,
        Self::StringPrototypeCharAt,
        Self::StringPrototypeCharCodeAt,
        Self::StringPrototypeIndexOf,
        Self::StringPrototypeAt,
        Self::StringPrototypeConcat,
        Self::StringPrototypeEndsWith,
        Self::StringPrototypeIncludes,
        Self::StringPrototypeLastIndexOf,
        Self::StringPrototypeRepeat,
        Self::StringPrototypeSlice,
        Self::StringPrototypeStartsWith,
        Self::StringPrototypeSubstring,
        Self::StringPrototypeCodePointAt,
        Self::StringPrototypePadEnd,
        Self::StringPrototypePadStart,
        Self::StringPrototypeTrim,
        Self::StringPrototypeTrimEnd,
        Self::StringPrototypeTrimStart,
    ];

    /// The intrinsic object this function is installed on.
    #[must_use]
    pub const fn holder(self) -> IntrinsicHolder {
        match self {
            Self::ObjectPrototypeHasOwnProperty
            | Self::ObjectPrototypeIsPrototypeOf
            | Self::ObjectPrototypePropertyIsEnumerable
            | Self::ObjectPrototypeToString => IntrinsicHolder::ObjectPrototype,
            Self::StringPrototypeCharAt
            | Self::StringPrototypeCharCodeAt
            | Self::StringPrototypeIndexOf
            | Self::StringPrototypeAt
            | Self::StringPrototypeConcat
            | Self::StringPrototypeEndsWith
            | Self::StringPrototypeIncludes
            | Self::StringPrototypeLastIndexOf
            | Self::StringPrototypeRepeat
            | Self::StringPrototypeSlice
            | Self::StringPrototypeStartsWith
            | Self::StringPrototypeSubstring
            | Self::StringPrototypeCodePointAt
            | Self::StringPrototypePadEnd
            | Self::StringPrototypePadStart
            | Self::StringPrototypeTrim
            | Self::StringPrototypeTrimEnd
            | Self::StringPrototypeTrimStart => IntrinsicHolder::StringPrototype,
        }
    }

    /// The identifier carried by the function object.
    #[must_use]
    pub const fn id(self) -> u32 {
        match self {
            Self::ObjectPrototypeHasOwnProperty => 0,
            Self::ObjectPrototypeIsPrototypeOf => 1,
            Self::ObjectPrototypePropertyIsEnumerable => 2,
            Self::ObjectPrototypeToString => 3,
            Self::StringPrototypeCharAt => 4,
            Self::StringPrototypeCharCodeAt => 5,
            Self::StringPrototypeIndexOf => 6,
            Self::StringPrototypeAt => 7,
            Self::StringPrototypeConcat => 8,
            Self::StringPrototypeEndsWith => 9,
            Self::StringPrototypeIncludes => 10,
            Self::StringPrototypeLastIndexOf => 11,
            Self::StringPrototypeRepeat => 12,
            Self::StringPrototypeSlice => 13,
            Self::StringPrototypeStartsWith => 14,
            Self::StringPrototypeSubstring => 15,
            Self::StringPrototypeCodePointAt => 16,
            Self::StringPrototypePadEnd => 17,
            Self::StringPrototypePadStart => 18,
            Self::StringPrototypeTrim => 19,
            Self::StringPrototypeTrimEnd => 20,
            Self::StringPrototypeTrimStart => 21,
        }
    }

    /// Index of this intrinsic in [`Self::ALL`].
    const fn index(self) -> usize {
        match self {
            Self::ObjectPrototypeHasOwnProperty => 0,
            Self::ObjectPrototypeIsPrototypeOf => 1,
            Self::ObjectPrototypePropertyIsEnumerable => 2,
            Self::ObjectPrototypeToString => 3,
            Self::StringPrototypeCharAt => 4,
            Self::StringPrototypeCharCodeAt => 5,
            Self::StringPrototypeIndexOf => 6,
            Self::StringPrototypeAt => 7,
            Self::StringPrototypeConcat => 8,
            Self::StringPrototypeEndsWith => 9,
            Self::StringPrototypeIncludes => 10,
            Self::StringPrototypeLastIndexOf => 11,
            Self::StringPrototypeRepeat => 12,
            Self::StringPrototypeSlice => 13,
            Self::StringPrototypeStartsWith => 14,
            Self::StringPrototypeSubstring => 15,
            Self::StringPrototypeCodePointAt => 16,
            Self::StringPrototypePadEnd => 17,
            Self::StringPrototypePadStart => 18,
            Self::StringPrototypeTrim => 19,
            Self::StringPrototypeTrimEnd => 20,
            Self::StringPrototypeTrimStart => 21,
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
            4 => Some(Self::StringPrototypeCharAt),
            5 => Some(Self::StringPrototypeCharCodeAt),
            6 => Some(Self::StringPrototypeIndexOf),
            7 => Some(Self::StringPrototypeAt),
            8 => Some(Self::StringPrototypeConcat),
            9 => Some(Self::StringPrototypeEndsWith),
            10 => Some(Self::StringPrototypeIncludes),
            11 => Some(Self::StringPrototypeLastIndexOf),
            12 => Some(Self::StringPrototypeRepeat),
            13 => Some(Self::StringPrototypeSlice),
            14 => Some(Self::StringPrototypeStartsWith),
            15 => Some(Self::StringPrototypeSubstring),
            16 => Some(Self::StringPrototypeCodePointAt),
            17 => Some(Self::StringPrototypePadEnd),
            18 => Some(Self::StringPrototypePadStart),
            19 => Some(Self::StringPrototypeTrim),
            20 => Some(Self::StringPrototypeTrimEnd),
            21 => Some(Self::StringPrototypeTrimStart),
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
            Self::StringPrototypeCharAt => "charAt",
            Self::StringPrototypeCharCodeAt => "charCodeAt",
            Self::StringPrototypeIndexOf => "indexOf",
            Self::StringPrototypeAt => "at",
            Self::StringPrototypeConcat => "concat",
            Self::StringPrototypeEndsWith => "endsWith",
            Self::StringPrototypeIncludes => "includes",
            Self::StringPrototypeLastIndexOf => "lastIndexOf",
            Self::StringPrototypeRepeat => "repeat",
            Self::StringPrototypeSlice => "slice",
            Self::StringPrototypeStartsWith => "startsWith",
            Self::StringPrototypeSubstring => "substring",
            Self::StringPrototypeCodePointAt => "codePointAt",
            Self::StringPrototypePadEnd => "padEnd",
            Self::StringPrototypePadStart => "padStart",
            Self::StringPrototypeTrim => "trim",
            Self::StringPrototypeTrimEnd => "trimEnd",
            Self::StringPrototypeTrimStart => "trimStart",
        }
    }

    /// The `length` property of the function object (17).
    #[must_use]
    pub const fn length(self) -> u32 {
        match self {
            Self::ObjectPrototypeToString
            | Self::StringPrototypeTrim
            | Self::StringPrototypeTrimEnd
            | Self::StringPrototypeTrimStart => 0,
            Self::ObjectPrototypeHasOwnProperty
            | Self::ObjectPrototypeIsPrototypeOf
            | Self::ObjectPrototypePropertyIsEnumerable
            | Self::StringPrototypeCharAt
            | Self::StringPrototypeCharCodeAt
            | Self::StringPrototypeIndexOf
            | Self::StringPrototypeAt
            | Self::StringPrototypeConcat
            | Self::StringPrototypeEndsWith
            | Self::StringPrototypeIncludes
            | Self::StringPrototypeLastIndexOf
            | Self::StringPrototypeRepeat
            | Self::StringPrototypeStartsWith
            | Self::StringPrototypeCodePointAt
            | Self::StringPrototypePadEnd
            | Self::StringPrototypePadStart => 1,
            Self::StringPrototypeSlice | Self::StringPrototypeSubstring => 2,
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

/// The property names `%String.prototype%` owns: 22.1.3, and the legacy string
/// HTML methods and `substr` of B.2.2 and B.2.3.
///
/// It serves the purpose [`OBJECT_PROTOTYPE_PROPERTIES`] serves: a String read
/// of one of these names is answered by the Prototype Chain.
pub const STRING_PROTOTYPE_PROPERTIES: [&str; 49] = [
    "at",
    "charAt",
    "charCodeAt",
    "codePointAt",
    "concat",
    "constructor",
    "endsWith",
    "includes",
    "indexOf",
    "isWellFormed",
    "lastIndexOf",
    "localeCompare",
    "match",
    "matchAll",
    "normalize",
    "padEnd",
    "padStart",
    "repeat",
    "replace",
    "replaceAll",
    "search",
    "slice",
    "split",
    "startsWith",
    "substring",
    "toLocaleLowerCase",
    "toLocaleUpperCase",
    "toLowerCase",
    "toString",
    "toUpperCase",
    "toWellFormed",
    "trim",
    "trimEnd",
    "trimStart",
    "valueOf",
    "substr",
    "anchor",
    "big",
    "blink",
    "bold",
    "fixed",
    "fontcolor",
    "fontsize",
    "italics",
    "link",
    "small",
    "strike",
    "sub",
    "sup",
];

/// Whether `%String.prototype%` or `%Object.prototype%` owns a property of this
/// name, which a String resolves on its Prototype Chain.
#[must_use]
pub fn string_prototype_owns(name: &[u16]) -> bool {
    object_prototype_owns(name)
        || STRING_PROTOTYPE_PROPERTIES
            .into_iter()
            .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

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
    string_prototype: Root,
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

        // 22.1.3: %String.prototype% is a String exotic object whose
        // [[Prototype]] is %Object.prototype%.
        let string_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let string_prototype = heap.push_root(Value::from_object(string_prototype))?;

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
            let holder = match intrinsic.holder() {
                IntrinsicHolder::ObjectPrototype => Self::rooted(heap, object_prototype)?,
                IntrinsicHolder::StringPrototype => Self::rooted(heap, string_prototype)?,
            }
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
            let key = PropertyKey::String(heap.strings.intern(intrinsic.name())?);
            heap.define_own_named(holder, key, Value::from_object(function), builtin_data())?;
        }

        Ok(Self {
            object_prototype,
            function_prototype,
            array_prototype,
            string_prototype,
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

    /// %String.prototype%.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn string_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.string_prototype)
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

    /// The native error type an object is an instance of, by its prototype.
    ///
    /// The embedding boundary needs this: an error that leaves the engine has
    /// to reach the host as an error of the same type.
    #[must_use]
    pub fn native_error_kind(
        &self,
        heap: &GenerationalHeap,
        object: ObjectRef,
    ) -> Option<NativeErrorKind> {
        let prototype = heap.get_object(object)?.prototype;
        NativeErrorKind::ALL
            .into_iter()
            .find(|kind| self.native_error_prototype(heap, *kind).ok() == Some(prototype))
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

fn intern(heap: &mut GenerationalHeap, name: &str) -> Result<PropertyKey, HeapError> {
    Ok(PropertyKey::String(heap.strings.intern(name)?))
}

#[cfg(test)]
mod tests {
    use super::{NativeErrorKind, Realm, WellKnownSymbol, builtin_data};
    use crate::engine::{agent::Agent, heap::GenerationalHeap, value::PropertyKey, value::Value};

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
        let name_key = PropertyKey::String(heap.strings.intern("name").unwrap());
        let message_key = PropertyKey::String(heap.strings.intern("message").unwrap());

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

        let name_key = PropertyKey::String(heap.strings.intern("name").unwrap());
        let inherited = heap.lookup_named(error, name_key).unwrap().unwrap();
        assert_eq!(inherited.holder_depth, 1);
        assert_eq!(text(&heap, inherited.value), "TypeError");

        // 20.5.6.1.1 step 3: "message" is an own non-enumerable property.
        let message_key = PropertyKey::String(heap.strings.intern("message").unwrap());
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

    #[test]
    fn a_symbol_key_is_distinct_from_every_string_key() {
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let object = realm.ordinary_object(&mut heap).unwrap();

        // 6.1.5.1: each well-known Symbol is its own key, and none of them is
        // the String of its description.
        let tag = WellKnownSymbol::ToStringTag.key();
        let described = PropertyKey::String(
            heap.strings
                .intern(WellKnownSymbol::ToStringTag.description())
                .unwrap(),
        );
        assert_ne!(tag, described);
        heap.define_own_named(object, tag, Value::from_smi(1), builtin_data())
            .unwrap();
        assert!(heap.own_named_flags(object, tag).unwrap().is_some());
        assert!(heap.own_named_flags(object, described).unwrap().is_none());

        // 10.1.11.1 lists every Symbol key after every String key, and a
        // `for`-`in` enumeration reaches none of them.
        let name = PropertyKey::String(heap.strings.intern("a").unwrap());
        heap.define_own_named(object, name, Value::from_smi(2), builtin_data())
            .unwrap();
        let keys = heap.own_keys(object).unwrap();
        assert_eq!(
            keys.iter()
                .map(|(key, _)| *key)
                .collect::<alloc::vec::Vec<_>>(),
            alloc::vec![name, tag]
        );
    }

    #[test]
    fn every_well_known_symbol_round_trips_through_its_reference() {
        for symbol in WellKnownSymbol::ALL {
            assert_eq!(
                WellKnownSymbol::from_reference(symbol.reference()),
                Some(symbol)
            );
            assert!(symbol.description().starts_with("Symbol."));
        }
    }
}
