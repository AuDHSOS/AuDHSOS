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
    value::{ObjectRef, VALUE_NULL, VALUE_UNDEFINED, VALUE_UNINITIALIZED, Value},
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

/// The property names clause 19 gives the global object of every Realm.
///
/// This Realm carries only the few of them the engine has built. A name of
/// this list that it does not carry is a gap in the migration, not a name the
/// program failed to define, and 9.1.1.2.7 must not report it as one.
pub const GLOBAL_PROPERTIES: [&str; 62] = [
    "AggregateError",
    "Array",
    "ArrayBuffer",
    "AsyncDisposableStack",
    "Atomics",
    "BigInt",
    "BigInt64Array",
    "BigUint64Array",
    "Boolean",
    "DataView",
    "Date",
    "DisposableStack",
    "Error",
    "EvalError",
    "FinalizationRegistry",
    "Float16Array",
    "Float32Array",
    "Float64Array",
    "Function",
    "Infinity",
    "Int16Array",
    "Int32Array",
    "Int8Array",
    "Iterator",
    "JSON",
    "Map",
    "Math",
    "NaN",
    "Number",
    "Object",
    "Promise",
    "Proxy",
    "RangeError",
    "ReferenceError",
    "Reflect",
    "RegExp",
    "Set",
    "SharedArrayBuffer",
    "String",
    "SuppressedError",
    "Symbol",
    "SyntaxError",
    "TypeError",
    "URIError",
    "Uint16Array",
    "Uint32Array",
    "Uint8Array",
    "Uint8ClampedArray",
    "WeakMap",
    "WeakRef",
    "WeakSet",
    "decodeURI",
    "decodeURIComponent",
    "encodeURI",
    "encodeURIComponent",
    "eval",
    "globalThis",
    "isFinite",
    "isNaN",
    "parseFloat",
    "parseInt",
    "undefined",
];

/// Whether clause 19 gives the global object a property of this name.
#[must_use]
pub fn global_properties_own(name: &[u16]) -> bool {
    GLOBAL_PROPERTIES
        .into_iter()
        .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

/// Whether an implemented intrinsic of `%Object.prototype%` has this name.
#[must_use]
pub fn object_prototype_intrinsic(name: &[u16]) -> Option<Intrinsic> {
    holder_intrinsic(IntrinsicHolder::ObjectPrototype, name)
}

/// Whether an implemented intrinsic of `%Array.prototype%` has this name.
#[must_use]
pub fn array_prototype_intrinsic(name: &[u16]) -> Option<Intrinsic> {
    holder_intrinsic(IntrinsicHolder::ArrayPrototype, name)
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
    /// `Array.prototype.values`, which is also `%Array.prototype%[@@iterator]`
    /// (23.1.3.38 and 23.1.3.40).
    ArrayPrototypeValues,
    /// `%ArrayIteratorPrototype%.next` (23.1.5.2.1).
    ArrayIteratorPrototypeNext,
    /// `Array.prototype.at` (23.1.3.1).
    ArrayPrototypeAt,
    /// `Array.prototype.includes` (23.1.3.16).
    ArrayPrototypeIncludes,
    /// `Array.prototype.indexOf` (23.1.3.17).
    ArrayPrototypeIndexOf,
    /// `Array.prototype.lastIndexOf` (23.1.3.20).
    ArrayPrototypeLastIndexOf,
    /// `Array.prototype.join` (23.1.3.18).
    ArrayPrototypeJoin,
    /// `Array.prototype.pop` (23.1.3.22).
    ArrayPrototypePop,
    /// `Array.prototype.push` (23.1.3.23).
    ArrayPrototypePush,
    /// `Array.prototype.reverse` (23.1.3.26).
    ArrayPrototypeReverse,
    /// `Array.prototype.slice` (23.1.3.28).
    ArrayPrototypeSlice,
    /// `Array.prototype.toString` (23.1.3.37).
    ArrayPrototypeToString,
    /// The `Array` constructor `%Array%` (23.1.1.1).
    ArrayConstructor,
    /// `Array.isArray` (23.1.2.3).
    ArrayIsArray,
    /// The `Object` constructor `%Object%` (20.1.1.1).
    ObjectConstructor,
    /// `Object.defineProperty` (20.1.2.4).
    ObjectDefineProperty,
    /// `Object.getOwnPropertyDescriptor` (20.1.2.8).
    ObjectGetOwnPropertyDescriptor,
    /// `Object.getOwnPropertyNames` (20.1.2.10).
    ObjectGetOwnPropertyNames,
    /// The `Function` constructor `%Function%` (20.2.1.1).
    FunctionConstructor,
    /// `Function.prototype.call` (20.2.3.3).
    FunctionPrototypeCall,
    /// `Function.prototype.bind` (20.2.3.2).
    FunctionPrototypeBind,
    /// `Math.pow` (21.3.2.26).
    MathPow,
    /// The `Error` constructor `%Error%` (20.5.1.1).
    ErrorConstructor,
    /// The `EvalError` constructor (20.5.6.1.1).
    EvalErrorConstructor,
    /// The `RangeError` constructor (20.5.6.1.1).
    RangeErrorConstructor,
    /// The `ReferenceError` constructor (20.5.6.1.1).
    ReferenceErrorConstructor,
    /// The `SyntaxError` constructor (20.5.6.1.1).
    SyntaxErrorConstructor,
    /// The `TypeError` constructor (20.5.6.1.1).
    TypeErrorConstructor,
    /// The `URIError` constructor (20.5.6.1.1).
    UriErrorConstructor,
    /// The `String` constructor `%String%` (22.1.1.1).
    StringConstructor,
    /// `Object.create` (20.1.2.2).
    ObjectCreate,
    /// `Object.defineProperties` (20.1.2.3).
    ObjectDefineProperties,
    /// `Object.getPrototypeOf` (20.1.2.12).
    ObjectGetPrototypeOf,
    /// `Object.keys` (20.1.2.19).
    ObjectKeys,
    /// `Object.is` (20.1.2.14).
    ObjectIs,
    /// `Object.hasOwn` (20.1.2.13).
    ObjectHasOwn,
    /// `Array.prototype.shift` (23.1.3.27).
    ArrayPrototypeShift,
    /// `Array.prototype.unshift` (23.1.3.37).
    ArrayPrototypeUnshift,
    /// `Array.prototype.splice` (23.1.3.31).
    ArrayPrototypeSplice,
    /// `Array.prototype.fill` (23.1.3.7).
    ArrayPrototypeFill,
    /// `Array.prototype.copyWithin` (23.1.3.4).
    ArrayPrototypeCopyWithin,
    /// `Array.prototype.concat` (23.1.3.2).
    ArrayPrototypeConcat,
    /// `Array.prototype.with` (23.1.3.39).
    ArrayPrototypeWith,
    /// `Array.prototype.toReversed` (23.1.3.33).
    ArrayPrototypeToReversed,
    /// `Math.abs` (21.3.2.1).
    MathAbs,
    /// `Math.ceil` (21.3.2.10).
    MathCeil,
    /// `Math.floor` (21.3.2.16).
    MathFloor,
    /// `Math.trunc` (21.3.2.35).
    MathTrunc,
    /// `Math.round` (21.3.2.28).
    MathRound,
    /// `Math.sign` (21.3.2.29).
    MathSign,
    /// `Math.max` (21.3.2.24).
    MathMax,
    /// `Math.min` (21.3.2.25).
    MathMin,
    /// `Math.clz32` (21.3.2.11).
    MathClz32,
    /// `Math.imul` (21.3.2.19).
    MathImul,
    /// `Math.fround` (21.3.2.17).
    MathFround,
    /// `Math.sin` (21.3.2.30).
    MathSin,
}

/// The intrinsic object a native function is installed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntrinsicHolder {
    /// `%Object.prototype%`.
    ObjectPrototype,
    /// `%String.prototype%`.
    StringPrototype,
    /// `%Array.prototype%`.
    ArrayPrototype,
    /// `%ArrayIteratorPrototype%`.
    ArrayIteratorPrototype,
    /// The global object, which 19.1 gives the constructors of clause 20 and
    /// after.
    Global,
    /// `%Array%`, which carries the functions 23.1.2 gives the constructor.
    ArrayConstructor,
    /// `%Object%`, which carries the functions 20.1.2 gives the constructor.
    ObjectConstructor,
    /// `%Function.prototype%`, which carries the methods 20.2.3 gives every
    /// function.
    FunctionPrototype,
    /// `%Math%`, the namespace object of 21.3.
    Math,
}

impl Intrinsic {
    /// Every intrinsic, in the order the Realm allocates them.
    pub const ALL: [Self; 78] = [
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
        Self::ArrayPrototypeValues,
        Self::ArrayIteratorPrototypeNext,
        Self::ArrayPrototypeAt,
        Self::ArrayPrototypeIncludes,
        Self::ArrayPrototypeIndexOf,
        Self::ArrayPrototypeLastIndexOf,
        Self::ArrayPrototypeJoin,
        Self::ArrayPrototypePop,
        Self::ArrayPrototypePush,
        Self::ArrayPrototypeReverse,
        Self::ArrayPrototypeSlice,
        Self::ArrayPrototypeToString,
        Self::ArrayConstructor,
        Self::ArrayIsArray,
        Self::ObjectConstructor,
        Self::ObjectDefineProperty,
        Self::ObjectGetOwnPropertyDescriptor,
        Self::ObjectGetOwnPropertyNames,
        Self::FunctionConstructor,
        Self::FunctionPrototypeCall,
        Self::FunctionPrototypeBind,
        Self::MathPow,
        Self::ErrorConstructor,
        Self::EvalErrorConstructor,
        Self::RangeErrorConstructor,
        Self::ReferenceErrorConstructor,
        Self::SyntaxErrorConstructor,
        Self::TypeErrorConstructor,
        Self::UriErrorConstructor,
        Self::StringConstructor,
        Self::ObjectCreate,
        Self::ObjectDefineProperties,
        Self::ObjectGetPrototypeOf,
        Self::ObjectKeys,
        Self::ObjectIs,
        Self::ObjectHasOwn,
        Self::ArrayPrototypeShift,
        Self::ArrayPrototypeUnshift,
        Self::ArrayPrototypeSplice,
        Self::ArrayPrototypeFill,
        Self::ArrayPrototypeCopyWithin,
        Self::ArrayPrototypeConcat,
        Self::ArrayPrototypeWith,
        Self::ArrayPrototypeToReversed,
        Self::MathAbs,
        Self::MathCeil,
        Self::MathFloor,
        Self::MathTrunc,
        Self::MathRound,
        Self::MathSign,
        Self::MathMax,
        Self::MathMin,
        Self::MathClz32,
        Self::MathImul,
        Self::MathFround,
        Self::MathSin,
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
            Self::ArrayPrototypeValues
            | Self::ArrayPrototypeAt
            | Self::ArrayPrototypeIncludes
            | Self::ArrayPrototypeIndexOf
            | Self::ArrayPrototypeLastIndexOf
            | Self::ArrayPrototypeJoin
            | Self::ArrayPrototypePop
            | Self::ArrayPrototypePush
            | Self::ArrayPrototypeReverse
            | Self::ArrayPrototypeSlice
            | Self::ArrayPrototypeToString
            | Self::ArrayPrototypeShift
            | Self::ArrayPrototypeUnshift
            | Self::ArrayPrototypeSplice
            | Self::ArrayPrototypeFill
            | Self::ArrayPrototypeCopyWithin
            | Self::ArrayPrototypeConcat
            | Self::ArrayPrototypeWith
            | Self::ArrayPrototypeToReversed => IntrinsicHolder::ArrayPrototype,
            Self::ArrayIteratorPrototypeNext => IntrinsicHolder::ArrayIteratorPrototype,
            Self::ArrayConstructor | Self::ObjectConstructor | Self::FunctionConstructor => {
                IntrinsicHolder::Global
            }
            Self::FunctionPrototypeCall | Self::FunctionPrototypeBind => {
                IntrinsicHolder::FunctionPrototype
            }
            Self::MathPow
            | Self::MathAbs
            | Self::MathCeil
            | Self::MathFloor
            | Self::MathTrunc
            | Self::MathRound
            | Self::MathSign
            | Self::MathMax
            | Self::MathMin
            | Self::MathClz32
            | Self::MathImul
            | Self::MathFround
            | Self::MathSin => IntrinsicHolder::Math,
            Self::ErrorConstructor
            | Self::EvalErrorConstructor
            | Self::RangeErrorConstructor
            | Self::ReferenceErrorConstructor
            | Self::SyntaxErrorConstructor
            | Self::TypeErrorConstructor
            | Self::UriErrorConstructor
            | Self::StringConstructor => IntrinsicHolder::Global,
            Self::ArrayIsArray => IntrinsicHolder::ArrayConstructor,
            Self::ObjectDefineProperty
            | Self::ObjectGetOwnPropertyDescriptor
            | Self::ObjectGetOwnPropertyNames
            | Self::ObjectCreate
            | Self::ObjectDefineProperties
            | Self::ObjectGetPrototypeOf
            | Self::ObjectKeys
            | Self::ObjectIs
            | Self::ObjectHasOwn => IntrinsicHolder::ObjectConstructor,
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
            Self::ArrayPrototypeValues => 22,
            Self::ArrayIteratorPrototypeNext => 23,
            Self::ArrayPrototypeAt => 24,
            Self::ArrayPrototypeIncludes => 25,
            Self::ArrayPrototypeIndexOf => 26,
            Self::ArrayPrototypeLastIndexOf => 27,
            Self::ArrayPrototypeJoin => 28,
            Self::ArrayPrototypePop => 29,
            Self::ArrayPrototypePush => 30,
            Self::ArrayPrototypeReverse => 31,
            Self::ArrayPrototypeSlice => 32,
            Self::ArrayPrototypeToString => 33,
            Self::ArrayConstructor => 34,
            Self::ArrayIsArray => 35,
            Self::ObjectConstructor => 36,
            Self::ObjectDefineProperty => 37,
            Self::ObjectGetOwnPropertyDescriptor => 38,
            Self::ObjectGetOwnPropertyNames => 39,
            Self::FunctionConstructor => 40,
            Self::FunctionPrototypeCall => 41,
            Self::FunctionPrototypeBind => 42,
            Self::MathPow => 43,
            Self::ErrorConstructor => 44,
            Self::EvalErrorConstructor => 45,
            Self::RangeErrorConstructor => 46,
            Self::ReferenceErrorConstructor => 47,
            Self::SyntaxErrorConstructor => 48,
            Self::TypeErrorConstructor => 49,
            Self::UriErrorConstructor => 50,
            Self::StringConstructor => 51,
            Self::ObjectCreate => 52,
            Self::ObjectDefineProperties => 53,
            Self::ObjectGetPrototypeOf => 54,
            Self::ObjectKeys => 55,
            Self::ObjectIs => 56,
            Self::ObjectHasOwn => 57,
            Self::ArrayPrototypeShift => 58,
            Self::ArrayPrototypeUnshift => 59,
            Self::ArrayPrototypeSplice => 60,
            Self::ArrayPrototypeFill => 61,
            Self::ArrayPrototypeCopyWithin => 62,
            Self::ArrayPrototypeConcat => 63,
            Self::ArrayPrototypeWith => 64,
            Self::ArrayPrototypeToReversed => 65,
            Self::MathAbs => 66,
            Self::MathCeil => 67,
            Self::MathFloor => 68,
            Self::MathTrunc => 69,
            Self::MathRound => 70,
            Self::MathSign => 71,
            Self::MathMax => 72,
            Self::MathMin => 73,
            Self::MathClz32 => 74,
            Self::MathImul => 75,
            Self::MathFround => 76,
            Self::MathSin => 77,
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
            Self::ArrayPrototypeValues => 22,
            Self::ArrayIteratorPrototypeNext => 23,
            Self::ArrayPrototypeAt => 24,
            Self::ArrayPrototypeIncludes => 25,
            Self::ArrayPrototypeIndexOf => 26,
            Self::ArrayPrototypeLastIndexOf => 27,
            Self::ArrayPrototypeJoin => 28,
            Self::ArrayPrototypePop => 29,
            Self::ArrayPrototypePush => 30,
            Self::ArrayPrototypeReverse => 31,
            Self::ArrayPrototypeSlice => 32,
            Self::ArrayPrototypeToString => 33,
            Self::ArrayConstructor => 34,
            Self::ArrayIsArray => 35,
            Self::ObjectConstructor => 36,
            Self::ObjectDefineProperty => 37,
            Self::ObjectGetOwnPropertyDescriptor => 38,
            Self::ObjectGetOwnPropertyNames => 39,
            Self::FunctionConstructor => 40,
            Self::FunctionPrototypeCall => 41,
            Self::FunctionPrototypeBind => 42,
            Self::MathPow => 43,
            Self::ErrorConstructor => 44,
            Self::EvalErrorConstructor => 45,
            Self::RangeErrorConstructor => 46,
            Self::ReferenceErrorConstructor => 47,
            Self::SyntaxErrorConstructor => 48,
            Self::TypeErrorConstructor => 49,
            Self::UriErrorConstructor => 50,
            Self::StringConstructor => 51,
            Self::ObjectCreate => 52,
            Self::ObjectDefineProperties => 53,
            Self::ObjectGetPrototypeOf => 54,
            Self::ObjectKeys => 55,
            Self::ObjectIs => 56,
            Self::ObjectHasOwn => 57,
            Self::ArrayPrototypeShift => 58,
            Self::ArrayPrototypeUnshift => 59,
            Self::ArrayPrototypeSplice => 60,
            Self::ArrayPrototypeFill => 61,
            Self::ArrayPrototypeCopyWithin => 62,
            Self::ArrayPrototypeConcat => 63,
            Self::ArrayPrototypeWith => 64,
            Self::ArrayPrototypeToReversed => 65,
            Self::MathAbs => 66,
            Self::MathCeil => 67,
            Self::MathFloor => 68,
            Self::MathTrunc => 69,
            Self::MathRound => 70,
            Self::MathSign => 71,
            Self::MathMax => 72,
            Self::MathMin => 73,
            Self::MathClz32 => 74,
            Self::MathImul => 75,
            Self::MathFround => 76,
            Self::MathSin => 77,
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
            22 => Some(Self::ArrayPrototypeValues),
            23 => Some(Self::ArrayIteratorPrototypeNext),
            24 => Some(Self::ArrayPrototypeAt),
            25 => Some(Self::ArrayPrototypeIncludes),
            26 => Some(Self::ArrayPrototypeIndexOf),
            27 => Some(Self::ArrayPrototypeLastIndexOf),
            28 => Some(Self::ArrayPrototypeJoin),
            29 => Some(Self::ArrayPrototypePop),
            30 => Some(Self::ArrayPrototypePush),
            31 => Some(Self::ArrayPrototypeReverse),
            32 => Some(Self::ArrayPrototypeSlice),
            33 => Some(Self::ArrayPrototypeToString),
            34 => Some(Self::ArrayConstructor),
            35 => Some(Self::ArrayIsArray),
            36 => Some(Self::ObjectConstructor),
            37 => Some(Self::ObjectDefineProperty),
            38 => Some(Self::ObjectGetOwnPropertyDescriptor),
            39 => Some(Self::ObjectGetOwnPropertyNames),
            40 => Some(Self::FunctionConstructor),
            41 => Some(Self::FunctionPrototypeCall),
            42 => Some(Self::FunctionPrototypeBind),
            43 => Some(Self::MathPow),
            44 => Some(Self::ErrorConstructor),
            45 => Some(Self::EvalErrorConstructor),
            46 => Some(Self::RangeErrorConstructor),
            47 => Some(Self::ReferenceErrorConstructor),
            48 => Some(Self::SyntaxErrorConstructor),
            49 => Some(Self::TypeErrorConstructor),
            50 => Some(Self::UriErrorConstructor),
            51 => Some(Self::StringConstructor),
            52 => Some(Self::ObjectCreate),
            53 => Some(Self::ObjectDefineProperties),
            54 => Some(Self::ObjectGetPrototypeOf),
            55 => Some(Self::ObjectKeys),
            56 => Some(Self::ObjectIs),
            57 => Some(Self::ObjectHasOwn),
            58 => Some(Self::ArrayPrototypeShift),
            59 => Some(Self::ArrayPrototypeUnshift),
            60 => Some(Self::ArrayPrototypeSplice),
            61 => Some(Self::ArrayPrototypeFill),
            62 => Some(Self::ArrayPrototypeCopyWithin),
            63 => Some(Self::ArrayPrototypeConcat),
            64 => Some(Self::ArrayPrototypeWith),
            65 => Some(Self::ArrayPrototypeToReversed),
            66 => Some(Self::MathAbs),
            67 => Some(Self::MathCeil),
            68 => Some(Self::MathFloor),
            69 => Some(Self::MathTrunc),
            70 => Some(Self::MathRound),
            71 => Some(Self::MathSign),
            72 => Some(Self::MathMax),
            73 => Some(Self::MathMin),
            74 => Some(Self::MathClz32),
            75 => Some(Self::MathImul),
            76 => Some(Self::MathFround),
            77 => Some(Self::MathSin),
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
            Self::ObjectPrototypeToString | Self::ArrayPrototypeToString => "toString",
            Self::ArrayConstructor => "Array",
            Self::ObjectConstructor => "Object",
            Self::FunctionConstructor => "Function",
            Self::FunctionPrototypeCall => "call",
            Self::FunctionPrototypeBind => "bind",
            Self::MathPow => "pow",
            Self::ErrorConstructor => "Error",
            Self::EvalErrorConstructor => "EvalError",
            Self::RangeErrorConstructor => "RangeError",
            Self::ReferenceErrorConstructor => "ReferenceError",
            Self::SyntaxErrorConstructor => "SyntaxError",
            Self::TypeErrorConstructor => "TypeError",
            Self::UriErrorConstructor => "URIError",
            Self::StringConstructor => "String",
            Self::ObjectCreate => "create",
            Self::ObjectDefineProperties => "defineProperties",
            Self::ObjectGetPrototypeOf => "getPrototypeOf",
            Self::ObjectKeys => "keys",
            Self::ObjectIs => "is",
            Self::ObjectHasOwn => "hasOwn",
            Self::ArrayPrototypeShift => "shift",
            Self::ArrayPrototypeUnshift => "unshift",
            Self::ArrayPrototypeSplice => "splice",
            Self::ArrayPrototypeFill => "fill",
            Self::ArrayPrototypeCopyWithin => "copyWithin",
            Self::ArrayPrototypeWith => "with",
            Self::ArrayPrototypeToReversed => "toReversed",
            Self::MathAbs => "abs",
            Self::MathCeil => "ceil",
            Self::MathFloor => "floor",
            Self::MathTrunc => "trunc",
            Self::MathRound => "round",
            Self::MathSign => "sign",
            Self::MathMax => "max",
            Self::MathMin => "min",
            Self::MathClz32 => "clz32",
            Self::MathImul => "imul",
            Self::MathFround => "fround",
            Self::MathSin => "sin",
            Self::ObjectDefineProperty => "defineProperty",
            Self::ObjectGetOwnPropertyDescriptor => "getOwnPropertyDescriptor",
            Self::ObjectGetOwnPropertyNames => "getOwnPropertyNames",
            Self::ArrayIsArray => "isArray",
            Self::StringPrototypeCharAt => "charAt",
            Self::StringPrototypeCharCodeAt => "charCodeAt",
            Self::StringPrototypeIndexOf | Self::ArrayPrototypeIndexOf => "indexOf",
            Self::StringPrototypeAt | Self::ArrayPrototypeAt => "at",
            Self::StringPrototypeConcat | Self::ArrayPrototypeConcat => "concat",
            Self::StringPrototypeEndsWith => "endsWith",
            Self::StringPrototypeIncludes | Self::ArrayPrototypeIncludes => "includes",
            Self::StringPrototypeLastIndexOf | Self::ArrayPrototypeLastIndexOf => "lastIndexOf",
            Self::StringPrototypeRepeat => "repeat",
            Self::StringPrototypeStartsWith => "startsWith",
            Self::StringPrototypeSubstring => "substring",
            Self::StringPrototypeCodePointAt => "codePointAt",
            Self::StringPrototypePadEnd => "padEnd",
            Self::StringPrototypePadStart => "padStart",
            Self::StringPrototypeTrim => "trim",
            Self::StringPrototypeTrimEnd => "trimEnd",
            Self::StringPrototypeTrimStart => "trimStart",
            Self::ArrayPrototypeValues => "values",
            Self::ArrayIteratorPrototypeNext => "next",
            Self::ArrayPrototypeJoin => "join",
            Self::ArrayPrototypePop => "pop",
            Self::ArrayPrototypePush => "push",
            Self::ArrayPrototypeReverse => "reverse",
            Self::StringPrototypeSlice | Self::ArrayPrototypeSlice => "slice",
        }
    }

    /// Whether this method coerces the argument at `index` to a primitive.
    ///
    /// An intrinsic runs without a call frame, so it cannot run a user
    /// `valueOf` or `toString`. An Object in such a position is therefore not
    /// lowered; a position that only compares or stores its argument takes any
    /// value.
    #[must_use]
    pub const fn coerces_argument(self, index: u16) -> bool {
        match self {
            // 20.1.3.3 compares, 20.1.3.6 and 23.1.3.38 read no argument, and
            // 23.1.5.2.1 takes none.
            Self::ObjectPrototypeIsPrototypeOf
            | Self::ObjectPrototypeToString
            | Self::ArrayPrototypeValues
            | Self::ArrayIteratorPrototypeNext
            | Self::ArrayPrototypePop
            | Self::ArrayPrototypePush
            | Self::ArrayPrototypeReverse
            | Self::ArrayPrototypeToString
            | Self::ArrayConstructor
            | Self::ArrayIsArray
            | Self::ObjectConstructor
            | Self::FunctionConstructor
            | Self::FunctionPrototypeCall
            | Self::FunctionPrototypeBind
            | Self::MathPow
            | Self::ErrorConstructor
            | Self::EvalErrorConstructor
            | Self::RangeErrorConstructor
            | Self::ReferenceErrorConstructor
            | Self::SyntaxErrorConstructor
            | Self::TypeErrorConstructor
            | Self::UriErrorConstructor
            | Self::StringConstructor
            | Self::ObjectCreate
            | Self::ObjectDefineProperties
            | Self::ObjectGetPrototypeOf
            | Self::ObjectKeys
            | Self::ObjectIs
            | Self::ArrayPrototypeShift
            | Self::ArrayPrototypeUnshift
            | Self::ArrayPrototypeSplice
            | Self::ArrayPrototypeFill
            | Self::ArrayPrototypeCopyWithin
            | Self::ArrayPrototypeConcat
            | Self::ArrayPrototypeWith
            | Self::ArrayPrototypeToReversed
            | Self::MathAbs
            | Self::MathCeil
            | Self::MathFloor
            | Self::MathTrunc
            | Self::MathRound
            | Self::MathSign
            | Self::MathMax
            | Self::MathMin
            | Self::MathClz32
            | Self::MathImul
            | Self::MathFround
            | Self::MathSin
            | Self::ObjectGetOwnPropertyNames => false,
            // 20.1.2.4, 20.1.2.8 and 20.1.2.13 apply ToPropertyKey to the
            // second argument.
            Self::ObjectDefineProperty
            | Self::ObjectGetOwnPropertyDescriptor
            | Self::ObjectHasOwn => index == 1,
            // 20.1.3.2 and 20.1.3.4 apply ToPropertyKey to the first argument.
            Self::ObjectPrototypeHasOwnProperty | Self::ObjectPrototypePropertyIsEnumerable => {
                index == 0
            }
            // 23.1.3.16, 23.1.3.17 and 23.1.3.20 compare the search element and
            // coerce only the index that follows it.
            Self::ArrayPrototypeIncludes
            | Self::ArrayPrototypeIndexOf
            | Self::ArrayPrototypeLastIndexOf => index > 0,
            // 22.1.3 and 23.1.3.1 coerce every argument they read.
            _ => true,
        }
    }

    /// The `length` property of the function object (17).
    #[must_use]
    pub const fn length(self) -> u32 {
        match self {
            Self::ObjectPrototypeToString
            | Self::StringPrototypeTrim
            | Self::StringPrototypeTrimEnd
            | Self::StringPrototypeTrimStart
            | Self::ArrayPrototypeValues
            | Self::ArrayIteratorPrototypeNext
            | Self::ArrayPrototypePop
            | Self::ArrayPrototypeReverse
            | Self::ArrayPrototypeToString
            | Self::ArrayPrototypeShift
            | Self::ArrayPrototypeToReversed => 0,
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
            | Self::StringPrototypePadStart
            | Self::ArrayPrototypeAt
            | Self::ArrayPrototypeIncludes
            | Self::ArrayPrototypeIndexOf
            | Self::ArrayPrototypeLastIndexOf
            | Self::ArrayPrototypeJoin
            | Self::ArrayPrototypePush
            | Self::ArrayConstructor
            | Self::ArrayIsArray
            | Self::ObjectConstructor
            | Self::FunctionConstructor
            | Self::FunctionPrototypeCall
            | Self::FunctionPrototypeBind
            | Self::ErrorConstructor
            | Self::EvalErrorConstructor
            | Self::RangeErrorConstructor
            | Self::ReferenceErrorConstructor
            | Self::SyntaxErrorConstructor
            | Self::TypeErrorConstructor
            | Self::UriErrorConstructor
            | Self::StringConstructor
            | Self::ObjectGetPrototypeOf
            | Self::ObjectKeys
            | Self::ArrayPrototypeUnshift
            | Self::ArrayPrototypeFill
            | Self::ArrayPrototypeConcat
            | Self::MathAbs
            | Self::MathCeil
            | Self::MathFloor
            | Self::MathTrunc
            | Self::MathRound
            | Self::MathSign
            | Self::MathClz32
            | Self::MathFround
            | Self::MathSin
            | Self::ObjectGetOwnPropertyNames => 1,
            Self::ObjectDefineProperty => 3,
            Self::MathPow
            | Self::ObjectGetOwnPropertyDescriptor
            | Self::ObjectCreate
            | Self::ObjectDefineProperties
            | Self::ObjectIs
            | Self::ObjectHasOwn
            | Self::StringPrototypeSlice
            | Self::StringPrototypeSubstring
            | Self::ArrayPrototypeSlice
            | Self::ArrayPrototypeSplice
            | Self::ArrayPrototypeCopyWithin
            | Self::ArrayPrototypeWith
            | Self::MathMax
            | Self::MathMin
            | Self::MathImul => 2,
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
pub const STRING_PROTOTYPE_PROPERTIES: [&str; 51] = [
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
    "trimLeft",
    "trimRight",
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

/// The property names 10.2 gives an ordinary function object and 20.2.3 gives
/// `%Function.prototype%`, excluding the one Symbol key the latter carries.
///
/// It serves the same purpose as [`OBJECT_PROTOTYPE_PROPERTIES`]: neither exists
/// yet, so a read of one of these names off a function object is a gap and a
/// write of one would shadow a property that is not writable.
pub const FUNCTION_PROPERTIES: [&str; 8] = [
    "apply",
    "bind",
    "call",
    "constructor",
    "length",
    "name",
    "prototype",
    "toString",
];

/// The property names 23.1.2 gives `%Array%`, beside the ones 17 gives every
/// built-in function.
///
/// This Realm builds `isArray` and `prototype` of them; a read of one of the
/// others is a gap, because answering undefined would say the constructor
/// does not have it.
pub const ARRAY_CONSTRUCTOR_PROPERTIES: [&str; 5] =
    ["from", "fromAsync", "isArray", "of", "prototype"];

/// The property names 20.1.2 gives `%Object%`, beside the ones 17 gives every
/// built-in function.
///
/// This Realm builds `prototype`, `defineProperty`,
/// `getOwnPropertyDescriptor` and `getOwnPropertyNames` of them; a read of one
/// of the others is a gap, because answering undefined would say the
/// constructor does not have it.
pub const OBJECT_CONSTRUCTOR_PROPERTIES: [&str; 24] = [
    "assign",
    "create",
    "defineProperties",
    "defineProperty",
    "entries",
    "freeze",
    "fromEntries",
    "getOwnPropertyDescriptor",
    "getOwnPropertyDescriptors",
    "getOwnPropertyNames",
    "getOwnPropertySymbols",
    "getPrototypeOf",
    "groupBy",
    "hasOwn",
    "is",
    "isExtensible",
    "isFrozen",
    "isSealed",
    "keys",
    "preventExtensions",
    "prototype",
    "seal",
    "setPrototypeOf",
    "values",
];

/// The property names 21.3 gives `%Math%`.
///
/// This Realm builds `pow` of them; a read of one of the others is a gap,
/// because answering undefined would say the namespace does not have it.
pub const MATH_PROPERTIES: [&str; 45] = [
    "E",
    "LN10",
    "LN2",
    "LOG10E",
    "LOG2E",
    "PI",
    "SQRT1_2",
    "SQRT2",
    "abs",
    "acos",
    "acosh",
    "asin",
    "asinh",
    "atan",
    "atan2",
    "atanh",
    "cbrt",
    "ceil",
    "clz32",
    "cos",
    "cosh",
    "exp",
    "expm1",
    "floor",
    "f16round",
    "fround",
    "hypot",
    "imul",
    "log",
    "log10",
    "log1p",
    "log2",
    "max",
    "min",
    "pow",
    "random",
    "round",
    "sign",
    "sin",
    "sinh",
    "sqrt",
    "sumPrecise",
    "tan",
    "tanh",
    "trunc",
];

/// The property names 22.1.2 gives `%String%`, beside the ones 17 gives every
/// built-in function.
///
/// This Realm builds `prototype` of them; a read of one of the others is a
/// gap, because answering undefined would say the constructor does not have
/// it.
pub const STRING_CONSTRUCTOR_PROPERTIES: [&str; 4] =
    ["fromCharCode", "fromCodePoint", "prototype", "raw"];

/// Whether `%String%` owns a property of this name.
#[must_use]
pub fn string_constructor_owns(name: &[u16]) -> bool {
    function_prototype_owns(name)
        || STRING_CONSTRUCTOR_PROPERTIES
            .into_iter()
            .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

/// Whether `%Math%` owns a property of this name.
#[must_use]
pub fn math_owns(name: &[u16]) -> bool {
    MATH_PROPERTIES
        .into_iter()
        .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

/// Whether `%Object%` owns a property of this name.
#[must_use]
pub fn object_constructor_owns(name: &[u16]) -> bool {
    function_prototype_owns(name)
        || OBJECT_CONSTRUCTOR_PROPERTIES
            .into_iter()
            .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

/// Whether `%Array%` owns a property of this name.
#[must_use]
pub fn array_constructor_owns(name: &[u16]) -> bool {
    function_prototype_owns(name)
        || ARRAY_CONSTRUCTOR_PROPERTIES
            .into_iter()
            .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

/// Whether a function object or `%Function.prototype%` owns a property of this
/// name, which a function resolves on its Prototype Chain.
#[must_use]
pub fn function_prototype_owns(name: &[u16]) -> bool {
    object_prototype_owns(name)
        || FUNCTION_PROPERTIES
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

    /// The constructor 20.5.6.1 gives this kind.
    #[must_use]
    pub const fn constructor(self) -> Intrinsic {
        match self {
            Self::EvalError => Intrinsic::EvalErrorConstructor,
            Self::RangeError => Intrinsic::RangeErrorConstructor,
            Self::ReferenceError => Intrinsic::ReferenceErrorConstructor,
            Self::SyntaxError => Intrinsic::SyntaxErrorConstructor,
            Self::TypeError => Intrinsic::TypeErrorConstructor,
            Self::UriError => Intrinsic::UriErrorConstructor,
        }
    }

    /// The kind one constructor makes, if it is one.
    #[must_use]
    pub const fn of(intrinsic: Intrinsic) -> Option<Self> {
        match intrinsic {
            Intrinsic::EvalErrorConstructor => Some(Self::EvalError),
            Intrinsic::RangeErrorConstructor => Some(Self::RangeError),
            Intrinsic::ReferenceErrorConstructor => Some(Self::ReferenceError),
            Intrinsic::SyntaxErrorConstructor => Some(Self::SyntaxError),
            Intrinsic::TypeErrorConstructor => Some(Self::TypeError),
            Intrinsic::UriErrorConstructor => Some(Self::UriError),
            _ => None,
        }
    }

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
    array_iterator_prototype: Root,
    error_prototype: Root,
    native_error_prototypes: [Root; NATIVE_ERROR_COUNT],
    intrinsics: [Root; Intrinsic::ALL.len()],
    global: GlobalEnvironment,
}

/// What a binding operation of 9.1.1 refuses, for the caller to raise with its
/// own Realm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingOutcome {
    /// 9.1.1.1.5: the binding exists but has no value yet.
    Uninitialized,
    /// 9.1.1.1.5: the binding is a `const`, or the property is not writable.
    Immutable,
    /// 9.1.1.2.5: strict evaluation reached a name nothing binds.
    Unresolvable,
    /// Clause 19 gives the global object this name and this Realm has not
    /// built it yet, so its absence is a gap and not an answer.
    Missing(&'static str),
}

/// The objects the intrinsic functions of a new Realm are installed on.
struct Holders {
    object_prototype: Root,
    function_prototype: Root,
    string_prototype: Root,
    array_prototype: Root,
    array_iterator_prototype: Root,
    global_object: Root,
    math: Root,
}

/// Global Environment Record of 9.1.1.4.
///
/// The `[[DeclarativeRecord]]` is an object of the heap rather than a map beside
/// it, so that the collector reaches its bindings the way it reaches every other
/// property. It is empty until `let`, `const` and `class` create bindings in it.
pub struct GlobalEnvironment {
    /// `[[ObjectRecord]]`: the Object Environment Record whose binding object
    /// is the global object.
    object_record: Root,
    /// `[[GlobalThisValue]]`.
    this_value: Root,
    /// `[[DeclarativeRecord]]`.
    declarative_record: Root,
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

        // 23.1.5.2: %ArrayIteratorPrototype% inherits from %IteratorPrototype%,
        // which is an ordinary object of %Object.prototype% until the Iterator
        // intrinsics exist.
        let iterator_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let array_iterator_prototype =
            heap.allocate_immortal_object(root_shape, Value::from_object(iterator_prototype))?;
        let array_iterator_prototype =
            heap.push_root(Value::from_object(array_iterator_prototype))?;

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

        // 9.1.1.4 binds the global object, and 19.1 gives it the constructors
        // of clause 20 and after, so it exists before the intrinsics that are
        // installed on it.
        let global_object = heap.allocate_immortal_object(root_shape, ordinary)?;
        let global_object = heap.push_root(Value::from_object(global_object))?;
        // 21.3 is an ordinary object and not a constructor, so it is made here
        // and the functions it carries are installed on it like any other.
        let math = heap.allocate_immortal_object(root_shape, ordinary)?;
        heap.set_object_kind(math, super::object::ObjectKind::Math)?;
        Self::define_math_constants(heap, math)?;
        let math = heap.push_root(Value::from_object(math))?;
        let intrinsics = Self::install_intrinsics(
            heap,
            &Holders {
                object_prototype,
                function_prototype,
                string_prototype,
                array_prototype,
                array_iterator_prototype,
                global_object,
                math,
            },
        )?;

        for (constructor, prototype) in [
            (Intrinsic::ArrayConstructor, array_prototype),
            (Intrinsic::ObjectConstructor, object_prototype),
            (Intrinsic::FunctionConstructor, function_prototype),
            (Intrinsic::StringConstructor, string_prototype),
        ] {
            Self::pair_constructor_with_prototype(heap, &intrinsics, constructor, prototype)?;
        }
        Self::pair_errors_with_their_prototypes(
            heap,
            &intrinsics,
            error_prototype,
            &native_error_prototypes,
        )?;

        // 19.1 gives the global object `Math` with the attributes 17 gives
        // every value of clause 19 that is not a constant.
        let global = Self::rooted(heap, global_object)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        let name = PropertyKey::String(heap.strings.intern("Math")?);
        let value = Self::rooted(heap, math)?;
        heap.define_own_named(global, name, value, builtin_data())?;

        // 9.1.1.4: the Global Environment Record binds the global object and
        // the declarations of every Script of this Realm. 19.1.1: `globalThis`
        // is the [[GlobalThisValue]] with the attributes of 17.
        let global_object = Self::rooted(heap, global_object)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        let declarative = heap.allocate_immortal_object(root_shape, VALUE_NULL)?;
        let global_this = Value::from_object(global_object);
        let global_this_name = PropertyKey::String(heap.strings.intern("globalThis")?);
        heap.define_own_named(global_object, global_this_name, global_this, builtin_data())?;
        let global = GlobalEnvironment {
            object_record: heap.push_root(global_this)?,
            this_value: heap.push_root(global_this)?,
            declarative_record: heap.push_root(Value::from_object(declarative))?,
        };

        Ok(Self {
            object_prototype,
            function_prototype,
            array_prototype,
            string_prototype,
            array_iterator_prototype,
            error_prototype,
            native_error_prototypes,
            intrinsics,
            global,
        })
    }

    /// The Global Environment Record of this Realm (9.1.1.4).
    #[must_use]
    pub const fn global_environment(&self) -> &GlobalEnvironment {
        &self.global
    }
}

impl GlobalEnvironment {
    /// `[[GlobalThisValue]]`, the value `globalThis` of 19.1.1 answers.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn this_value(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Realm::rooted(heap, self.this_value)
    }

    /// The binding object of the `[[ObjectRecord]]`, which is the global object.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded or
    /// no longer names an object.
    pub fn global_object(&self, heap: &GenerationalHeap) -> Result<ObjectRef, HeapError> {
        Realm::rooted(heap, self.object_record)?
            .as_object()
            .ok_or(HeapError::InvalidReference)
    }

    /// The binding object of the `[[DeclarativeRecord]]`, which holds the
    /// bindings `let`, `const` and `class` create at the top level of a Script.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded or
    /// no longer names an object.
    pub fn declarative_object(&self, heap: &GenerationalHeap) -> Result<ObjectRef, HeapError> {
        Realm::rooted(heap, self.declarative_record)?
            .as_object()
            .ok_or(HeapError::InvalidReference)
    }

    /// `HasBinding` of 9.1.1.4.1: the declarative record first, then the
    /// binding object.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] for a stale root or a malformed Prototype Chain.
    pub fn has_binding(
        &self,
        heap: &GenerationalHeap,
        name: PropertyKey,
    ) -> Result<bool, HeapError> {
        if heap
            .own_named_flags(self.declarative_object(heap)?, name)?
            .is_some()
        {
            return Ok(true);
        }
        // 9.1.1.2.1 asks HasProperty, so the Prototype Chain of the global
        // object counts.
        Ok(heap
            .lookup_named(self.global_object(heap)?, name)?
            .is_some())
    }

    /// `HasLexicalDeclaration` of 9.1.1.4.12.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] for a stale root.
    pub fn has_lexical_declaration(
        &self,
        heap: &GenerationalHeap,
        name: PropertyKey,
    ) -> Result<bool, HeapError> {
        Ok(heap
            .own_named_flags(self.declarative_object(heap)?, name)?
            .is_some())
    }

    /// `HasRestrictedGlobalProperty` of 9.1.1.4.13: an own property of the
    /// global object that cannot be configured away.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] for a stale root.
    pub fn has_restricted_global_property(
        &self,
        heap: &GenerationalHeap,
        name: PropertyKey,
    ) -> Result<bool, HeapError> {
        Ok(heap
            .own_named_flags(self.global_object(heap)?, name)?
            .is_some_and(|flags| !flags.configurable))
    }

    /// `CreateGlobalVarBinding` of 9.1.1.4.16 with `deletable` false, which is
    /// what a Script declaration asks for.
    ///
    /// 9.1.1.4.14 `CanDeclareGlobalVar` answers true for every name here: it
    /// asks whether the global object already has the property or is
    /// extensible, and this engine has no `[[PreventExtensions]]`, so every
    /// object is extensible.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] when the property cannot be defined.
    pub fn create_global_var_binding(
        &self,
        heap: &mut GenerationalHeap,
        name: PropertyKey,
    ) -> Result<(), HeapError> {
        let global = self.global_object(heap)?;
        if heap.own_named_flags(global, name)?.is_some() {
            return Ok(());
        }
        // 9.1.1.2.2 defines it writable and enumerable, configurable only when
        // the binding is deletable.
        heap.define_own_named(
            global,
            name,
            VALUE_UNDEFINED,
            PropertyFlags {
                writable: true,
                enumerable: true,
                configurable: false,
                is_accessor: false,
            },
        )?;
        Ok(())
    }

    /// `CanDeclareGlobalFunction` of 9.1.1.4.15.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] for a stale root.
    pub fn can_declare_global_function(
        &self,
        heap: &GenerationalHeap,
        name: PropertyKey,
    ) -> Result<bool, HeapError> {
        let Some(flags) = heap.own_named_flags(self.global_object(heap)?, name)? else {
            // No such property, and every object of this engine is extensible.
            return Ok(true);
        };
        Ok(flags.configurable || (!flags.is_accessor && flags.writable && flags.enumerable))
    }

    /// `CreateGlobalFunctionBinding` of 9.1.1.4.17 with `deletable` false.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] when the property cannot be defined.
    pub fn create_global_function_binding(
        &self,
        heap: &mut GenerationalHeap,
        name: PropertyKey,
        value: Value,
    ) -> Result<(), HeapError> {
        let global = self.global_object(heap)?;
        // A property that is already there and cannot be configured keeps its
        // attributes and takes only the value.
        let flags = match heap.own_named_flags(global, name)? {
            Some(flags) if !flags.configurable => flags,
            _ => PropertyFlags {
                writable: true,
                enumerable: true,
                configurable: false,
                is_accessor: false,
            },
        };
        heap.define_own_named(global, name, value, flags)?;
        Ok(())
    }

    /// `CreateMutableBinding` of 9.1.1.4.2 and `CreateImmutableBinding` of
    /// 9.1.1.4.3, which both put an uninitialized binding in the declarative
    /// record.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] when the binding cannot be defined.
    pub fn create_lexical_binding(
        &self,
        heap: &mut GenerationalHeap,
        name: PropertyKey,
        mutable: bool,
    ) -> Result<(), HeapError> {
        let declarative = self.declarative_object(heap)?;
        heap.define_own_named(
            declarative,
            name,
            VALUE_UNINITIALIZED,
            PropertyFlags {
                writable: mutable,
                enumerable: true,
                configurable: false,
                is_accessor: false,
            },
        )?;
        Ok(())
    }

    /// `InitializeBinding` of 9.1.1.4.4 for a name the declarative record
    /// holds uninitialized.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] when the binding cannot be written.
    pub fn initialize_lexical_binding(
        &self,
        heap: &mut GenerationalHeap,
        name: PropertyKey,
        value: Value,
    ) -> Result<(), HeapError> {
        let declarative = self.declarative_object(heap)?;
        let flags = heap
            .own_named_flags(declarative, name)?
            .ok_or(HeapError::InvalidReference)?;
        heap.define_own_named(declarative, name, value, flags)?;
        Ok(())
    }

    /// `SetMutableBinding` of 9.1.1.4.5, which reaches 9.1.1.1.5 for a name the
    /// declarative record binds and 9.1.1.2.5 for every other.
    ///
    /// Answers `Err(BindingOutcome)` where the specification throws, so that
    /// the caller raises the error with the `Realm` the running execution
    /// belongs to.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] for a stale root or a malformed Prototype Chain.
    pub fn set_mutable_binding(
        &self,
        heap: &mut GenerationalHeap,
        name: PropertyKey,
        value: Value,
        strict: bool,
    ) -> Result<Result<(), BindingOutcome>, HeapError> {
        let declarative = self.declarative_object(heap)?;
        if let Some(flags) = heap.own_named_flags(declarative, name)? {
            // 9.1.1.1.5: an uninitialized binding is a ReferenceError, and one
            // that is not writable is a TypeError under strict evaluation.
            let current = heap
                .lookup_named(declarative, name)?
                .map_or(VALUE_UNDEFINED, |property| property.value);
            if current == VALUE_UNINITIALIZED {
                return Ok(Err(BindingOutcome::Uninitialized));
            }
            if !flags.writable {
                return Ok(Err(BindingOutcome::Immutable));
            }
            heap.define_own_named(declarative, name, value, flags)?;
            return Ok(Ok(()));
        }
        // 9.1.1.2.5: a name the binding object does not have is a
        // ReferenceError under strict evaluation and a new property otherwise.
        let global = self.global_object(heap)?;
        let existing = heap.own_named_flags(global, name)?;
        if existing.is_none() && strict {
            return Ok(Err(BindingOutcome::Unresolvable));
        }
        let flags = existing.unwrap_or(PropertyFlags::ordinary_data());
        if !flags.writable {
            return Ok(Err(BindingOutcome::Immutable));
        }
        heap.define_own_named(global, name, value, flags)?;
        Ok(Ok(()))
    }

    /// `GetBindingValue` of 9.1.1.4.6, which reaches 9.1.1.1.6 for a name the
    /// declarative record binds and 9.1.1.2.7 for every other.
    ///
    /// Answers `Err(BindingOutcome)` where the specification throws, so that
    /// the caller raises the error with the `Realm` the running execution
    /// belongs to.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] for a stale root or a malformed Prototype Chain.
    pub fn get_binding_value(
        &self,
        heap: &GenerationalHeap,
        name: PropertyKey,
    ) -> Result<Result<Value, BindingOutcome>, HeapError> {
        let declarative = self.declarative_object(heap)?;
        if let Some(property) = heap.lookup_named(declarative, name)?
            && property.holder_depth == 0
        {
            // 9.1.1.1.6: a binding that has no value yet is a ReferenceError.
            if property.value == VALUE_UNINITIALIZED {
                return Ok(Err(BindingOutcome::Uninitialized));
            }
            return Ok(Ok(property.value));
        }
        if let Some(property) = heap.lookup_named(self.global_object(heap)?, name)? {
            return Ok(Ok(property.value));
        }
        // Clause 19 names a property this Realm has not built, so reporting it
        // as undefined would be a wrong answer rather than a missing feature.
        let units = name
            .as_string()
            .and_then(|name| heap.strings.to_utf16(Value::from_string(name)));
        if let Some(units) = units
            && let Some(missing) = GLOBAL_PROPERTIES
                .into_iter()
                .find(|owned| owned.encode_utf16().eq(units.iter().copied()))
        {
            return Ok(Err(BindingOutcome::Missing(missing)));
        }
        Ok(Err(BindingOutcome::Unresolvable))
    }
}

impl Realm {
    /// Allocates the native function of every intrinsic and installs it on
    /// the object 17 gives it.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    /// The values 21.3.1 gives `%Math%`, each of them not writable, not
    /// enumerable and not configurable.
    ///
    /// Every one is the binary64 nearest the number the clause names, which is
    /// what a decimal literal of that many digits parses to.
    fn define_math_constants(
        heap: &mut GenerationalHeap,
        math: ObjectRef,
    ) -> Result<(), HeapError> {
        let flags = PropertyFlags {
            writable: false,
            enumerable: false,
            configurable: false,
            is_accessor: false,
        };
        for (name, value) in [
            ("E", core::f64::consts::E),
            ("LN10", core::f64::consts::LN_10),
            ("LN2", core::f64::consts::LN_2),
            ("LOG10E", core::f64::consts::LOG10_E),
            ("LOG2E", core::f64::consts::LOG2_E),
            ("PI", core::f64::consts::PI),
            ("SQRT1_2", core::f64::consts::FRAC_1_SQRT_2),
            ("SQRT2", core::f64::consts::SQRT_2),
        ] {
            let key = intern(heap, name)?;
            heap.define_own_named(math, key, Value::from_f64(value), flags)?;
        }
        Ok(())
    }

    fn install_intrinsics(
        heap: &mut GenerationalHeap,
        holders: &Holders,
    ) -> Result<[Root; Intrinsic::ALL.len()], HeapError> {
        let mut intrinsics = [holders.object_prototype; Intrinsic::ALL.len()];
        let function_parent = Self::rooted(heap, holders.function_prototype)?;
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
        }
        // Installing is a pass of its own, so that an intrinsic whose holder
        // is another intrinsic does not depend on where `Intrinsic::ALL` puts
        // the two of them.
        for intrinsic in Intrinsic::ALL {
            let function = Self::rooted(
                heap,
                *intrinsics
                    .get(intrinsic.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?;
            let holder = match intrinsic.holder() {
                IntrinsicHolder::ObjectPrototype => Self::rooted(heap, holders.object_prototype)?,
                IntrinsicHolder::StringPrototype => Self::rooted(heap, holders.string_prototype)?,
                IntrinsicHolder::ArrayPrototype => Self::rooted(heap, holders.array_prototype)?,
                IntrinsicHolder::ArrayIteratorPrototype => {
                    Self::rooted(heap, holders.array_iterator_prototype)?
                }
                IntrinsicHolder::Global => Self::rooted(heap, holders.global_object)?,
                IntrinsicHolder::FunctionPrototype => {
                    Self::rooted(heap, holders.function_prototype)?
                }
                IntrinsicHolder::Math => Self::rooted(heap, holders.math)?,
                IntrinsicHolder::ObjectConstructor => Self::rooted(
                    heap,
                    *intrinsics
                        .get(Intrinsic::ObjectConstructor.index())
                        .ok_or(HeapError::InvalidReference)?,
                )?,
                IntrinsicHolder::ArrayConstructor => Self::rooted(
                    heap,
                    *intrinsics
                        .get(Intrinsic::ArrayConstructor.index())
                        .ok_or(HeapError::InvalidReference)?,
                )?,
            }
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
            let key = PropertyKey::String(heap.strings.intern(intrinsic.name())?);
            heap.define_own_named(holder, key, function, builtin_data())?;
            // 23.1.3.40: %Array.prototype%[@@iterator] is the same function
            // object as `values`.
            if intrinsic == Intrinsic::ArrayPrototypeValues {
                heap.define_own_named(
                    holder,
                    WellKnownSymbol::Iterator.key(),
                    function,
                    builtin_data(),
                )?;
            }
        }

        Ok(intrinsics)
    }

    /// Ties every error constructor to its prototype.
    ///
    /// 20.5.1.2 and 20.5.6.2 give each constructor its prototype, and 20.5.3.1
    /// and 20.5.6.3.1 give each prototype back the constructor it belongs to.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn pair_errors_with_their_prototypes(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        error_prototype: Root,
        native_error_prototypes: &[Root; NATIVE_ERROR_COUNT],
    ) -> Result<(), HeapError> {
        Self::pair_constructor_with_prototype(
            heap,
            intrinsics,
            Intrinsic::ErrorConstructor,
            error_prototype,
        )?;
        for kind in NativeErrorKind::ALL {
            Self::pair_constructor_with_prototype(
                heap,
                intrinsics,
                kind.constructor(),
                *native_error_prototypes
                    .get(kind.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?;
        }
        Ok(())
    }

    /// Ties one constructor and its prototype to one another.
    ///
    /// 17 gives a constructor its `prototype` as the one property of it that
    /// is neither writable, enumerable nor configurable, and gives that
    /// prototype back the `constructor` it belongs to.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn pair_constructor_with_prototype(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        which: Intrinsic,
        prototype: Root,
    ) -> Result<(), HeapError> {
        let constructor = Self::rooted(
            heap,
            *intrinsics
                .get(which.index())
                .ok_or(HeapError::InvalidReference)?,
        )?;
        let constructor_object = constructor.as_object().ok_or(HeapError::InvalidReference)?;
        let prototype_key = intern(heap, "prototype")?;
        heap.define_own_named(
            constructor_object,
            prototype_key,
            Self::rooted(heap, prototype)?,
            PropertyFlags {
                writable: false,
                enumerable: false,
                configurable: false,
                is_accessor: false,
            },
        )?;
        let constructor_key = intern(heap, "constructor")?;
        let prototype_object = Self::rooted(heap, prototype)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        heap.define_own_named(
            prototype_object,
            constructor_key,
            constructor,
            builtin_data(),
        )?;
        Ok(())
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

    /// %`ArrayIteratorPrototype`%.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn array_iterator_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.array_iterator_prototype)
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
        let units: alloc::vec::Vec<u16> = message.encode_utf16().collect();
        let text = (!message.is_empty()).then_some(units.as_slice());
        self.create_native_error_units(heap, kind, text)
    }

    /// The same, for a message a Script gave rather than this engine.
    ///
    /// An absent message is left out, which 20.5.6.1.1 does for undefined, so
    /// the empty String of the Prototype answers a read of it. An empty
    /// message that was passed is written, because a Script can tell the two
    /// apart.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] when the error cannot be allocated.
    pub fn create_native_error_units(
        &self,
        heap: &mut GenerationalHeap,
        kind: NativeErrorKind,
        message: Option<&[u16]>,
    ) -> Result<ObjectRef, HeapError> {
        let prototype = self.native_error_prototype(heap, kind)?;
        Self::create_error_with(heap, prototype, message)
    }

    /// `Error ( message )` of 20.5.1.1, whose Prototype is `%Error.prototype%`.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] when the error cannot be allocated.
    pub fn create_error_units(
        &self,
        heap: &mut GenerationalHeap,
        message: Option<&[u16]>,
    ) -> Result<ObjectRef, HeapError> {
        let prototype = self.error_prototype(heap)?;
        Self::create_error_with(heap, prototype, message)
    }

    /// The object 20.5.1.1 makes, under the Prototype its constructor names.
    fn create_error_with(
        heap: &mut GenerationalHeap,
        prototype: Value,
        message: Option<&[u16]>,
    ) -> Result<ObjectRef, HeapError> {
        let shape = heap.shapes.root_shape();
        let error = heap.allocate_object(shape, prototype)?;
        heap.set_object_kind(error, super::object::ObjectKind::Error)?;
        if let Some(message) = message {
            let name = intern(heap, "message")?;
            let text = heap.strings.allocate_units(message)?;
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
