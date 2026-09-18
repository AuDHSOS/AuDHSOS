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
    interpreter::PrimitiveHint,
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
    // B.2.2.12 and B.2.2.13 give `%String.prototype%` two names for the two
    // function objects of 22.1.3.31 and 22.1.3.32.
    if holder == IntrinsicHolder::StringPrototype {
        for (alias, intrinsic) in [
            ("trimLeft", Intrinsic::StringPrototypeTrimStart),
            ("trimRight", Intrinsic::StringPrototypeTrimEnd),
        ] {
            if alias.encode_utf16().eq(name.iter().copied()) {
                return Some(intrinsic);
            }
        }
    }
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

    /// The name 20.4.2 gives this Symbol on `%Symbol%`, which is the
    /// description without the `Symbol.` its text carries.
    #[must_use]
    pub const fn property(self) -> &'static str {
        match self {
            Self::AsyncIterator => "asyncIterator",
            Self::HasInstance => "hasInstance",
            Self::IsConcatSpreadable => "isConcatSpreadable",
            Self::Iterator => "iterator",
            Self::Match => "match",
            Self::MatchAll => "matchAll",
            Self::Replace => "replace",
            Self::Search => "search",
            Self::Species => "species",
            Self::Split => "split",
            Self::ToPrimitive => "toPrimitive",
            Self::ToStringTag => "toStringTag",
            Self::Unscopables => "unscopables",
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
    /// `String.prototype.split` (22.1.3.23).
    StringPrototypeSplit,
    /// `String.prototype.match` (22.1.3.14).
    StringPrototypeMatch,
    /// `String.prototype.search` (22.1.3.20).
    StringPrototypeSearch,
    /// `isNaN` (19.2.3).
    IsNaN,
    /// `isFinite` (19.2.2).
    IsFinite,
    /// `parseInt` (19.2.5).
    ParseInt,
    /// `parseFloat` (19.2.4).
    ParseFloat,
    /// `Symbol.prototype.toString` (20.4.3.3).
    SymbolPrototypeToString,
    /// `Symbol.prototype.valueOf` (20.4.3.4).
    SymbolPrototypeValueOf,
    /// `Symbol.for` (20.4.2.2).
    SymbolFor,
    /// `Symbol.keyFor` (20.4.2.3).
    SymbolKeyFor,
    /// `Array.prototype.findLast` (23.1.3.12).
    ArrayPrototypeFindLast,
    /// `Array.prototype.findLastIndex` (23.1.3.13).
    ArrayPrototypeFindLastIndex,
    /// `Array.prototype.keys` (23.1.3.17).
    ArrayPrototypeKeys,
    /// `Array.prototype.entries` (23.1.3.4).
    ArrayPrototypeEntries,
    /// `Function.prototype.apply` (20.2.3.1).
    FunctionPrototypeApply,
    /// `Reflect.apply` (28.1.1).
    ReflectApply,
    /// `Object.setPrototypeOf` (20.1.2.22).
    ObjectSetPrototypeOf,
    /// `Reflect.setPrototypeOf` (28.1.14).
    ReflectSetPrototypeOf,
    /// `Function.prototype.toString` (20.2.3.5).
    FunctionPrototypeToString,
    /// `Object.prototype.valueOf` (20.1.3.7).
    ObjectPrototypeValueOf,
    /// `%Function.prototype%` (20.2.3), which is itself a built-in function
    /// that takes any argument and answers undefined.
    FunctionPrototype,
    /// `Error.prototype.toString` (20.5.3.4).
    ErrorPrototypeToString,
    /// `Reflect.construct` (28.1.2).
    ReflectConstruct,
    /// `Array.prototype.flat` (23.1.3.14).
    ArrayPrototypeFlat,
    /// `Array.prototype.sort` (23.1.3.30).
    ArrayPrototypeSort,
    /// `Array.prototype.toSorted` (23.1.3.34).
    ArrayPrototypeToSorted,
    /// `Array.prototype.toSpliced` (23.1.3.35).
    ArrayPrototypeToSpliced,
    /// `get [Symbol.species]` of 23.1.2.5 and 22.2.5.2, which answers the
    /// `this` value.
    SpeciesGetter,
    /// `String.prototype.replace` (22.1.3.19).
    StringPrototypeReplace,
    /// `%RegExp.prototype%[@@replace]` (22.2.6.11).
    RegExpPrototypeReplace,
    /// `Array.of` (23.1.2.3).
    ArrayOf,
    /// `Array.from` (23.1.2.1).
    ArrayFrom,
    /// `eval` (19.2.1).
    Eval,
    /// `String.fromCharCode` (22.1.2.1).
    StringFromCharCode,
    /// `String.fromCodePoint` (22.1.2.2).
    StringFromCodePoint,
    /// `String.raw` (22.1.2.4).
    StringRaw,
    /// `String.prototype.isWellFormed` (22.1.3.9).
    StringPrototypeIsWellFormed,
    /// `String.prototype.toWellFormed` (22.1.3.29).
    StringPrototypeToWellFormed,
    /// `String.prototype.substr` (B.2.2.1).
    StringPrototypeSubstr,
    /// `String.prototype.localeCompare` (22.1.3.12).
    StringPrototypeLocaleCompare,
    /// `get flags` of 22.2.6.4.
    RegExpPrototypeFlags,
    /// `get source` of 22.2.6.13.
    RegExpPrototypeSource,
    /// `get hasIndices` of 22.2.6.6.
    RegExpPrototypeHasIndices,
    /// `get global` of 22.2.6.5.
    RegExpPrototypeGlobal,
    /// `get ignoreCase` of 22.2.6.7.
    RegExpPrototypeIgnoreCase,
    /// `get multiline` of 22.2.6.9.
    RegExpPrototypeMultiline,
    /// `get dotAll` of 22.2.6.3.
    RegExpPrototypeDotAll,
    /// `get unicode` of 22.2.6.18.
    RegExpPrototypeUnicode,
    /// `get unicodeSets` of 22.2.6.19.
    RegExpPrototypeUnicodeSets,
    /// `get sticky` of 22.2.6.14.
    RegExpPrototypeSticky,
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
    /// `Array.prototype.forEach` (23.1.3.15).
    ArrayPrototypeForEach,
    /// `Array.prototype.map` (23.1.3.21).
    ArrayPrototypeMap,
    /// `Array.prototype.filter` (23.1.3.8).
    ArrayPrototypeFilter,
    /// `Array.prototype.every` (23.1.3.6).
    ArrayPrototypeEvery,
    /// `Array.prototype.some` (23.1.3.29).
    ArrayPrototypeSome,
    /// `Array.prototype.find` (23.1.3.9).
    ArrayPrototypeFind,
    /// `Array.prototype.findIndex` (23.1.3.10).
    ArrayPrototypeFindIndex,
    /// `Array.prototype.reduce` (23.1.3.24).
    ArrayPrototypeReduce,
    /// `Array.prototype.reduceRight` (23.1.3.25).
    ArrayPrototypeReduceRight,
    /// `%IteratorPrototype%[@@iterator]` (27.1.2.1), which answers `this`.
    IteratorPrototypeIterator,
    /// `Object.preventExtensions` (20.1.2.20).
    ObjectPreventExtensions,
    /// `Object.isExtensible` (20.1.2.16).
    ObjectIsExtensible,
    /// `Object.seal` (20.1.2.22).
    ObjectSeal,
    /// `Object.isSealed` (20.1.2.18).
    ObjectIsSealed,
    /// `Object.freeze` (20.1.2.6).
    ObjectFreeze,
    /// `Object.isFrozen` (20.1.2.17).
    ObjectIsFrozen,
    /// `Object.values` (20.1.2.24).
    ObjectValues,
    /// `Object.entries` (20.1.2.5).
    ObjectEntries,
    /// The `Number` constructor `%Number%` (21.1.1.1).
    NumberConstructor,
    /// `Number.isFinite` (21.1.2.2).
    NumberIsFinite,
    /// `Number.isInteger` (21.1.2.3).
    NumberIsInteger,
    /// `Number.isNaN` (21.1.2.4).
    NumberIsNaN,
    /// `Number.isSafeInteger` (21.1.2.5).
    NumberIsSafeInteger,
    /// The `Boolean` constructor `%Boolean%` (20.3.1.1).
    BooleanConstructor,
    /// `Reflect.defineProperty` (28.1.3).
    ReflectDefineProperty,
    /// `Reflect.deleteProperty` (28.1.4).
    ReflectDeleteProperty,
    /// `Reflect.get` (28.1.5).
    ReflectGet,
    /// `Reflect.getOwnPropertyDescriptor` (28.1.6).
    ReflectGetOwnPropertyDescriptor,
    /// `Reflect.getPrototypeOf` (28.1.7).
    ReflectGetPrototypeOf,
    /// `Reflect.has` (28.1.8).
    ReflectHas,
    /// `Reflect.isExtensible` (28.1.9).
    ReflectIsExtensible,
    /// `Reflect.ownKeys` (28.1.10).
    ReflectOwnKeys,
    /// `Reflect.preventExtensions` (28.1.11).
    ReflectPreventExtensions,
    /// `Number.prototype.valueOf`, 21.1.3.7.
    NumberPrototypeValueOf,
    /// `Number.prototype.toString`, 21.1.3.6.
    NumberPrototypeToString,
    /// `Boolean.prototype.valueOf`, 20.3.3.3.
    BooleanPrototypeValueOf,
    /// `Boolean.prototype.toString`, 20.3.3.2.
    BooleanPrototypeToString,
    /// `String.prototype.valueOf`, 22.1.3.32.
    StringPrototypeValueOf,
    /// `String.prototype.toString`, 22.1.3.28.
    StringPrototypeToString,
    /// `%ThrowTypeError%`, 10.2.4.1, which throws whenever it is called.
    ThrowTypeError,
    /// `Symbol`, 20.4.1.1.
    SymbolConstructor,
    /// `RegExp`, 22.2.4.1.
    RegExpConstructor,
    /// `RegExp.prototype.exec`, 22.2.6.8.
    RegExpPrototypeExec,
    /// `RegExp.prototype.test`, 22.2.6.16.
    RegExpPrototypeTest,
    /// `RegExp.prototype.toString`, 22.2.6.17.
    RegExpPrototypeToString,
    /// `JSON.parse`, 25.5.1.
    JsonParse,
    /// `JSON.stringify`, 25.5.2.
    JsonStringify,
    /// `Promise`, 27.2.3.1.
    PromiseConstructor,
    /// `Promise.resolve`, 27.2.4.7.
    PromiseResolve,
    /// `Promise.reject`, 27.2.4.6.
    PromiseReject,
    /// `Promise.prototype.then`, 27.2.5.4.
    PromisePrototypeThen,
    /// `Promise.prototype.catch`, 27.2.5.1.
    PromisePrototypeCatch,
    /// The resolve function of 27.2.1.3.2, which 27.2.1.3 makes anew for each
    /// promise and gives the pair's shared state.
    PromiseResolveFunction,
    /// The reject function of 27.2.1.3.1.
    PromiseRejectFunction,
    /// `print`, which no clause of the specification names: the embedding
    /// gives the global object one, and a Test262 file of the flag `async`
    /// reports through it.
    Print,
    /// `Promise.all`, 27.2.4.1.
    PromiseAll,
    /// `Promise.race`, 27.2.4.5.
    PromiseRace,
    /// `Promise.allSettled`, 27.2.4.2.
    PromiseAllSettled,
    /// `Promise.withResolvers`, 27.2.4.9.
    PromiseWithResolvers,
    /// The resolve element function of 27.2.4.1.3.
    PromiseAllElement,
    /// The fulfil element function of 27.2.4.2.2.
    PromiseAllSettledFulfilled,
    /// The reject element function of 27.2.4.2.3.
    PromiseAllSettledRejected,
    /// `RegExp.prototype[@@match]`, 22.2.6.8.
    RegExpPrototypeMatch,
    /// `RegExp.prototype[@@search]`, 22.2.6.12.
    RegExpPrototypeSearch,
    /// `RegExp.prototype[@@split]`, 22.2.6.14.
    RegExpPrototypeSplit,
    /// The fulfilled closure of 27.7.5.3, which takes the body of an async
    /// function back where it waited.
    AsyncResume,
    /// The rejected closure of 27.7.5.3, which takes it back by throwing.
    AsyncThrow,
    /// `String.prototype.anchor`, B.2.2.
    StringPrototypeAnchor,
    /// `String.prototype.big`, B.2.2.
    StringPrototypeBig,
    /// `String.prototype.blink`, B.2.2.
    StringPrototypeBlink,
    /// `String.prototype.bold`, B.2.2.
    StringPrototypeBold,
    /// `String.prototype.fixed`, B.2.2.
    StringPrototypeFixed,
    /// `String.prototype.fontcolor`, B.2.2.
    StringPrototypeFontcolor,
    /// `String.prototype.fontsize`, B.2.2.
    StringPrototypeFontsize,
    /// `String.prototype.italics`, B.2.2.
    StringPrototypeItalics,
    /// `String.prototype.link`, B.2.2.
    StringPrototypeLink,
    /// `String.prototype.small`, B.2.2.
    StringPrototypeSmall,
    /// `String.prototype.strike`, B.2.2.
    StringPrototypeStrike,
    /// `String.prototype.sub`, B.2.2.
    StringPrototypeSub,
    /// `String.prototype.sup`, B.2.2.
    StringPrototypeSup,
    /// `Object.getOwnPropertySymbols`, 20.1.2.11.
    ObjectGetOwnPropertySymbols,
    /// `Object.getOwnPropertyDescriptors`, 20.1.2.9.
    ObjectGetOwnPropertyDescriptors,
    /// `Object.assign`, 20.1.2.1.
    ObjectAssign,
    /// `Reflect.set`, 28.1.13.
    ReflectSet,
    /// `Array.prototype.flatMap`, 23.1.3.13.
    ArrayPrototypeFlatMap,
    /// `Object.fromEntries`, 20.1.2.7.
    ObjectFromEntries,
    /// `Math.acos`, 21.3.2.
    MathAcos,
    /// `Math.acosh`, 21.3.2.
    MathAcosh,
    /// `Math.asin`, 21.3.2.
    MathAsin,
    /// `Math.asinh`, 21.3.2.
    MathAsinh,
    /// `Math.atan`, 21.3.2.
    MathAtan,
    /// `Math.atan2`, 21.3.2.
    MathAtan2,
    /// `Math.atanh`, 21.3.2.
    MathAtanh,
    /// `Math.cbrt`, 21.3.2.
    MathCbrt,
    /// `Math.cos`, 21.3.2.
    MathCos,
    /// `Math.cosh`, 21.3.2.
    MathCosh,
    /// `Math.exp`, 21.3.2.
    MathExp,
    /// `Math.expm1`, 21.3.2.
    MathExpm1,
    /// `Math.f16round`, 21.3.2.
    MathF16round,
    /// `Math.hypot`, 21.3.2.
    MathHypot,
    /// `Math.log`, 21.3.2.
    MathLog,
    /// `Math.log10`, 21.3.2.
    MathLog10,
    /// `Math.log1p`, 21.3.2.
    MathLog1p,
    /// `Math.log2`, 21.3.2.
    MathLog2,
    /// `Math.random`, 21.3.2.
    MathRandom,
    /// `Math.sinh`, 21.3.2.
    MathSinh,
    /// `Math.sqrt`, 21.3.2.
    MathSqrt,
    /// `Math.tan`, 21.3.2.
    MathTan,
    /// `Math.tanh`, 21.3.2.
    MathTanh,
    /// `%Map%`, 24.1.1.1.
    MapConstructor,
    /// `%Map.prototype.get%`, 24.1.3.6.
    MapPrototypeGet,
    /// `%Map.prototype.set%`, 24.1.3.9.
    MapPrototypeSet,
    /// `%Map.prototype.has%`, 24.1.3.7.
    MapPrototypeHas,
    /// `%Map.prototype.delete%`, 24.1.3.3.
    MapPrototypeDelete,
    /// `%Map.prototype.clear%`, 24.1.3.1.
    MapPrototypeClear,
    /// `get %Map.prototype.size%`, 24.1.3.10.
    MapPrototypeSize,
    /// `%Set%`, 24.2.1.1.
    SetConstructor,
    /// `%Set.prototype.add%`, 24.2.3.1.
    SetPrototypeAdd,
    /// `%Set.prototype.has%`, 24.2.3.8.
    SetPrototypeHas,
    /// `%Set.prototype.delete%`, 24.2.3.4.
    SetPrototypeDelete,
    /// `%Set.prototype.clear%`, 24.2.3.2.
    SetPrototypeClear,
    /// `get %Set.prototype.size%`, 24.2.3.14.
    SetPrototypeSize,
    /// `%Map.prototype.entries%`, 24.1.3.4.
    MapPrototypeEntries,
    /// `%Map.prototype.keys%`, 24.1.3.8.
    MapPrototypeKeys,
    /// `%Map.prototype.values%`, 24.1.3.12.
    MapPrototypeValues,
    /// `%Set.prototype.values%`, 24.2.3.16, which 24.2.3.10 and 24.2.3.17
    /// answer as well.
    SetPrototypeValues,
    /// `%Set.prototype.entries%`, 24.2.3.5.
    SetPrototypeEntries,
    /// `%MapIteratorPrototype%.next`, 24.1.5.2.1.
    MapIteratorPrototypeNext,
    /// `%SetIteratorPrototype%.next`, 24.2.5.2.1.
    SetIteratorPrototypeNext,
    /// `%Set.prototype.union%`, 24.2.3.18.
    SetPrototypeUnion,
    /// `%Set.prototype.intersection%`, 24.2.3.9.
    SetPrototypeIntersection,
    /// `%Set.prototype.difference%`, 24.2.3.6.
    SetPrototypeDifference,
    /// `%Set.prototype.symmetricDifference%`, 24.2.3.15.
    SetPrototypeSymmetricDifference,
    /// `%Set.prototype.isSubsetOf%`, 24.2.3.12.
    SetPrototypeIsSubsetOf,
    /// `%Set.prototype.isSupersetOf%`, 24.2.3.13.
    SetPrototypeIsSupersetOf,
    /// `%Set.prototype.isDisjointFrom%`, 24.2.3.11.
    SetPrototypeIsDisjointFrom,
    /// `%Map.prototype.forEach%`, 24.1.3.5.
    MapPrototypeForEach,
    /// `%Set.prototype.forEach%`, 24.2.3.7.
    SetPrototypeForEach,
    /// `%WeakMap%`, 24.3.1.1.
    WeakMapConstructor,
    /// `%WeakMap.prototype.get%`, 24.3.3.3.
    WeakMapPrototypeGet,
    /// `%WeakMap.prototype.set%`, 24.3.3.5.
    WeakMapPrototypeSet,
    /// `%WeakMap.prototype.has%`, 24.3.3.4.
    WeakMapPrototypeHas,
    /// `%WeakMap.prototype.delete%`, 24.3.3.2.
    WeakMapPrototypeDelete,
    /// `%WeakSet%`, 24.4.1.1.
    WeakSetConstructor,
    /// `%WeakSet.prototype.add%`, 24.4.3.1.
    WeakSetPrototypeAdd,
    /// `%WeakSet.prototype.has%`, 24.4.3.4.
    WeakSetPrototypeHas,
    /// `%WeakSet.prototype.delete%`, 24.4.3.3.
    WeakSetPrototypeDelete,
    /// `%Map.prototype.getOrInsert%`, 24.1.3.7.
    MapPrototypeGetOrInsert,
    /// `%Map.prototype.getOrInsertComputed%`, 24.1.3.8.
    MapPrototypeGetOrInsertComputed,
    /// `%WeakMap.prototype.getOrInsert%`, 24.3.3.4.
    WeakMapPrototypeGetOrInsert,
    /// `%WeakMap.prototype.getOrInsertComputed%`, 24.3.3.5.
    WeakMapPrototypeGetOrInsertComputed,
    /// `JSON.rawJSON` of the rawJSON proposal.
    JsonRawJson,
    /// `JSON.isRawJSON` of the same proposal.
    JsonIsRawJson,
    /// `encodeURI`, 19.2.6.4.
    EncodeUri,
    /// `encodeURIComponent`, 19.2.6.5.
    EncodeUriComponent,
    /// `decodeURI`, 19.2.6.2.
    DecodeUri,
    /// `decodeURIComponent`, 19.2.6.3.
    DecodeUriComponent,
    /// `escape`, B.2.1.1.
    Escape,
    /// `unescape`, B.2.1.2.
    Unescape,
    /// `Date`, 21.4.2.1.
    DateConstructor,
    /// `Date.now`, 21.4.3.1.
    DateNow,
    /// `Date.UTC`, 21.4.3.4.
    DateUtc,
    /// `Date.parse`, 21.4.3.2.
    DateParse,
    /// `Date.prototype.valueOf`, 21.4.4.44.
    DatePrototypeValueOf,
    /// `Date.prototype.getTime`, 21.4.4.10.
    DatePrototypeGetTime,
    /// `Date.prototype.setTime`, 21.4.4.27.
    DatePrototypeSetTime,
    /// `Date.prototype.getTimezoneOffset`, 21.4.4.11.
    DatePrototypeGetTimezoneOffset,
    /// `Date.prototype.getFullYear`, 21.4.4.4.
    DatePrototypeGetFullYear,
    /// `Date.prototype.getUTCFullYear`, 21.4.4.14.
    DatePrototypeGetUtcFullYear,
    /// `Date.prototype.getMonth`, 21.4.4.8.
    DatePrototypeGetMonth,
    /// `Date.prototype.getUTCMonth`, 21.4.4.18.
    DatePrototypeGetUtcMonth,
    /// `Date.prototype.getDate`, 21.4.4.2.
    DatePrototypeGetDate,
    /// `Date.prototype.getUTCDate`, 21.4.4.12.
    DatePrototypeGetUtcDate,
    /// `Date.prototype.getDay`, 21.4.4.3.
    DatePrototypeGetDay,
    /// `Date.prototype.getUTCDay`, 21.4.4.13.
    DatePrototypeGetUtcDay,
    /// `Date.prototype.getHours`, 21.4.4.5.
    DatePrototypeGetHours,
    /// `Date.prototype.getUTCHours`, 21.4.4.15.
    DatePrototypeGetUtcHours,
    /// `Date.prototype.getMinutes`, 21.4.4.7.
    DatePrototypeGetMinutes,
    /// `Date.prototype.getUTCMinutes`, 21.4.4.17.
    DatePrototypeGetUtcMinutes,
    /// `Date.prototype.getSeconds`, 21.4.4.9.
    DatePrototypeGetSeconds,
    /// `Date.prototype.getUTCSeconds`, 21.4.4.19.
    DatePrototypeGetUtcSeconds,
    /// `Date.prototype.getMilliseconds`, 21.4.4.6.
    DatePrototypeGetMilliseconds,
    /// `Date.prototype.getUTCMilliseconds`, 21.4.4.16.
    DatePrototypeGetUtcMilliseconds,
    /// `Date.prototype.toISOString`, 21.4.4.43.
    DatePrototypeToIsoString,
    /// `Date.prototype.toJSON`, 21.4.4.42.
    DatePrototypeToJson,
    /// `Date.prototype.setMilliseconds`, 21.4.4.23.
    DatePrototypeSetMilliseconds,
    /// `Date.prototype.setUTCMilliseconds`, 21.4.4.31.
    DatePrototypeSetUtcMilliseconds,
    /// `Date.prototype.setSeconds`, 21.4.4.26.
    DatePrototypeSetSeconds,
    /// `Date.prototype.setUTCSeconds`, 21.4.4.34.
    DatePrototypeSetUtcSeconds,
    /// `Date.prototype.setMinutes`, 21.4.4.24.
    DatePrototypeSetMinutes,
    /// `Date.prototype.setUTCMinutes`, 21.4.4.32.
    DatePrototypeSetUtcMinutes,
    /// `Date.prototype.setHours`, 21.4.4.22.
    DatePrototypeSetHours,
    /// `Date.prototype.setUTCHours`, 21.4.4.30.
    DatePrototypeSetUtcHours,
    /// `Date.prototype.setDate`, 21.4.4.20.
    DatePrototypeSetDate,
    /// `Date.prototype.setUTCDate`, 21.4.4.28.
    DatePrototypeSetUtcDate,
    /// `Date.prototype.setMonth`, 21.4.4.25.
    DatePrototypeSetMonth,
    /// `Date.prototype.setUTCMonth`, 21.4.4.33.
    DatePrototypeSetUtcMonth,
    /// `Date.prototype.setFullYear`, 21.4.4.21.
    DatePrototypeSetFullYear,
    /// `Date.prototype.setUTCFullYear`, 21.4.4.29.
    DatePrototypeSetUtcFullYear,
    /// `Date.prototype.setYear`, B.2.3.2.
    DatePrototypeSetYear,
    /// `Date.prototype.toString`, 21.4.4.41.
    DatePrototypeToString,
    /// `Date.prototype.toDateString`, 21.4.4.35.
    DatePrototypeToDateString,
    /// `Date.prototype.toTimeString`, 21.4.4.42.
    DatePrototypeToTimeString,
    /// `Date.prototype.toUTCString`, 21.4.4.43.
    DatePrototypeToUtcString,
    /// `Date.prototype.toLocaleString`, 21.4.4.39.
    DatePrototypeToLocaleString,
    /// `Date.prototype.toLocaleDateString`, 21.4.4.38.
    DatePrototypeToLocaleDateString,
    /// `Date.prototype.toLocaleTimeString`, 21.4.4.40.
    DatePrototypeToLocaleTimeString,
    /// `ArrayBuffer`, 25.1.4.1.
    ArrayBufferConstructor,
    /// `isView`, 25.1.5.1.
    ArrayBufferIsView,
    /// `slice`, 25.1.6.7.
    ArrayBufferPrototypeSlice,
    /// `get byteLength`, 25.1.6.2.
    ArrayBufferPrototypeByteLength,
    /// `get detached`, 25.1.6.3.
    ArrayBufferPrototypeDetached,
    /// `get resizable`, 25.1.6.6.
    ArrayBufferPrototypeResizable,
    /// `get maxByteLength`, 25.1.6.4.
    ArrayBufferPrototypeMaxByteLength,
    /// `DataView`, 25.3.3.1.
    DataViewConstructor,
    /// `get buffer`, 25.3.4.1.
    DataViewPrototypeBuffer,
    /// `get byteLength`, 25.3.4.2.
    DataViewPrototypeByteLength,
    /// `get byteOffset`, 25.3.4.3.
    DataViewPrototypeByteOffset,
    /// `getInt8`, 25.3.4.
    DataViewPrototypeGetInt8,
    /// `setInt8`, 25.3.4.
    DataViewPrototypeSetInt8,
    /// `getUint8`, 25.3.4.
    DataViewPrototypeGetUint8,
    /// `setUint8`, 25.3.4.
    DataViewPrototypeSetUint8,
    /// `getInt16`, 25.3.4.
    DataViewPrototypeGetInt16,
    /// `setInt16`, 25.3.4.
    DataViewPrototypeSetInt16,
    /// `getUint16`, 25.3.4.
    DataViewPrototypeGetUint16,
    /// `setUint16`, 25.3.4.
    DataViewPrototypeSetUint16,
    /// `getInt32`, 25.3.4.
    DataViewPrototypeGetInt32,
    /// `setInt32`, 25.3.4.
    DataViewPrototypeSetInt32,
    /// `getUint32`, 25.3.4.
    DataViewPrototypeGetUint32,
    /// `setUint32`, 25.3.4.
    DataViewPrototypeSetUint32,
    /// `getFloat32`, 25.3.4.
    DataViewPrototypeGetFloat32,
    /// `setFloat32`, 25.3.4.
    DataViewPrototypeSetFloat32,
    /// `getFloat64`, 25.3.4.
    DataViewPrototypeGetFloat64,
    /// `setFloat64`, 25.3.4.
    DataViewPrototypeSetFloat64,
    /// `TypedArray`, 23.2.1.1.
    TypedArrayBase,
    /// `Int8Array`, 23.2.6.
    TypedArrayInt8Constructor,
    /// `Uint8Array`, 23.2.6.
    TypedArrayUint8Constructor,
    /// `Uint8ClampedArray`, 23.2.6.
    TypedArrayUint8ClampedConstructor,
    /// `Int16Array`, 23.2.6.
    TypedArrayInt16Constructor,
    /// `Uint16Array`, 23.2.6.
    TypedArrayUint16Constructor,
    /// `Int32Array`, 23.2.6.
    TypedArrayInt32Constructor,
    /// `Uint32Array`, 23.2.6.
    TypedArrayUint32Constructor,
    /// `Float32Array`, 23.2.6.
    TypedArrayFloat32Constructor,
    /// `Float64Array`, 23.2.6.
    TypedArrayFloat64Constructor,
    /// `Float16Array`, 23.2.6.
    TypedArrayFloat16Constructor,
    /// `BigInt64Array`, 23.2.6.
    TypedArrayBigInt64Constructor,
    /// `BigUint64Array`, 23.2.6.
    TypedArrayBigUint64Constructor,
    /// `get buffer`, 23.2.3.1.
    TypedArrayPrototypeBuffer,
    /// `get byteLength`, 23.2.3.2.
    TypedArrayPrototypeByteLength,
    /// `get byteOffset`, 23.2.3.3.
    TypedArrayPrototypeByteOffset,
    /// `get length`, 23.2.3.19.
    TypedArrayPrototypeLength,
    /// `get [@@toStringTag]`, 23.2.3.38.
    TypedArrayPrototypeToStringTag,
    /// `at`, 23.2.3.1.
    TypedArrayPrototypeAt,
    /// `copyWithin`, 23.2.3.6.
    TypedArrayPrototypeCopyWithin,
    /// `entries`, 23.2.3.7.
    TypedArrayPrototypeEntries,
    /// `fill`, 23.2.3.9.
    TypedArrayPrototypeFill,
    /// `includes`, 23.2.3.14.
    TypedArrayPrototypeIncludes,
    /// `indexOf`, 23.2.3.15.
    TypedArrayPrototypeIndexOf,
    /// `join`, 23.2.3.16.
    TypedArrayPrototypeJoin,
    /// `keys`, 23.2.3.17.
    TypedArrayPrototypeKeys,
    /// `lastIndexOf`, 23.2.3.18.
    TypedArrayPrototypeLastIndexOf,
    /// `reverse`, 23.2.3.21.
    TypedArrayPrototypeReverse,
    /// `set`, 23.2.3.23.
    TypedArrayPrototypeSet,
    /// `slice`, 23.2.3.24.
    TypedArrayPrototypeSlice,
    /// `sort`, 23.2.3.26.
    TypedArrayPrototypeSort,
    /// `subarray`, 23.2.3.27.
    TypedArrayPrototypeSubarray,
    /// `toReversed`, 23.2.3.31.
    TypedArrayPrototypeToReversed,
    /// `toSorted`, 23.2.3.32.
    TypedArrayPrototypeToSorted,
    /// `values`, 23.2.3.35.
    TypedArrayPrototypeValues,
    /// `with`, 23.2.3.36.
    TypedArrayPrototypeWith,
    /// `every`, 23.2.3.8.
    TypedArrayPrototypeEvery,
    /// `find`, 23.2.3.10.
    TypedArrayPrototypeFind,
    /// `findIndex`, 23.2.3.11.
    TypedArrayPrototypeFindIndex,
    /// `findLast`, 23.2.3.12.
    TypedArrayPrototypeFindLast,
    /// `findLastIndex`, 23.2.3.13.
    TypedArrayPrototypeFindLastIndex,
    /// `forEach`, 23.2.3.14.
    TypedArrayPrototypeForEach,
    /// `reduce`, 23.2.3.22.
    TypedArrayPrototypeReduce,
    /// `reduceRight`, 23.2.3.23.
    TypedArrayPrototypeReduceRight,
    /// `some`, 23.2.3.25.
    TypedArrayPrototypeSome,
    /// `Iterator`, 27.1.3.1.1.
    IteratorConstructor,
    /// `toArray`, 27.1.3.3.12.
    IteratorPrototypeToArray,
    /// `forEach`, 27.1.3.3.7.
    IteratorPrototypeForEach,
    /// `some`, 27.1.3.3.10.
    IteratorPrototypeSome,
    /// `every`, 27.1.3.3.3.
    IteratorPrototypeEvery,
    /// `find`, 27.1.3.3.5.
    IteratorPrototypeFind,
    /// `reduce`, 27.1.3.3.9.
    IteratorPrototypeReduce,
    /// `next`, 27.5.1.2.
    GeneratorPrototypeNext,
    /// `return`, 27.5.1.3.
    GeneratorPrototypeReturn,
    /// `throw`, 27.5.1.4.
    GeneratorPrototypeThrow,
    /// `next`, 27.6.1.2.
    AsyncGeneratorPrototypeNext,
    /// `return`, 27.6.1.3.
    AsyncGeneratorPrototypeReturn,
    /// `throw`, 27.6.1.4.
    AsyncGeneratorPrototypeThrow,
    /// `next`, 27.1.4.2.1.
    AsyncFromSyncIteratorPrototypeNext,
    /// `return`, 27.1.4.2.2.
    AsyncFromSyncIteratorPrototypeReturn,
    /// `throw`, 27.1.4.2.3.
    AsyncFromSyncIteratorPrototypeThrow,
    /// The closure 27.1.4.4 step 5 makes, which carries the `done` its step 2
    /// read and answers the result object of 7.4.10.
    AsyncFromSyncUnwrap,
    /// The closure 27.1.4.4 step 13 makes, which closes the sync iterator
    /// where the value it answered rejects.
    AsyncFromSyncCloseIterator,
    /// `[Symbol.asyncIterator]`, 27.1.4.1, which answers the iterator itself.
    AsyncIteratorPrototypeAsyncIterator,
    /// `[Symbol.hasInstance]`, 20.2.3.6, which 7.3.21 answers for.
    FunctionPrototypeHasInstance,
    /// `[Symbol.toPrimitive]`, 20.4.3.5, which answers the Symbol itself.
    SymbolPrototypeToPrimitive,
    /// `Error.isError`, 20.5.2.1, which answers whether a value carries
    /// `[[ErrorData]]`.
    ErrorIsError,
    /// `get constructor`, 27.1.3.3.1.1.
    IteratorPrototypeConstructorGet,
    /// `set constructor`, 27.1.3.3.1.2.
    IteratorPrototypeConstructorSet,
    /// `get [Symbol.toStringTag]`, 27.1.3.3.15.1.
    IteratorPrototypeToStringTagGet,
    /// `set [Symbol.toStringTag]`, 27.1.3.3.15.2.
    IteratorPrototypeToStringTagSet,
    /// `$262.detachArrayBuffer`, the `DetachArrayBuffer` of 25.1.3.4 the host
    /// of the conformance suite exposes.
    HostDetachArrayBuffer,
    /// `$262.gc`, which the host of the conformance suite exposes and which
    /// no clause of the specification names.
    HostGc,
    /// `BigInt`, 21.2.1.1.
    BigIntConstructor,
    /// `asIntN`, 21.2.2.1.
    BigIntAsIntN,
    /// `asUintN`, 21.2.2.2.
    BigIntAsUintN,
    /// `toString`, 21.2.3.3.
    BigIntPrototypeToString,
    /// `toLocaleString`, 21.2.3.2.
    BigIntPrototypeToLocaleString,
    /// `valueOf`, 21.2.3.4.
    BigIntPrototypeValueOf,
    /// `SharedArrayBuffer`, 25.2.3.1.
    SharedArrayBufferConstructor,
    /// `slice`, 25.2.5.4.
    SharedArrayBufferPrototypeSlice,
    /// `grow`, 25.2.5.2.
    SharedArrayBufferPrototypeGrow,
    /// `get byteLength`, 25.2.5.1.
    SharedArrayBufferPrototypeByteLength,
    /// `get growable`, 25.2.5.3.
    SharedArrayBufferPrototypeGrowable,
    /// `get maxByteLength`, 25.2.5.5.
    SharedArrayBufferPrototypeMaxByteLength,
    /// `add`, 25.4.3.
    AtomicsAdd,
    /// `and`, 25.4.4.
    AtomicsAnd,
    /// `compareExchange`, 25.4.5.
    AtomicsCompareExchange,
    /// `exchange`, 25.4.6.
    AtomicsExchange,
    /// `isLockFree`, 25.4.7.
    AtomicsIsLockFree,
    /// `load`, 25.4.8.
    AtomicsLoad,
    /// `or`, 25.4.9.
    AtomicsOr,
    /// `pause`, 25.4.10.
    AtomicsPause,
    /// `store`, 25.4.11.
    AtomicsStore,
    /// `sub`, 25.4.12.
    AtomicsSub,
    /// `wait`, 25.4.13.
    AtomicsWait,
    /// `notify`, 25.4.16.
    AtomicsNotify,
    /// `xor`, 25.4.17.
    AtomicsXor,
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
    /// `%GeneratorPrototype%`, which 27.5.1 gives every Generator.
    GeneratorPrototype,
    /// `%AsyncGeneratorPrototype%`, which 27.6.1 gives every `AsyncGenerator`.
    AsyncGeneratorPrototype,
    /// `%AsyncFromSyncIteratorPrototype%`, which 27.1.4.2 carries and no
    /// Script reaches.
    AsyncFromSyncIteratorPrototype,
    /// `%AsyncIteratorPrototype%`, which 27.1.4.1 carries.
    AsyncIteratorPrototype,
    /// The global object, which 19.1 gives the constructors of clause 20 and
    /// after.
    Global,
    /// `%Array%`, which carries the functions 23.1.2 gives the constructor.
    ArrayConstructor,
    /// `%Number%`, which carries the functions 21.1.2 gives the constructor.
    NumberConstructor,
    /// `%Object%`, which carries the functions 20.1.2 gives the constructor.
    ObjectConstructor,
    /// `%Function.prototype%`, which carries the methods 20.2.3 gives every
    /// function.
    FunctionPrototype,
    /// `%Math%`, the namespace object of 21.3.
    Math,
    /// `%Reflect%`, the namespace object of 28.1.
    Reflect,
    /// `%Number.prototype%`, which carries the methods 21.1.3 gives it.
    NumberPrototype,
    /// `%Boolean.prototype%`, which carries the methods 20.3.3 gives it.
    BooleanPrototype,
    /// `%RegExp.prototype%`, which carries the methods 22.2.6 gives it.
    RegExpPrototype,
    /// `%JSON%`, the namespace object of 25.5.
    Json,
    /// `%String%`, which carries the functions 22.1.2 gives the constructor.
    StringConstructor,
    /// `%Symbol.prototype%`, which carries the methods 20.4.3 gives it.
    SymbolPrototype,
    /// `%Symbol%`, which carries the functions 20.4.2 gives the constructor.
    SymbolConstructor,
    /// `%Error%`, which carries the function 20.5.2 gives the constructor.
    ErrorConstructor,
    /// `%Error.prototype%`, which carries the methods 20.5.3 gives it.
    ErrorPrototype,
    /// `%Map.prototype%`, which carries the methods 24.1.3 gives it.
    MapPrototype,
    /// `%Set.prototype%`, which carries the methods 24.2.3 gives it.
    SetPrototype,
    /// `%Date.prototype%`, which carries the methods 21.4.4 gives it.
    DatePrototype,
    /// `%Date%`, which carries the functions 21.4.3 gives the constructor.
    DateConstructor,
    /// `%ArrayBuffer.prototype%`, which carries what 25.1.6 gives it.
    ArrayBufferPrototype,
    /// `%ArrayBuffer%`, which carries the function 25.1.5 gives it.
    ArrayBufferConstructor,
    /// `%DataView.prototype%`, which carries what 25.3.4 gives it.
    DataViewPrototype,
    /// `%TypedArray.prototype%`, which carries what 23.2.3 gives it.
    TypedArrayPrototype,
    /// `%SharedArrayBuffer.prototype%`, which carries what 25.2.5 gives it.
    SharedArrayBufferPrototype,
    /// `%BigInt.prototype%`, which carries what 21.2.3 gives it.
    BigIntPrototype,
    /// `%BigInt%`, which carries the two widths of 21.2.2.
    BigIntConstructor,
    /// The namespace object of 25.4.
    Atomics,
    /// `%WeakMap.prototype%`, which carries the methods 24.3.3 gives it.
    WeakMapPrototype,
    /// `%WeakSet.prototype%`, which carries the methods 24.4.3 gives it.
    WeakSetPrototype,
    /// `%IteratorPrototype%`, 27.1.2.
    IteratorPrototype,
    /// `%MapIteratorPrototype%`, 24.1.5.2.
    MapIteratorPrototype,
    /// `%SetIteratorPrototype%`, 24.2.5.2.
    SetIteratorPrototype,
    /// `%Promise.prototype%`, which carries the methods 27.2.5 gives it.
    PromisePrototype,
    /// `%Promise%`, which carries the functions 27.2.4 gives the constructor.
    PromiseConstructor,
}

impl Intrinsic {
    /// Every intrinsic, in the order the Realm allocates them.
    pub const ALL: [Self; 461] = [
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
        Self::ArrayPrototypeForEach,
        Self::ArrayPrototypeMap,
        Self::ArrayPrototypeFilter,
        Self::ArrayPrototypeEvery,
        Self::ArrayPrototypeSome,
        Self::ArrayPrototypeFind,
        Self::ArrayPrototypeFindIndex,
        Self::ArrayPrototypeReduce,
        Self::ArrayPrototypeReduceRight,
        Self::IteratorPrototypeIterator,
        Self::ObjectPreventExtensions,
        Self::ObjectIsExtensible,
        Self::ObjectSeal,
        Self::ObjectIsSealed,
        Self::ObjectFreeze,
        Self::ObjectIsFrozen,
        Self::ObjectValues,
        Self::ObjectEntries,
        Self::NumberConstructor,
        Self::NumberIsFinite,
        Self::NumberIsInteger,
        Self::NumberIsNaN,
        Self::NumberIsSafeInteger,
        Self::BooleanConstructor,
        Self::ReflectDefineProperty,
        Self::ReflectDeleteProperty,
        Self::ReflectGet,
        Self::ReflectGetOwnPropertyDescriptor,
        Self::ReflectGetPrototypeOf,
        Self::ReflectHas,
        Self::ReflectIsExtensible,
        Self::ReflectOwnKeys,
        Self::ReflectPreventExtensions,
        Self::NumberPrototypeValueOf,
        Self::NumberPrototypeToString,
        Self::BooleanPrototypeValueOf,
        Self::BooleanPrototypeToString,
        Self::StringPrototypeValueOf,
        Self::StringPrototypeToString,
        Self::ThrowTypeError,
        Self::SymbolConstructor,
        Self::RegExpConstructor,
        Self::RegExpPrototypeExec,
        Self::RegExpPrototypeTest,
        Self::RegExpPrototypeToString,
        Self::JsonParse,
        Self::JsonStringify,
        Self::StringPrototypeSplit,
        Self::StringPrototypeMatch,
        Self::StringPrototypeSearch,
        Self::IsNaN,
        Self::IsFinite,
        Self::ParseInt,
        Self::ParseFloat,
        Self::SymbolPrototypeToString,
        Self::SymbolPrototypeValueOf,
        Self::SymbolFor,
        Self::SymbolKeyFor,
        Self::ArrayPrototypeFindLast,
        Self::ArrayPrototypeFindLastIndex,
        Self::ArrayPrototypeKeys,
        Self::ArrayPrototypeEntries,
        Self::FunctionPrototypeApply,
        Self::ReflectApply,
        Self::ObjectSetPrototypeOf,
        Self::ReflectSetPrototypeOf,
        Self::FunctionPrototypeToString,
        Self::ObjectPrototypeValueOf,
        Self::FunctionPrototype,
        Self::ErrorPrototypeToString,
        Self::ReflectConstruct,
        Self::ArrayPrototypeFlat,
        Self::ArrayPrototypeSort,
        Self::ArrayPrototypeToSorted,
        Self::ArrayPrototypeToSpliced,
        Self::SpeciesGetter,
        Self::StringPrototypeReplace,
        Self::RegExpPrototypeReplace,
        Self::ArrayOf,
        Self::ArrayFrom,
        Self::Eval,
        Self::StringFromCharCode,
        Self::StringFromCodePoint,
        Self::StringRaw,
        Self::StringPrototypeIsWellFormed,
        Self::StringPrototypeToWellFormed,
        Self::StringPrototypeSubstr,
        Self::StringPrototypeLocaleCompare,
        Self::RegExpPrototypeFlags,
        Self::RegExpPrototypeSource,
        Self::RegExpPrototypeHasIndices,
        Self::RegExpPrototypeGlobal,
        Self::RegExpPrototypeIgnoreCase,
        Self::RegExpPrototypeMultiline,
        Self::RegExpPrototypeDotAll,
        Self::RegExpPrototypeUnicode,
        Self::RegExpPrototypeUnicodeSets,
        Self::RegExpPrototypeSticky,
        Self::PromiseConstructor,
        Self::PromiseResolve,
        Self::PromiseReject,
        Self::PromisePrototypeThen,
        Self::PromisePrototypeCatch,
        Self::PromiseResolveFunction,
        Self::PromiseRejectFunction,
        Self::Print,
        Self::PromiseAll,
        Self::PromiseRace,
        Self::PromiseAllSettled,
        Self::PromiseWithResolvers,
        Self::PromiseAllElement,
        Self::PromiseAllSettledFulfilled,
        Self::PromiseAllSettledRejected,
        Self::RegExpPrototypeMatch,
        Self::RegExpPrototypeSearch,
        Self::RegExpPrototypeSplit,
        Self::AsyncResume,
        Self::AsyncThrow,
        Self::StringPrototypeAnchor,
        Self::StringPrototypeBig,
        Self::StringPrototypeBlink,
        Self::StringPrototypeBold,
        Self::StringPrototypeFixed,
        Self::StringPrototypeFontcolor,
        Self::StringPrototypeFontsize,
        Self::StringPrototypeItalics,
        Self::StringPrototypeLink,
        Self::StringPrototypeSmall,
        Self::StringPrototypeStrike,
        Self::StringPrototypeSub,
        Self::StringPrototypeSup,
        Self::ObjectGetOwnPropertySymbols,
        Self::ObjectGetOwnPropertyDescriptors,
        Self::ObjectAssign,
        Self::ReflectSet,
        Self::ArrayPrototypeFlatMap,
        Self::ObjectFromEntries,
        Self::MathAcos,
        Self::MathAcosh,
        Self::MathAsin,
        Self::MathAsinh,
        Self::MathAtan,
        Self::MathAtan2,
        Self::MathAtanh,
        Self::MathCbrt,
        Self::MathCos,
        Self::MathCosh,
        Self::MathExp,
        Self::MathExpm1,
        Self::MathF16round,
        Self::MathHypot,
        Self::MathLog,
        Self::MathLog10,
        Self::MathLog1p,
        Self::MathLog2,
        Self::MathRandom,
        Self::MathSinh,
        Self::MathSqrt,
        Self::MathTan,
        Self::MathTanh,
        Self::MapConstructor,
        Self::MapPrototypeGet,
        Self::MapPrototypeSet,
        Self::MapPrototypeHas,
        Self::MapPrototypeDelete,
        Self::MapPrototypeClear,
        Self::MapPrototypeSize,
        Self::SetConstructor,
        Self::SetPrototypeAdd,
        Self::SetPrototypeHas,
        Self::SetPrototypeDelete,
        Self::SetPrototypeClear,
        Self::SetPrototypeSize,
        Self::MapPrototypeEntries,
        Self::MapPrototypeKeys,
        Self::MapPrototypeValues,
        Self::SetPrototypeValues,
        Self::SetPrototypeEntries,
        Self::MapIteratorPrototypeNext,
        Self::SetIteratorPrototypeNext,
        Self::SetPrototypeUnion,
        Self::SetPrototypeIntersection,
        Self::SetPrototypeDifference,
        Self::SetPrototypeSymmetricDifference,
        Self::SetPrototypeIsSubsetOf,
        Self::SetPrototypeIsSupersetOf,
        Self::SetPrototypeIsDisjointFrom,
        Self::MapPrototypeForEach,
        Self::SetPrototypeForEach,
        Self::WeakMapConstructor,
        Self::WeakMapPrototypeGet,
        Self::WeakMapPrototypeSet,
        Self::WeakMapPrototypeHas,
        Self::WeakMapPrototypeDelete,
        Self::WeakSetConstructor,
        Self::WeakSetPrototypeAdd,
        Self::WeakSetPrototypeHas,
        Self::WeakSetPrototypeDelete,
        Self::MapPrototypeGetOrInsert,
        Self::MapPrototypeGetOrInsertComputed,
        Self::WeakMapPrototypeGetOrInsert,
        Self::WeakMapPrototypeGetOrInsertComputed,
        Self::JsonRawJson,
        Self::JsonIsRawJson,
        Self::EncodeUri,
        Self::EncodeUriComponent,
        Self::DecodeUri,
        Self::DecodeUriComponent,
        Self::Escape,
        Self::Unescape,
        Self::DateConstructor,
        Self::DateNow,
        Self::DateUtc,
        Self::DateParse,
        Self::DatePrototypeValueOf,
        Self::DatePrototypeGetTime,
        Self::DatePrototypeSetTime,
        Self::DatePrototypeGetTimezoneOffset,
        Self::DatePrototypeGetFullYear,
        Self::DatePrototypeGetUtcFullYear,
        Self::DatePrototypeGetMonth,
        Self::DatePrototypeGetUtcMonth,
        Self::DatePrototypeGetDate,
        Self::DatePrototypeGetUtcDate,
        Self::DatePrototypeGetDay,
        Self::DatePrototypeGetUtcDay,
        Self::DatePrototypeGetHours,
        Self::DatePrototypeGetUtcHours,
        Self::DatePrototypeGetMinutes,
        Self::DatePrototypeGetUtcMinutes,
        Self::DatePrototypeGetSeconds,
        Self::DatePrototypeGetUtcSeconds,
        Self::DatePrototypeGetMilliseconds,
        Self::DatePrototypeGetUtcMilliseconds,
        Self::DatePrototypeToIsoString,
        Self::DatePrototypeToJson,
        Self::DatePrototypeSetMilliseconds,
        Self::DatePrototypeSetUtcMilliseconds,
        Self::DatePrototypeSetSeconds,
        Self::DatePrototypeSetUtcSeconds,
        Self::DatePrototypeSetMinutes,
        Self::DatePrototypeSetUtcMinutes,
        Self::DatePrototypeSetHours,
        Self::DatePrototypeSetUtcHours,
        Self::DatePrototypeSetDate,
        Self::DatePrototypeSetUtcDate,
        Self::DatePrototypeSetMonth,
        Self::DatePrototypeSetUtcMonth,
        Self::DatePrototypeSetFullYear,
        Self::DatePrototypeSetUtcFullYear,
        Self::DatePrototypeSetYear,
        Self::DatePrototypeToString,
        Self::DatePrototypeToDateString,
        Self::DatePrototypeToTimeString,
        Self::DatePrototypeToUtcString,
        Self::DatePrototypeToLocaleString,
        Self::DatePrototypeToLocaleDateString,
        Self::DatePrototypeToLocaleTimeString,
        Self::ArrayBufferConstructor,
        Self::ArrayBufferIsView,
        Self::ArrayBufferPrototypeSlice,
        Self::ArrayBufferPrototypeByteLength,
        Self::ArrayBufferPrototypeDetached,
        Self::ArrayBufferPrototypeResizable,
        Self::ArrayBufferPrototypeMaxByteLength,
        Self::DataViewConstructor,
        Self::DataViewPrototypeBuffer,
        Self::DataViewPrototypeByteLength,
        Self::DataViewPrototypeByteOffset,
        Self::DataViewPrototypeGetInt8,
        Self::DataViewPrototypeSetInt8,
        Self::DataViewPrototypeGetUint8,
        Self::DataViewPrototypeSetUint8,
        Self::DataViewPrototypeGetInt16,
        Self::DataViewPrototypeSetInt16,
        Self::DataViewPrototypeGetUint16,
        Self::DataViewPrototypeSetUint16,
        Self::DataViewPrototypeGetInt32,
        Self::DataViewPrototypeSetInt32,
        Self::DataViewPrototypeGetUint32,
        Self::DataViewPrototypeSetUint32,
        Self::DataViewPrototypeGetFloat32,
        Self::DataViewPrototypeSetFloat32,
        Self::DataViewPrototypeGetFloat64,
        Self::DataViewPrototypeSetFloat64,
        Self::TypedArrayBase,
        Self::TypedArrayInt8Constructor,
        Self::TypedArrayUint8Constructor,
        Self::TypedArrayUint8ClampedConstructor,
        Self::TypedArrayInt16Constructor,
        Self::TypedArrayUint16Constructor,
        Self::TypedArrayInt32Constructor,
        Self::TypedArrayUint32Constructor,
        Self::TypedArrayFloat32Constructor,
        Self::TypedArrayFloat64Constructor,
        Self::TypedArrayFloat16Constructor,
        Self::TypedArrayBigInt64Constructor,
        Self::TypedArrayBigUint64Constructor,
        Self::TypedArrayPrototypeBuffer,
        Self::TypedArrayPrototypeByteLength,
        Self::TypedArrayPrototypeByteOffset,
        Self::TypedArrayPrototypeLength,
        Self::TypedArrayPrototypeToStringTag,
        Self::TypedArrayPrototypeAt,
        Self::TypedArrayPrototypeCopyWithin,
        Self::TypedArrayPrototypeEntries,
        Self::TypedArrayPrototypeFill,
        Self::TypedArrayPrototypeIncludes,
        Self::TypedArrayPrototypeIndexOf,
        Self::TypedArrayPrototypeJoin,
        Self::TypedArrayPrototypeKeys,
        Self::TypedArrayPrototypeLastIndexOf,
        Self::TypedArrayPrototypeReverse,
        Self::TypedArrayPrototypeSet,
        Self::TypedArrayPrototypeSlice,
        Self::TypedArrayPrototypeSort,
        Self::TypedArrayPrototypeSubarray,
        Self::TypedArrayPrototypeToReversed,
        Self::TypedArrayPrototypeToSorted,
        Self::TypedArrayPrototypeValues,
        Self::TypedArrayPrototypeWith,
        Self::TypedArrayPrototypeEvery,
        Self::TypedArrayPrototypeFind,
        Self::TypedArrayPrototypeFindIndex,
        Self::TypedArrayPrototypeFindLast,
        Self::TypedArrayPrototypeFindLastIndex,
        Self::TypedArrayPrototypeForEach,
        Self::TypedArrayPrototypeReduce,
        Self::TypedArrayPrototypeReduceRight,
        Self::TypedArrayPrototypeSome,
        Self::IteratorConstructor,
        Self::IteratorPrototypeToArray,
        Self::IteratorPrototypeForEach,
        Self::IteratorPrototypeSome,
        Self::IteratorPrototypeEvery,
        Self::IteratorPrototypeFind,
        Self::IteratorPrototypeReduce,
        Self::GeneratorPrototypeNext,
        Self::GeneratorPrototypeReturn,
        Self::GeneratorPrototypeThrow,
        Self::AsyncGeneratorPrototypeNext,
        Self::AsyncGeneratorPrototypeReturn,
        Self::AsyncGeneratorPrototypeThrow,
        Self::AsyncFromSyncIteratorPrototypeNext,
        Self::AsyncFromSyncIteratorPrototypeReturn,
        Self::AsyncFromSyncIteratorPrototypeThrow,
        Self::AsyncFromSyncUnwrap,
        Self::AsyncFromSyncCloseIterator,
        Self::AsyncIteratorPrototypeAsyncIterator,
        Self::FunctionPrototypeHasInstance,
        Self::SymbolPrototypeToPrimitive,
        Self::ErrorIsError,
        Self::IteratorPrototypeConstructorGet,
        Self::IteratorPrototypeConstructorSet,
        Self::IteratorPrototypeToStringTagGet,
        Self::IteratorPrototypeToStringTagSet,
        Self::HostDetachArrayBuffer,
        Self::HostGc,
        Self::BigIntConstructor,
        Self::BigIntAsIntN,
        Self::BigIntAsUintN,
        Self::BigIntPrototypeToString,
        Self::BigIntPrototypeToLocaleString,
        Self::BigIntPrototypeValueOf,
        Self::SharedArrayBufferConstructor,
        Self::SharedArrayBufferPrototypeSlice,
        Self::SharedArrayBufferPrototypeGrow,
        Self::SharedArrayBufferPrototypeByteLength,
        Self::SharedArrayBufferPrototypeGrowable,
        Self::SharedArrayBufferPrototypeMaxByteLength,
        Self::AtomicsAdd,
        Self::AtomicsAnd,
        Self::AtomicsCompareExchange,
        Self::AtomicsExchange,
        Self::AtomicsIsLockFree,
        Self::AtomicsLoad,
        Self::AtomicsOr,
        Self::AtomicsPause,
        Self::AtomicsStore,
        Self::AtomicsSub,
        Self::AtomicsWait,
        Self::AtomicsNotify,
        Self::AtomicsXor,
    ];

    /// The intrinsic object this function is installed on.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "one table names every intrinsic beside the object it is installed on"
    )]
    pub const fn holder(self) -> IntrinsicHolder {
        match self {
            Self::ObjectPrototypeHasOwnProperty
            | Self::ObjectPrototypeIsPrototypeOf
            | Self::ObjectPrototypePropertyIsEnumerable
            | Self::ObjectPrototypeToString
            | Self::ObjectPrototypeValueOf => IntrinsicHolder::ObjectPrototype,
            Self::StringPrototypeIsWellFormed
            | Self::StringPrototypeToWellFormed
            | Self::StringPrototypeSubstr
            | Self::StringPrototypeLocaleCompare
            | Self::StringPrototypeCharAt
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
            | Self::StringPrototypeTrimStart
            | Self::StringPrototypeSplit
            | Self::StringPrototypeReplace
            | Self::StringPrototypeMatch
            | Self::StringPrototypeSearch
            | Self::StringPrototypeAnchor
            | Self::StringPrototypeBig
            | Self::StringPrototypeBlink
            | Self::StringPrototypeBold
            | Self::StringPrototypeFixed
            | Self::StringPrototypeFontcolor
            | Self::StringPrototypeFontsize
            | Self::StringPrototypeItalics
            | Self::StringPrototypeLink
            | Self::StringPrototypeSmall
            | Self::StringPrototypeStrike
            | Self::StringPrototypeSub
            | Self::StringPrototypeSup
            => IntrinsicHolder::StringPrototype,
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
            | Self::ArrayPrototypeToReversed
            | Self::ArrayPrototypeFlat
            | Self::ArrayPrototypeSort
            | Self::ArrayPrototypeToSorted
            | Self::ArrayPrototypeToSpliced
            | Self::ArrayPrototypeForEach
            | Self::ArrayPrototypeMap
            | Self::ArrayPrototypeFilter
            | Self::ArrayPrototypeEvery
            | Self::ArrayPrototypeSome
            | Self::ArrayPrototypeFind
            | Self::ArrayPrototypeFindIndex
            | Self::ArrayPrototypeFindLast
            | Self::ArrayPrototypeFindLastIndex
            | Self::ArrayPrototypeKeys
            | Self::ArrayPrototypeEntries
            | Self::ArrayPrototypeReduce
            | Self::ArrayPrototypeReduceRight
            | Self::ArrayPrototypeFlatMap => IntrinsicHolder::ArrayPrototype,
            Self::ArrayIteratorPrototypeNext => IntrinsicHolder::ArrayIteratorPrototype,
            // 27.1.2.1 stands on %IteratorPrototype%, which every iterator of
            // the specification inherits.
            Self::IteratorPrototypeIterator
            | Self::IteratorPrototypeToArray
            | Self::IteratorPrototypeForEach
            | Self::IteratorPrototypeSome
            | Self::IteratorPrototypeEvery
            | Self::IteratorPrototypeFind
            | Self::IteratorPrototypeReduce
            | Self::IteratorPrototypeConstructorGet
            | Self::IteratorPrototypeConstructorSet
            | Self::IteratorPrototypeToStringTagGet
            | Self::IteratorPrototypeToStringTagSet => IntrinsicHolder::IteratorPrototype,
            Self::GeneratorPrototypeNext
            | Self::GeneratorPrototypeReturn
            | Self::GeneratorPrototypeThrow => IntrinsicHolder::GeneratorPrototype,
            Self::AsyncGeneratorPrototypeNext
            | Self::AsyncGeneratorPrototypeReturn
            | Self::AsyncGeneratorPrototypeThrow => IntrinsicHolder::AsyncGeneratorPrototype,
            Self::AsyncFromSyncIteratorPrototypeNext
            | Self::AsyncFromSyncIteratorPrototypeReturn
            | Self::AsyncFromSyncIteratorPrototypeThrow => {
                IntrinsicHolder::AsyncFromSyncIteratorPrototype
            }
            Self::AsyncIteratorPrototypeAsyncIterator => IntrinsicHolder::AsyncIteratorPrototype,
            Self::SymbolPrototypeToPrimitive => IntrinsicHolder::SymbolPrototype,
            Self::ErrorIsError => IntrinsicHolder::ErrorConstructor,
            Self::ArrayConstructor | Self::ObjectConstructor | Self::FunctionConstructor => {
                IntrinsicHolder::Global
            }
            Self::FunctionPrototype
            | Self::FunctionPrototypeCall
            | Self::FunctionPrototypeBind
            | Self::FunctionPrototypeApply
            | Self::FunctionPrototypeToString
            | Self::FunctionPrototypeHasInstance => IntrinsicHolder::FunctionPrototype,
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
            | Self::MathSin
            | Self::MathAcos
            | Self::MathAcosh
            | Self::MathAsin
            | Self::MathAsinh
            | Self::MathAtan
            | Self::MathAtan2
            | Self::MathAtanh
            | Self::MathCbrt
            | Self::MathCos
            | Self::MathCosh
            | Self::MathExp
            | Self::MathExpm1
            | Self::MathF16round
            | Self::MathHypot
            | Self::MathLog
            | Self::MathLog10
            | Self::MathLog1p
            | Self::MathLog2
            | Self::MathRandom
            | Self::MathSinh
            | Self::MathSqrt
            | Self::MathTan
            | Self::MathTanh => IntrinsicHolder::Math,
            Self::ReflectApply
            | Self::ReflectConstruct
            | Self::ReflectSetPrototypeOf
            | Self::ReflectDefineProperty
            | Self::ReflectDeleteProperty
            | Self::ReflectGet
            | Self::ReflectGetOwnPropertyDescriptor
            | Self::ReflectGetPrototypeOf
            | Self::ReflectHas
            | Self::ReflectIsExtensible
            | Self::ReflectOwnKeys
            | Self::ReflectPreventExtensions
            | Self::ReflectSet => IntrinsicHolder::Reflect,
            Self::NumberPrototypeValueOf | Self::NumberPrototypeToString => {
                IntrinsicHolder::NumberPrototype
            }
            Self::BooleanPrototypeValueOf | Self::BooleanPrototypeToString => {
                IntrinsicHolder::BooleanPrototype
            }
            Self::StringPrototypeValueOf | Self::StringPrototypeToString => {
                IntrinsicHolder::StringPrototype
            }
            Self::SymbolPrototypeToString | Self::SymbolPrototypeValueOf => {
                IntrinsicHolder::SymbolPrototype
            }
            Self::SymbolFor | Self::SymbolKeyFor => IntrinsicHolder::SymbolConstructor,
            Self::RegExpPrototypeExec
            | Self::RegExpPrototypeReplace
            | Self::RegExpPrototypeTest
            | Self::RegExpPrototypeToString
            | Self::RegExpPrototypeMatch
            | Self::RegExpPrototypeSearch
            | Self::RegExpPrototypeSplit => IntrinsicHolder::RegExpPrototype,
            Self::ErrorPrototypeToString => IntrinsicHolder::ErrorPrototype,
            Self::MapIteratorPrototypeNext => IntrinsicHolder::MapIteratorPrototype,
            Self::SetIteratorPrototypeNext => IntrinsicHolder::SetIteratorPrototype,
            Self::MapPrototypeGet
            | Self::MapPrototypeSet
            | Self::MapPrototypeHas
            | Self::MapPrototypeDelete
            | Self::MapPrototypeClear
            | Self::MapPrototypeEntries
            | Self::MapPrototypeKeys
            | Self::MapPrototypeValues
            | Self::MapPrototypeGetOrInsert
            | Self::MapPrototypeGetOrInsertComputed
            | Self::MapPrototypeForEach => IntrinsicHolder::MapPrototype,
            Self::SetPrototypeAdd
            | Self::SetPrototypeHas
            | Self::SetPrototypeDelete
            | Self::SetPrototypeClear
            | Self::SetPrototypeValues
            | Self::SetPrototypeEntries
            | Self::SetPrototypeUnion
            | Self::SetPrototypeIntersection
            | Self::SetPrototypeDifference
            | Self::SetPrototypeSymmetricDifference
            | Self::SetPrototypeIsSubsetOf
            | Self::SetPrototypeIsSupersetOf
            | Self::SetPrototypeIsDisjointFrom
            | Self::SetPrototypeForEach => IntrinsicHolder::SetPrototype,
            Self::WeakMapPrototypeGet
            | Self::WeakMapPrototypeSet
            | Self::WeakMapPrototypeHas
            | Self::WeakMapPrototypeDelete
            | Self::WeakMapPrototypeGetOrInsert
            | Self::WeakMapPrototypeGetOrInsertComputed => IntrinsicHolder::WeakMapPrototype,
            Self::WeakSetPrototypeAdd
            | Self::WeakSetPrototypeHas
            | Self::WeakSetPrototypeDelete => IntrinsicHolder::WeakSetPrototype,
            Self::JsonParse | Self::JsonStringify | Self::JsonRawJson | Self::JsonIsRawJson => {
                IntrinsicHolder::Json
            }
            Self::DateNow | Self::DateUtc | Self::DateParse => IntrinsicHolder::DateConstructor,
            Self::ArrayBufferIsView => IntrinsicHolder::ArrayBufferConstructor,
            Self::ArrayBufferPrototypeSlice => IntrinsicHolder::ArrayBufferPrototype,
            Self::BigIntPrototypeToString
            | Self::BigIntPrototypeToLocaleString
            | Self::BigIntPrototypeValueOf => IntrinsicHolder::BigIntPrototype,
            Self::BigIntAsIntN | Self::BigIntAsUintN => IntrinsicHolder::BigIntConstructor,
            Self::SharedArrayBufferPrototypeSlice
            | Self::SharedArrayBufferPrototypeGrow
            | Self::SharedArrayBufferPrototypeByteLength
            | Self::SharedArrayBufferPrototypeGrowable
            | Self::SharedArrayBufferPrototypeMaxByteLength => {
                IntrinsicHolder::SharedArrayBufferPrototype
            }
            Self::AtomicsAdd
            | Self::AtomicsAnd
            | Self::AtomicsCompareExchange
            | Self::AtomicsExchange
            | Self::AtomicsIsLockFree
            | Self::AtomicsLoad
            | Self::AtomicsOr
            | Self::AtomicsPause
            | Self::AtomicsStore
            | Self::AtomicsSub
            | Self::AtomicsWait
            | Self::AtomicsNotify
            | Self::AtomicsXor => IntrinsicHolder::Atomics,
            Self::DataViewPrototypeGetInt8
            | Self::DataViewPrototypeSetInt8
            | Self::DataViewPrototypeGetUint8
            | Self::DataViewPrototypeSetUint8
            | Self::DataViewPrototypeGetInt16
            | Self::DataViewPrototypeSetInt16
            | Self::DataViewPrototypeGetUint16
            | Self::DataViewPrototypeSetUint16
            | Self::DataViewPrototypeGetInt32
            | Self::DataViewPrototypeSetInt32
            | Self::DataViewPrototypeGetUint32
            | Self::DataViewPrototypeSetUint32
            | Self::DataViewPrototypeGetFloat32
            | Self::DataViewPrototypeSetFloat32
            | Self::DataViewPrototypeGetFloat64
            | Self::DataViewPrototypeSetFloat64 => IntrinsicHolder::DataViewPrototype,
            Self::TypedArrayPrototypeBuffer
            | Self::TypedArrayPrototypeByteLength
            | Self::TypedArrayPrototypeByteOffset
            | Self::TypedArrayPrototypeLength
            | Self::TypedArrayPrototypeAt
            | Self::TypedArrayPrototypeCopyWithin
            | Self::TypedArrayPrototypeEntries
            | Self::TypedArrayPrototypeFill
            | Self::TypedArrayPrototypeIncludes
            | Self::TypedArrayPrototypeIndexOf
            | Self::TypedArrayPrototypeJoin
            | Self::TypedArrayPrototypeKeys
            | Self::TypedArrayPrototypeLastIndexOf
            | Self::TypedArrayPrototypeReverse
            | Self::TypedArrayPrototypeSet
            | Self::TypedArrayPrototypeSlice
            | Self::TypedArrayPrototypeSort
            | Self::TypedArrayPrototypeSubarray
            | Self::TypedArrayPrototypeToReversed
            | Self::TypedArrayPrototypeToSorted
            | Self::TypedArrayPrototypeValues
            | Self::TypedArrayPrototypeWith
            | Self::TypedArrayPrototypeEvery
            | Self::TypedArrayPrototypeFind
            | Self::TypedArrayPrototypeFindIndex
            | Self::TypedArrayPrototypeFindLast
            | Self::TypedArrayPrototypeFindLastIndex
            | Self::TypedArrayPrototypeForEach
            | Self::TypedArrayPrototypeReduce
            | Self::TypedArrayPrototypeReduceRight
            | Self::TypedArrayPrototypeSome
            | Self::TypedArrayPrototypeToStringTag => IntrinsicHolder::TypedArrayPrototype,
            Self::DatePrototypeValueOf
            | Self::DatePrototypeGetTime
            | Self::DatePrototypeSetTime
            | Self::DatePrototypeGetTimezoneOffset
            | Self::DatePrototypeGetFullYear
            | Self::DatePrototypeGetUtcFullYear
            | Self::DatePrototypeGetMonth
            | Self::DatePrototypeGetUtcMonth
            | Self::DatePrototypeGetDate
            | Self::DatePrototypeGetUtcDate
            | Self::DatePrototypeGetDay
            | Self::DatePrototypeGetUtcDay
            | Self::DatePrototypeGetHours
            | Self::DatePrototypeGetUtcHours
            | Self::DatePrototypeGetMinutes
            | Self::DatePrototypeGetUtcMinutes
            | Self::DatePrototypeGetSeconds
            | Self::DatePrototypeGetUtcSeconds
            | Self::DatePrototypeGetMilliseconds
            | Self::DatePrototypeGetUtcMilliseconds
            | Self::DatePrototypeToIsoString
            | Self::DatePrototypeToJson
            | Self::DatePrototypeSetMilliseconds
            | Self::DatePrototypeSetUtcMilliseconds
            | Self::DatePrototypeSetSeconds
            | Self::DatePrototypeSetUtcSeconds
            | Self::DatePrototypeSetMinutes
            | Self::DatePrototypeSetUtcMinutes
            | Self::DatePrototypeSetHours
            | Self::DatePrototypeSetUtcHours
            | Self::DatePrototypeSetDate
            | Self::DatePrototypeSetUtcDate
            | Self::DatePrototypeSetMonth
            | Self::DatePrototypeSetUtcMonth
            | Self::DatePrototypeSetFullYear
            | Self::DatePrototypeSetUtcFullYear
            | Self::DatePrototypeSetYear
            | Self::DatePrototypeToString
            | Self::DatePrototypeToDateString
            | Self::DatePrototypeToTimeString
            | Self::DatePrototypeToUtcString
            | Self::DatePrototypeToLocaleString
            | Self::DatePrototypeToLocaleDateString
            | Self::DatePrototypeToLocaleTimeString => IntrinsicHolder::DatePrototype,
            Self::PromisePrototypeThen | Self::PromisePrototypeCatch => {
                IntrinsicHolder::PromisePrototype
            }
            Self::PromiseResolve
            | Self::PromiseReject
            | Self::PromiseAll
            | Self::PromiseRace
            | Self::PromiseAllSettled
            | Self::PromiseWithResolvers => IntrinsicHolder::PromiseConstructor,
            Self::StringFromCharCode | Self::StringFromCodePoint | Self::StringRaw => {
                IntrinsicHolder::StringConstructor
            }
            // 10.2.4.1 stands on no object: the `callee` of a strict
            // arguments object is the only way to reach it, and nothing
            // installs it on the holder this names.
            Self::Eval
            | Self::RegExpPrototypeFlags
            | Self::RegExpPrototypeSource
            | Self::RegExpPrototypeHasIndices
            | Self::RegExpPrototypeGlobal
            | Self::RegExpPrototypeIgnoreCase
            | Self::RegExpPrototypeMultiline
            | Self::RegExpPrototypeDotAll
            | Self::RegExpPrototypeUnicode
            | Self::RegExpPrototypeUnicodeSets
            | Self::RegExpPrototypeSticky
            | Self::SymbolConstructor
            | Self::RegExpConstructor
            | Self::ThrowTypeError
            | Self::SpeciesGetter
            | Self::ErrorConstructor
            | Self::EvalErrorConstructor
            | Self::RangeErrorConstructor
            | Self::ReferenceErrorConstructor
            | Self::SyntaxErrorConstructor
            | Self::TypeErrorConstructor
            | Self::UriErrorConstructor
            | Self::StringConstructor
            | Self::NumberConstructor
            | Self::BooleanConstructor
            | Self::IsNaN
            | Self::IsFinite
            | Self::ParseInt
            | Self::ParseFloat
            | Self::EncodeUri
            | Self::EncodeUriComponent
            | Self::DecodeUri
            | Self::DecodeUriComponent
            | Self::Escape
            | Self::Unescape
            | Self::DateConstructor
            | Self::ArrayBufferConstructor
            | Self::DataViewConstructor
            // 21.2.1 and 25.2.4 give the global object their constructors.
            | Self::BigIntConstructor
            | Self::SharedArrayBufferConstructor
            // 23.2.6 gives the global object the nine of table 71.
            | Self::TypedArrayInt8Constructor
            | Self::TypedArrayUint8Constructor
            | Self::TypedArrayUint8ClampedConstructor
            | Self::TypedArrayInt16Constructor
            | Self::TypedArrayUint16Constructor
            | Self::TypedArrayInt32Constructor
            | Self::TypedArrayUint32Constructor
            | Self::TypedArrayFloat32Constructor
            | Self::TypedArrayFloat64Constructor
            | Self::TypedArrayFloat16Constructor
            | Self::TypedArrayBigInt64Constructor
            | Self::TypedArrayBigUint64Constructor
            // 25.3.4 gives three of them as accessors too.
            | Self::DataViewPrototypeBuffer
            | Self::DataViewPrototypeByteLength
            | Self::DataViewPrototypeByteOffset
            // 23.2.1 names no holder, and nothing installs it.
            | Self::TypedArrayBase
            // 25.1.6 gives four of them as accessors, which the Realm installs
            // beside the methods, so the Global holder installs none of them.
            | Self::ArrayBufferPrototypeByteLength
            | Self::ArrayBufferPrototypeDetached
            | Self::ArrayBufferPrototypeResizable
            | Self::ArrayBufferPrototypeMaxByteLength
            // 27.2.1.3 stands on no object, so the Global holder never
            // installs either resolving function.
            | Self::PromiseConstructor
            | Self::MapConstructor
            | Self::SetConstructor
            | Self::WeakMapConstructor
            | Self::WeakSetConstructor
            // 24.1.3.10 and 24.2.3.14 are accessors of their Prototype, which
            // the Global holder never installs.
            | Self::MapPrototypeSize
            | Self::SetPrototypeSize
            | Self::PromiseResolveFunction
            | Self::PromiseRejectFunction
            // 27.2.4.1.3, 27.2.4.2.2 and 27.2.4.2.3 stand on no object
            // either: the combinator that made one is the only caller.
            | Self::PromiseAllElement
            | Self::PromiseAllSettledFulfilled
            | Self::PromiseAllSettledRejected
            // 27.7.5.3 stands on no object either: the job queue is the only
            // caller of the pair it makes.
            | Self::AsyncResume
            | Self::AsyncThrow
            | Self::AsyncFromSyncUnwrap
            | Self::AsyncFromSyncCloseIterator
            | Self::Print
            // The host object of the conformance suite carries the two, which
            // the Realm attaches where the embedding asks for it.
            | Self::IteratorConstructor
            | Self::HostDetachArrayBuffer
            | Self::HostGc => IntrinsicHolder::Global,
            Self::ArrayIsArray | Self::ArrayOf | Self::ArrayFrom => {
                IntrinsicHolder::ArrayConstructor
            }
            Self::ObjectDefineProperty
            | Self::ObjectGetOwnPropertyDescriptor
            | Self::ObjectGetOwnPropertyNames
            | Self::ObjectCreate
            | Self::ObjectDefineProperties
            | Self::ObjectGetPrototypeOf
            | Self::ObjectSetPrototypeOf
            | Self::ObjectKeys
            | Self::ObjectIs
            | Self::ObjectHasOwn
            | Self::ObjectPreventExtensions
            | Self::ObjectIsExtensible
            | Self::ObjectSeal
            | Self::ObjectIsSealed
            | Self::ObjectFreeze
            | Self::ObjectIsFrozen
            | Self::ObjectValues
            | Self::ObjectEntries
            | Self::ObjectGetOwnPropertySymbols
            | Self::ObjectGetOwnPropertyDescriptors
            | Self::ObjectAssign
            | Self::ObjectFromEntries => IntrinsicHolder::ObjectConstructor,
            Self::NumberIsFinite
            | Self::NumberIsInteger
            | Self::NumberIsNaN
            | Self::NumberIsSafeInteger => IntrinsicHolder::NumberConstructor,
        }
    }

    /// The identifier carried by the function object.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "one table names every intrinsic beside the identifier it carries"
    )]
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
            Self::ArrayPrototypeForEach => 78,
            Self::ArrayPrototypeMap => 79,
            Self::ArrayPrototypeFilter => 80,
            Self::ArrayPrototypeEvery => 81,
            Self::ArrayPrototypeSome => 82,
            Self::ArrayPrototypeFind => 83,
            Self::ArrayPrototypeFindIndex => 84,
            Self::ArrayPrototypeReduce => 85,
            Self::ArrayPrototypeReduceRight => 86,
            Self::IteratorPrototypeIterator => 87,
            Self::ObjectPreventExtensions => 88,
            Self::ObjectIsExtensible => 89,
            Self::ObjectSeal => 90,
            Self::ObjectIsSealed => 91,
            Self::ObjectFreeze => 92,
            Self::ObjectIsFrozen => 93,
            Self::ObjectValues => 94,
            Self::ObjectEntries => 95,
            Self::NumberConstructor => 96,
            Self::NumberIsFinite => 97,
            Self::NumberIsInteger => 98,
            Self::NumberIsNaN => 99,
            Self::NumberIsSafeInteger => 100,
            Self::BooleanConstructor => 101,
            Self::ReflectDefineProperty => 102,
            Self::ReflectDeleteProperty => 103,
            Self::ReflectGet => 104,
            Self::ReflectGetOwnPropertyDescriptor => 105,
            Self::ReflectGetPrototypeOf => 106,
            Self::ReflectHas => 107,
            Self::ReflectIsExtensible => 108,
            Self::ReflectOwnKeys => 109,
            Self::ReflectPreventExtensions => 110,
            Self::NumberPrototypeValueOf => 111,
            Self::NumberPrototypeToString => 112,
            Self::BooleanPrototypeValueOf => 113,
            Self::BooleanPrototypeToString => 114,
            Self::StringPrototypeValueOf => 115,
            Self::StringPrototypeToString => 116,
            Self::ThrowTypeError => 117,
            Self::SymbolConstructor => 118,
            Self::RegExpConstructor => 119,
            Self::RegExpPrototypeExec => 120,
            Self::RegExpPrototypeTest => 121,
            Self::RegExpPrototypeToString => 122,
            Self::JsonParse => 123,
            Self::JsonStringify => 124,
            Self::StringPrototypeSplit => 125,
            Self::StringPrototypeMatch => 126,
            Self::StringPrototypeSearch => 127,
            Self::IsNaN => 128,
            Self::IsFinite => 129,
            Self::ParseInt => 130,
            Self::ParseFloat => 131,
            Self::SymbolPrototypeToString => 132,
            Self::SymbolPrototypeValueOf => 133,
            Self::SymbolFor => 134,
            Self::SymbolKeyFor => 135,
            Self::ArrayPrototypeFindLast => 136,
            Self::ArrayPrototypeFindLastIndex => 137,
            Self::ArrayPrototypeKeys => 138,
            Self::ArrayPrototypeEntries => 139,
            Self::FunctionPrototypeApply => 140,
            Self::ReflectApply => 141,
            Self::ObjectSetPrototypeOf => 142,
            Self::ReflectSetPrototypeOf => 143,
            Self::FunctionPrototypeToString => 144,
            Self::ObjectPrototypeValueOf => 145,
            Self::FunctionPrototype => 146,
            Self::ErrorPrototypeToString => 147,
            Self::ReflectConstruct => 148,
            Self::ArrayPrototypeFlat => 149,
            Self::ArrayPrototypeSort => 150,
            Self::ArrayPrototypeToSorted => 151,
            Self::ArrayPrototypeToSpliced => 152,
            Self::SpeciesGetter => 153,
            Self::StringPrototypeReplace => 154,
            Self::RegExpPrototypeReplace => 155,
            Self::ArrayOf => 156,
            Self::ArrayFrom => 157,
            Self::Eval => 158,
            Self::StringFromCharCode => 169,
            Self::StringFromCodePoint => 170,
            Self::StringRaw => 171,
            Self::StringPrototypeIsWellFormed => 172,
            Self::StringPrototypeToWellFormed => 173,
            Self::StringPrototypeSubstr => 174,
            Self::StringPrototypeLocaleCompare => 175,
            Self::RegExpPrototypeFlags => 159,
            Self::RegExpPrototypeSource => 160,
            Self::RegExpPrototypeHasIndices => 161,
            Self::RegExpPrototypeGlobal => 162,
            Self::RegExpPrototypeIgnoreCase => 163,
            Self::RegExpPrototypeMultiline => 164,
            Self::RegExpPrototypeDotAll => 165,
            Self::RegExpPrototypeUnicode => 166,
            Self::RegExpPrototypeUnicodeSets => 167,
            Self::RegExpPrototypeSticky => 168,
            Self::PromiseConstructor => 176,
            Self::PromiseResolve => 177,
            Self::PromiseReject => 178,
            Self::PromisePrototypeThen => 179,
            Self::PromisePrototypeCatch => 180,
            Self::PromiseResolveFunction => 181,
            Self::PromiseRejectFunction => 182,
            Self::Print => 183,
            Self::PromiseAll => 184,
            Self::PromiseRace => 185,
            Self::PromiseAllSettled => 186,
            Self::PromiseWithResolvers => 187,
            Self::PromiseAllElement => 188,
            Self::PromiseAllSettledFulfilled => 189,
            Self::PromiseAllSettledRejected => 190,
            Self::RegExpPrototypeMatch => 191,
            Self::RegExpPrototypeSearch => 192,
            Self::RegExpPrototypeSplit => 193,
            Self::AsyncResume => 194,
            Self::AsyncThrow => 195,
            Self::StringPrototypeAnchor => 196,
            Self::StringPrototypeBig => 197,
            Self::StringPrototypeBlink => 198,
            Self::StringPrototypeBold => 199,
            Self::StringPrototypeFixed => 200,
            Self::StringPrototypeFontcolor => 201,
            Self::StringPrototypeFontsize => 202,
            Self::StringPrototypeItalics => 203,
            Self::StringPrototypeLink => 204,
            Self::StringPrototypeSmall => 205,
            Self::StringPrototypeStrike => 206,
            Self::StringPrototypeSub => 207,
            Self::StringPrototypeSup => 208,
            Self::ObjectGetOwnPropertySymbols => 209,
            Self::ObjectGetOwnPropertyDescriptors => 210,
            Self::ObjectAssign => 211,
            Self::ReflectSet => 212,
            Self::ArrayPrototypeFlatMap => 213,
            Self::ObjectFromEntries => 214,
            Self::MathAcos => 215,
            Self::MathAcosh => 216,
            Self::MathAsin => 217,
            Self::MathAsinh => 218,
            Self::MathAtan => 219,
            Self::MathAtan2 => 220,
            Self::MathAtanh => 221,
            Self::MathCbrt => 222,
            Self::MathCos => 223,
            Self::MathCosh => 224,
            Self::MathExp => 225,
            Self::MathExpm1 => 226,
            Self::MathF16round => 227,
            Self::MathHypot => 228,
            Self::MathLog => 229,
            Self::MathLog10 => 230,
            Self::MathLog1p => 231,
            Self::MathLog2 => 232,
            Self::MathRandom => 233,
            Self::MathSinh => 234,
            Self::MathSqrt => 235,
            Self::MathTan => 236,
            Self::MathTanh => 237,
            Self::MapConstructor => 238,
            Self::MapPrototypeGet => 239,
            Self::MapPrototypeSet => 240,
            Self::MapPrototypeHas => 241,
            Self::MapPrototypeDelete => 242,
            Self::MapPrototypeClear => 243,
            Self::MapPrototypeSize => 244,
            Self::SetConstructor => 245,
            Self::SetPrototypeAdd => 246,
            Self::SetPrototypeHas => 247,
            Self::SetPrototypeDelete => 248,
            Self::SetPrototypeClear => 249,
            Self::SetPrototypeSize => 250,
            Self::MapPrototypeEntries => 251,
            Self::MapPrototypeKeys => 252,
            Self::MapPrototypeValues => 253,
            Self::SetPrototypeValues => 254,
            Self::SetPrototypeEntries => 255,
            Self::MapIteratorPrototypeNext => 256,
            Self::SetIteratorPrototypeNext => 257,
            Self::SetPrototypeUnion => 258,
            Self::SetPrototypeIntersection => 259,
            Self::SetPrototypeDifference => 260,
            Self::SetPrototypeSymmetricDifference => 261,
            Self::SetPrototypeIsSubsetOf => 262,
            Self::SetPrototypeIsSupersetOf => 263,
            Self::SetPrototypeIsDisjointFrom => 264,
            Self::MapPrototypeForEach => 265,
            Self::SetPrototypeForEach => 266,
            Self::WeakMapConstructor => 267,
            Self::WeakMapPrototypeGet => 268,
            Self::WeakMapPrototypeSet => 269,
            Self::WeakMapPrototypeHas => 270,
            Self::WeakMapPrototypeDelete => 271,
            Self::WeakSetConstructor => 272,
            Self::WeakSetPrototypeAdd => 273,
            Self::WeakSetPrototypeHas => 274,
            Self::WeakSetPrototypeDelete => 275,
            Self::MapPrototypeGetOrInsert => 276,
            Self::MapPrototypeGetOrInsertComputed => 277,
            Self::WeakMapPrototypeGetOrInsert => 278,
            Self::WeakMapPrototypeGetOrInsertComputed => 279,
            Self::JsonRawJson => 280,
            Self::JsonIsRawJson => 281,
            Self::EncodeUri => 282,
            Self::EncodeUriComponent => 283,
            Self::DecodeUri => 284,
            Self::DecodeUriComponent => 285,
            Self::Escape => 286,
            Self::Unescape => 287,
            Self::DateConstructor => 288,
            Self::DateNow => 289,
            Self::DateUtc => 290,
            Self::DateParse => 291,
            Self::DatePrototypeValueOf => 292,
            Self::DatePrototypeGetTime => 293,
            Self::DatePrototypeSetTime => 294,
            Self::DatePrototypeGetTimezoneOffset => 295,
            Self::DatePrototypeGetFullYear => 296,
            Self::DatePrototypeGetUtcFullYear => 297,
            Self::DatePrototypeGetMonth => 298,
            Self::DatePrototypeGetUtcMonth => 299,
            Self::DatePrototypeGetDate => 300,
            Self::DatePrototypeGetUtcDate => 301,
            Self::DatePrototypeGetDay => 302,
            Self::DatePrototypeGetUtcDay => 303,
            Self::DatePrototypeGetHours => 304,
            Self::DatePrototypeGetUtcHours => 305,
            Self::DatePrototypeGetMinutes => 306,
            Self::DatePrototypeGetUtcMinutes => 307,
            Self::DatePrototypeGetSeconds => 308,
            Self::DatePrototypeGetUtcSeconds => 309,
            Self::DatePrototypeGetMilliseconds => 310,
            Self::DatePrototypeGetUtcMilliseconds => 311,
            Self::DatePrototypeToIsoString => 312,
            Self::DatePrototypeToJson => 313,
            Self::DatePrototypeSetMilliseconds => 314,
            Self::DatePrototypeSetUtcMilliseconds => 315,
            Self::DatePrototypeSetSeconds => 316,
            Self::DatePrototypeSetUtcSeconds => 317,
            Self::DatePrototypeSetMinutes => 318,
            Self::DatePrototypeSetUtcMinutes => 319,
            Self::DatePrototypeSetHours => 320,
            Self::DatePrototypeSetUtcHours => 321,
            Self::DatePrototypeSetDate => 322,
            Self::DatePrototypeSetUtcDate => 323,
            Self::DatePrototypeSetMonth => 324,
            Self::DatePrototypeSetUtcMonth => 325,
            Self::DatePrototypeSetFullYear => 326,
            Self::DatePrototypeSetUtcFullYear => 327,
            Self::DatePrototypeSetYear => 328,
            Self::DatePrototypeToString => 329,
            Self::DatePrototypeToDateString => 330,
            Self::DatePrototypeToTimeString => 331,
            Self::DatePrototypeToUtcString => 332,
            Self::DatePrototypeToLocaleString => 333,
            Self::DatePrototypeToLocaleDateString => 334,
            Self::DatePrototypeToLocaleTimeString => 335,
            Self::ArrayBufferConstructor => 336,
            Self::ArrayBufferIsView => 337,
            Self::ArrayBufferPrototypeSlice => 338,
            Self::ArrayBufferPrototypeByteLength => 339,
            Self::ArrayBufferPrototypeDetached => 340,
            Self::ArrayBufferPrototypeResizable => 341,
            Self::ArrayBufferPrototypeMaxByteLength => 342,
            Self::DataViewConstructor => 343,
            Self::DataViewPrototypeBuffer => 344,
            Self::DataViewPrototypeByteLength => 345,
            Self::DataViewPrototypeByteOffset => 346,
            Self::DataViewPrototypeGetInt8 => 347,
            Self::DataViewPrototypeSetInt8 => 348,
            Self::DataViewPrototypeGetUint8 => 349,
            Self::DataViewPrototypeSetUint8 => 350,
            Self::DataViewPrototypeGetInt16 => 351,
            Self::DataViewPrototypeSetInt16 => 352,
            Self::DataViewPrototypeGetUint16 => 353,
            Self::DataViewPrototypeSetUint16 => 354,
            Self::DataViewPrototypeGetInt32 => 355,
            Self::DataViewPrototypeSetInt32 => 356,
            Self::DataViewPrototypeGetUint32 => 357,
            Self::DataViewPrototypeSetUint32 => 358,
            Self::DataViewPrototypeGetFloat32 => 359,
            Self::DataViewPrototypeSetFloat32 => 360,
            Self::DataViewPrototypeGetFloat64 => 361,
            Self::DataViewPrototypeSetFloat64 => 362,
            Self::TypedArrayBase => 363,
            Self::TypedArrayInt8Constructor => 364,
            Self::TypedArrayUint8Constructor => 365,
            Self::TypedArrayUint8ClampedConstructor => 366,
            Self::TypedArrayInt16Constructor => 367,
            Self::TypedArrayUint16Constructor => 368,
            Self::TypedArrayInt32Constructor => 369,
            Self::TypedArrayUint32Constructor => 370,
            Self::TypedArrayFloat32Constructor => 371,
            Self::TypedArrayFloat64Constructor => 372,
            Self::TypedArrayPrototypeBuffer => 373,
            Self::TypedArrayPrototypeByteLength => 374,
            Self::TypedArrayPrototypeByteOffset => 375,
            Self::TypedArrayPrototypeLength => 376,
            Self::TypedArrayPrototypeToStringTag => 377,
            Self::BigIntConstructor => 397,
            Self::BigIntAsIntN => 398,
            Self::BigIntAsUintN => 399,
            Self::BigIntPrototypeToString => 400,
            Self::BigIntPrototypeToLocaleString => 401,
            Self::BigIntPrototypeValueOf => 402,
            Self::TypedArrayFloat16Constructor => 403,
            Self::TypedArrayBigInt64Constructor => 404,
            Self::TypedArrayBigUint64Constructor => 405,
            Self::TypedArrayPrototypeAt => 406,
            Self::TypedArrayPrototypeCopyWithin => 407,
            Self::TypedArrayPrototypeEntries => 408,
            Self::TypedArrayPrototypeFill => 409,
            Self::TypedArrayPrototypeIncludes => 410,
            Self::TypedArrayPrototypeIndexOf => 411,
            Self::TypedArrayPrototypeJoin => 412,
            Self::TypedArrayPrototypeKeys => 413,
            Self::TypedArrayPrototypeLastIndexOf => 414,
            Self::TypedArrayPrototypeReverse => 415,
            Self::TypedArrayPrototypeSet => 416,
            Self::TypedArrayPrototypeSlice => 417,
            Self::TypedArrayPrototypeSort => 418,
            Self::TypedArrayPrototypeSubarray => 419,
            Self::TypedArrayPrototypeToReversed => 420,
            Self::TypedArrayPrototypeToSorted => 421,
            Self::TypedArrayPrototypeValues => 422,
            Self::TypedArrayPrototypeWith => 423,
            Self::TypedArrayPrototypeEvery => 424,
            Self::TypedArrayPrototypeFind => 425,
            Self::TypedArrayPrototypeFindIndex => 426,
            Self::TypedArrayPrototypeFindLast => 427,
            Self::TypedArrayPrototypeFindLastIndex => 428,
            Self::TypedArrayPrototypeForEach => 429,
            Self::TypedArrayPrototypeReduce => 430,
            Self::TypedArrayPrototypeReduceRight => 431,
            Self::TypedArrayPrototypeSome => 432,
            Self::IteratorConstructor => 435,
            Self::IteratorPrototypeToArray => 440,
            Self::IteratorPrototypeForEach => 441,
            Self::IteratorPrototypeSome => 442,
            Self::IteratorPrototypeEvery => 443,
            Self::IteratorPrototypeFind => 444,
            Self::IteratorPrototypeReduce => 445,
            Self::GeneratorPrototypeNext => 446,
            Self::GeneratorPrototypeReturn => 447,
            Self::GeneratorPrototypeThrow => 448,
            Self::AsyncGeneratorPrototypeNext => 449,
            Self::AsyncGeneratorPrototypeReturn => 450,
            Self::AsyncGeneratorPrototypeThrow => 451,
            Self::AsyncFromSyncIteratorPrototypeNext => 452,
            Self::AsyncFromSyncIteratorPrototypeReturn => 453,
            Self::AsyncFromSyncIteratorPrototypeThrow => 454,
            Self::AsyncFromSyncUnwrap => 455,
            Self::AsyncFromSyncCloseIterator => 456,
            Self::AsyncIteratorPrototypeAsyncIterator => 457,
            Self::FunctionPrototypeHasInstance => 458,
            Self::SymbolPrototypeToPrimitive => 459,
            Self::ErrorIsError => 460,
            Self::IteratorPrototypeConstructorGet => 436,
            Self::IteratorPrototypeConstructorSet => 437,
            Self::IteratorPrototypeToStringTagGet => 438,
            Self::IteratorPrototypeToStringTagSet => 439,
            Self::HostDetachArrayBuffer => 433,
            Self::HostGc => 434,
            Self::SharedArrayBufferConstructor => 378,
            Self::SharedArrayBufferPrototypeSlice => 379,
            Self::SharedArrayBufferPrototypeGrow => 380,
            Self::SharedArrayBufferPrototypeByteLength => 381,
            Self::SharedArrayBufferPrototypeGrowable => 382,
            Self::SharedArrayBufferPrototypeMaxByteLength => 383,
            Self::AtomicsAdd => 384,
            Self::AtomicsAnd => 385,
            Self::AtomicsCompareExchange => 386,
            Self::AtomicsExchange => 387,
            Self::AtomicsIsLockFree => 388,
            Self::AtomicsLoad => 389,
            Self::AtomicsOr => 390,
            Self::AtomicsPause => 391,
            Self::AtomicsStore => 392,
            Self::AtomicsSub => 393,
            Self::AtomicsWait => 394,
            Self::AtomicsNotify => 395,
            Self::AtomicsXor => 396,
        }
    }

    /// Index of this intrinsic in [`Self::ALL`].
    #[expect(
        clippy::too_many_lines,
        reason = "one table names every intrinsic beside its place in ALL"
    )]
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
            Self::ArrayPrototypeForEach => 78,
            Self::ArrayPrototypeMap => 79,
            Self::ArrayPrototypeFilter => 80,
            Self::ArrayPrototypeEvery => 81,
            Self::ArrayPrototypeSome => 82,
            Self::ArrayPrototypeFind => 83,
            Self::ArrayPrototypeFindIndex => 84,
            Self::ArrayPrototypeReduce => 85,
            Self::ArrayPrototypeReduceRight => 86,
            Self::IteratorPrototypeIterator => 87,
            Self::ObjectPreventExtensions => 88,
            Self::ObjectIsExtensible => 89,
            Self::ObjectSeal => 90,
            Self::ObjectIsSealed => 91,
            Self::ObjectFreeze => 92,
            Self::ObjectIsFrozen => 93,
            Self::ObjectValues => 94,
            Self::ObjectEntries => 95,
            Self::NumberConstructor => 96,
            Self::NumberIsFinite => 97,
            Self::NumberIsInteger => 98,
            Self::NumberIsNaN => 99,
            Self::NumberIsSafeInteger => 100,
            Self::BooleanConstructor => 101,
            Self::ReflectDefineProperty => 102,
            Self::ReflectDeleteProperty => 103,
            Self::ReflectGet => 104,
            Self::ReflectGetOwnPropertyDescriptor => 105,
            Self::ReflectGetPrototypeOf => 106,
            Self::ReflectHas => 107,
            Self::ReflectIsExtensible => 108,
            Self::ReflectOwnKeys => 109,
            Self::ReflectPreventExtensions => 110,
            Self::NumberPrototypeValueOf => 111,
            Self::NumberPrototypeToString => 112,
            Self::BooleanPrototypeValueOf => 113,
            Self::BooleanPrototypeToString => 114,
            Self::StringPrototypeValueOf => 115,
            Self::StringPrototypeToString => 116,
            Self::ThrowTypeError => 117,
            Self::SymbolConstructor => 118,
            Self::RegExpConstructor => 119,
            Self::RegExpPrototypeExec => 120,
            Self::RegExpPrototypeTest => 121,
            Self::RegExpPrototypeToString => 122,
            Self::JsonParse => 123,
            Self::JsonStringify => 124,
            Self::StringPrototypeSplit => 125,
            Self::StringPrototypeMatch => 126,
            Self::StringPrototypeSearch => 127,
            Self::IsNaN => 128,
            Self::IsFinite => 129,
            Self::ParseInt => 130,
            Self::ParseFloat => 131,
            Self::SymbolPrototypeToString => 132,
            Self::SymbolPrototypeValueOf => 133,
            Self::SymbolFor => 134,
            Self::SymbolKeyFor => 135,
            Self::ArrayPrototypeFindLast => 136,
            Self::ArrayPrototypeFindLastIndex => 137,
            Self::ArrayPrototypeKeys => 138,
            Self::ArrayPrototypeEntries => 139,
            Self::FunctionPrototypeApply => 140,
            Self::ReflectApply => 141,
            Self::ObjectSetPrototypeOf => 142,
            Self::ReflectSetPrototypeOf => 143,
            Self::FunctionPrototypeToString => 144,
            Self::ObjectPrototypeValueOf => 145,
            Self::FunctionPrototype => 146,
            Self::ErrorPrototypeToString => 147,
            Self::ReflectConstruct => 148,
            Self::ArrayPrototypeFlat => 149,
            Self::ArrayPrototypeSort => 150,
            Self::ArrayPrototypeToSorted => 151,
            Self::ArrayPrototypeToSpliced => 152,
            Self::SpeciesGetter => 153,
            Self::StringPrototypeReplace => 154,
            Self::RegExpPrototypeReplace => 155,
            Self::ArrayOf => 156,
            Self::ArrayFrom => 157,
            Self::Eval => 158,
            Self::StringFromCharCode => 169,
            Self::StringFromCodePoint => 170,
            Self::StringRaw => 171,
            Self::StringPrototypeIsWellFormed => 172,
            Self::StringPrototypeToWellFormed => 173,
            Self::StringPrototypeSubstr => 174,
            Self::StringPrototypeLocaleCompare => 175,
            Self::RegExpPrototypeFlags => 159,
            Self::RegExpPrototypeSource => 160,
            Self::RegExpPrototypeHasIndices => 161,
            Self::RegExpPrototypeGlobal => 162,
            Self::RegExpPrototypeIgnoreCase => 163,
            Self::RegExpPrototypeMultiline => 164,
            Self::RegExpPrototypeDotAll => 165,
            Self::RegExpPrototypeUnicode => 166,
            Self::RegExpPrototypeUnicodeSets => 167,
            Self::RegExpPrototypeSticky => 168,
            Self::PromiseConstructor => 176,
            Self::PromiseResolve => 177,
            Self::PromiseReject => 178,
            Self::PromisePrototypeThen => 179,
            Self::PromisePrototypeCatch => 180,
            Self::PromiseResolveFunction => 181,
            Self::PromiseRejectFunction => 182,
            Self::Print => 183,
            Self::PromiseAll => 184,
            Self::PromiseRace => 185,
            Self::PromiseAllSettled => 186,
            Self::PromiseWithResolvers => 187,
            Self::PromiseAllElement => 188,
            Self::PromiseAllSettledFulfilled => 189,
            Self::PromiseAllSettledRejected => 190,
            Self::RegExpPrototypeMatch => 191,
            Self::RegExpPrototypeSearch => 192,
            Self::RegExpPrototypeSplit => 193,
            Self::AsyncResume => 194,
            Self::AsyncThrow => 195,
            Self::StringPrototypeAnchor => 196,
            Self::StringPrototypeBig => 197,
            Self::StringPrototypeBlink => 198,
            Self::StringPrototypeBold => 199,
            Self::StringPrototypeFixed => 200,
            Self::StringPrototypeFontcolor => 201,
            Self::StringPrototypeFontsize => 202,
            Self::StringPrototypeItalics => 203,
            Self::StringPrototypeLink => 204,
            Self::StringPrototypeSmall => 205,
            Self::StringPrototypeStrike => 206,
            Self::StringPrototypeSub => 207,
            Self::StringPrototypeSup => 208,
            Self::ObjectGetOwnPropertySymbols => 209,
            Self::ObjectGetOwnPropertyDescriptors => 210,
            Self::ObjectAssign => 211,
            Self::ReflectSet => 212,
            Self::ArrayPrototypeFlatMap => 213,
            Self::ObjectFromEntries => 214,
            Self::MathAcos => 215,
            Self::MathAcosh => 216,
            Self::MathAsin => 217,
            Self::MathAsinh => 218,
            Self::MathAtan => 219,
            Self::MathAtan2 => 220,
            Self::MathAtanh => 221,
            Self::MathCbrt => 222,
            Self::MathCos => 223,
            Self::MathCosh => 224,
            Self::MathExp => 225,
            Self::MathExpm1 => 226,
            Self::MathF16round => 227,
            Self::MathHypot => 228,
            Self::MathLog => 229,
            Self::MathLog10 => 230,
            Self::MathLog1p => 231,
            Self::MathLog2 => 232,
            Self::MathRandom => 233,
            Self::MathSinh => 234,
            Self::MathSqrt => 235,
            Self::MathTan => 236,
            Self::MathTanh => 237,
            Self::MapConstructor => 238,
            Self::MapPrototypeGet => 239,
            Self::MapPrototypeSet => 240,
            Self::MapPrototypeHas => 241,
            Self::MapPrototypeDelete => 242,
            Self::MapPrototypeClear => 243,
            Self::MapPrototypeSize => 244,
            Self::SetConstructor => 245,
            Self::SetPrototypeAdd => 246,
            Self::SetPrototypeHas => 247,
            Self::SetPrototypeDelete => 248,
            Self::SetPrototypeClear => 249,
            Self::SetPrototypeSize => 250,
            Self::MapPrototypeEntries => 251,
            Self::MapPrototypeKeys => 252,
            Self::MapPrototypeValues => 253,
            Self::SetPrototypeValues => 254,
            Self::SetPrototypeEntries => 255,
            Self::MapIteratorPrototypeNext => 256,
            Self::SetIteratorPrototypeNext => 257,
            Self::SetPrototypeUnion => 258,
            Self::SetPrototypeIntersection => 259,
            Self::SetPrototypeDifference => 260,
            Self::SetPrototypeSymmetricDifference => 261,
            Self::SetPrototypeIsSubsetOf => 262,
            Self::SetPrototypeIsSupersetOf => 263,
            Self::SetPrototypeIsDisjointFrom => 264,
            Self::MapPrototypeForEach => 265,
            Self::SetPrototypeForEach => 266,
            Self::WeakMapConstructor => 267,
            Self::WeakMapPrototypeGet => 268,
            Self::WeakMapPrototypeSet => 269,
            Self::WeakMapPrototypeHas => 270,
            Self::WeakMapPrototypeDelete => 271,
            Self::WeakSetConstructor => 272,
            Self::WeakSetPrototypeAdd => 273,
            Self::WeakSetPrototypeHas => 274,
            Self::WeakSetPrototypeDelete => 275,
            Self::MapPrototypeGetOrInsert => 276,
            Self::MapPrototypeGetOrInsertComputed => 277,
            Self::WeakMapPrototypeGetOrInsert => 278,
            Self::WeakMapPrototypeGetOrInsertComputed => 279,
            Self::JsonRawJson => 280,
            Self::JsonIsRawJson => 281,
            Self::EncodeUri => 282,
            Self::EncodeUriComponent => 283,
            Self::DecodeUri => 284,
            Self::DecodeUriComponent => 285,
            Self::Escape => 286,
            Self::Unescape => 287,
            Self::DateConstructor => 288,
            Self::DateNow => 289,
            Self::DateUtc => 290,
            Self::DateParse => 291,
            Self::DatePrototypeValueOf => 292,
            Self::DatePrototypeGetTime => 293,
            Self::DatePrototypeSetTime => 294,
            Self::DatePrototypeGetTimezoneOffset => 295,
            Self::DatePrototypeGetFullYear => 296,
            Self::DatePrototypeGetUtcFullYear => 297,
            Self::DatePrototypeGetMonth => 298,
            Self::DatePrototypeGetUtcMonth => 299,
            Self::DatePrototypeGetDate => 300,
            Self::DatePrototypeGetUtcDate => 301,
            Self::DatePrototypeGetDay => 302,
            Self::DatePrototypeGetUtcDay => 303,
            Self::DatePrototypeGetHours => 304,
            Self::DatePrototypeGetUtcHours => 305,
            Self::DatePrototypeGetMinutes => 306,
            Self::DatePrototypeGetUtcMinutes => 307,
            Self::DatePrototypeGetSeconds => 308,
            Self::DatePrototypeGetUtcSeconds => 309,
            Self::DatePrototypeGetMilliseconds => 310,
            Self::DatePrototypeGetUtcMilliseconds => 311,
            Self::DatePrototypeToIsoString => 312,
            Self::DatePrototypeToJson => 313,
            Self::DatePrototypeSetMilliseconds => 314,
            Self::DatePrototypeSetUtcMilliseconds => 315,
            Self::DatePrototypeSetSeconds => 316,
            Self::DatePrototypeSetUtcSeconds => 317,
            Self::DatePrototypeSetMinutes => 318,
            Self::DatePrototypeSetUtcMinutes => 319,
            Self::DatePrototypeSetHours => 320,
            Self::DatePrototypeSetUtcHours => 321,
            Self::DatePrototypeSetDate => 322,
            Self::DatePrototypeSetUtcDate => 323,
            Self::DatePrototypeSetMonth => 324,
            Self::DatePrototypeSetUtcMonth => 325,
            Self::DatePrototypeSetFullYear => 326,
            Self::DatePrototypeSetUtcFullYear => 327,
            Self::DatePrototypeSetYear => 328,
            Self::DatePrototypeToString => 329,
            Self::DatePrototypeToDateString => 330,
            Self::DatePrototypeToTimeString => 331,
            Self::DatePrototypeToUtcString => 332,
            Self::DatePrototypeToLocaleString => 333,
            Self::DatePrototypeToLocaleDateString => 334,
            Self::DatePrototypeToLocaleTimeString => 335,
            Self::ArrayBufferConstructor => 336,
            Self::ArrayBufferIsView => 337,
            Self::ArrayBufferPrototypeSlice => 338,
            Self::ArrayBufferPrototypeByteLength => 339,
            Self::ArrayBufferPrototypeDetached => 340,
            Self::ArrayBufferPrototypeResizable => 341,
            Self::ArrayBufferPrototypeMaxByteLength => 342,
            Self::DataViewConstructor => 343,
            Self::DataViewPrototypeBuffer => 344,
            Self::DataViewPrototypeByteLength => 345,
            Self::DataViewPrototypeByteOffset => 346,
            Self::DataViewPrototypeGetInt8 => 347,
            Self::DataViewPrototypeSetInt8 => 348,
            Self::DataViewPrototypeGetUint8 => 349,
            Self::DataViewPrototypeSetUint8 => 350,
            Self::DataViewPrototypeGetInt16 => 351,
            Self::DataViewPrototypeSetInt16 => 352,
            Self::DataViewPrototypeGetUint16 => 353,
            Self::DataViewPrototypeSetUint16 => 354,
            Self::DataViewPrototypeGetInt32 => 355,
            Self::DataViewPrototypeSetInt32 => 356,
            Self::DataViewPrototypeGetUint32 => 357,
            Self::DataViewPrototypeSetUint32 => 358,
            Self::DataViewPrototypeGetFloat32 => 359,
            Self::DataViewPrototypeSetFloat32 => 360,
            Self::DataViewPrototypeGetFloat64 => 361,
            Self::DataViewPrototypeSetFloat64 => 362,
            Self::TypedArrayBase => 363,
            Self::TypedArrayInt8Constructor => 364,
            Self::TypedArrayUint8Constructor => 365,
            Self::TypedArrayUint8ClampedConstructor => 366,
            Self::TypedArrayInt16Constructor => 367,
            Self::TypedArrayUint16Constructor => 368,
            Self::TypedArrayInt32Constructor => 369,
            Self::TypedArrayUint32Constructor => 370,
            Self::TypedArrayFloat32Constructor => 371,
            Self::TypedArrayFloat64Constructor => 372,
            Self::TypedArrayPrototypeBuffer => 373,
            Self::TypedArrayPrototypeByteLength => 374,
            Self::TypedArrayPrototypeByteOffset => 375,
            Self::TypedArrayPrototypeLength => 376,
            Self::TypedArrayPrototypeToStringTag => 377,
            Self::BigIntConstructor => 397,
            Self::BigIntAsIntN => 398,
            Self::BigIntAsUintN => 399,
            Self::BigIntPrototypeToString => 400,
            Self::BigIntPrototypeToLocaleString => 401,
            Self::BigIntPrototypeValueOf => 402,
            Self::TypedArrayFloat16Constructor => 403,
            Self::TypedArrayBigInt64Constructor => 404,
            Self::TypedArrayBigUint64Constructor => 405,
            Self::TypedArrayPrototypeAt => 406,
            Self::TypedArrayPrototypeCopyWithin => 407,
            Self::TypedArrayPrototypeEntries => 408,
            Self::TypedArrayPrototypeFill => 409,
            Self::TypedArrayPrototypeIncludes => 410,
            Self::TypedArrayPrototypeIndexOf => 411,
            Self::TypedArrayPrototypeJoin => 412,
            Self::TypedArrayPrototypeKeys => 413,
            Self::TypedArrayPrototypeLastIndexOf => 414,
            Self::TypedArrayPrototypeReverse => 415,
            Self::TypedArrayPrototypeSet => 416,
            Self::TypedArrayPrototypeSlice => 417,
            Self::TypedArrayPrototypeSort => 418,
            Self::TypedArrayPrototypeSubarray => 419,
            Self::TypedArrayPrototypeToReversed => 420,
            Self::TypedArrayPrototypeToSorted => 421,
            Self::TypedArrayPrototypeValues => 422,
            Self::TypedArrayPrototypeWith => 423,
            Self::TypedArrayPrototypeEvery => 424,
            Self::TypedArrayPrototypeFind => 425,
            Self::TypedArrayPrototypeFindIndex => 426,
            Self::TypedArrayPrototypeFindLast => 427,
            Self::TypedArrayPrototypeFindLastIndex => 428,
            Self::TypedArrayPrototypeForEach => 429,
            Self::TypedArrayPrototypeReduce => 430,
            Self::TypedArrayPrototypeReduceRight => 431,
            Self::TypedArrayPrototypeSome => 432,
            Self::IteratorConstructor => 435,
            Self::IteratorPrototypeToArray => 440,
            Self::IteratorPrototypeForEach => 441,
            Self::IteratorPrototypeSome => 442,
            Self::IteratorPrototypeEvery => 443,
            Self::IteratorPrototypeFind => 444,
            Self::IteratorPrototypeReduce => 445,
            Self::GeneratorPrototypeNext => 446,
            Self::GeneratorPrototypeReturn => 447,
            Self::GeneratorPrototypeThrow => 448,
            Self::AsyncGeneratorPrototypeNext => 449,
            Self::AsyncGeneratorPrototypeReturn => 450,
            Self::AsyncGeneratorPrototypeThrow => 451,
            Self::AsyncFromSyncIteratorPrototypeNext => 452,
            Self::AsyncFromSyncIteratorPrototypeReturn => 453,
            Self::AsyncFromSyncIteratorPrototypeThrow => 454,
            Self::AsyncFromSyncUnwrap => 455,
            Self::AsyncFromSyncCloseIterator => 456,
            Self::AsyncIteratorPrototypeAsyncIterator => 457,
            Self::FunctionPrototypeHasInstance => 458,
            Self::SymbolPrototypeToPrimitive => 459,
            Self::ErrorIsError => 460,
            Self::IteratorPrototypeConstructorGet => 436,
            Self::IteratorPrototypeConstructorSet => 437,
            Self::IteratorPrototypeToStringTagGet => 438,
            Self::IteratorPrototypeToStringTagSet => 439,
            Self::HostDetachArrayBuffer => 433,
            Self::HostGc => 434,
            Self::SharedArrayBufferConstructor => 378,
            Self::SharedArrayBufferPrototypeSlice => 379,
            Self::SharedArrayBufferPrototypeGrow => 380,
            Self::SharedArrayBufferPrototypeByteLength => 381,
            Self::SharedArrayBufferPrototypeGrowable => 382,
            Self::SharedArrayBufferPrototypeMaxByteLength => 383,
            Self::AtomicsAdd => 384,
            Self::AtomicsAnd => 385,
            Self::AtomicsCompareExchange => 386,
            Self::AtomicsExchange => 387,
            Self::AtomicsIsLockFree => 388,
            Self::AtomicsLoad => 389,
            Self::AtomicsOr => 390,
            Self::AtomicsPause => 391,
            Self::AtomicsStore => 392,
            Self::AtomicsSub => 393,
            Self::AtomicsWait => 394,
            Self::AtomicsNotify => 395,
            Self::AtomicsXor => 396,
        }
    }

    /// The intrinsic one identifier denotes.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "one table names every identifier beside the intrinsic it denotes"
    )]
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
            78 => Some(Self::ArrayPrototypeForEach),
            79 => Some(Self::ArrayPrototypeMap),
            80 => Some(Self::ArrayPrototypeFilter),
            81 => Some(Self::ArrayPrototypeEvery),
            82 => Some(Self::ArrayPrototypeSome),
            83 => Some(Self::ArrayPrototypeFind),
            84 => Some(Self::ArrayPrototypeFindIndex),
            85 => Some(Self::ArrayPrototypeReduce),
            86 => Some(Self::ArrayPrototypeReduceRight),
            87 => Some(Self::IteratorPrototypeIterator),
            88 => Some(Self::ObjectPreventExtensions),
            89 => Some(Self::ObjectIsExtensible),
            90 => Some(Self::ObjectSeal),
            91 => Some(Self::ObjectIsSealed),
            92 => Some(Self::ObjectFreeze),
            93 => Some(Self::ObjectIsFrozen),
            94 => Some(Self::ObjectValues),
            95 => Some(Self::ObjectEntries),
            96 => Some(Self::NumberConstructor),
            97 => Some(Self::NumberIsFinite),
            98 => Some(Self::NumberIsInteger),
            99 => Some(Self::NumberIsNaN),
            100 => Some(Self::NumberIsSafeInteger),
            101 => Some(Self::BooleanConstructor),
            102 => Some(Self::ReflectDefineProperty),
            103 => Some(Self::ReflectDeleteProperty),
            104 => Some(Self::ReflectGet),
            105 => Some(Self::ReflectGetOwnPropertyDescriptor),
            106 => Some(Self::ReflectGetPrototypeOf),
            107 => Some(Self::ReflectHas),
            108 => Some(Self::ReflectIsExtensible),
            109 => Some(Self::ReflectOwnKeys),
            110 => Some(Self::ReflectPreventExtensions),
            111 => Some(Self::NumberPrototypeValueOf),
            112 => Some(Self::NumberPrototypeToString),
            113 => Some(Self::BooleanPrototypeValueOf),
            114 => Some(Self::BooleanPrototypeToString),
            115 => Some(Self::StringPrototypeValueOf),
            116 => Some(Self::StringPrototypeToString),
            117 => Some(Self::ThrowTypeError),
            118 => Some(Self::SymbolConstructor),
            119 => Some(Self::RegExpConstructor),
            120 => Some(Self::RegExpPrototypeExec),
            121 => Some(Self::RegExpPrototypeTest),
            122 => Some(Self::RegExpPrototypeToString),
            123 => Some(Self::JsonParse),
            124 => Some(Self::JsonStringify),
            125 => Some(Self::StringPrototypeSplit),
            126 => Some(Self::StringPrototypeMatch),
            127 => Some(Self::StringPrototypeSearch),
            128 => Some(Self::IsNaN),
            129 => Some(Self::IsFinite),
            130 => Some(Self::ParseInt),
            131 => Some(Self::ParseFloat),
            132 => Some(Self::SymbolPrototypeToString),
            133 => Some(Self::SymbolPrototypeValueOf),
            134 => Some(Self::SymbolFor),
            135 => Some(Self::SymbolKeyFor),
            136 => Some(Self::ArrayPrototypeFindLast),
            137 => Some(Self::ArrayPrototypeFindLastIndex),
            138 => Some(Self::ArrayPrototypeKeys),
            139 => Some(Self::ArrayPrototypeEntries),
            140 => Some(Self::FunctionPrototypeApply),
            141 => Some(Self::ReflectApply),
            142 => Some(Self::ObjectSetPrototypeOf),
            143 => Some(Self::ReflectSetPrototypeOf),
            144 => Some(Self::FunctionPrototypeToString),
            145 => Some(Self::ObjectPrototypeValueOf),
            146 => Some(Self::FunctionPrototype),
            147 => Some(Self::ErrorPrototypeToString),
            148 => Some(Self::ReflectConstruct),
            149 => Some(Self::ArrayPrototypeFlat),
            150 => Some(Self::ArrayPrototypeSort),
            151 => Some(Self::ArrayPrototypeToSorted),
            152 => Some(Self::ArrayPrototypeToSpliced),
            153 => Some(Self::SpeciesGetter),
            154 => Some(Self::StringPrototypeReplace),
            155 => Some(Self::RegExpPrototypeReplace),
            156 => Some(Self::ArrayOf),
            157 => Some(Self::ArrayFrom),
            158 => Some(Self::Eval),
            169 => Some(Self::StringFromCharCode),
            170 => Some(Self::StringFromCodePoint),
            171 => Some(Self::StringRaw),
            172 => Some(Self::StringPrototypeIsWellFormed),
            173 => Some(Self::StringPrototypeToWellFormed),
            174 => Some(Self::StringPrototypeSubstr),
            175 => Some(Self::StringPrototypeLocaleCompare),
            159 => Some(Self::RegExpPrototypeFlags),
            160 => Some(Self::RegExpPrototypeSource),
            161 => Some(Self::RegExpPrototypeHasIndices),
            162 => Some(Self::RegExpPrototypeGlobal),
            163 => Some(Self::RegExpPrototypeIgnoreCase),
            164 => Some(Self::RegExpPrototypeMultiline),
            165 => Some(Self::RegExpPrototypeDotAll),
            166 => Some(Self::RegExpPrototypeUnicode),
            167 => Some(Self::RegExpPrototypeUnicodeSets),
            168 => Some(Self::RegExpPrototypeSticky),
            176 => Some(Self::PromiseConstructor),
            177 => Some(Self::PromiseResolve),
            178 => Some(Self::PromiseReject),
            179 => Some(Self::PromisePrototypeThen),
            180 => Some(Self::PromisePrototypeCatch),
            181 => Some(Self::PromiseResolveFunction),
            182 => Some(Self::PromiseRejectFunction),
            183 => Some(Self::Print),
            184 => Some(Self::PromiseAll),
            185 => Some(Self::PromiseRace),
            186 => Some(Self::PromiseAllSettled),
            187 => Some(Self::PromiseWithResolvers),
            188 => Some(Self::PromiseAllElement),
            189 => Some(Self::PromiseAllSettledFulfilled),
            190 => Some(Self::PromiseAllSettledRejected),
            191 => Some(Self::RegExpPrototypeMatch),
            192 => Some(Self::RegExpPrototypeSearch),
            193 => Some(Self::RegExpPrototypeSplit),
            194 => Some(Self::AsyncResume),
            195 => Some(Self::AsyncThrow),
            196 => Some(Self::StringPrototypeAnchor),
            197 => Some(Self::StringPrototypeBig),
            198 => Some(Self::StringPrototypeBlink),
            199 => Some(Self::StringPrototypeBold),
            200 => Some(Self::StringPrototypeFixed),
            201 => Some(Self::StringPrototypeFontcolor),
            202 => Some(Self::StringPrototypeFontsize),
            203 => Some(Self::StringPrototypeItalics),
            204 => Some(Self::StringPrototypeLink),
            205 => Some(Self::StringPrototypeSmall),
            206 => Some(Self::StringPrototypeStrike),
            207 => Some(Self::StringPrototypeSub),
            208 => Some(Self::StringPrototypeSup),
            209 => Some(Self::ObjectGetOwnPropertySymbols),
            210 => Some(Self::ObjectGetOwnPropertyDescriptors),
            211 => Some(Self::ObjectAssign),
            212 => Some(Self::ReflectSet),
            213 => Some(Self::ArrayPrototypeFlatMap),
            214 => Some(Self::ObjectFromEntries),
            215 => Some(Self::MathAcos),
            216 => Some(Self::MathAcosh),
            217 => Some(Self::MathAsin),
            218 => Some(Self::MathAsinh),
            219 => Some(Self::MathAtan),
            220 => Some(Self::MathAtan2),
            221 => Some(Self::MathAtanh),
            222 => Some(Self::MathCbrt),
            223 => Some(Self::MathCos),
            224 => Some(Self::MathCosh),
            225 => Some(Self::MathExp),
            226 => Some(Self::MathExpm1),
            227 => Some(Self::MathF16round),
            228 => Some(Self::MathHypot),
            229 => Some(Self::MathLog),
            230 => Some(Self::MathLog10),
            231 => Some(Self::MathLog1p),
            232 => Some(Self::MathLog2),
            233 => Some(Self::MathRandom),
            234 => Some(Self::MathSinh),
            235 => Some(Self::MathSqrt),
            236 => Some(Self::MathTan),
            237 => Some(Self::MathTanh),
            238 => Some(Self::MapConstructor),
            239 => Some(Self::MapPrototypeGet),
            240 => Some(Self::MapPrototypeSet),
            241 => Some(Self::MapPrototypeHas),
            242 => Some(Self::MapPrototypeDelete),
            243 => Some(Self::MapPrototypeClear),
            244 => Some(Self::MapPrototypeSize),
            245 => Some(Self::SetConstructor),
            246 => Some(Self::SetPrototypeAdd),
            247 => Some(Self::SetPrototypeHas),
            248 => Some(Self::SetPrototypeDelete),
            249 => Some(Self::SetPrototypeClear),
            250 => Some(Self::SetPrototypeSize),
            251 => Some(Self::MapPrototypeEntries),
            252 => Some(Self::MapPrototypeKeys),
            253 => Some(Self::MapPrototypeValues),
            254 => Some(Self::SetPrototypeValues),
            255 => Some(Self::SetPrototypeEntries),
            256 => Some(Self::MapIteratorPrototypeNext),
            257 => Some(Self::SetIteratorPrototypeNext),
            258 => Some(Self::SetPrototypeUnion),
            259 => Some(Self::SetPrototypeIntersection),
            260 => Some(Self::SetPrototypeDifference),
            261 => Some(Self::SetPrototypeSymmetricDifference),
            262 => Some(Self::SetPrototypeIsSubsetOf),
            263 => Some(Self::SetPrototypeIsSupersetOf),
            264 => Some(Self::SetPrototypeIsDisjointFrom),
            265 => Some(Self::MapPrototypeForEach),
            266 => Some(Self::SetPrototypeForEach),
            267 => Some(Self::WeakMapConstructor),
            268 => Some(Self::WeakMapPrototypeGet),
            269 => Some(Self::WeakMapPrototypeSet),
            270 => Some(Self::WeakMapPrototypeHas),
            271 => Some(Self::WeakMapPrototypeDelete),
            272 => Some(Self::WeakSetConstructor),
            273 => Some(Self::WeakSetPrototypeAdd),
            274 => Some(Self::WeakSetPrototypeHas),
            275 => Some(Self::WeakSetPrototypeDelete),
            276 => Some(Self::MapPrototypeGetOrInsert),
            277 => Some(Self::MapPrototypeGetOrInsertComputed),
            278 => Some(Self::WeakMapPrototypeGetOrInsert),
            279 => Some(Self::WeakMapPrototypeGetOrInsertComputed),
            280 => Some(Self::JsonRawJson),
            281 => Some(Self::JsonIsRawJson),
            282 => Some(Self::EncodeUri),
            283 => Some(Self::EncodeUriComponent),
            284 => Some(Self::DecodeUri),
            285 => Some(Self::DecodeUriComponent),
            286 => Some(Self::Escape),
            287 => Some(Self::Unescape),
            288 => Some(Self::DateConstructor),
            289 => Some(Self::DateNow),
            290 => Some(Self::DateUtc),
            291 => Some(Self::DateParse),
            292 => Some(Self::DatePrototypeValueOf),
            293 => Some(Self::DatePrototypeGetTime),
            294 => Some(Self::DatePrototypeSetTime),
            295 => Some(Self::DatePrototypeGetTimezoneOffset),
            296 => Some(Self::DatePrototypeGetFullYear),
            297 => Some(Self::DatePrototypeGetUtcFullYear),
            298 => Some(Self::DatePrototypeGetMonth),
            299 => Some(Self::DatePrototypeGetUtcMonth),
            300 => Some(Self::DatePrototypeGetDate),
            301 => Some(Self::DatePrototypeGetUtcDate),
            302 => Some(Self::DatePrototypeGetDay),
            303 => Some(Self::DatePrototypeGetUtcDay),
            304 => Some(Self::DatePrototypeGetHours),
            305 => Some(Self::DatePrototypeGetUtcHours),
            306 => Some(Self::DatePrototypeGetMinutes),
            307 => Some(Self::DatePrototypeGetUtcMinutes),
            308 => Some(Self::DatePrototypeGetSeconds),
            309 => Some(Self::DatePrototypeGetUtcSeconds),
            310 => Some(Self::DatePrototypeGetMilliseconds),
            311 => Some(Self::DatePrototypeGetUtcMilliseconds),
            312 => Some(Self::DatePrototypeToIsoString),
            313 => Some(Self::DatePrototypeToJson),
            314 => Some(Self::DatePrototypeSetMilliseconds),
            315 => Some(Self::DatePrototypeSetUtcMilliseconds),
            316 => Some(Self::DatePrototypeSetSeconds),
            317 => Some(Self::DatePrototypeSetUtcSeconds),
            318 => Some(Self::DatePrototypeSetMinutes),
            319 => Some(Self::DatePrototypeSetUtcMinutes),
            320 => Some(Self::DatePrototypeSetHours),
            321 => Some(Self::DatePrototypeSetUtcHours),
            322 => Some(Self::DatePrototypeSetDate),
            323 => Some(Self::DatePrototypeSetUtcDate),
            324 => Some(Self::DatePrototypeSetMonth),
            325 => Some(Self::DatePrototypeSetUtcMonth),
            326 => Some(Self::DatePrototypeSetFullYear),
            327 => Some(Self::DatePrototypeSetUtcFullYear),
            328 => Some(Self::DatePrototypeSetYear),
            329 => Some(Self::DatePrototypeToString),
            330 => Some(Self::DatePrototypeToDateString),
            331 => Some(Self::DatePrototypeToTimeString),
            332 => Some(Self::DatePrototypeToUtcString),
            333 => Some(Self::DatePrototypeToLocaleString),
            334 => Some(Self::DatePrototypeToLocaleDateString),
            335 => Some(Self::DatePrototypeToLocaleTimeString),
            336 => Some(Self::ArrayBufferConstructor),
            337 => Some(Self::ArrayBufferIsView),
            338 => Some(Self::ArrayBufferPrototypeSlice),
            339 => Some(Self::ArrayBufferPrototypeByteLength),
            340 => Some(Self::ArrayBufferPrototypeDetached),
            341 => Some(Self::ArrayBufferPrototypeResizable),
            342 => Some(Self::ArrayBufferPrototypeMaxByteLength),
            343 => Some(Self::DataViewConstructor),
            344 => Some(Self::DataViewPrototypeBuffer),
            345 => Some(Self::DataViewPrototypeByteLength),
            346 => Some(Self::DataViewPrototypeByteOffset),
            347 => Some(Self::DataViewPrototypeGetInt8),
            348 => Some(Self::DataViewPrototypeSetInt8),
            349 => Some(Self::DataViewPrototypeGetUint8),
            350 => Some(Self::DataViewPrototypeSetUint8),
            351 => Some(Self::DataViewPrototypeGetInt16),
            352 => Some(Self::DataViewPrototypeSetInt16),
            353 => Some(Self::DataViewPrototypeGetUint16),
            354 => Some(Self::DataViewPrototypeSetUint16),
            355 => Some(Self::DataViewPrototypeGetInt32),
            356 => Some(Self::DataViewPrototypeSetInt32),
            357 => Some(Self::DataViewPrototypeGetUint32),
            358 => Some(Self::DataViewPrototypeSetUint32),
            359 => Some(Self::DataViewPrototypeGetFloat32),
            360 => Some(Self::DataViewPrototypeSetFloat32),
            361 => Some(Self::DataViewPrototypeGetFloat64),
            362 => Some(Self::DataViewPrototypeSetFloat64),
            363 => Some(Self::TypedArrayBase),
            364 => Some(Self::TypedArrayInt8Constructor),
            365 => Some(Self::TypedArrayUint8Constructor),
            366 => Some(Self::TypedArrayUint8ClampedConstructor),
            367 => Some(Self::TypedArrayInt16Constructor),
            368 => Some(Self::TypedArrayUint16Constructor),
            369 => Some(Self::TypedArrayInt32Constructor),
            370 => Some(Self::TypedArrayUint32Constructor),
            371 => Some(Self::TypedArrayFloat32Constructor),
            372 => Some(Self::TypedArrayFloat64Constructor),
            373 => Some(Self::TypedArrayPrototypeBuffer),
            374 => Some(Self::TypedArrayPrototypeByteLength),
            375 => Some(Self::TypedArrayPrototypeByteOffset),
            376 => Some(Self::TypedArrayPrototypeLength),
            377 => Some(Self::TypedArrayPrototypeToStringTag),
            397 => Some(Self::BigIntConstructor),
            398 => Some(Self::BigIntAsIntN),
            399 => Some(Self::BigIntAsUintN),
            400 => Some(Self::BigIntPrototypeToString),
            401 => Some(Self::BigIntPrototypeToLocaleString),
            402 => Some(Self::BigIntPrototypeValueOf),
            403 => Some(Self::TypedArrayFloat16Constructor),
            404 => Some(Self::TypedArrayBigInt64Constructor),
            405 => Some(Self::TypedArrayBigUint64Constructor),
            406 => Some(Self::TypedArrayPrototypeAt),
            407 => Some(Self::TypedArrayPrototypeCopyWithin),
            408 => Some(Self::TypedArrayPrototypeEntries),
            409 => Some(Self::TypedArrayPrototypeFill),
            410 => Some(Self::TypedArrayPrototypeIncludes),
            411 => Some(Self::TypedArrayPrototypeIndexOf),
            412 => Some(Self::TypedArrayPrototypeJoin),
            413 => Some(Self::TypedArrayPrototypeKeys),
            414 => Some(Self::TypedArrayPrototypeLastIndexOf),
            415 => Some(Self::TypedArrayPrototypeReverse),
            416 => Some(Self::TypedArrayPrototypeSet),
            417 => Some(Self::TypedArrayPrototypeSlice),
            418 => Some(Self::TypedArrayPrototypeSort),
            419 => Some(Self::TypedArrayPrototypeSubarray),
            420 => Some(Self::TypedArrayPrototypeToReversed),
            421 => Some(Self::TypedArrayPrototypeToSorted),
            422 => Some(Self::TypedArrayPrototypeValues),
            423 => Some(Self::TypedArrayPrototypeWith),
            424 => Some(Self::TypedArrayPrototypeEvery),
            425 => Some(Self::TypedArrayPrototypeFind),
            426 => Some(Self::TypedArrayPrototypeFindIndex),
            427 => Some(Self::TypedArrayPrototypeFindLast),
            428 => Some(Self::TypedArrayPrototypeFindLastIndex),
            429 => Some(Self::TypedArrayPrototypeForEach),
            430 => Some(Self::TypedArrayPrototypeReduce),
            431 => Some(Self::TypedArrayPrototypeReduceRight),
            432 => Some(Self::TypedArrayPrototypeSome),
            435 => Some(Self::IteratorConstructor),
            440 => Some(Self::IteratorPrototypeToArray),
            441 => Some(Self::IteratorPrototypeForEach),
            442 => Some(Self::IteratorPrototypeSome),
            443 => Some(Self::IteratorPrototypeEvery),
            444 => Some(Self::IteratorPrototypeFind),
            445 => Some(Self::IteratorPrototypeReduce),
            446 => Some(Self::GeneratorPrototypeNext),
            447 => Some(Self::GeneratorPrototypeReturn),
            448 => Some(Self::GeneratorPrototypeThrow),
            449 => Some(Self::AsyncGeneratorPrototypeNext),
            450 => Some(Self::AsyncGeneratorPrototypeReturn),
            451 => Some(Self::AsyncGeneratorPrototypeThrow),
            452 => Some(Self::AsyncFromSyncIteratorPrototypeNext),
            453 => Some(Self::AsyncFromSyncIteratorPrototypeReturn),
            454 => Some(Self::AsyncFromSyncIteratorPrototypeThrow),
            455 => Some(Self::AsyncFromSyncUnwrap),
            456 => Some(Self::AsyncFromSyncCloseIterator),
            457 => Some(Self::AsyncIteratorPrototypeAsyncIterator),
            458 => Some(Self::FunctionPrototypeHasInstance),
            459 => Some(Self::SymbolPrototypeToPrimitive),
            460 => Some(Self::ErrorIsError),
            436 => Some(Self::IteratorPrototypeConstructorGet),
            437 => Some(Self::IteratorPrototypeConstructorSet),
            438 => Some(Self::IteratorPrototypeToStringTagGet),
            439 => Some(Self::IteratorPrototypeToStringTagSet),
            433 => Some(Self::HostDetachArrayBuffer),
            434 => Some(Self::HostGc),
            378 => Some(Self::SharedArrayBufferConstructor),
            379 => Some(Self::SharedArrayBufferPrototypeSlice),
            380 => Some(Self::SharedArrayBufferPrototypeGrow),
            381 => Some(Self::SharedArrayBufferPrototypeByteLength),
            382 => Some(Self::SharedArrayBufferPrototypeGrowable),
            383 => Some(Self::SharedArrayBufferPrototypeMaxByteLength),
            384 => Some(Self::AtomicsAdd),
            385 => Some(Self::AtomicsAnd),
            386 => Some(Self::AtomicsCompareExchange),
            387 => Some(Self::AtomicsExchange),
            388 => Some(Self::AtomicsIsLockFree),
            389 => Some(Self::AtomicsLoad),
            390 => Some(Self::AtomicsOr),
            391 => Some(Self::AtomicsPause),
            392 => Some(Self::AtomicsStore),
            393 => Some(Self::AtomicsSub),
            394 => Some(Self::AtomicsWait),
            395 => Some(Self::AtomicsNotify),
            396 => Some(Self::AtomicsXor),
            _ => None,
        }
    }

    /// The `name` property of the function object (17).
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "one table names every intrinsic beside the name 17 gives it"
    )]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ObjectPrototypeHasOwnProperty => "hasOwnProperty",
            Self::ObjectPrototypeIsPrototypeOf => "isPrototypeOf",
            Self::ObjectPrototypePropertyIsEnumerable => "propertyIsEnumerable",
            Self::ObjectPrototypeToString
            | Self::ArrayPrototypeToString
            | Self::NumberPrototypeToString
            | Self::BooleanPrototypeToString
            | Self::RegExpPrototypeToString
            | Self::SymbolPrototypeToString
            | Self::FunctionPrototypeToString
            | Self::ErrorPrototypeToString
            | Self::StringPrototypeToString
            | Self::DatePrototypeToString
            | Self::BigIntPrototypeToString => "toString",
            Self::NumberPrototypeValueOf
            | Self::BooleanPrototypeValueOf
            | Self::SymbolPrototypeValueOf
            | Self::StringPrototypeValueOf
            | Self::ObjectPrototypeValueOf
            | Self::DatePrototypeValueOf
            | Self::BigIntPrototypeValueOf => "valueOf",
            // 27.2.1.3 makes the pair of resolving functions with no name,
            // as 10.2.4.1 and 20.2.3 carry none either.
            Self::ThrowTypeError
            | Self::FunctionPrototype
            | Self::PromiseResolveFunction
            | Self::PromiseRejectFunction
            | Self::PromiseAllElement
            | Self::PromiseAllSettledFulfilled
            | Self::PromiseAllSettledRejected
            | Self::AsyncResume
            | Self::AsyncThrow
            | Self::AsyncFromSyncUnwrap
            | Self::AsyncFromSyncCloseIterator => "",
            Self::StringPrototypeAnchor => "anchor",
            Self::StringPrototypeBig => "big",
            Self::StringPrototypeBlink => "blink",
            Self::StringPrototypeBold => "bold",
            Self::StringPrototypeFixed => "fixed",
            Self::StringPrototypeFontcolor => "fontcolor",
            Self::StringPrototypeFontsize => "fontsize",
            Self::StringPrototypeItalics => "italics",
            Self::StringPrototypeLink => "link",
            Self::StringPrototypeSmall => "small",
            Self::StringPrototypeStrike => "strike",
            Self::StringPrototypeSub | Self::AtomicsSub => "sub",
            Self::StringPrototypeSup => "sup",
            Self::ObjectGetOwnPropertySymbols => "getOwnPropertySymbols",
            Self::ObjectGetOwnPropertyDescriptors => "getOwnPropertyDescriptors",
            Self::ObjectAssign => "assign",
            Self::ReflectSet
            | Self::MapPrototypeSet
            | Self::WeakMapPrototypeSet
            | Self::TypedArrayPrototypeSet => "set",
            Self::ArrayPrototypeFlatMap => "flatMap",
            Self::ObjectFromEntries => "fromEntries",
            Self::MathAcos => "acos",
            Self::MathAcosh => "acosh",
            Self::MathAsin => "asin",
            Self::MathAsinh => "asinh",
            Self::MathAtan => "atan",
            Self::MathAtan2 => "atan2",
            Self::MathAtanh => "atanh",
            Self::MathCbrt => "cbrt",
            Self::MathCos => "cos",
            Self::MathCosh => "cosh",
            Self::MathExp => "exp",
            Self::MathExpm1 => "expm1",
            Self::MathF16round => "f16round",
            Self::MathHypot => "hypot",
            Self::MathLog => "log",
            Self::MathLog10 => "log10",
            Self::MathLog1p => "log1p",
            Self::MathLog2 => "log2",
            Self::MathRandom => "random",
            Self::MathSinh => "sinh",
            Self::MathSqrt => "sqrt",
            Self::MathTan => "tan",
            Self::MathTanh => "tanh",
            Self::MapConstructor => "Map",
            Self::WeakMapConstructor => "WeakMap",
            Self::JsonRawJson => "rawJSON",
            Self::JsonIsRawJson => "isRawJSON",
            Self::EncodeUri => "encodeURI",
            Self::EncodeUriComponent => "encodeURIComponent",
            Self::DecodeUri => "decodeURI",
            Self::DecodeUriComponent => "decodeURIComponent",
            Self::Escape => "escape",
            Self::Unescape => "unescape",
            Self::DateConstructor => "Date",
            Self::DateNow => "now",
            Self::DateUtc => "UTC",
            Self::DatePrototypeGetTime => "getTime",
            Self::DatePrototypeSetTime => "setTime",
            Self::DatePrototypeGetTimezoneOffset => "getTimezoneOffset",
            Self::DatePrototypeGetFullYear => "getFullYear",
            Self::DatePrototypeGetUtcFullYear => "getUTCFullYear",
            Self::DatePrototypeGetMonth => "getMonth",
            Self::DatePrototypeGetUtcMonth => "getUTCMonth",
            Self::DatePrototypeGetDate => "getDate",
            Self::DatePrototypeGetUtcDate => "getUTCDate",
            Self::DatePrototypeGetDay => "getDay",
            Self::DatePrototypeGetUtcDay => "getUTCDay",
            Self::DatePrototypeGetHours => "getHours",
            Self::DatePrototypeGetUtcHours => "getUTCHours",
            Self::DatePrototypeGetMinutes => "getMinutes",
            Self::DatePrototypeGetUtcMinutes => "getUTCMinutes",
            Self::DatePrototypeGetSeconds => "getSeconds",
            Self::DatePrototypeGetUtcSeconds => "getUTCSeconds",
            Self::DatePrototypeGetMilliseconds => "getMilliseconds",
            Self::DatePrototypeGetUtcMilliseconds => "getUTCMilliseconds",
            Self::DatePrototypeToIsoString => "toISOString",
            Self::DatePrototypeToJson => "toJSON",
            Self::DatePrototypeToDateString => "toDateString",
            Self::DatePrototypeToTimeString => "toTimeString",
            Self::DatePrototypeToUtcString => "toUTCString",
            Self::DatePrototypeToLocaleString | Self::BigIntPrototypeToLocaleString => {
                "toLocaleString"
            }
            Self::DatePrototypeToLocaleDateString => "toLocaleDateString",
            Self::DatePrototypeToLocaleTimeString => "toLocaleTimeString",
            Self::ArrayBufferConstructor => "ArrayBuffer",
            Self::ArrayBufferIsView => "isView",
            Self::ArrayBufferPrototypeByteLength
            | Self::DataViewPrototypeByteLength
            | Self::TypedArrayPrototypeByteLength
            | Self::SharedArrayBufferPrototypeByteLength => "get byteLength",
            Self::ArrayBufferPrototypeDetached => "get detached",
            Self::ArrayBufferPrototypeResizable => "get resizable",
            Self::ArrayBufferPrototypeMaxByteLength
            | Self::SharedArrayBufferPrototypeMaxByteLength => "get maxByteLength",
            Self::DataViewConstructor => "DataView",
            Self::DataViewPrototypeBuffer | Self::TypedArrayPrototypeBuffer => "get buffer",
            Self::DataViewPrototypeByteOffset | Self::TypedArrayPrototypeByteOffset => {
                "get byteOffset"
            }
            Self::DataViewPrototypeGetInt8 => "getInt8",
            Self::DataViewPrototypeSetInt8 => "setInt8",
            Self::DataViewPrototypeGetUint8 => "getUint8",
            Self::DataViewPrototypeSetUint8 => "setUint8",
            Self::DataViewPrototypeGetInt16 => "getInt16",
            Self::DataViewPrototypeSetInt16 => "setInt16",
            Self::DataViewPrototypeGetUint16 => "getUint16",
            Self::DataViewPrototypeSetUint16 => "setUint16",
            Self::DataViewPrototypeGetInt32 => "getInt32",
            Self::DataViewPrototypeSetInt32 => "setInt32",
            Self::DataViewPrototypeGetUint32 => "getUint32",
            Self::DataViewPrototypeSetUint32 => "setUint32",
            Self::DataViewPrototypeGetFloat32 => "getFloat32",
            Self::DataViewPrototypeSetFloat32 => "setFloat32",
            Self::DataViewPrototypeGetFloat64 => "getFloat64",
            Self::DataViewPrototypeSetFloat64 => "setFloat64",
            Self::TypedArrayBase => "TypedArray",
            Self::TypedArrayInt8Constructor => "Int8Array",
            Self::TypedArrayUint8Constructor => "Uint8Array",
            Self::TypedArrayUint8ClampedConstructor => "Uint8ClampedArray",
            Self::TypedArrayInt16Constructor => "Int16Array",
            Self::TypedArrayUint16Constructor => "Uint16Array",
            Self::TypedArrayInt32Constructor => "Int32Array",
            Self::TypedArrayUint32Constructor => "Uint32Array",
            Self::TypedArrayFloat32Constructor => "Float32Array",
            Self::TypedArrayFloat64Constructor => "Float64Array",
            Self::TypedArrayFloat16Constructor => "Float16Array",
            Self::TypedArrayBigInt64Constructor => "BigInt64Array",
            Self::TypedArrayBigUint64Constructor => "BigUint64Array",
            Self::TypedArrayPrototypeLength => "get length",
            Self::TypedArrayPrototypeToStringTag | Self::IteratorPrototypeToStringTagGet => {
                "get [Symbol.toStringTag]"
            }
            Self::TypedArrayPrototypeSubarray => "subarray",
            Self::IteratorConstructor => "Iterator",
            Self::IteratorPrototypeToArray => "toArray",
            Self::IteratorPrototypeConstructorGet => "get constructor",
            Self::IteratorPrototypeConstructorSet => "set constructor",
            Self::IteratorPrototypeToStringTagSet => "set [Symbol.toStringTag]",
            Self::HostDetachArrayBuffer => "detachArrayBuffer",
            Self::HostGc => "gc",
            Self::BigIntConstructor => "BigInt",
            Self::BigIntAsIntN => "asIntN",
            Self::BigIntAsUintN => "asUintN",
            Self::SharedArrayBufferConstructor => "SharedArrayBuffer",
            Self::SharedArrayBufferPrototypeGrow => "grow",
            Self::SharedArrayBufferPrototypeGrowable => "get growable",
            Self::AtomicsAnd => "and",
            Self::AtomicsCompareExchange => "compareExchange",
            Self::AtomicsExchange => "exchange",
            Self::AtomicsIsLockFree => "isLockFree",
            Self::AtomicsLoad => "load",
            Self::AtomicsOr => "or",
            Self::AtomicsPause => "pause",
            Self::AtomicsStore => "store",
            Self::AtomicsWait => "wait",
            Self::AtomicsNotify => "notify",
            Self::AtomicsXor => "xor",
            Self::DatePrototypeSetMilliseconds => "setMilliseconds",
            Self::DatePrototypeSetUtcMilliseconds => "setUTCMilliseconds",
            Self::DatePrototypeSetSeconds => "setSeconds",
            Self::DatePrototypeSetUtcSeconds => "setUTCSeconds",
            Self::DatePrototypeSetMinutes => "setMinutes",
            Self::DatePrototypeSetUtcMinutes => "setUTCMinutes",
            Self::DatePrototypeSetHours => "setHours",
            Self::DatePrototypeSetUtcHours => "setUTCHours",
            Self::DatePrototypeSetDate => "setDate",
            Self::DatePrototypeSetUtcDate => "setUTCDate",
            Self::DatePrototypeSetMonth => "setMonth",
            Self::DatePrototypeSetUtcMonth => "setUTCMonth",
            Self::DatePrototypeSetFullYear => "setFullYear",
            Self::DatePrototypeSetUtcFullYear => "setUTCFullYear",
            Self::DatePrototypeSetYear => "setYear",
            Self::MapPrototypeGetOrInsert | Self::WeakMapPrototypeGetOrInsert => "getOrInsert",
            Self::MapPrototypeGetOrInsertComputed | Self::WeakMapPrototypeGetOrInsertComputed => {
                "getOrInsertComputed"
            }
            Self::WeakSetConstructor => "WeakSet",
            Self::MapPrototypeDelete
            | Self::SetPrototypeDelete
            | Self::WeakMapPrototypeDelete
            | Self::WeakSetPrototypeDelete => "delete",
            Self::MapPrototypeClear | Self::SetPrototypeClear => "clear",
            Self::MapPrototypeSize | Self::SetPrototypeSize => "get size",
            Self::SetConstructor => "Set",
            Self::SetPrototypeAdd | Self::WeakSetPrototypeAdd | Self::AtomicsAdd => "add",
            Self::SetPrototypeUnion => "union",
            Self::SetPrototypeIntersection => "intersection",
            Self::SetPrototypeDifference => "difference",
            Self::SetPrototypeSymmetricDifference => "symmetricDifference",
            Self::SetPrototypeIsSubsetOf => "isSubsetOf",
            Self::SetPrototypeIsSupersetOf => "isSupersetOf",
            Self::SetPrototypeIsDisjointFrom => "isDisjointFrom",
            Self::SpeciesGetter => "get [Symbol.species]",
            Self::RegExpPrototypeFlags => "get flags",
            Self::RegExpPrototypeSource => "get source",
            Self::RegExpPrototypeHasIndices => "get hasIndices",
            Self::RegExpPrototypeGlobal => "get global",
            Self::RegExpPrototypeIgnoreCase => "get ignoreCase",
            Self::RegExpPrototypeMultiline => "get multiline",
            Self::RegExpPrototypeDotAll => "get dotAll",
            Self::RegExpPrototypeUnicode => "get unicode",
            Self::RegExpPrototypeUnicodeSets => "get unicodeSets",
            Self::RegExpPrototypeSticky => "get sticky",
            Self::SymbolConstructor => "Symbol",
            Self::StringFromCharCode => "fromCharCode",
            Self::StringFromCodePoint => "fromCodePoint",
            Self::StringRaw => "raw",
            Self::StringPrototypeIsWellFormed => "isWellFormed",
            Self::StringPrototypeToWellFormed => "toWellFormed",
            Self::StringPrototypeSubstr => "substr",
            Self::StringPrototypeLocaleCompare => "localeCompare",

            Self::RegExpConstructor => "RegExp",
            Self::RegExpPrototypeExec => "exec",
            Self::JsonParse | Self::DateParse => "parse",
            Self::JsonStringify => "stringify",
            Self::PromiseConstructor => "Promise",
            Self::PromiseResolve => "resolve",
            Self::PromiseReject => "reject",
            Self::PromisePrototypeThen => "then",
            Self::PromisePrototypeCatch => "catch",

            Self::Print => "print",
            Self::PromiseAll => "all",
            Self::PromiseRace => "race",
            Self::PromiseAllSettled => "allSettled",
            Self::PromiseWithResolvers => "withResolvers",
            Self::RegExpPrototypeMatch => "[Symbol.match]",
            Self::RegExpPrototypeSearch => "[Symbol.search]",
            Self::RegExpPrototypeSplit => "[Symbol.split]",
            Self::RegExpPrototypeTest => "test",
            Self::ArrayConstructor => "Array",
            Self::ObjectConstructor => "Object",
            Self::FunctionConstructor => "Function",
            Self::FunctionPrototypeCall => "call",
            Self::FunctionPrototypeApply | Self::ReflectApply => "apply",
            Self::ReflectConstruct => "construct",
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
            Self::ObjectGetPrototypeOf | Self::ReflectGetPrototypeOf => "getPrototypeOf",
            Self::ObjectSetPrototypeOf | Self::ReflectSetPrototypeOf => "setPrototypeOf",
            Self::ObjectKeys
            | Self::ArrayPrototypeKeys
            | Self::MapPrototypeKeys
            | Self::TypedArrayPrototypeKeys => "keys",
            Self::ObjectIs => "is",
            Self::ObjectHasOwn => "hasOwn",
            Self::ArrayPrototypeShift => "shift",
            Self::ArrayPrototypeUnshift => "unshift",
            Self::ArrayPrototypeSplice => "splice",
            Self::ArrayOf => "of",
            Self::ArrayFrom => "from",
            Self::Eval => "eval",
            Self::ArrayPrototypeFlat => "flat",
            Self::ArrayPrototypeSort | Self::TypedArrayPrototypeSort => "sort",
            Self::ArrayPrototypeToSorted | Self::TypedArrayPrototypeToSorted => "toSorted",
            Self::ArrayPrototypeToSpliced => "toSpliced",
            Self::ArrayPrototypeFill | Self::TypedArrayPrototypeFill => "fill",
            Self::ArrayPrototypeCopyWithin | Self::TypedArrayPrototypeCopyWithin => "copyWithin",
            Self::ArrayPrototypeWith | Self::TypedArrayPrototypeWith => "with",
            Self::ArrayPrototypeToReversed | Self::TypedArrayPrototypeToReversed => "toReversed",
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
            Self::ArrayPrototypeForEach
            | Self::MapPrototypeForEach
            | Self::SetPrototypeForEach
            | Self::TypedArrayPrototypeForEach
            | Self::IteratorPrototypeForEach => "forEach",
            Self::ArrayPrototypeMap => "map",
            Self::ArrayPrototypeFilter => "filter",
            Self::ArrayPrototypeEvery
            | Self::TypedArrayPrototypeEvery
            | Self::IteratorPrototypeEvery => "every",
            Self::ArrayPrototypeSome
            | Self::TypedArrayPrototypeSome
            | Self::IteratorPrototypeSome => "some",
            Self::ArrayPrototypeFind
            | Self::TypedArrayPrototypeFind
            | Self::IteratorPrototypeFind => "find",
            Self::ArrayPrototypeFindLast | Self::TypedArrayPrototypeFindLast => "findLast",
            Self::ArrayPrototypeFindLastIndex | Self::TypedArrayPrototypeFindLastIndex => {
                "findLastIndex"
            }
            Self::ArrayPrototypeFindIndex | Self::TypedArrayPrototypeFindIndex => "findIndex",
            Self::ArrayPrototypeReduce
            | Self::TypedArrayPrototypeReduce
            | Self::IteratorPrototypeReduce => "reduce",
            Self::ArrayPrototypeReduceRight | Self::TypedArrayPrototypeReduceRight => "reduceRight",
            Self::IteratorPrototypeIterator => "[Symbol.iterator]",
            Self::AsyncIteratorPrototypeAsyncIterator => "[Symbol.asyncIterator]",
            Self::FunctionPrototypeHasInstance => "[Symbol.hasInstance]",
            Self::SymbolPrototypeToPrimitive => "[Symbol.toPrimitive]",
            Self::ErrorIsError => "isError",
            Self::ObjectPreventExtensions | Self::ReflectPreventExtensions => "preventExtensions",
            Self::ObjectIsExtensible | Self::ReflectIsExtensible => "isExtensible",
            Self::ObjectSeal => "seal",
            Self::ObjectIsSealed => "isSealed",
            Self::ObjectFreeze => "freeze",
            Self::ObjectIsFrozen => "isFrozen",
            Self::ObjectEntries
            | Self::ArrayPrototypeEntries
            | Self::MapPrototypeEntries
            | Self::SetPrototypeEntries
            | Self::TypedArrayPrototypeEntries => "entries",
            Self::NumberConstructor => "Number",
            Self::NumberIsFinite | Self::IsFinite => "isFinite",
            Self::NumberIsInteger => "isInteger",
            Self::NumberIsNaN | Self::IsNaN => "isNaN",
            Self::NumberIsSafeInteger => "isSafeInteger",
            Self::BooleanConstructor => "Boolean",
            Self::ReflectDeleteProperty => "deleteProperty",
            Self::ReflectGet | Self::MapPrototypeGet | Self::WeakMapPrototypeGet => "get",
            Self::ReflectHas
            | Self::MapPrototypeHas
            | Self::SetPrototypeHas
            | Self::WeakMapPrototypeHas
            | Self::WeakSetPrototypeHas => "has",
            Self::ReflectOwnKeys => "ownKeys",
            Self::ObjectDefineProperty | Self::ReflectDefineProperty => "defineProperty",
            Self::ObjectGetOwnPropertyDescriptor | Self::ReflectGetOwnPropertyDescriptor => {
                "getOwnPropertyDescriptor"
            }
            Self::ObjectGetOwnPropertyNames => "getOwnPropertyNames",
            Self::ArrayIsArray => "isArray",
            Self::StringPrototypeCharAt => "charAt",
            Self::StringPrototypeCharCodeAt => "charCodeAt",
            Self::StringPrototypeIndexOf
            | Self::ArrayPrototypeIndexOf
            | Self::TypedArrayPrototypeIndexOf => "indexOf",
            Self::StringPrototypeAt | Self::ArrayPrototypeAt | Self::TypedArrayPrototypeAt => "at",
            Self::StringPrototypeConcat | Self::ArrayPrototypeConcat => "concat",
            Self::StringPrototypeEndsWith => "endsWith",
            Self::StringPrototypeIncludes
            | Self::ArrayPrototypeIncludes
            | Self::TypedArrayPrototypeIncludes => "includes",
            Self::StringPrototypeLastIndexOf
            | Self::ArrayPrototypeLastIndexOf
            | Self::TypedArrayPrototypeLastIndexOf => "lastIndexOf",
            Self::StringPrototypeRepeat => "repeat",
            Self::StringPrototypeStartsWith => "startsWith",
            Self::StringPrototypeSubstring => "substring",
            Self::StringPrototypeCodePointAt => "codePointAt",
            Self::StringPrototypePadEnd => "padEnd",
            Self::StringPrototypePadStart => "padStart",
            Self::StringPrototypeTrim => "trim",
            Self::StringPrototypeTrimEnd => "trimEnd",
            Self::StringPrototypeTrimStart => "trimStart",
            Self::StringPrototypeSplit => "split",
            Self::StringPrototypeReplace => "replace",
            Self::RegExpPrototypeReplace => "[Symbol.replace]",
            Self::StringPrototypeMatch => "match",
            Self::StringPrototypeSearch => "search",
            Self::ParseInt => "parseInt",
            Self::ParseFloat => "parseFloat",
            Self::SymbolFor => "for",
            Self::SymbolKeyFor => "keyFor",
            Self::ArrayPrototypeValues
            | Self::ObjectValues
            | Self::MapPrototypeValues
            | Self::SetPrototypeValues
            | Self::TypedArrayPrototypeValues => "values",
            Self::ArrayIteratorPrototypeNext
            | Self::MapIteratorPrototypeNext
            | Self::SetIteratorPrototypeNext
            | Self::GeneratorPrototypeNext
            | Self::AsyncGeneratorPrototypeNext
            | Self::AsyncFromSyncIteratorPrototypeNext => "next",
            Self::GeneratorPrototypeReturn
            | Self::AsyncGeneratorPrototypeReturn
            | Self::AsyncFromSyncIteratorPrototypeReturn => "return",
            Self::GeneratorPrototypeThrow
            | Self::AsyncGeneratorPrototypeThrow
            | Self::AsyncFromSyncIteratorPrototypeThrow => "throw",
            Self::ArrayPrototypeJoin | Self::TypedArrayPrototypeJoin => "join",
            Self::ArrayPrototypePop => "pop",
            Self::ArrayPrototypePush => "push",
            Self::ArrayPrototypeReverse | Self::TypedArrayPrototypeReverse => "reverse",
            Self::StringPrototypeSlice
            | Self::ArrayPrototypeSlice
            | Self::ArrayBufferPrototypeSlice
            | Self::SharedArrayBufferPrototypeSlice
            | Self::TypedArrayPrototypeSlice => "slice",
        }
    }

    /// The arguments this intrinsic sends through 7.1.1 before it does anything
    /// else, in the order its clause converts them, each with the hint that
    /// clause gives.
    ///
    /// Only an operation whose conversions come before every effect it has
    /// belongs here: the native runs from the beginning once one more argument
    /// is a primitive, so a conversion must not be able to observe a step the
    /// native already took.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "one table names every intrinsic beside the arguments it converts"
    )]
    pub const fn coerced_arguments(self) -> &'static [(u16, PrimitiveHint)] {
        /// One position, converted by `ToIntegerOrInfinity` or `ToNumber`.
        const NUMBER: &[(u16, PrimitiveHint)] = &[(0, PrimitiveHint::Number)];
        /// Two of them.
        const NUMBERS: &[(u16, PrimitiveHint)] =
            &[(0, PrimitiveHint::Number), (1, PrimitiveHint::Number)];
        /// One text, converted by `ToString`.
        const TEXT: &[(u16, PrimitiveHint)] = &[(0, PrimitiveHint::String)];
        /// A text to search for, then where to start.
        const TEXT_THEN_NUMBER: &[(u16, PrimitiveHint)] =
            &[(0, PrimitiveHint::String), (1, PrimitiveHint::Number)];
        /// Three positions, all of them numbers.
        const THREE: &[(u16, PrimitiveHint)] = &[
            (0, PrimitiveHint::Number),
            (1, PrimitiveHint::Number),
            (2, PrimitiveHint::Number),
        ];
        /// Four of them, which the setters of 21.4.4 take.
        const FOUR: &[(u16, PrimitiveHint)] = &[
            (0, PrimitiveHint::Number),
            (1, PrimitiveHint::Number),
            (2, PrimitiveHint::Number),
            (3, PrimitiveHint::Number),
        ];
        /// Seven of them, which 21.4.3.4 takes.
        const SEVEN: &[(u16, PrimitiveHint)] = &[
            (0, PrimitiveHint::Number),
            (1, PrimitiveHint::Number),
            (2, PrimitiveHint::Number),
            (3, PrimitiveHint::Number),
            (4, PrimitiveHint::Number),
            (5, PrimitiveHint::Number),
            (6, PrimitiveHint::Number),
        ];
        /// The second position alone.
        const SECOND: &[(u16, PrimitiveHint)] = &[(1, PrimitiveHint::Number)];
        /// The second and the third.
        const SECOND_THIRD: &[(u16, PrimitiveHint)] =
            &[(1, PrimitiveHint::Number), (2, PrimitiveHint::Number)];
        match self {
            // 22.1.1.1 step 2, and 20.5.1.1 step 3 with 20.5.6.1.1 beside it.
            Self::StringConstructor
            | Self::ErrorConstructor
            | Self::EvalErrorConstructor
            | Self::RangeErrorConstructor
            | Self::ReferenceErrorConstructor
            | Self::SyntaxErrorConstructor
            | Self::TypeErrorConstructor
            | Self::UriErrorConstructor
            // 23.1.3.18: the separator, which `ToString` converts.
            | Self::ArrayPrototypeJoin
            // 19.2.4 applies `ToString` to its argument.
            | Self::ParseFloat
            // 20.4.2.2 applies `ToString` to its key.
            | Self::SymbolFor
            // 22.2.7.1 and 22.2.6.16 apply `ToString` to the text they search,
            // and so do the Symbol-keyed methods of 22.2.6.
            | Self::RegExpPrototypeExec
            | Self::RegExpPrototypeMatch
            | Self::RegExpPrototypeSearch
            | Self::RegExpPrototypeSplit
            | Self::RegExpPrototypeTest
            // 20.1.3.2 and 20.1.3.4 apply `ToPropertyKey` to the name, which
            // sends an Object through 7.1.1 with the hint `string`.
            | Self::ObjectPrototypeHasOwnProperty
            | Self::ObjectPrototypePropertyIsEnumerable
            // The embedding writes one line of text, so its argument takes
            // the conversion of 7.1.17 like every other text.
            | Self::Print
            // B.2.2.2.1 step 4 applies `ToString` to the attribute value.
            | Self::StringPrototypeAnchor
            | Self::StringPrototypeFontcolor
            | Self::StringPrototypeFontsize
            | Self::StringPrototypeLink => TEXT,
            // 22.1.3: one position, which `ToIntegerOrInfinity` converts.
            Self::StringPrototypeCharAt
            | Self::StringPrototypeCharCodeAt
            | Self::StringPrototypeCodePointAt
            | Self::StringPrototypeAt
            | Self::StringPrototypeRepeat
            | Self::ArrayPrototypeAt
            // 21.3.2: every argument goes through `ToNumber`.
            | Self::MathAbs
            | Self::MathCeil
            | Self::MathFloor
            | Self::MathTrunc
            | Self::MathRound
            | Self::MathSign
            | Self::MathClz32
            | Self::MathFround
            | Self::MathSin
            // 19.2.2 and 19.2.3 apply `ToNumber` to their argument.
            | Self::IsNaN
            | Self::IsFinite
            // 23.1.3.14 converts the depth it was given.
            | Self::ArrayPrototypeFlat
            // 23.1.3.39 converts the index and stores the value as it is;
            // 21.1.1.1 converts the one argument it reads.
            | Self::ArrayPrototypeWith
            | Self::NumberConstructor
            | Self::MathAcos
            | Self::MathAcosh
            | Self::MathAsin
            | Self::MathAsinh
            | Self::MathAtan
            | Self::MathAtanh
            | Self::MathCbrt
            | Self::MathCos
            | Self::MathCosh
            | Self::MathExp
            | Self::MathExpm1
            | Self::MathF16round
            | Self::MathLog
            | Self::MathLog10
            | Self::MathLog1p
            | Self::MathLog2
            | Self::MathSinh
            | Self::MathSqrt
            | Self::MathTan
            | Self::MathTanh
            // 21.4.4 converts every field a setter of a date writes.
            | Self::DatePrototypeSetTime
            | Self::DatePrototypeSetMilliseconds
            | Self::DatePrototypeSetUtcMilliseconds
            | Self::DatePrototypeSetYear
            | Self::BigIntConstructor
            | Self::BigIntPrototypeToString
            // 25.1.4.1, 25.2.3.1 and 25.2.5.2 read a length.
            | Self::ArrayBufferConstructor
            | Self::SharedArrayBufferConstructor
            | Self::SharedArrayBufferPrototypeGrow
            // 23.2.3.1 counts from the end of the array.
            | Self::TypedArrayPrototypeAt
            // 25.4.7 answers for a width.
            | Self::AtomicsIsLockFree
            // 25.3.4 reads the index and, where it writes, the value beside it.
            | Self::DataViewPrototypeGetInt8
            | Self::DataViewPrototypeGetUint8
            | Self::DataViewPrototypeGetInt16
            | Self::DataViewPrototypeGetUint16
            | Self::DataViewPrototypeGetInt32
            | Self::DataViewPrototypeGetUint32
            | Self::DataViewPrototypeGetFloat32
            | Self::DataViewPrototypeGetFloat64
            // 25.3.1.2 step 2 refuses an index that is no index before it
            // reads the value, so the clause converts the value itself.
            | Self::DataViewPrototypeSetInt8
            | Self::DataViewPrototypeSetUint8
            | Self::DataViewPrototypeSetInt16
            | Self::DataViewPrototypeSetUint16
            | Self::DataViewPrototypeSetInt32
            | Self::DataViewPrototypeSetUint32
            | Self::DataViewPrototypeSetFloat32
            | Self::DataViewPrototypeSetFloat64 => NUMBER,
            // 22.1.3.9, 22.1.3.11, 22.1.3.7, 22.1.3.8 and 22.1.3.24: the text
            // to search for, then where to start.
            Self::StringPrototypeIndexOf
            | Self::StringPrototypeLastIndexOf
            | Self::StringPrototypeIncludes
            | Self::StringPrototypeEndsWith
            | Self::StringPrototypeStartsWith => TEXT_THEN_NUMBER,
            // 22.1.3.21, 22.1.3.25, 23.1.3.28 and 23.1.3.29: two positions.
            Self::StringPrototypeSlice
            | Self::StringPrototypeSubstring
            | Self::ArrayPrototypeSlice
            | Self::ArrayPrototypeToSpliced
            | Self::ArrayPrototypeSplice
            | Self::MathPow
            | Self::MathImul
            | Self::MathAtan2
            | Self::DatePrototypeSetSeconds
            | Self::DatePrototypeSetUtcSeconds
            | Self::DatePrototypeSetMonth
            | Self::DatePrototypeSetUtcMonth
            // 21.2.2 takes the width and the value.
            | Self::BigIntAsIntN
            | Self::BigIntAsUintN
            // 25.1.6.7, 25.2.5.4, 23.2.3.24 and 23.2.3.27 take a range.
            | Self::ArrayBufferPrototypeSlice
            | Self::SharedArrayBufferPrototypeSlice
            | Self::TypedArrayPrototypeSlice
            | Self::TypedArrayPrototypeSubarray
            // 23.2.3.36 takes the index and the value.
            | Self::TypedArrayPrototypeWith => NUMBERS,
            // 23.1.3.4: the target, then the two positions it copies from.
            Self::ArrayPrototypeCopyWithin => &[
                (0, PrimitiveHint::Number),
                (1, PrimitiveHint::Number),
                (2, PrimitiveHint::Number),
            ],
            // 23.1.3.6: the value is stored as it is, and the two positions
            // that follow it are converted.
            Self::ArrayPrototypeFill => &[(1, PrimitiveHint::Number), (2, PrimitiveHint::Number)],
            // 20.1.2.4, 20.1.2.8, 20.1.2.13 and 28.1 apply `ToPropertyKey` to
            // the name, which sends an Object through 7.1.1 with the hint
            // `string`.
            Self::ObjectDefineProperty
            | Self::ObjectGetOwnPropertyDescriptor
            | Self::ObjectHasOwn
            | Self::ReflectDefineProperty
            | Self::ReflectDeleteProperty
            | Self::ReflectGet
            | Self::ReflectGetOwnPropertyDescriptor
            | Self::ReflectHas => &[(1, PrimitiveHint::String)],
            // 22.1.3.16 and 22.1.3.17: the length, then the text to pad with.
            Self::StringPrototypePadStart | Self::StringPrototypePadEnd => {
                &[(0, PrimitiveHint::Number), (1, PrimitiveHint::String)]
            }
            // 19.2.5 applies `ToString` to the text and `ToInt32` to the radix.
            Self::ParseInt => &[(0, PrimitiveHint::String), (1, PrimitiveHint::Number)],
            // 19.2.6 sends the argument through `ToString` before anything.
            Self::EncodeUri
            | Self::EncodeUriComponent
            | Self::DecodeUri
            | Self::DecodeUriComponent
            // B.2.1 does the same.
            | Self::Escape
            | Self::Unescape => &[(0, PrimitiveHint::String)],
            // 23.1.3.17, 23.1.3.20 and 23.1.3.14: the element is compared as
            // it is, and only the index is converted.
            Self::ArrayPrototypeIndexOf
            | Self::ArrayPrototypeLastIndexOf
            | Self::ArrayPrototypeIncludes
            // 22.1.3.23 step 6 applies `ToUint32` to the limit; the separator
            // reaches `@@split`, which is a call of its own.
            | Self::StringPrototypeSplit => &[(1, PrimitiveHint::Number)],
            Self::DatePrototypeSetMinutes
            | Self::DatePrototypeSetUtcMinutes
            | Self::DatePrototypeSetDate
            | Self::DatePrototypeSetUtcDate
            | Self::DatePrototypeSetFullYear
            | Self::DatePrototypeSetUtcFullYear
            // 23.2.3.9 takes the value and the range it writes it over, and
            // 23.2.3.6 the three positions it moves between.
            | Self::TypedArrayPrototypeFill
            | Self::TypedArrayPrototypeCopyWithin => THREE,
            Self::DatePrototypeSetHours | Self::DatePrototypeSetUtcHours => FOUR,
            Self::DateUtc => SEVEN,
            // 23.2.3.14, 23.2.3.15 and 23.2.3.18 compare the element as it is
            // and convert the position after it, and 23.2.3.23 takes the
            // source as it is and converts the offset.
            Self::TypedArrayPrototypeIncludes
            | Self::TypedArrayPrototypeIndexOf
            | Self::TypedArrayPrototypeLastIndexOf
            | Self::TypedArrayPrototypeSet
            // 25.4 reads the array as it is, converts the index and checks
            // its range before it reads anything after it.
            | Self::AtomicsLoad
            | Self::AtomicsAdd
            | Self::AtomicsAnd
            | Self::AtomicsExchange
            | Self::AtomicsOr
            | Self::AtomicsStore
            | Self::AtomicsSub
            | Self::AtomicsXor
            | Self::AtomicsNotify
            | Self::AtomicsCompareExchange
            | Self::AtomicsWait => SECOND,
            // 25.3.3.1 takes the block as it is and converts the offset and
            // the length beside it. 23.2.5.1 reads neither where its first
            // argument is no block, so it converts them itself.
            Self::DataViewConstructor => SECOND_THIRD,
            _ => &[],
        }
    }

    /// Whether the engine converts the argument at `index` itself, which is
    /// what lets a lowering pass an Object there.
    #[must_use]
    pub const fn converts_argument(self, index: u16) -> bool {
        let mut entries = self.coerced_arguments();
        while let [(at, _), rest @ ..] = entries {
            if *at == index {
                return true;
            }
            entries = rest;
        }
        false
    }

    /// Whether this method coerces the argument at `index` to a primitive.
    ///
    /// An intrinsic runs without a call frame, so it cannot run a user
    /// `valueOf` or `toString`. An Object in such a position is therefore not
    /// lowered; a position that only compares or stores its argument takes any
    /// value.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "one table names every intrinsic beside the key it coerces"
    )]
    pub const fn coerces_argument(self, index: u16) -> bool {
        match self {
            // 20.1.3.3 compares, 20.1.3.6 and 23.1.3.38 read no argument, and
            // 23.1.5.2.1 takes none.
            Self::ObjectPrototypeIsPrototypeOf
            | Self::ObjectPrototypeToString
            | Self::ObjectPrototypeValueOf
            | Self::FunctionPrototype
            | Self::ErrorPrototypeToString
            | Self::SpeciesGetter
            | Self::RegExpPrototypeFlags
            | Self::RegExpPrototypeSource
            | Self::RegExpPrototypeHasIndices
            | Self::RegExpPrototypeGlobal
            | Self::RegExpPrototypeIgnoreCase
            | Self::RegExpPrototypeMultiline
            | Self::RegExpPrototypeDotAll
            | Self::RegExpPrototypeUnicode
            | Self::RegExpPrototypeUnicodeSets
            | Self::RegExpPrototypeSticky
            | Self::ArrayOf
            | Self::ArrayFrom
            | Self::Eval
            | Self::StringPrototypeReplace
            | Self::RegExpPrototypeReplace
            | Self::ArrayPrototypeSort
            | Self::ArrayPrototypeToSorted
            | Self::ArrayPrototypeValues
            | Self::ArrayPrototypeKeys
            | Self::ArrayPrototypeEntries
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
            | Self::FunctionPrototypeApply
            | Self::ReflectApply
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
            | Self::ObjectSetPrototypeOf
            | Self::ReflectSetPrototypeOf
            | Self::ObjectKeys
            | Self::ObjectIs
            | Self::ArrayPrototypeShift
            | Self::ArrayPrototypeUnshift
            | Self::ArrayPrototypeConcat
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
            | Self::ArrayPrototypeForEach
            | Self::MapPrototypeForEach
            | Self::SetPrototypeForEach
            | Self::ArrayPrototypeMap
            | Self::ArrayPrototypeFilter
            | Self::ArrayPrototypeEvery
            | Self::ArrayPrototypeSome
            | Self::ArrayPrototypeFind
            | Self::ArrayPrototypeFindIndex
            | Self::ArrayPrototypeFindLast
            | Self::ArrayPrototypeFindLastIndex
            | Self::ArrayPrototypeReduce
            | Self::ArrayPrototypeReduceRight
            | Self::IteratorPrototypeIterator
            | Self::ObjectPreventExtensions
            | Self::ObjectIsExtensible
            | Self::ObjectSeal
            | Self::ObjectIsSealed
            | Self::ObjectFreeze
            | Self::ObjectIsFrozen
            | Self::ObjectValues
            | Self::ObjectEntries
            | Self::NumberIsFinite
            | Self::NumberIsInteger
            | Self::NumberIsNaN
            | Self::NumberIsSafeInteger
            | Self::BooleanConstructor
            | Self::ReflectGetPrototypeOf
            | Self::ReflectIsExtensible
            | Self::ReflectOwnKeys
            | Self::ReflectPreventExtensions
            | Self::NumberPrototypeValueOf
            | Self::BooleanPrototypeValueOf
            | Self::BooleanPrototypeToString
            | Self::StringPrototypeValueOf
            | Self::StringPrototypeToString
            | Self::ThrowTypeError
            | Self::SymbolConstructor
            | Self::RegExpConstructor
            | Self::RegExpPrototypeToString
            | Self::JsonParse
            | Self::JsonStringify
            | Self::ObjectGetOwnPropertyNames
            | Self::ObjectGetOwnPropertySymbols
            | Self::ObjectGetOwnPropertyDescriptors
            | Self::IsNaN
            | Self::IsFinite
            | Self::ParseInt
            | Self::ParseFloat
            | Self::SymbolPrototypeToString
            | Self::SymbolPrototypeValueOf
            | Self::FunctionPrototypeToString
            // 23.1.3.13 takes a callback and 20.1.2.7 an iterable, and both
            // are stored or called rather than converted.
            | Self::ArrayPrototypeFlatMap
            | Self::ObjectFromEntries
            | Self::ObjectAssign
            | Self::ReflectSet
            | Self::SymbolKeyFor
            // 23.2.3.26 and 23.2.3.32 take a comparator, and the methods of
            // 23.2.3 that walk take a callback and a receiver beside it.
            | Self::TypedArrayPrototypeSort
            | Self::TypedArrayPrototypeToSorted
            | Self::TypedArrayPrototypeEvery
            | Self::TypedArrayPrototypeSome
            | Self::TypedArrayPrototypeForEach
            | Self::TypedArrayPrototypeFind
            | Self::TypedArrayPrototypeFindIndex
            | Self::TypedArrayPrototypeFindLast
            | Self::TypedArrayPrototypeFindLastIndex
            | Self::TypedArrayPrototypeReduce
            | Self::TypedArrayPrototypeReduceRight
            | Self::TypedArrayPrototypeKeys
            | Self::TypedArrayPrototypeValues
            | Self::TypedArrayPrototypeEntries
            | Self::TypedArrayPrototypeReverse
            | Self::TypedArrayPrototypeToReversed
            | Self::TypedArrayPrototypeBuffer
            | Self::TypedArrayPrototypeByteLength
            | Self::TypedArrayPrototypeByteOffset
            | Self::TypedArrayPrototypeLength
            | Self::TypedArrayPrototypeToStringTag
            | Self::TypedArrayBase
            | Self::SharedArrayBufferPrototypeByteLength
            | Self::SharedArrayBufferPrototypeGrowable
            | Self::SharedArrayBufferPrototypeMaxByteLength
            | Self::BigIntPrototypeValueOf
            | Self::BigIntPrototypeToLocaleString
            | Self::AtomicsPause => false,
            // 20.1.2.4, 20.1.2.8 and 20.1.2.13 apply ToPropertyKey to the
            // second argument.
            Self::ObjectDefineProperty
            | Self::ObjectGetOwnPropertyDescriptor
            | Self::ObjectHasOwn
            | Self::ReflectDefineProperty
            | Self::ReflectDeleteProperty
            | Self::ReflectGet
            | Self::ReflectGetOwnPropertyDescriptor
            | Self::ReflectConstruct
            | Self::ReflectHas => index == 1,
            // 20.1.3.2 and 20.1.3.4 apply ToPropertyKey to the first argument.
            Self::ObjectPrototypeHasOwnProperty | Self::ObjectPrototypePropertyIsEnumerable => {
                index == 0
            }
            // 23.1.3.16, 23.1.3.17, 23.1.3.20 and 23.1.3.6 take the element as
            // it is and coerce only the positions that follow it.
            Self::ArrayPrototypeIncludes
            | Self::ArrayPrototypeIndexOf
            | Self::ArrayPrototypeLastIndexOf
            | Self::ArrayPrototypeFill
            // 23.2.3.14, 23.2.3.15 and 23.2.3.18 compare the element as it is.
            | Self::TypedArrayPrototypeIncludes
            | Self::TypedArrayPrototypeIndexOf
            | Self::TypedArrayPrototypeLastIndexOf
            // 23.2.5.1 and 25.3.3.1 take the block as it is, 25.4 the array,
            // and 23.2.3.23 the source it copies from.
            | Self::TypedArrayInt8Constructor
            | Self::TypedArrayUint8Constructor
            | Self::TypedArrayUint8ClampedConstructor
            | Self::TypedArrayInt16Constructor
            | Self::TypedArrayUint16Constructor
            | Self::TypedArrayInt32Constructor
            | Self::TypedArrayUint32Constructor
            | Self::TypedArrayFloat32Constructor
            | Self::TypedArrayFloat64Constructor
            | Self::TypedArrayFloat16Constructor
            | Self::TypedArrayBigInt64Constructor
            | Self::TypedArrayBigUint64Constructor
            | Self::DataViewConstructor
            | Self::TypedArrayPrototypeSet
            | Self::AtomicsAdd
            | Self::AtomicsAnd
            | Self::AtomicsCompareExchange
            | Self::AtomicsExchange
            | Self::AtomicsLoad
            | Self::AtomicsOr
            | Self::AtomicsStore
            | Self::AtomicsSub
            | Self::AtomicsWait
            | Self::AtomicsNotify
            | Self::AtomicsXor => index > 0,
            // 23.1.3.29 coerces the start and the count, and takes every item
            // after them as it is.
            Self::ArrayPrototypeSplice
            // 25.3.4 reads the index and the value and takes the byte order as
            // the value it is.
            | Self::DataViewPrototypeSetInt8
            | Self::DataViewPrototypeSetUint8
            | Self::DataViewPrototypeSetInt16
            | Self::DataViewPrototypeSetUint16
            | Self::DataViewPrototypeSetInt32
            | Self::DataViewPrototypeSetUint32
            | Self::DataViewPrototypeSetFloat32
            | Self::DataViewPrototypeSetFloat64 => index < 2,
            // 23.1.3.39 coerces the index and stores the value as it is;
            // 21.1.1.1 reads one argument.
            Self::ArrayPrototypeWith
            | Self::NumberConstructor
            // 25.1.4.1 and 25.2.3.1 read the length and take the options as
            // the object they are.
            | Self::ArrayBufferConstructor
            | Self::SharedArrayBufferConstructor
            // 25.3.4 reads the index and takes the byte order as it is.
            | Self::DataViewPrototypeGetInt8
            | Self::DataViewPrototypeGetUint8
            | Self::DataViewPrototypeGetInt16
            | Self::DataViewPrototypeGetUint16
            | Self::DataViewPrototypeGetInt32
            | Self::DataViewPrototypeGetUint32
            | Self::DataViewPrototypeGetFloat32
            | Self::DataViewPrototypeGetFloat64 => index == 0,
            // 22.1.3 and 23.1.3.1 coerce every argument they read.
            _ => true,
        }
    }

    /// Whether the clause sends its `this` value through `ToString` before it
    /// does anything else (22.1.3).
    ///
    /// 22.1.3.32 and 22.1.3.35 take a String or its wrapper and answer the
    /// `[[StringData]]` of one, so neither converts.
    #[must_use]
    pub const fn coerces_its_receiver(self) -> bool {
        matches!(self.holder(), IntrinsicHolder::StringPrototype)
            && !matches!(
                self,
                Self::StringPrototypeToString | Self::StringPrototypeValueOf
            )
    }

    /// Whether this intrinsic is a constructor of this Realm that makes an
    /// object of its own with 10.1.13.
    #[must_use]
    pub const fn constructs_an_object(self) -> bool {
        matches!(
            self,
            Self::ArrayConstructor
                | Self::ObjectConstructor
                | Self::RegExpConstructor
                | Self::ErrorConstructor
                | Self::EvalErrorConstructor
                | Self::RangeErrorConstructor
                | Self::ReferenceErrorConstructor
                | Self::SyntaxErrorConstructor
                | Self::TypeErrorConstructor
                | Self::UriErrorConstructor
                | Self::StringConstructor
                | Self::NumberConstructor
                | Self::BooleanConstructor
                | Self::PromiseConstructor
        )
    }

    /// The `length` property of the function object (17).
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "one table names every intrinsic beside the length 17 gives it"
    )]
    pub const fn length(self) -> u32 {
        match self {
            Self::ThrowTypeError
            | Self::FunctionPrototype
            | Self::SpeciesGetter
            | Self::RegExpPrototypeFlags
            | Self::RegExpPrototypeSource
            | Self::RegExpPrototypeHasIndices
            | Self::RegExpPrototypeGlobal
            | Self::RegExpPrototypeIgnoreCase
            | Self::RegExpPrototypeMultiline
            | Self::RegExpPrototypeDotAll
            | Self::RegExpPrototypeUnicode
            | Self::RegExpPrototypeUnicodeSets
            | Self::RegExpPrototypeSticky
            | Self::SymbolConstructor
            | Self::RegExpPrototypeToString
            | Self::ObjectPrototypeToString
            | Self::ObjectPrototypeValueOf
            | Self::ErrorPrototypeToString
            | Self::NumberPrototypeValueOf
            | Self::BooleanPrototypeValueOf
            | Self::BooleanPrototypeToString
            | Self::StringPrototypeValueOf
            | Self::StringPrototypeToString
            | Self::SymbolPrototypeToString
            | Self::SymbolPrototypeValueOf
            | Self::FunctionPrototypeToString
            | Self::StringPrototypeTrim
            | Self::StringPrototypeTrimEnd
            | Self::StringPrototypeTrimStart
            | Self::ArrayPrototypeValues
            | Self::ArrayPrototypeKeys
            | Self::ArrayPrototypeEntries
            | Self::ArrayIteratorPrototypeNext
            | Self::ArrayPrototypePop
            | Self::ArrayPrototypeReverse
            | Self::ArrayPrototypeToString
            | Self::ArrayPrototypeShift
            | Self::IteratorPrototypeIterator
            | Self::AsyncIteratorPrototypeAsyncIterator
            | Self::ArrayPrototypeFlat
            | Self::ArrayOf
            | Self::StringPrototypeIsWellFormed
            | Self::StringPrototypeToWellFormed
            | Self::ArrayPrototypeToReversed
            | Self::PromiseWithResolvers
            | Self::StringPrototypeBig
            | Self::StringPrototypeBlink
            | Self::StringPrototypeBold
            | Self::StringPrototypeFixed
            | Self::StringPrototypeItalics
            | Self::StringPrototypeSmall
            | Self::StringPrototypeStrike
            | Self::StringPrototypeSub
            | Self::StringPrototypeSup
            | Self::MathRandom
            | Self::MapConstructor
            | Self::WeakMapConstructor
            | Self::WeakSetConstructor
            | Self::MapPrototypeClear
            | Self::MapPrototypeSize
            | Self::SetConstructor
            | Self::SetPrototypeClear
            | Self::SetPrototypeSize
            | Self::MapPrototypeEntries
            | Self::MapPrototypeKeys
            | Self::MapPrototypeValues
            | Self::SetPrototypeValues
            | Self::SetPrototypeEntries
            | Self::DateNow
            | Self::DatePrototypeValueOf
            | Self::DatePrototypeGetTime
            | Self::DatePrototypeGetTimezoneOffset
            | Self::DatePrototypeGetFullYear
            | Self::DatePrototypeGetUtcFullYear
            | Self::DatePrototypeGetMonth
            | Self::DatePrototypeGetUtcMonth
            | Self::DatePrototypeGetDate
            | Self::DatePrototypeGetUtcDate
            | Self::DatePrototypeGetDay
            | Self::DatePrototypeGetUtcDay
            | Self::DatePrototypeGetHours
            | Self::DatePrototypeGetUtcHours
            | Self::DatePrototypeGetMinutes
            | Self::DatePrototypeGetUtcMinutes
            | Self::DatePrototypeGetSeconds
            | Self::DatePrototypeGetUtcSeconds
            | Self::DatePrototypeGetMilliseconds
            | Self::DatePrototypeGetUtcMilliseconds
            | Self::DatePrototypeToIsoString
            | Self::DatePrototypeToString
            | Self::DatePrototypeToDateString
            | Self::DatePrototypeToTimeString
            | Self::DatePrototypeToUtcString
            | Self::DatePrototypeToLocaleString
            | Self::DatePrototypeToLocaleDateString
            | Self::DatePrototypeToLocaleTimeString
            | Self::ArrayBufferPrototypeByteLength
            | Self::ArrayBufferPrototypeDetached
            | Self::ArrayBufferPrototypeResizable
            | Self::ArrayBufferPrototypeMaxByteLength
            | Self::DataViewPrototypeBuffer
            | Self::DataViewPrototypeByteLength
            | Self::DataViewPrototypeByteOffset
            | Self::TypedArrayPrototypeBuffer
            | Self::TypedArrayPrototypeByteLength
            | Self::TypedArrayPrototypeByteOffset
            | Self::TypedArrayPrototypeLength
            | Self::TypedArrayPrototypeToStringTag
            | Self::SharedArrayBufferPrototypeByteLength
            | Self::SharedArrayBufferPrototypeGrowable
            | Self::SharedArrayBufferPrototypeMaxByteLength
            | Self::AtomicsPause
            | Self::IteratorConstructor
            | Self::IteratorPrototypeToArray
            | Self::IteratorPrototypeConstructorGet
            | Self::IteratorPrototypeToStringTagGet
            | Self::HostGc
            | Self::BigIntPrototypeToLocaleString
            | Self::BigIntPrototypeValueOf
            | Self::TypedArrayPrototypeEntries
            | Self::TypedArrayPrototypeKeys
            | Self::TypedArrayPrototypeReverse
            | Self::TypedArrayPrototypeToReversed
            | Self::TypedArrayPrototypeValues
            | Self::TypedArrayBase
            | Self::MapIteratorPrototypeNext
            | Self::SetIteratorPrototypeNext => 0,
            Self::StringFromCharCode
            | Self::StringFromCodePoint
            | Self::StringRaw
            | Self::ObjectPrototypeHasOwnProperty
            | Self::ObjectPrototypeIsPrototypeOf
            | Self::ObjectPrototypePropertyIsEnumerable
            | Self::StringPrototypeMatch
            | Self::StringPrototypeSearch
            | Self::IsNaN
            | Self::IsFinite
            | Self::ParseFloat
            | Self::SymbolFor
            | Self::SymbolKeyFor
            | Self::StringPrototypeLocaleCompare
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
            | Self::ArrayPrototypeForEach
            | Self::MapPrototypeForEach
            | Self::SetPrototypeForEach
            | Self::ArrayPrototypeMap
            | Self::ArrayPrototypeFilter
            | Self::ArrayPrototypeEvery
            | Self::ArrayPrototypeSome
            | Self::ArrayPrototypeFind
            | Self::ArrayPrototypeFindIndex
            | Self::ArrayPrototypeFindLast
            | Self::ArrayPrototypeFindLastIndex
            | Self::ArrayPrototypeReduce
            | Self::ArrayPrototypeReduceRight
            | Self::ObjectPreventExtensions
            | Self::ObjectIsExtensible
            | Self::ObjectSeal
            | Self::ObjectIsSealed
            | Self::ObjectFreeze
            | Self::ObjectIsFrozen
            | Self::ObjectValues
            | Self::ObjectEntries
            | Self::NumberConstructor
            | Self::NumberIsFinite
            | Self::NumberIsInteger
            | Self::NumberIsNaN
            | Self::NumberIsSafeInteger
            | Self::BooleanConstructor
            | Self::RegExpConstructor
            | Self::RegExpPrototypeExec
            | Self::RegExpPrototypeTest
            | Self::JsonParse
            | Self::NumberPrototypeToString
            | Self::ReflectGetPrototypeOf
            | Self::ReflectIsExtensible
            | Self::ReflectOwnKeys
            | Self::ReflectPreventExtensions
            | Self::ArrayPrototypeSort
            | Self::ArrayPrototypeToSorted
            | Self::ArrayFrom
            | Self::Eval
            | Self::ObjectGetOwnPropertyNames
            | Self::ObjectGetOwnPropertySymbols
            | Self::ObjectGetOwnPropertyDescriptors
            | Self::ArrayPrototypeFlatMap
            | Self::ObjectFromEntries
            | Self::MathAcos
            | Self::MathAcosh
            | Self::MathAsin
            | Self::MathAsinh
            | Self::MathAtan
            | Self::MathAtanh
            | Self::MathCbrt
            | Self::MathCos
            | Self::MathCosh
            | Self::MathExp
            | Self::MathExpm1
            | Self::MathF16round
            | Self::MathLog
            | Self::MathLog10
            | Self::MathLog1p
            | Self::MathLog2
            | Self::MathSinh
            | Self::MathSqrt
            | Self::MathTan
            | Self::MathTanh
            | Self::MapPrototypeGet
            | Self::MapPrototypeHas
            | Self::MapPrototypeDelete
            | Self::JsonRawJson
            | Self::JsonIsRawJson
            | Self::EncodeUri
            | Self::EncodeUriComponent
            | Self::DecodeUri
            | Self::DecodeUriComponent
            | Self::Escape
            | Self::Unescape
            | Self::DateParse
            | Self::DatePrototypeSetTime
            | Self::DatePrototypeToJson
            | Self::DatePrototypeSetMilliseconds
            | Self::DatePrototypeSetUtcMilliseconds
            | Self::DatePrototypeSetDate
            | Self::DatePrototypeSetUtcDate
            | Self::DatePrototypeSetYear
            | Self::ArrayBufferConstructor
            | Self::ArrayBufferIsView
            | Self::DataViewConstructor
            | Self::DataViewPrototypeGetInt8
            | Self::DataViewPrototypeGetUint8
            | Self::DataViewPrototypeGetInt16
            | Self::DataViewPrototypeGetUint16
            | Self::DataViewPrototypeGetInt32
            | Self::DataViewPrototypeGetUint32
            | Self::DataViewPrototypeGetFloat32
            | Self::DataViewPrototypeGetFloat64
            | Self::WeakMapPrototypeGet
            | Self::WeakMapPrototypeHas
            | Self::WeakMapPrototypeDelete
            | Self::WeakSetPrototypeAdd
            | Self::WeakSetPrototypeHas
            | Self::WeakSetPrototypeDelete
            | Self::SetPrototypeAdd
            | Self::SetPrototypeHas
            | Self::SetPrototypeDelete
            | Self::SetPrototypeUnion
            | Self::SetPrototypeIntersection
            | Self::SetPrototypeDifference
            | Self::SetPrototypeSymmetricDifference
            | Self::SetPrototypeIsSubsetOf
            | Self::SetPrototypeIsSupersetOf
            | Self::SetPrototypeIsDisjointFrom
            | Self::PromiseConstructor
            | Self::PromiseResolve
            | Self::PromiseReject
            | Self::PromisePrototypeCatch
            | Self::PromiseResolveFunction
            | Self::PromiseRejectFunction
            | Self::PromiseAll
            | Self::PromiseRace
            | Self::PromiseAllSettled
            | Self::PromiseAllElement
            | Self::PromiseAllSettledFulfilled
            | Self::PromiseAllSettledRejected
            | Self::RegExpPrototypeMatch
            | Self::RegExpPrototypeSearch
            | Self::AsyncResume
            | Self::AsyncThrow
            | Self::StringPrototypeAnchor
            | Self::StringPrototypeFontcolor
            | Self::StringPrototypeFontsize
            | Self::StringPrototypeLink
            | Self::Print
            // 25.2.4 and 25.4.7 each take one argument.
            | Self::SharedArrayBufferConstructor
            | Self::SharedArrayBufferPrototypeGrow
            | Self::AtomicsIsLockFree
            | Self::BigIntConstructor
            | Self::BigIntPrototypeToString
            | Self::IteratorPrototypeConstructorSet
            | Self::IteratorPrototypeForEach
            | Self::IteratorPrototypeSome
            | Self::IteratorPrototypeEvery
            | Self::IteratorPrototypeFind
            | Self::IteratorPrototypeReduce
            | Self::GeneratorPrototypeNext
            | Self::GeneratorPrototypeReturn
            | Self::GeneratorPrototypeThrow
            | Self::AsyncGeneratorPrototypeNext
            | Self::AsyncGeneratorPrototypeReturn
            | Self::AsyncGeneratorPrototypeThrow
            | Self::AsyncFromSyncIteratorPrototypeNext
            | Self::AsyncFromSyncIteratorPrototypeReturn
            | Self::AsyncFromSyncIteratorPrototypeThrow
            | Self::AsyncFromSyncUnwrap
            | Self::AsyncFromSyncCloseIterator
            | Self::FunctionPrototypeHasInstance
            | Self::SymbolPrototypeToPrimitive
            | Self::ErrorIsError
            | Self::IteratorPrototypeToStringTagSet
            | Self::HostDetachArrayBuffer
            | Self::TypedArrayPrototypeAt
            | Self::TypedArrayPrototypeFill
            | Self::TypedArrayPrototypeIncludes
            | Self::TypedArrayPrototypeIndexOf
            | Self::TypedArrayPrototypeJoin
            | Self::TypedArrayPrototypeLastIndexOf
            | Self::TypedArrayPrototypeSet
            | Self::TypedArrayPrototypeSort
            | Self::TypedArrayPrototypeToSorted
            | Self::TypedArrayPrototypeEvery
            | Self::TypedArrayPrototypeFind
            | Self::TypedArrayPrototypeFindIndex
            | Self::TypedArrayPrototypeFindLast
            | Self::TypedArrayPrototypeFindLastIndex
            | Self::TypedArrayPrototypeForEach
            | Self::TypedArrayPrototypeReduce
            | Self::TypedArrayPrototypeReduceRight
            | Self::TypedArrayPrototypeSome => 1,
            Self::ObjectDefineProperty
            | Self::ReflectDefineProperty
            | Self::ReflectApply
            | Self::ReflectSet
            | Self::DatePrototypeSetMinutes
            | Self::DatePrototypeSetUtcMinutes
            | Self::DatePrototypeSetFullYear
            | Self::DatePrototypeSetUtcFullYear
            // 23.2.6 gives each of the nine a length of three.
            | Self::TypedArrayInt8Constructor
            | Self::TypedArrayUint8Constructor
            | Self::TypedArrayUint8ClampedConstructor
            | Self::TypedArrayInt16Constructor
            | Self::TypedArrayUint16Constructor
            | Self::TypedArrayInt32Constructor
            | Self::TypedArrayUint32Constructor
            | Self::TypedArrayFloat32Constructor
            | Self::TypedArrayFloat64Constructor
            | Self::TypedArrayFloat16Constructor
            | Self::TypedArrayBigInt64Constructor
            | Self::TypedArrayBigUint64Constructor
            // 25.4 gives the read-modify-writes three arguments and 25.4.16
            // the same.
            | Self::AtomicsAdd
            | Self::AtomicsAnd
            | Self::AtomicsExchange
            | Self::AtomicsOr
            | Self::AtomicsStore
            | Self::AtomicsSub
            | Self::AtomicsXor
            | Self::AtomicsNotify => 3,
            Self::MathPow
            | Self::ObjectGetOwnPropertyDescriptor
            | Self::ObjectCreate
            | Self::ObjectDefineProperties
            | Self::ObjectIs
            | Self::ObjectHasOwn
            | Self::ReflectDeleteProperty
            | Self::ReflectGet
            | Self::ReflectGetOwnPropertyDescriptor
            | Self::ReflectHas
            | Self::StringPrototypeSlice
            | Self::StringPrototypeSubstr
            | Self::StringPrototypeSubstring
            | Self::ArrayPrototypeSlice
            | Self::ArrayPrototypeSplice
            | Self::ArrayPrototypeCopyWithin
            | Self::ArrayPrototypeWith
            | Self::MathMax
            | Self::MathMin
            | Self::JsonStringify
            | Self::MathImul
            | Self::ParseInt
            | Self::FunctionPrototypeApply
            | Self::ObjectSetPrototypeOf
            | Self::ReflectSetPrototypeOf
            | Self::ReflectConstruct
            | Self::ArrayPrototypeToSpliced
            | Self::StringPrototypeReplace
            | Self::RegExpPrototypeReplace
            | Self::ObjectAssign
            | Self::StringPrototypeSplit
            | Self::RegExpPrototypeSplit
            | Self::MathAtan2
            | Self::MathHypot
            | Self::MapPrototypeSet
            | Self::WeakMapPrototypeSet
            | Self::MapPrototypeGetOrInsert
            | Self::MapPrototypeGetOrInsertComputed
            | Self::WeakMapPrototypeGetOrInsert
            | Self::WeakMapPrototypeGetOrInsertComputed
            | Self::DatePrototypeSetSeconds
            | Self::DatePrototypeSetUtcSeconds
            | Self::DatePrototypeSetMonth
            | Self::DatePrototypeSetUtcMonth
            | Self::ArrayBufferPrototypeSlice
            | Self::DataViewPrototypeSetInt8
            | Self::DataViewPrototypeSetUint8
            | Self::DataViewPrototypeSetInt16
            | Self::DataViewPrototypeSetUint16
            | Self::DataViewPrototypeSetInt32
            | Self::DataViewPrototypeSetUint32
            | Self::DataViewPrototypeSetFloat32
            | Self::DataViewPrototypeSetFloat64
            | Self::PromisePrototypeThen
            // 25.2.5.4 and 25.4.8 each take two.
            | Self::SharedArrayBufferPrototypeSlice
            | Self::AtomicsLoad
            | Self::BigIntAsIntN
            | Self::BigIntAsUintN
            | Self::TypedArrayPrototypeCopyWithin
            | Self::TypedArrayPrototypeSlice
            | Self::TypedArrayPrototypeSubarray
            | Self::TypedArrayPrototypeWith => 2,
            // 21.4.2.1 and 21.4.3.4 take a year, a month, a day, an hour, a
            // minute, a second and a millisecond.
            Self::DatePrototypeSetHours
            | Self::DatePrototypeSetUtcHours
            // 25.4.5 and 25.4.13 each take four.
            | Self::AtomicsCompareExchange
            | Self::AtomicsWait => 4,
            Self::DateConstructor | Self::DateUtc => 7,
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
/// `%Function.prototype%`, excluding the one Symbol key the latter carries, the
/// two restricted properties it does carry, and `prototype`.
///
/// It serves the same purpose as [`OBJECT_PROTOTYPE_PROPERTIES`]: neither exists
/// yet, so a read of one of these names off a function object is a gap and a
/// write of one would shadow a property that is not writable. `prototype` is
/// not one of them: 10.2.5 gives it to a constructor and to nothing else, so a
/// function object that does not carry it answers undefined for that name.
pub const FUNCTION_PROPERTIES: [&str; 7] = [
    "apply",
    "bind",
    "call",
    "constructor",
    "length",
    "name",
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

/// The property names 20.4.2 gives `%Symbol%`, beside the ones 17 gives every
/// built-in function and the thirteen Symbols of table 1.
pub const SYMBOL_CONSTRUCTOR_PROPERTIES: [&str; 3] = ["for", "keyFor", "prototype"];

/// The property names 20.4.3 gives `%Symbol.prototype%`.
pub const SYMBOL_PROTOTYPE_PROPERTIES: [&str; 4] =
    ["constructor", "description", "toString", "valueOf"];

/// Whether `%Symbol.prototype%` owns a property of this name.
#[must_use]
pub fn symbol_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&SYMBOL_PROTOTYPE_PROPERTIES, name)
}

/// Whether `%Symbol%` owns a property of this name.
#[must_use]
pub fn symbol_constructor_owns(name: &[u16]) -> bool {
    function_prototype_owns(name)
        || SYMBOL_CONSTRUCTOR_PROPERTIES
            .into_iter()
            .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

/// Whether `%String%` owns a property of this name.
#[must_use]
pub fn string_constructor_owns(name: &[u16]) -> bool {
    function_prototype_owns(name)
        || STRING_CONSTRUCTOR_PROPERTIES
            .into_iter()
            .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

/// The property names 27.2.4 gives `%Promise%` beyond what 17 gives every
/// built-in function, and this Realm has not built.
pub const PROMISE_CONSTRUCTOR_PROPERTIES: [&str; 2] = ["any", "try"];

/// Whether `%Promise%` owns a property of this name that this Realm has not
/// built.
#[must_use]
pub fn promise_constructor_owns(name: &[u16]) -> bool {
    function_prototype_owns(name)
        || PROMISE_CONSTRUCTOR_PROPERTIES
            .into_iter()
            .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

/// The property names 24.1.3 gives `%Map.prototype%`.
pub const MAP_PROTOTYPE_PROPERTIES: [&str; 12] = [
    "clear",
    "constructor",
    "delete",
    "entries",
    "forEach",
    "get",
    "getOrInsert",
    "getOrInsertComputed",
    "has",
    "keys",
    "set",
    "values",
];

/// Whether `%Map.prototype%` or `%Object.prototype%` owns a property of this
/// name, which a Map resolves on its Prototype Chain.
#[must_use]
pub fn map_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&MAP_PROTOTYPE_PROPERTIES, name)
}

/// The property names 24.2.3 gives `%Set.prototype%`.
pub const SET_PROTOTYPE_PROPERTIES: [&str; 17] = [
    "add",
    "clear",
    "constructor",
    "delete",
    "difference",
    "entries",
    "forEach",
    "has",
    "intersection",
    "isDisjointFrom",
    "isSubsetOf",
    "isSupersetOf",
    "keys",
    "symmetricDifference",
    "union",
    "values",
    // 24.2.3.14 is an accessor, which the read reaches the same way.
    "size",
];

/// Whether `%Set.prototype%` or `%Object.prototype%` owns a property of this
/// name, which a Set resolves on its Prototype Chain.
#[must_use]
pub fn set_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&SET_PROTOTYPE_PROPERTIES, name)
}

/// The property names 24.3.3 gives `%WeakMap.prototype%`.
pub const WEAK_MAP_PROTOTYPE_PROPERTIES: [&str; 7] = [
    "constructor",
    "delete",
    "get",
    "getOrInsert",
    "getOrInsertComputed",
    "has",
    "set",
];

/// Whether `%WeakMap.prototype%` or `%Object.prototype%` owns a property of
/// this name, which a `WeakMap` resolves on its Prototype Chain.
#[must_use]
pub fn weak_map_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&WEAK_MAP_PROTOTYPE_PROPERTIES, name)
}

/// The property names 24.4.3 gives `%WeakSet.prototype%`.
pub const WEAK_SET_PROTOTYPE_PROPERTIES: [&str; 4] = ["add", "constructor", "delete", "has"];

/// Whether `%WeakSet.prototype%` or `%Object.prototype%` owns a property of
/// this name, which a `WeakSet` resolves on its Prototype Chain.
#[must_use]
pub fn weak_set_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&WEAK_SET_PROTOTYPE_PROPERTIES, name)
}

/// The property names 21.4.4 gives `%Date.prototype%`, and B.2.3 the three
/// of Annex B.
pub const DATE_PROTOTYPE_PROPERTIES: [&str; 47] = [
    "constructor",
    "getDate",
    "getDay",
    "getFullYear",
    "getHours",
    "getMilliseconds",
    "getMinutes",
    "getMonth",
    "getSeconds",
    "getTime",
    "getTimezoneOffset",
    "getUTCDate",
    "getUTCDay",
    "getUTCFullYear",
    "getUTCHours",
    "getUTCMilliseconds",
    "getUTCMinutes",
    "getUTCMonth",
    "getUTCSeconds",
    "getYear",
    "setDate",
    "setFullYear",
    "setHours",
    "setMilliseconds",
    "setMinutes",
    "setMonth",
    "setSeconds",
    "setTime",
    "setUTCDate",
    "setUTCFullYear",
    "setUTCHours",
    "setUTCMilliseconds",
    "setUTCMinutes",
    "setUTCMonth",
    "setUTCSeconds",
    "setYear",
    "toDateString",
    "toGMTString",
    "toISOString",
    "toJSON",
    "toLocaleDateString",
    "toLocaleString",
    "toLocaleTimeString",
    "toString",
    "toTimeString",
    "toUTCString",
    "valueOf",
];

/// Whether `%Date.prototype%` or `%Object.prototype%` owns a property of
/// this name, which a Date resolves on its Prototype Chain.
#[must_use]
pub fn date_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&DATE_PROTOTYPE_PROPERTIES, name)
}

/// The property names 25.1.6 gives `%ArrayBuffer.prototype%`.
pub const ARRAY_BUFFER_PROTOTYPE_PROPERTIES: [&str; 9] = [
    "byteLength",
    "constructor",
    "detached",
    "maxByteLength",
    "resizable",
    "resize",
    "slice",
    "transfer",
    "transferToFixedLength",
];

/// Whether `%ArrayBuffer.prototype%` or `%Object.prototype%` owns a property
/// of this name, which a block resolves on its Prototype Chain.
#[must_use]
pub fn array_buffer_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&ARRAY_BUFFER_PROTOTYPE_PROPERTIES, name)
}

/// The property names 25.3.4 gives `%DataView.prototype%`.
pub const DATA_VIEW_PROTOTYPE_PROPERTIES: [&str; 25] = [
    "buffer",
    "byteLength",
    "byteOffset",
    "constructor",
    "getBigInt64",
    "getBigUint64",
    "getFloat16",
    "getFloat32",
    "getFloat64",
    "getInt16",
    "getInt32",
    "getInt8",
    "getUint16",
    "getUint32",
    "getUint8",
    "setBigInt64",
    "setBigUint64",
    "setFloat16",
    "setFloat32",
    "setFloat64",
    "setInt16",
    "setInt32",
    "setInt8",
    "setUint16",
    "setUint32",
];

/// Whether `%DataView.prototype%` or `%Object.prototype%` owns a property of
/// this name, which a view resolves on its Prototype Chain.
#[must_use]
pub fn data_view_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&DATA_VIEW_PROTOTYPE_PROPERTIES, name)
}

/// The property names 23.2.3 gives `%TypedArray.prototype%`.
pub const TYPED_ARRAY_PROTOTYPE_PROPERTIES: [&str; 36] = [
    "at",
    "buffer",
    "byteLength",
    "byteOffset",
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
    "forEach",
    "includes",
    "indexOf",
    "join",
    "keys",
    "lastIndexOf",
    "length",
    "map",
    "reduce",
    "reduceRight",
    "reverse",
    "set",
    "slice",
    "some",
    "sort",
    "subarray",
    "toLocaleString",
    "toReversed",
    "toSorted",
    "toString",
    "values",
    "with",
];

/// Whether `%TypedArray.prototype%` or `%Object.prototype%` owns a property
/// of this name, which an array of 23.2 resolves on its Prototype Chain.
#[must_use]
pub fn typed_array_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&TYPED_ARRAY_PROTOTYPE_PROPERTIES, name)
}

/// The property names the host object of the conformance suite carries.
pub const HOST_262_PROPERTIES: [&str; 7] = [
    "AbstractModuleSource",
    "agent",
    "createRealm",
    "detachArrayBuffer",
    "evalScript",
    "gc",
    "global",
];

/// Whether the host object of the conformance suite owns a property of this
/// name.
#[must_use]
pub fn host_262_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&HOST_262_PROPERTIES, name)
}

/// The property names 25.4 gives its namespace object.
pub const ATOMICS_PROPERTIES: [&str; 14] = [
    "add",
    "and",
    "compareExchange",
    "exchange",
    "isLockFree",
    "load",
    "notify",
    "or",
    "pause",
    "store",
    "sub",
    "wait",
    "waitAsync",
    "xor",
];

/// Whether the namespace object of 25.4 owns a property of this name.
#[must_use]
pub fn atomics_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&ATOMICS_PROPERTIES, name)
}

/// The property names 27.2.5 gives `%Promise.prototype%`.
pub const PROMISE_PROTOTYPE_PROPERTIES: [&str; 4] = ["catch", "constructor", "finally", "then"];

/// Whether `%Promise.prototype%` or `%Object.prototype%` owns a property of
/// this name, which a Promise resolves on its Prototype Chain.
#[must_use]
pub fn promise_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&PROMISE_PROTOTYPE_PROPERTIES, name)
}

/// The property names 25.5 gives `%JSON%`.
pub const JSON_PROPERTIES: [&str; 4] = ["isRawJSON", "parse", "rawJSON", "stringify"];

/// Whether `%JSON%` owns a property of this name.
#[must_use]
pub fn json_owns(name: &[u16]) -> bool {
    JSON_PROPERTIES
        .into_iter()
        .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

/// The property names 22.2.6 gives `%RegExp.prototype%`.
pub const REGEXP_PROTOTYPE_PROPERTIES: [&str; 16] = [
    "compile",
    "constructor",
    "dotAll",
    "exec",
    "flags",
    "global",
    "hasIndices",
    "ignoreCase",
    "multiline",
    "source",
    "sticky",
    "test",
    "toString",
    "unicode",
    "unicodeSets",
    "lastIndex",
];

/// The accessors 22.2.6 gives `%RegExp.prototype%`, in the order it lists them.
pub const REGEXP_ACCESSORS: [Intrinsic; 10] = [
    Intrinsic::RegExpPrototypeFlags,
    Intrinsic::RegExpPrototypeSource,
    Intrinsic::RegExpPrototypeHasIndices,
    Intrinsic::RegExpPrototypeGlobal,
    Intrinsic::RegExpPrototypeIgnoreCase,
    Intrinsic::RegExpPrototypeMultiline,
    Intrinsic::RegExpPrototypeDotAll,
    Intrinsic::RegExpPrototypeUnicode,
    Intrinsic::RegExpPrototypeUnicodeSets,
    Intrinsic::RegExpPrototypeSticky,
];

/// The eight flag accessors of 22.2.6, in the order 22.2.6.4 reads them.
pub const REGEXP_FLAG_ACCESSORS: [Intrinsic; 8] = [
    Intrinsic::RegExpPrototypeHasIndices,
    Intrinsic::RegExpPrototypeGlobal,
    Intrinsic::RegExpPrototypeIgnoreCase,
    Intrinsic::RegExpPrototypeMultiline,
    Intrinsic::RegExpPrototypeDotAll,
    Intrinsic::RegExpPrototypeUnicode,
    Intrinsic::RegExpPrototypeUnicodeSets,
    Intrinsic::RegExpPrototypeSticky,
];

/// Whether `%RegExp.prototype%` owns a property of this name.
#[must_use]
pub fn regexp_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&REGEXP_PROTOTYPE_PROPERTIES, name)
}

/// The property names 20.3.3 gives `%Boolean.prototype%`.
pub const BOOLEAN_PROTOTYPE_PROPERTIES: [&str; 3] = ["constructor", "toString", "valueOf"];

/// The property names 21.1.3 gives `%Number.prototype%`.
pub const NUMBER_PROTOTYPE_PROPERTIES: [&str; 7] = [
    "constructor",
    "toExponential",
    "toFixed",
    "toLocaleString",
    "toPrecision",
    "toString",
    "valueOf",
];

/// The property names 20.5.3 gives `%Error.prototype%`.
pub const ERROR_PROTOTYPE_PROPERTIES: [&str; 4] = ["constructor", "message", "name", "toString"];

/// The property names 23.1.5.2 gives `%ArrayIteratorPrototype%`.
pub const ARRAY_ITERATOR_PROTOTYPE_PROPERTIES: [&str; 1] = ["next"];

/// Whether `%Number.prototype%` or `%Object.prototype%` owns a property of
/// this name, which a Number resolves on its Prototype Chain.
#[must_use]
pub fn number_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&NUMBER_PROTOTYPE_PROPERTIES, name)
}

/// Whether `%Boolean.prototype%` or `%Object.prototype%` owns a property of
/// this name, which a Boolean resolves on its Prototype Chain.
#[must_use]
pub fn boolean_prototype_owns(name: &[u16]) -> bool {
    wrapper_prototype_owns(&BOOLEAN_PROTOTYPE_PROPERTIES, name)
}

/// Whether one of these names, or a name `%Object.prototype%` owns, is owned
/// by the Prototype an instance of that kind resolves on.
#[must_use]
pub fn wrapper_prototype_owns(names: &[&str], name: &[u16]) -> bool {
    object_prototype_owns(name)
        || names
            .iter()
            .any(|owned| owned.encode_utf16().eq(name.iter().copied()))
}

/// The property names 28.1 gives `%Reflect%`.
pub const REFLECT_PROPERTIES: [&str; 13] = [
    "apply",
    "construct",
    "defineProperty",
    "deleteProperty",
    "get",
    "getOwnPropertyDescriptor",
    "getPrototypeOf",
    "has",
    "isExtensible",
    "ownKeys",
    "preventExtensions",
    "set",
    "setPrototypeOf",
];

/// Whether `%Reflect%` owns a property of this name.
#[must_use]
pub fn reflect_owns(name: &[u16]) -> bool {
    REFLECT_PROPERTIES
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
    number_prototype: Root,
    boolean_prototype: Root,
    regexp_prototype: Root,
    symbol_prototype: Root,
    promise_prototype: Root,
    map_prototype: Root,
    set_prototype: Root,
    date_prototype: Root,
    array_buffer_prototype: Root,
    data_view_prototype: Root,
    typed_array_prototype: Root,
    typed_array_prototypes: [Root; TYPED_ARRAY_KINDS],
    shared_array_buffer_prototype: Root,
    bigint_prototype: Root,
    weak_map_prototype: Root,
    weak_set_prototype: Root,
    map_iterator_prototype: Root,
    set_iterator_prototype: Root,
    array_iterator_prototype: Root,
    iterator_prototype: Root,
    /// `%GeneratorPrototype%` of 27.5.1, which every Generator inherits.
    generator_prototype: Root,
    /// `%GeneratorFunction.prototype%` of 27.3.3, which every generator
    /// function inherits and whose `prototype` is `%GeneratorPrototype%`.
    generator_function_prototype: Root,
    /// `%AsyncGeneratorPrototype%` of 27.6.1, which every `AsyncGenerator`
    /// inherits, and `%AsyncGeneratorFunction.prototype%` of 27.4.3.
    async_generator_prototype: Root,
    async_generator_function_prototype: Root,
    /// `%AsyncIteratorPrototype%` of 27.1.4, which 27.6.1 stands on.
    async_iterator_prototype: Root,
    /// `%AsyncFromSyncIteratorPrototype%` of 27.1.4.2, which 27.1.4.1 gives
    /// the object it wraps a sync iterator in.
    async_from_sync_iterator_prototype: Root,
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
    /// The binding object holds the name as an accessor, which only an
    /// instruction of a Script can read or write.
    Accessor,
}

/// The rows of table 71.
pub const TYPED_ARRAY_KINDS: usize = 12;

/// Each row of table 71: the constructor and the bytes one element takes.
const TYPED_ARRAY_ROWS: [(Intrinsic, u8); TYPED_ARRAY_KINDS] = [
    (Intrinsic::TypedArrayInt8Constructor, 1),
    (Intrinsic::TypedArrayUint8Constructor, 1),
    (Intrinsic::TypedArrayUint8ClampedConstructor, 1),
    (Intrinsic::TypedArrayInt16Constructor, 2),
    (Intrinsic::TypedArrayUint16Constructor, 2),
    (Intrinsic::TypedArrayInt32Constructor, 4),
    (Intrinsic::TypedArrayUint32Constructor, 4),
    (Intrinsic::TypedArrayFloat32Constructor, 4),
    (Intrinsic::TypedArrayFloat64Constructor, 8),
    (Intrinsic::TypedArrayFloat16Constructor, 2),
    (Intrinsic::TypedArrayBigInt64Constructor, 8),
    (Intrinsic::TypedArrayBigUint64Constructor, 8),
];

/// The objects the intrinsic functions of a new Realm are installed on.
struct Holders {
    object_prototype: Root,
    error_prototype: Root,
    function_prototype: Root,
    string_prototype: Root,
    array_prototype: Root,
    array_iterator_prototype: Root,
    global_object: Root,
    math: Root,
    reflect: Root,
    json: Root,
    number_prototype: Root,
    boolean_prototype: Root,
    regexp_prototype: Root,
    symbol_prototype: Root,
    promise_prototype: Root,
    map_prototype: Root,
    set_prototype: Root,
    date_prototype: Root,
    array_buffer_prototype: Root,
    data_view_prototype: Root,
    typed_array_prototype: Root,
    shared_array_buffer_prototype: Root,
    bigint_prototype: Root,
    atomics: Root,
    weak_map_prototype: Root,
    weak_set_prototype: Root,
    map_iterator_prototype: Root,
    set_iterator_prototype: Root,
    iterator_prototype: Root,
    generator_prototype: Root,
    async_generator_prototype: Root,
    async_from_sync_iterator_prototype: Root,
    async_iterator_prototype: Root,
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
    #[expect(
        clippy::too_many_lines,
        reason = "one function builds every intrinsic of a Realm, in the order they depend on each other"
    )]
    pub fn new(heap: &mut GenerationalHeap) -> Result<Self, HeapError> {
        let root_shape = heap.shapes.root_shape();

        // 20.1.3: %Object.prototype% has a null [[Prototype]].
        let object_prototype = heap.allocate_immortal_object(root_shape, VALUE_NULL)?;
        let object_prototype = heap.push_root(Value::from_object(object_prototype))?;
        let ordinary = Self::rooted(heap, object_prototype)?;

        // 20.2.3: %Function.prototype% is a built-in function whose
        // [[Prototype]] is %Object.prototype%. Every other intrinsic stands on
        // it, so it is made here and the pass that installs them takes it as
        // it is.
        let function_prototype =
            heap.allocate_immortal_native(ordinary, Intrinsic::FunctionPrototype.id(), 0)?;
        Self::define_builtin_length_and_name(heap, function_prototype, 0, "")?;
        let function_prototype = heap.push_root(Value::from_object(function_prototype))?;

        // 23.1.3: %Array.prototype% is an Array exotic object.
        let array_prototype = heap.allocate_immortal_array(ordinary, 0)?;
        let array_prototype = heap.push_root(Value::from_object(array_prototype))?;

        // 22.1.3: %String.prototype% is a String exotic object whose
        // [[Prototype]] is %Object.prototype% and whose [[StringData]] is the
        // empty String, so 22.1.3.1 and the rest take it as a receiver.
        let string_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let empty = Value::from_string(heap.strings.allocate_units(&[])?);
        heap.set_object_kind(
            string_prototype,
            super::object::ObjectKind::StringWrapper(empty),
        )?;
        let string_prototype = heap.push_root(Value::from_object(string_prototype))?;

        // 21.1.3: %Number.prototype% is a Number object whose [[Prototype]] is
        // %Object.prototype% and whose [[NumberData]] is +0.
        let number_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        heap.set_object_kind(
            number_prototype,
            super::object::ObjectKind::NumberWrapper(0.0),
        )?;
        let number_prototype = heap.push_root(Value::from_object(number_prototype))?;

        // 20.3.3: %Boolean.prototype% is a Boolean object whose [[Prototype]]
        // is %Object.prototype% and whose [[BooleanData]] is false.
        let boolean_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        heap.set_object_kind(
            boolean_prototype,
            super::object::ObjectKind::BooleanWrapper(false),
        )?;
        let boolean_prototype = heap.push_root(Value::from_object(boolean_prototype))?;

        // 22.2.6: %RegExp.prototype% is an ordinary object and not a RegExp.
        let regexp_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let regexp_prototype = heap.push_root(Value::from_object(regexp_prototype))?;

        // 27.2.5: %Promise.prototype% is an ordinary object and not a Promise.
        let promise_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let promise_prototype = heap.push_root(Value::from_object(promise_prototype))?;

        // 24.1.3 and 24.2.3: %Map.prototype% and %Set.prototype% are ordinary
        // objects and neither a Map nor a Set.
        let map_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let map_prototype = heap.push_root(Value::from_object(map_prototype))?;
        let set_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let set_prototype = heap.push_root(Value::from_object(set_prototype))?;

        // 21.4.4: %Date.prototype% is an ordinary object and no Date.
        let date_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let date_prototype = heap.push_root(Value::from_object(date_prototype))?;

        // 25.1.6: %ArrayBuffer.prototype% is an ordinary object and carries no
        // block of its own.
        let array_buffer_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let array_buffer_prototype = heap.push_root(Value::from_object(array_buffer_prototype))?;

        // 25.3.4: %DataView.prototype% is an ordinary object and no view.
        let data_view_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let data_view_prototype = heap.push_root(Value::from_object(data_view_prototype))?;

        // 21.2.3: %BigInt.prototype% is an ordinary object and no BigInt.
        let bigint_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let bigint_prototype = heap.push_root(Value::from_object(bigint_prototype))?;

        // 25.2.5: %SharedArrayBuffer.prototype% is an ordinary object and no
        // block.
        let shared_array_buffer_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let shared_array_buffer_prototype =
            heap.push_root(Value::from_object(shared_array_buffer_prototype))?;

        // 23.2.3: %TypedArray.prototype% is an ordinary object, and 23.2.7
        // gives each row of table 71 a prototype that inherits from it.
        let typed_array_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let typed_array_prototype = heap.push_root(Value::from_object(typed_array_prototype))?;
        let shared = Self::rooted(heap, typed_array_prototype)?;
        let mut typed_array_prototypes = [typed_array_prototype; TYPED_ARRAY_KINDS];
        for slot in &mut typed_array_prototypes {
            let object = heap.allocate_immortal_object(root_shape, shared)?;
            *slot = heap.push_root(Value::from_object(object))?;
        }

        // 24.3.3 and 24.4.3: %WeakMap.prototype% and %WeakSet.prototype% are
        // ordinary objects and neither a WeakMap nor a WeakSet.
        let weak_map_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let weak_map_prototype = heap.push_root(Value::from_object(weak_map_prototype))?;
        let weak_set_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let weak_set_prototype = heap.push_root(Value::from_object(weak_set_prototype))?;

        // 20.4.3: %Symbol.prototype% is an ordinary object and not a Symbol.
        let symbol_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let symbol_prototype = heap.push_root(Value::from_object(symbol_prototype))?;

        // 23.1.5.2: %ArrayIteratorPrototype% inherits from %IteratorPrototype%,
        // which is an ordinary object of %Object.prototype% until the Iterator
        // intrinsics exist.
        let iterator_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let iterator_prototype_root = heap.push_root(Value::from_object(iterator_prototype))?;
        let map_iterator_prototype =
            heap.allocate_immortal_object(root_shape, Value::from_object(iterator_prototype))?;
        let map_iterator_prototype = heap.push_root(Value::from_object(map_iterator_prototype))?;
        let set_iterator_prototype =
            heap.allocate_immortal_object(root_shape, Value::from_object(iterator_prototype))?;
        let set_iterator_prototype = heap.push_root(Value::from_object(set_iterator_prototype))?;
        let array_iterator_prototype =
            heap.allocate_immortal_object(root_shape, Value::from_object(iterator_prototype))?;
        let array_iterator_prototype =
            heap.push_root(Value::from_object(array_iterator_prototype))?;
        // 27.5.1 gives every Generator `%GeneratorPrototype%`, which inherits
        // from `%IteratorPrototype%`, and 27.3.3 gives every generator
        // function `%GeneratorFunction.prototype%`, whose `prototype` is that
        // same object.
        let generator_prototype =
            heap.allocate_immortal_object(root_shape, Value::from_object(iterator_prototype))?;
        let generator_prototype = heap.push_root(Value::from_object(generator_prototype))?;
        let generator_function_prototype =
            heap.allocate_immortal_object(root_shape, Self::rooted(heap, function_prototype)?)?;
        let generator_function_prototype =
            heap.push_root(Value::from_object(generator_function_prototype))?;
        // 27.1.4 gives every async iterator `%AsyncIteratorPrototype%`, which
        // 27.6.1 stands on, and 27.4.3 gives every async generator function
        // `%AsyncGeneratorFunction.prototype%`.
        let async_iterator_prototype = heap.allocate_immortal_object(root_shape, ordinary)?;
        let async_iterator_prototype =
            heap.push_root(Value::from_object(async_iterator_prototype))?;
        let async_generator_prototype = heap
            .allocate_immortal_object(root_shape, Self::rooted(heap, async_iterator_prototype)?)?;
        let async_generator_prototype =
            heap.push_root(Value::from_object(async_generator_prototype))?;
        let async_generator_function_prototype =
            heap.allocate_immortal_object(root_shape, Self::rooted(heap, function_prototype)?)?;
        let async_generator_function_prototype =
            heap.push_root(Value::from_object(async_generator_function_prototype))?;
        // 27.1.4.2 stands on `%AsyncIteratorPrototype%` and carries the three
        // methods of the wrapper alone; no Script reaches it.
        let async_from_sync_iterator_prototype = heap
            .allocate_immortal_object(root_shape, Self::rooted(heap, async_iterator_prototype)?)?;
        let async_from_sync_iterator_prototype =
            heap.push_root(Value::from_object(async_from_sync_iterator_prototype))?;

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
        // 28.1 is an ordinary object like %Math%, and 19.4.4 gives it to the
        // global object under its own name.
        let reflect = heap.allocate_immortal_object(root_shape, ordinary)?;
        heap.set_object_kind(reflect, super::object::ObjectKind::Reflect)?;
        let reflect = heap.push_root(Value::from_object(reflect))?;
        // 25.4 is an ordinary object like %Math%, and 25.4 gives it to the
        // global object under its own name.
        let atomics = heap.allocate_immortal_object(root_shape, ordinary)?;
        heap.set_object_kind(atomics, super::object::ObjectKind::Atomics)?;
        let atomics = heap.push_root(Value::from_object(atomics))?;
        // 25.5 is an ordinary object like %Math%, and 19.4.2 gives it to the
        // global object under its own name.
        let json = heap.allocate_immortal_object(root_shape, ordinary)?;
        heap.set_object_kind(json, super::object::ObjectKind::Json)?;
        let json = heap.push_root(Value::from_object(json))?;
        let intrinsics = Self::install_intrinsics(
            heap,
            &Holders {
                object_prototype,
                error_prototype,
                function_prototype,
                string_prototype,
                array_prototype,
                array_iterator_prototype,
                global_object,
                math,
                reflect,
                json,
                number_prototype,
                boolean_prototype,
                regexp_prototype,
                symbol_prototype,
                promise_prototype,
                map_prototype,
                set_prototype,
                date_prototype,
                array_buffer_prototype,
                data_view_prototype,
                typed_array_prototype,
                shared_array_buffer_prototype,
                bigint_prototype,
                atomics,
                weak_map_prototype,
                weak_set_prototype,
                map_iterator_prototype,
                set_iterator_prototype,
                iterator_prototype: iterator_prototype_root,
                generator_prototype,
                async_generator_prototype,
                async_from_sync_iterator_prototype,
                async_iterator_prototype,
            },
        )?;

        for (constructor, prototype) in [
            (Intrinsic::ArrayConstructor, array_prototype),
            (Intrinsic::ObjectConstructor, object_prototype),
            (Intrinsic::FunctionConstructor, function_prototype),
            (Intrinsic::StringConstructor, string_prototype),
            (Intrinsic::NumberConstructor, number_prototype),
            (Intrinsic::BooleanConstructor, boolean_prototype),
            (Intrinsic::RegExpConstructor, regexp_prototype),
            (Intrinsic::SymbolConstructor, symbol_prototype),
            (Intrinsic::PromiseConstructor, promise_prototype),
            (Intrinsic::MapConstructor, map_prototype),
            (Intrinsic::SetConstructor, set_prototype),
            (Intrinsic::DateConstructor, date_prototype),
            (Intrinsic::ArrayBufferConstructor, array_buffer_prototype),
            (Intrinsic::DataViewConstructor, data_view_prototype),
            (Intrinsic::TypedArrayBase, typed_array_prototype),
            (
                Intrinsic::SharedArrayBufferConstructor,
                shared_array_buffer_prototype,
            ),
            (Intrinsic::BigIntConstructor, bigint_prototype),
            (Intrinsic::WeakMapConstructor, weak_map_prototype),
            (Intrinsic::WeakSetConstructor, weak_set_prototype),
        ] {
            Self::pair_constructor_with_prototype(heap, &intrinsics, constructor, prototype)?;
        }
        Self::pair_typed_arrays(
            heap,
            &intrinsics,
            typed_array_prototype,
            &typed_array_prototypes,
        )?;
        Self::pair_errors_with_their_prototypes(
            heap,
            &intrinsics,
            error_prototype,
            &native_error_prototypes,
        )?;
        Self::define_species_getters(heap, &intrinsics)?;
        Self::define_restricted_properties(heap, &intrinsics, function_prototype)?;
        Self::define_regexp_accessors(heap, &intrinsics, regexp_prototype)?;
        Self::define_collection_size_getters(heap, &intrinsics, map_prototype, set_prototype)?;
        Self::define_array_buffer_getters(heap, &intrinsics, array_buffer_prototype)?;
        Self::define_data_view_getters(heap, &intrinsics, data_view_prototype)?;
        Self::define_typed_array_getters(heap, &intrinsics, typed_array_prototype)?;
        Self::define_iterator_accessors(heap, &intrinsics, iterator_prototype_root)?;
        // 27.1.3.2.3 gives `%Iterator%` its prototype; the prototype carries the
        // accessor of 27.1.3.3.1 rather than the data `constructor` every other
        // one of the specification has, so it takes no pairing of its own.
        Self::define_prototype_property(
            heap,
            &intrinsics,
            Intrinsic::IteratorConstructor,
            iterator_prototype_root,
        )?;
        // 23.2.3.33: %TypedArray.prototype.toString% is the same function
        // object as %Array.prototype.toString%.
        Self::define_alias(
            heap,
            &intrinsics,
            typed_array_prototype,
            "toString",
            Intrinsic::ArrayPrototypeToString,
        )?;
        Self::define_shared_block_getters(heap, &intrinsics, shared_array_buffer_prototype)?;
        Self::define_trim_aliases(heap, &intrinsics, string_prototype, date_prototype)?;
        Self::define_unscopables(heap, array_prototype)?;
        // 27.3.3.3 gives `%GeneratorFunction.prototype%` the prototype every
        // Generator inherits, and 27.5.1.1 names it back.
        Self::define_link(
            heap,
            generator_function_prototype,
            "prototype",
            generator_prototype,
        )?;
        Self::define_link(
            heap,
            generator_prototype,
            "constructor",
            generator_function_prototype,
        )?;
        Self::define_link(
            heap,
            async_generator_function_prototype,
            "prototype",
            async_generator_prototype,
        )?;
        Self::define_link(
            heap,
            async_generator_prototype,
            "constructor",
            async_generator_function_prototype,
        )?;
        Self::define_to_string_tags(
            heap,
            &[
                (math, "Math"),
                (json, "JSON"),
                (reflect, "Reflect"),
                (symbol_prototype, "Symbol"),
                (array_iterator_prototype, "Array Iterator"),
                // 27.5.1.5 and 27.3.3.2 name the two of clause 27.
                (generator_prototype, "Generator"),
                (generator_function_prototype, "GeneratorFunction"),
                (async_generator_prototype, "AsyncGenerator"),
                (async_generator_function_prototype, "AsyncGeneratorFunction"),
                (promise_prototype, "Promise"),
                (map_prototype, "Map"),
                (set_prototype, "Set"),
                (map_iterator_prototype, "Map Iterator"),
                (set_iterator_prototype, "Set Iterator"),
                (array_buffer_prototype, "ArrayBuffer"),
                (data_view_prototype, "DataView"),
                (shared_array_buffer_prototype, "SharedArrayBuffer"),
                (bigint_prototype, "BigInt"),
                (atomics, "Atomics"),
                (weak_map_prototype, "WeakMap"),
                (weak_set_prototype, "WeakSet"),
            ],
        )?;

        // 19.1 gives the global object `Math` with the attributes 17 gives
        // every value of clause 19 that is not a constant.
        let global = Self::rooted(heap, global_object)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        for (label, namespace) in [
            ("Math", math),
            ("Reflect", reflect),
            ("JSON", json),
            ("Atomics", atomics),
        ] {
            let name = PropertyKey::String(heap.strings.intern(label)?);
            let value = Self::rooted(heap, namespace)?;
            heap.define_own_named(global, name, value, builtin_data())?;
        }

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
            number_prototype,
            boolean_prototype,
            regexp_prototype,
            symbol_prototype,
            promise_prototype,
            map_prototype,
            set_prototype,
            date_prototype,
            array_buffer_prototype,
            data_view_prototype,
            typed_array_prototype,
            typed_array_prototypes,
            shared_array_buffer_prototype,
            bigint_prototype,
            weak_map_prototype,
            weak_set_prototype,
            map_iterator_prototype,
            set_iterator_prototype,
            array_iterator_prototype,
            iterator_prototype: iterator_prototype_root,
            generator_prototype,
            generator_function_prototype,
            async_generator_prototype,
            async_generator_function_prototype,
            async_iterator_prototype,
            async_from_sync_iterator_prototype,
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
            if flags.is_accessor {
                return Ok(Err(BindingOutcome::Accessor));
            }
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
        if flags.is_accessor {
            return Ok(Err(BindingOutcome::Accessor));
        }
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
            if property.flags.is_accessor {
                return Ok(Err(BindingOutcome::Accessor));
            }
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
    /// The values 21.1.2 gives `%Number%`, each of them not writable, not
    /// enumerable and not configurable.
    fn define_number_constants(
        heap: &mut GenerationalHeap,
        constructor: ObjectRef,
    ) -> Result<(), HeapError> {
        let flags = PropertyFlags {
            writable: false,
            enumerable: false,
            configurable: false,
            is_accessor: false,
        };
        for (name, value) in [
            // 21.1.2.1 is the difference between 1 and the next binary64 above
            // it, and 21.1.2.9 the smallest subnormal.
            ("EPSILON", f64::EPSILON),
            ("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
            ("MAX_VALUE", f64::MAX),
            ("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
            ("MIN_VALUE", 5e-324),
            ("NaN", f64::NAN),
            ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
            ("POSITIVE_INFINITY", f64::INFINITY),
        ] {
            let key = intern(heap, name)?;
            heap.define_own_named(constructor, key, Value::from_f64(value), flags)?;
        }
        Ok(())
    }

    /// The values 21.3.1 gives `%Math%`, each of them not writable, not
    /// enumerable and not configurable.
    ///
    /// Every one is the binary64 nearest the number the clause names, which is
    /// what a decimal literal of that many digits parses to.
    /// The thirteen Symbols 20.4.2 gives `%Symbol%`, each of them not
    /// writable, not enumerable and not configurable.
    fn define_well_known_symbols(
        heap: &mut GenerationalHeap,
        symbol: ObjectRef,
    ) -> Result<(), HeapError> {
        let flags = PropertyFlags {
            writable: false,
            enumerable: false,
            configurable: false,
            is_accessor: false,
        };
        for well_known in WellKnownSymbol::ALL {
            let key = intern(heap, well_known.property())?;
            let value = Value::from_symbol(well_known.reference());
            heap.define_own_named(symbol, key, value, flags)?;
        }
        Ok(())
    }

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

    /// `Array.prototype[@@unscopables]` of 23.1.3.37.
    ///
    /// An ordinary object with no Prototype, whose own properties are the
    /// names 13.3.1.1 keeps out of a `with` binding, each of them true.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    /// Gives each holder the `@@toStringTag` its clause names, with the
    /// attributes 20.1.3.6 reads and 17 gives every such value.
    ///
    /// 21.3.1.9 names `%Math%`, 25.5.3 `%JSON%`, 28.1.14 `%Reflect%`, 20.4.3.5
    /// `%Symbol.prototype%` and 23.1.5.2.2 `%ArrayIteratorPrototype%`.
    fn define_to_string_tags(
        heap: &mut GenerationalHeap,
        holders: &[(Root, &str)],
    ) -> Result<(), HeapError> {
        for (holder, tag) in holders {
            let object = Self::rooted(heap, *holder)?
                .as_object()
                .ok_or(HeapError::InvalidReference)?;
            let value = Value::from_string(heap.strings.intern(tag)?);
            heap.define_own_named(
                object,
                WellKnownSymbol::ToStringTag.key(),
                value,
                builtin_metadata(),
            )?;
        }
        Ok(())
    }

    fn define_unscopables(heap: &mut GenerationalHeap, prototype: Root) -> Result<(), HeapError> {
        /// The names 23.1.3.37 lists, in the order it lists them.
        const NAMES: [&str; 16] = [
            "at",
            "copyWithin",
            "entries",
            "fill",
            "find",
            "findIndex",
            "findLast",
            "findLastIndex",
            "flat",
            "flatMap",
            "includes",
            "keys",
            "toReversed",
            "toSorted",
            "toSpliced",
            "values",
        ];
        let shape = heap.shapes.root_shape();
        let list = heap.allocate_immortal_object(shape, VALUE_NULL)?;
        let ordinary = PropertyFlags {
            writable: true,
            enumerable: true,
            configurable: true,
            is_accessor: false,
        };
        for name in NAMES {
            let key = intern(heap, name)?;
            heap.define_own_named(list, key, Value::from_bool(true), ordinary)?;
        }
        let holder = Self::rooted(heap, prototype)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        heap.define_own_named(
            holder,
            WellKnownSymbol::Unscopables.key(),
            Value::from_object(list),
            builtin_metadata(),
        )?;
        Ok(())
    }

    /// `caller` and `arguments` of 20.2.3.
    ///
    /// Both are accessors whose getter and setter are `%ThrowTypeError%`, so
    /// reading either off any function object raises the `TypeError` of 10.2.4.1
    /// rather than answering a frame of the caller.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn define_restricted_properties(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        prototype: Root,
    ) -> Result<(), HeapError> {
        let thrower = Self::rooted(
            heap,
            *intrinsics
                .get(Intrinsic::ThrowTypeError.index())
                .ok_or(HeapError::InvalidReference)?,
        )?;
        let holder = Self::rooted(heap, prototype)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        for name in ["caller", "arguments"] {
            let shape = heap.shapes.root_shape();
            let pair = heap.allocate_immortal_object(shape, super::value::VALUE_NULL)?;
            heap.set_object_kind(
                pair,
                super::object::ObjectKind::Accessor {
                    get: thrower,
                    set: thrower,
                },
            )?;
            let key = PropertyKey::String(heap.strings.intern(name)?);
            heap.define_own_named(
                holder,
                key,
                Value::from_object(pair),
                PropertyFlags {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    is_accessor: true,
                },
            )?;
        }
        Ok(())
    }

    /// The accessors 22.2.6 gives `%RegExp.prototype%`.
    ///
    /// Each has a getter and no setter, which 17 makes undefined.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    /// B.2.2.12 and B.2.2.13: `trimLeft` and `trimRight` are the same
    /// function objects as `trimStart` and `trimEnd`.
    fn define_trim_aliases(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        string_prototype: Root,
        date_prototype: Root,
    ) -> Result<(), HeapError> {
        let holder = Self::rooted(heap, string_prototype)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        // B.2.3.3 names the same function object 21.4.4.43 already is.
        let dates = Self::rooted(heap, date_prototype)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        for (intrinsic, alias, holder) in [
            (Intrinsic::StringPrototypeTrimStart, "trimLeft", holder),
            (Intrinsic::StringPrototypeTrimEnd, "trimRight", holder),
            (Intrinsic::DatePrototypeToUtcString, "toGMTString", dates),
        ] {
            let function = Self::rooted(
                heap,
                *intrinsics
                    .get(intrinsic.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?;
            let key = PropertyKey::String(heap.strings.intern(alias)?);
            heap.define_own_named(holder, key, function, builtin_data())?;
        }
        Ok(())
    }

    /// The four accessors 25.1.6 gives `%ArrayBuffer.prototype%`.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn define_array_buffer_getters(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        array_buffer_prototype: Root,
    ) -> Result<(), HeapError> {
        let holder = Self::rooted(heap, array_buffer_prototype)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        for (intrinsic, name) in [
            (Intrinsic::ArrayBufferPrototypeByteLength, "byteLength"),
            (Intrinsic::ArrayBufferPrototypeDetached, "detached"),
            (Intrinsic::ArrayBufferPrototypeResizable, "resizable"),
            (
                Intrinsic::ArrayBufferPrototypeMaxByteLength,
                "maxByteLength",
            ),
        ] {
            let getter = Self::rooted(
                heap,
                *intrinsics
                    .get(intrinsic.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?;
            let shape = heap.shapes.root_shape();
            let pair = heap.allocate_immortal_object(shape, super::value::VALUE_NULL)?;
            heap.set_object_kind(
                pair,
                super::object::ObjectKind::Accessor {
                    get: getter,
                    set: super::value::VALUE_UNDEFINED,
                },
            )?;
            let key = PropertyKey::String(heap.strings.intern(name)?);
            heap.define_own_named(
                holder,
                key,
                Value::from_object(pair),
                PropertyFlags {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    is_accessor: true,
                },
            )?;
        }
        Ok(())
    }

    /// The three accessors 25.3.4 gives `%DataView.prototype%`.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn define_data_view_getters(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        data_view_prototype: Root,
    ) -> Result<(), HeapError> {
        let holder = Self::rooted(heap, data_view_prototype)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        for (intrinsic, name) in [
            (Intrinsic::DataViewPrototypeBuffer, "buffer"),
            (Intrinsic::DataViewPrototypeByteLength, "byteLength"),
            (Intrinsic::DataViewPrototypeByteOffset, "byteOffset"),
        ] {
            let getter = Self::rooted(
                heap,
                *intrinsics
                    .get(intrinsic.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?;
            let shape = heap.shapes.root_shape();
            let pair = heap.allocate_immortal_object(shape, super::value::VALUE_NULL)?;
            heap.set_object_kind(
                pair,
                super::object::ObjectKind::Accessor {
                    get: getter,
                    set: super::value::VALUE_UNDEFINED,
                },
            )?;
            let key = PropertyKey::String(heap.strings.intern(name)?);
            heap.define_own_named(
                holder,
                key,
                Value::from_object(pair),
                PropertyFlags {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    is_accessor: true,
                },
            )?;
        }
        Ok(())
    }

    /// The four accessors of 23.2.3 and the tag of 23.2.3.38.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn define_typed_array_getters(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        typed_array_prototype: Root,
    ) -> Result<(), HeapError> {
        let holder = Self::rooted(heap, typed_array_prototype)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        for (intrinsic, key) in [
            (
                Intrinsic::TypedArrayPrototypeBuffer,
                PropertyKey::String(heap.strings.intern("buffer")?),
            ),
            (
                Intrinsic::TypedArrayPrototypeByteLength,
                PropertyKey::String(heap.strings.intern("byteLength")?),
            ),
            (
                Intrinsic::TypedArrayPrototypeByteOffset,
                PropertyKey::String(heap.strings.intern("byteOffset")?),
            ),
            (
                Intrinsic::TypedArrayPrototypeLength,
                PropertyKey::String(heap.strings.intern("length")?),
            ),
            (
                Intrinsic::TypedArrayPrototypeToStringTag,
                WellKnownSymbol::ToStringTag.key(),
            ),
        ] {
            let getter = Self::rooted(
                heap,
                *intrinsics
                    .get(intrinsic.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?;
            let shape = heap.shapes.root_shape();
            let pair = heap.allocate_immortal_object(shape, super::value::VALUE_NULL)?;
            heap.set_object_kind(
                pair,
                super::object::ObjectKind::Accessor {
                    get: getter,
                    set: super::value::VALUE_UNDEFINED,
                },
            )?;
            heap.define_own_named(
                holder,
                key,
                Value::from_object(pair),
                PropertyFlags {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    is_accessor: true,
                },
            )?;
        }
        Ok(())
    }

    /// Gives the holder a name whose value is the function object of another
    /// intrinsic, which the clauses that share one ask for.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn define_alias(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        holder: Root,
        name: &str,
        which: Intrinsic,
    ) -> Result<(), HeapError> {
        let holder = Self::rooted(heap, holder)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        let function = Self::rooted(
            heap,
            *intrinsics
                .get(which.index())
                .ok_or(HeapError::InvalidReference)?,
        )?;
        let key = PropertyKey::String(heap.strings.intern(name)?);
        heap.define_own_named(holder, key, function, builtin_data())?;
        Ok(())
    }

    /// Gives a constructor the `prototype` 17 gives it, without the
    /// `constructor` the prototype of every other clause carries back.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    /// One of the two links of 27.3.3 and 27.5.1, which are data properties
    /// that cannot be written and can be configured.
    fn define_link(
        heap: &mut GenerationalHeap,
        holder: Root,
        name: &str,
        target: Root,
    ) -> Result<(), HeapError> {
        let holder = Self::rooted(heap, holder)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        let key = intern(heap, name)?;
        let value = Self::rooted(heap, target)?;
        heap.define_own_named(
            holder,
            key,
            value,
            PropertyFlags {
                writable: false,
                enumerable: false,
                configurable: true,
                is_accessor: false,
            },
        )?;
        Ok(())
    }

    fn define_prototype_property(
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
        )?
        .as_object()
        .ok_or(HeapError::InvalidReference)?;
        let key = intern(heap, "prototype")?;
        let value = Self::rooted(heap, prototype)?;
        heap.define_own_named(
            constructor,
            key,
            value,
            PropertyFlags {
                writable: false,
                enumerable: false,
                configurable: false,
                is_accessor: false,
            },
        )?;
        Ok(())
    }

    /// The `constructor` of 27.1.3.3.1 and the `@@toStringTag` of 27.1.3.3.15,
    /// which are accessor pairs rather than the data properties every other
    /// prototype of the specification carries there.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn define_iterator_accessors(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        iterator_prototype: Root,
    ) -> Result<(), HeapError> {
        let holder = Self::rooted(heap, iterator_prototype)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        for (getter, setter, key) in [
            (
                Intrinsic::IteratorPrototypeConstructorGet,
                Intrinsic::IteratorPrototypeConstructorSet,
                PropertyKey::String(heap.strings.intern("constructor")?),
            ),
            (
                Intrinsic::IteratorPrototypeToStringTagGet,
                Intrinsic::IteratorPrototypeToStringTagSet,
                WellKnownSymbol::ToStringTag.key(),
            ),
        ] {
            let get = Self::rooted(
                heap,
                *intrinsics
                    .get(getter.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?;
            let set = Self::rooted(
                heap,
                *intrinsics
                    .get(setter.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?;
            let shape = heap.shapes.root_shape();
            let pair = heap.allocate_immortal_object(shape, super::value::VALUE_NULL)?;
            heap.set_object_kind(pair, super::object::ObjectKind::Accessor { get, set })?;
            heap.define_own_named(
                holder,
                key,
                Value::from_object(pair),
                PropertyFlags {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    is_accessor: true,
                },
            )?;
        }
        Ok(())
    }

    /// The three accessors of 25.2.5.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn define_shared_block_getters(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        shared_array_buffer_prototype: Root,
    ) -> Result<(), HeapError> {
        let holder = Self::rooted(heap, shared_array_buffer_prototype)?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
        for (intrinsic, name) in [
            (
                Intrinsic::SharedArrayBufferPrototypeByteLength,
                "byteLength",
            ),
            (Intrinsic::SharedArrayBufferPrototypeGrowable, "growable"),
            (
                Intrinsic::SharedArrayBufferPrototypeMaxByteLength,
                "maxByteLength",
            ),
        ] {
            let getter = Self::rooted(
                heap,
                *intrinsics
                    .get(intrinsic.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?;
            let shape = heap.shapes.root_shape();
            let pair = heap.allocate_immortal_object(shape, super::value::VALUE_NULL)?;
            heap.set_object_kind(
                pair,
                super::object::ObjectKind::Accessor {
                    get: getter,
                    set: super::value::VALUE_UNDEFINED,
                },
            )?;
            let key = PropertyKey::String(heap.strings.intern(name)?);
            heap.define_own_named(
                holder,
                key,
                Value::from_object(pair),
                PropertyFlags {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    is_accessor: true,
                },
            )?;
        }
        Ok(())
    }

    /// 23.2.6 and 23.2.7: each row of table 71 gets its prototype, inherits
    /// from `%TypedArray%` and carries `BYTES_PER_ELEMENT` on both objects.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn pair_typed_arrays(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        typed_array_prototype: Root,
        prototypes: &[Root; TYPED_ARRAY_KINDS],
    ) -> Result<(), HeapError> {
        let base = Self::rooted(
            heap,
            *intrinsics
                .get(Intrinsic::TypedArrayBase.index())
                .ok_or(HeapError::InvalidReference)?,
        )?;
        let shared = Self::rooted(heap, typed_array_prototype)?;
        let frozen = PropertyFlags {
            writable: false,
            enumerable: false,
            configurable: false,
            is_accessor: false,
        };
        for (index, (which, size)) in TYPED_ARRAY_ROWS.iter().enumerate() {
            let prototype = *prototypes.get(index).ok_or(HeapError::InvalidReference)?;
            Self::pair_constructor_with_prototype(heap, intrinsics, *which, prototype)?;
            let constructor = Self::rooted(
                heap,
                *intrinsics
                    .get(which.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
            // 23.2.6.2 step 1: each of the nine inherits from `%TypedArray%`.
            heap.set_object_prototype(constructor, base)?;
            let holder = Self::rooted(heap, prototype)?
                .as_object()
                .ok_or(HeapError::InvalidReference)?;
            heap.set_object_prototype(holder, shared)?;
            let key = PropertyKey::String(heap.strings.intern("BYTES_PER_ELEMENT")?);
            let bytes = Value::from_smi(i32::from(*size));
            heap.define_own_named(constructor, key, bytes, frozen)?;
            heap.define_own_named(holder, key, bytes, frozen)?;
        }
        Ok(())
    }

    /// The `size` 24.1.3.10 and 24.2.3.14 give their Prototype.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn define_collection_size_getters(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        map_prototype: Root,
        set_prototype: Root,
    ) -> Result<(), HeapError> {
        for (intrinsic, prototype) in [
            (Intrinsic::MapPrototypeSize, map_prototype),
            (Intrinsic::SetPrototypeSize, set_prototype),
        ] {
            let getter = Self::rooted(
                heap,
                *intrinsics
                    .get(intrinsic.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?;
            let holder = Self::rooted(heap, prototype)?
                .as_object()
                .ok_or(HeapError::InvalidReference)?;
            let shape = heap.shapes.root_shape();
            let pair = heap.allocate_immortal_object(shape, super::value::VALUE_NULL)?;
            heap.set_object_kind(
                pair,
                super::object::ObjectKind::Accessor {
                    get: getter,
                    set: super::value::VALUE_UNDEFINED,
                },
            )?;
            let key = PropertyKey::String(heap.strings.intern("size")?);
            heap.define_own_named(
                holder,
                key,
                Value::from_object(pair),
                PropertyFlags {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    is_accessor: true,
                },
            )?;
        }
        Ok(())
    }

    fn define_regexp_accessors(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
        prototype: Root,
    ) -> Result<(), HeapError> {
        for intrinsic in REGEXP_ACCESSORS {
            let getter = Self::rooted(
                heap,
                *intrinsics
                    .get(intrinsic.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?;
            let holder = Self::rooted(heap, prototype)?
                .as_object()
                .ok_or(HeapError::InvalidReference)?;
            let shape = heap.shapes.root_shape();
            let pair = heap.allocate_immortal_object(shape, super::value::VALUE_NULL)?;
            heap.set_object_kind(
                pair,
                super::object::ObjectKind::Accessor {
                    get: getter,
                    set: super::value::VALUE_UNDEFINED,
                },
            )?;
            let name = intrinsic
                .name()
                .strip_prefix("get ")
                .ok_or(HeapError::InvalidReference)?;
            let key = PropertyKey::String(heap.strings.intern(name)?);
            heap.define_own_named(
                holder,
                key,
                Value::from_object(pair),
                PropertyFlags {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    is_accessor: true,
                },
            )?;
        }
        Ok(())
    }

    /// `get [Symbol.species]` of 23.1.2.5 and 22.2.5.2.
    ///
    /// One getter stands for both, because each answers the `this` value it
    /// was called on; the pair has no setter, which 17 makes undefined.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn define_species_getters(
        heap: &mut GenerationalHeap,
        intrinsics: &[Root],
    ) -> Result<(), HeapError> {
        let getter = Self::rooted(
            heap,
            *intrinsics
                .get(Intrinsic::SpeciesGetter.index())
                .ok_or(HeapError::InvalidReference)?,
        )?;
        for constructor in [
            Intrinsic::ArrayConstructor,
            Intrinsic::RegExpConstructor,
            Intrinsic::PromiseConstructor,
            // 25.1.5.2 and 25.2.4.2 give the two blocks the same accessor.
            Intrinsic::ArrayBufferConstructor,
            Intrinsic::SharedArrayBufferConstructor,
        ] {
            let holder = Self::rooted(
                heap,
                *intrinsics
                    .get(constructor.index())
                    .ok_or(HeapError::InvalidReference)?,
            )?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
            let shape = heap.shapes.root_shape();
            let pair = heap.allocate_immortal_object(shape, super::value::VALUE_NULL)?;
            heap.set_object_kind(
                pair,
                super::object::ObjectKind::Accessor {
                    get: getter,
                    set: super::value::VALUE_UNDEFINED,
                },
            )?;
            heap.define_own_named(
                holder,
                WellKnownSymbol::Species.key(),
                Value::from_object(pair),
                PropertyFlags {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    is_accessor: true,
                },
            )?;
        }
        Ok(())
    }

    /// The `length` and `name` 17 gives a built-in function.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    fn define_builtin_length_and_name(
        heap: &mut GenerationalHeap,
        function: super::value::ObjectRef,
        length: u32,
        name: &str,
    ) -> Result<(), HeapError> {
        let length_key = intern(heap, "length")?;
        let length = Value::from_smi(i32::try_from(length).unwrap_or(i32::MAX));
        heap.define_own_named(function, length_key, length, builtin_metadata())?;
        let name_key = intern(heap, "name")?;
        let name = heap.strings.allocate_str(name)?;
        heap.define_own_named(
            function,
            name_key,
            Value::from_string(name),
            builtin_metadata(),
        )?;
        Ok(())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one function installs every intrinsic on its holder"
    )]
    fn install_intrinsics(
        heap: &mut GenerationalHeap,
        holders: &Holders,
    ) -> Result<[Root; Intrinsic::ALL.len()], HeapError> {
        let mut intrinsics = [holders.object_prototype; Intrinsic::ALL.len()];
        let function_parent = Self::rooted(heap, holders.function_prototype)?;
        for intrinsic in Intrinsic::ALL {
            // The Realm made %Function.prototype% before this pass, because
            // every other intrinsic has it as its [[Prototype]].
            if intrinsic == Intrinsic::FunctionPrototype {
                *intrinsics
                    .get_mut(intrinsic.index())
                    .ok_or(HeapError::InvalidReference)? = holders.function_prototype;
                continue;
            }
            let function =
                heap.allocate_immortal_native(function_parent, intrinsic.id(), intrinsic.length())?;
            Self::define_builtin_length_and_name(
                heap,
                function,
                intrinsic.length(),
                intrinsic.name(),
            )?;
            // 10.2.4.1: `%ThrowTypeError%` is not extensible and its two
            // properties are not configurable, so 7.3.15 answers it frozen.
            if intrinsic == Intrinsic::ThrowTypeError {
                let frozen = PropertyFlags {
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    is_accessor: false,
                };
                heap.reshape_all(function, Some(false))?;
                let length_key = intern(heap, "length")?;
                let held = heap
                    .lookup_named(function, length_key)?
                    .map_or(VALUE_UNDEFINED, |property| property.value);
                heap.define_own_named(function, length_key, held, frozen)?;
                let name_key = intern(heap, "name")?;
                let held = heap
                    .lookup_named(function, name_key)?
                    .map_or(VALUE_UNDEFINED, |property| property.value);
                heap.define_own_named(function, name_key, held, frozen)?;
                heap.prevent_extensions(function)?;
            }
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
                IntrinsicHolder::Reflect => Self::rooted(heap, holders.reflect)?,
                IntrinsicHolder::Json => Self::rooted(heap, holders.json)?,
                IntrinsicHolder::ErrorPrototype => Self::rooted(heap, holders.error_prototype)?,
                IntrinsicHolder::NumberPrototype => Self::rooted(heap, holders.number_prototype)?,
                IntrinsicHolder::BooleanPrototype => Self::rooted(heap, holders.boolean_prototype)?,
                IntrinsicHolder::RegExpPrototype => Self::rooted(heap, holders.regexp_prototype)?,
                IntrinsicHolder::PromisePrototype => Self::rooted(heap, holders.promise_prototype)?,
                IntrinsicHolder::MapPrototype => Self::rooted(heap, holders.map_prototype)?,
                IntrinsicHolder::DatePrototype => Self::rooted(heap, holders.date_prototype)?,
                IntrinsicHolder::ArrayBufferPrototype => {
                    Self::rooted(heap, holders.array_buffer_prototype)?
                }
                IntrinsicHolder::DataViewPrototype => {
                    Self::rooted(heap, holders.data_view_prototype)?
                }
                IntrinsicHolder::TypedArrayPrototype => {
                    Self::rooted(heap, holders.typed_array_prototype)?
                }
                IntrinsicHolder::SharedArrayBufferPrototype => {
                    Self::rooted(heap, holders.shared_array_buffer_prototype)?
                }
                IntrinsicHolder::Atomics => Self::rooted(heap, holders.atomics)?,
                IntrinsicHolder::BigIntPrototype => Self::rooted(heap, holders.bigint_prototype)?,
                IntrinsicHolder::BigIntConstructor => Self::rooted(
                    heap,
                    *intrinsics
                        .get(Intrinsic::BigIntConstructor.index())
                        .ok_or(HeapError::InvalidReference)?,
                )?,
                IntrinsicHolder::ArrayBufferConstructor => Self::rooted(
                    heap,
                    *intrinsics
                        .get(Intrinsic::ArrayBufferConstructor.index())
                        .ok_or(HeapError::InvalidReference)?,
                )?,
                IntrinsicHolder::DateConstructor => Self::rooted(
                    heap,
                    *intrinsics
                        .get(Intrinsic::DateConstructor.index())
                        .ok_or(HeapError::InvalidReference)?,
                )?,
                IntrinsicHolder::ErrorConstructor => Self::rooted(
                    heap,
                    *intrinsics
                        .get(Intrinsic::ErrorConstructor.index())
                        .ok_or(HeapError::InvalidReference)?,
                )?,
                IntrinsicHolder::WeakMapPrototype => {
                    Self::rooted(heap, holders.weak_map_prototype)?
                }
                IntrinsicHolder::WeakSetPrototype => {
                    Self::rooted(heap, holders.weak_set_prototype)?
                }
                IntrinsicHolder::SetPrototype => Self::rooted(heap, holders.set_prototype)?,
                IntrinsicHolder::IteratorPrototype => {
                    Self::rooted(heap, holders.iterator_prototype)?
                }
                IntrinsicHolder::GeneratorPrototype => {
                    Self::rooted(heap, holders.generator_prototype)?
                }
                IntrinsicHolder::AsyncGeneratorPrototype => {
                    Self::rooted(heap, holders.async_generator_prototype)?
                }
                IntrinsicHolder::AsyncFromSyncIteratorPrototype => {
                    Self::rooted(heap, holders.async_from_sync_iterator_prototype)?
                }
                IntrinsicHolder::AsyncIteratorPrototype => {
                    Self::rooted(heap, holders.async_iterator_prototype)?
                }
                IntrinsicHolder::MapIteratorPrototype => {
                    Self::rooted(heap, holders.map_iterator_prototype)?
                }
                IntrinsicHolder::SetIteratorPrototype => {
                    Self::rooted(heap, holders.set_iterator_prototype)?
                }
                IntrinsicHolder::PromiseConstructor => Self::rooted(
                    heap,
                    *intrinsics
                        .get(Intrinsic::PromiseConstructor.index())
                        .ok_or(HeapError::InvalidReference)?,
                )?,
                IntrinsicHolder::SymbolPrototype => Self::rooted(heap, holders.symbol_prototype)?,
                IntrinsicHolder::SymbolConstructor => Self::rooted(
                    heap,
                    *intrinsics
                        .get(Intrinsic::SymbolConstructor.index())
                        .ok_or(HeapError::InvalidReference)?,
                )?,
                IntrinsicHolder::ObjectConstructor => Self::rooted(
                    heap,
                    *intrinsics
                        .get(Intrinsic::ObjectConstructor.index())
                        .ok_or(HeapError::InvalidReference)?,
                )?,
                IntrinsicHolder::StringConstructor => Self::rooted(
                    heap,
                    *intrinsics
                        .get(Intrinsic::StringConstructor.index())
                        .ok_or(HeapError::InvalidReference)?,
                )?,
                IntrinsicHolder::ArrayConstructor => Self::rooted(
                    heap,
                    *intrinsics
                        .get(Intrinsic::ArrayConstructor.index())
                        .ok_or(HeapError::InvalidReference)?,
                )?,
                IntrinsicHolder::NumberConstructor => Self::rooted(
                    heap,
                    *intrinsics
                        .get(Intrinsic::NumberConstructor.index())
                        .ok_or(HeapError::InvalidReference)?,
                )?,
            }
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
            // 10.2.4.1 stands on no object at all, and 20.2.3 is the holder
            // rather than something on one, so nothing installs either.
            // 23.1.2.5 and 22.2.5.2 are accessors installed under a Symbol
            // key beside their constructors, not names on a holder.
            if matches!(
                intrinsic,
                Intrinsic::ThrowTypeError
                    | Intrinsic::AsyncResume
                    | Intrinsic::AsyncThrow
                    | Intrinsic::PromiseResolveFunction
                    | Intrinsic::PromiseRejectFunction
                    | Intrinsic::PromiseAllElement
                    | Intrinsic::PromiseAllSettledFulfilled
                    | Intrinsic::PromiseAllSettledRejected
                    | Intrinsic::AsyncFromSyncUnwrap
                    | Intrinsic::AsyncFromSyncCloseIterator
                    | Intrinsic::FunctionPrototype
                    | Intrinsic::SpeciesGetter
                    | Intrinsic::RegExpPrototypeFlags
                    | Intrinsic::RegExpPrototypeSource
                    | Intrinsic::RegExpPrototypeHasIndices
                    | Intrinsic::RegExpPrototypeGlobal
                    | Intrinsic::RegExpPrototypeIgnoreCase
                    | Intrinsic::RegExpPrototypeMultiline
                    | Intrinsic::RegExpPrototypeDotAll
                    | Intrinsic::RegExpPrototypeUnicode
                    | Intrinsic::RegExpPrototypeUnicodeSets
                    | Intrinsic::RegExpPrototypeSticky
                    | Intrinsic::TypedArrayBase
                    | Intrinsic::TypedArrayPrototypeBuffer
                    | Intrinsic::TypedArrayPrototypeByteLength
                    | Intrinsic::TypedArrayPrototypeByteOffset
                    | Intrinsic::TypedArrayPrototypeLength
                    | Intrinsic::TypedArrayPrototypeToStringTag
                    | Intrinsic::IteratorPrototypeConstructorGet
                    | Intrinsic::IteratorPrototypeConstructorSet
                    | Intrinsic::IteratorPrototypeToStringTagGet
                    | Intrinsic::IteratorPrototypeToStringTagSet
                    | Intrinsic::HostDetachArrayBuffer
                    | Intrinsic::HostGc
                    | Intrinsic::SharedArrayBufferPrototypeByteLength
                    | Intrinsic::SharedArrayBufferPrototypeGrowable
                    | Intrinsic::SharedArrayBufferPrototypeMaxByteLength
                    // 24.1.3.10, 24.2.3.14, 25.1.6 and 25.3.4 give accessors,
                    // which the Realm installs beside the methods.
                    | Intrinsic::MapPrototypeSize
                    | Intrinsic::SetPrototypeSize
                    | Intrinsic::ArrayBufferPrototypeByteLength
                    | Intrinsic::ArrayBufferPrototypeDetached
                    | Intrinsic::ArrayBufferPrototypeResizable
                    | Intrinsic::ArrayBufferPrototypeMaxByteLength
                    | Intrinsic::DataViewPrototypeBuffer
                    | Intrinsic::DataViewPrototypeByteLength
                    | Intrinsic::DataViewPrototypeByteOffset
            ) {
                continue;
            }
            // 20.4.2 gives `%Symbol%` the thirteen Symbols of table 1 as soon
            // as it exists.
            if intrinsic == Intrinsic::SymbolConstructor {
                let key = PropertyKey::String(heap.strings.intern(intrinsic.name())?);
                heap.define_own_named(holder, key, function, builtin_data())?;
                let constructor = function.as_object().ok_or(HeapError::InvalidReference)?;
                Self::define_well_known_symbols(heap, constructor)?;
                continue;
            }
            // 27.1.2.1 is a Symbol-keyed property, so it takes no String name
            // on its holder.
            if intrinsic == Intrinsic::IteratorPrototypeIterator {
                heap.define_own_named(
                    holder,
                    WellKnownSymbol::Iterator.key(),
                    function,
                    builtin_data(),
                )?;
                continue;
            }
            // 27.1.4.1 is a Symbol-keyed property too.
            if intrinsic == Intrinsic::AsyncIteratorPrototypeAsyncIterator {
                heap.define_own_named(
                    holder,
                    WellKnownSymbol::AsyncIterator.key(),
                    function,
                    builtin_data(),
                )?;
                continue;
            }
            // 20.2.3.6 is neither writable nor configurable; 20.4.3.5 is
            // configurable and not writable, as every Symbol-keyed method of
            // clause 20 is.
            if intrinsic == Intrinsic::FunctionPrototypeHasInstance {
                heap.define_own_named(
                    holder,
                    WellKnownSymbol::HasInstance.key(),
                    function,
                    PropertyFlags {
                        writable: false,
                        enumerable: false,
                        configurable: false,
                        is_accessor: false,
                    },
                )?;
                continue;
            }
            if intrinsic == Intrinsic::SymbolPrototypeToPrimitive {
                heap.define_own_named(
                    holder,
                    WellKnownSymbol::ToPrimitive.key(),
                    function,
                    PropertyFlags {
                        writable: false,
                        enumerable: false,
                        configurable: true,
                        is_accessor: false,
                    },
                )?;
                continue;
            }
            // 22.2.6 gives %RegExp.prototype% four Symbol-keyed methods.
            if let Some(key) = match intrinsic {
                Intrinsic::RegExpPrototypeReplace => Some(WellKnownSymbol::Replace.key()),
                Intrinsic::RegExpPrototypeMatch => Some(WellKnownSymbol::Match.key()),
                Intrinsic::RegExpPrototypeSearch => Some(WellKnownSymbol::Search.key()),
                Intrinsic::RegExpPrototypeSplit => Some(WellKnownSymbol::Split.key()),
                _ => None,
            } {
                heap.define_own_named(holder, key, function, builtin_data())?;
                continue;
            }
            let key = PropertyKey::String(heap.strings.intern(intrinsic.name())?);
            heap.define_own_named(holder, key, function, builtin_data())?;
            // 21.1.2 gives %Number% values as well as functions, and they go
            // on it as soon as it exists.
            if intrinsic == Intrinsic::NumberConstructor {
                let constructor = function.as_object().ok_or(HeapError::InvalidReference)?;
                Self::define_number_constants(heap, constructor)?;
            }
            // 21.1.2.12 and 21.1.2.13 are the same function objects as the
            // two of 19.2.4 and 19.2.5 on the global object.
            if matches!(intrinsic, Intrinsic::ParseInt | Intrinsic::ParseFloat) {
                let number = *intrinsics
                    .get(Intrinsic::NumberConstructor.index())
                    .ok_or(HeapError::InvalidReference)?;
                let number = Self::rooted(heap, number)?
                    .as_object()
                    .ok_or(HeapError::InvalidReference)?;
                heap.define_own_named(number, key, function, builtin_data())?;
            }
            // 23.2.3.37: %TypedArray.prototype%[@@iterator] is the same
            // function object as `values`.
            if intrinsic == Intrinsic::TypedArrayPrototypeValues {
                heap.define_own_named(
                    holder,
                    WellKnownSymbol::Iterator.key(),
                    function,
                    builtin_data(),
                )?;
            }
            // 24.1.3.14: %Map.prototype%[@@iterator] is the same function
            // object as `entries`.
            if intrinsic == Intrinsic::MapPrototypeEntries {
                heap.define_own_named(
                    holder,
                    WellKnownSymbol::Iterator.key(),
                    function,
                    builtin_data(),
                )?;
            }
            // 24.2.3.17 and 24.2.3.10: %Set.prototype%[@@iterator] and
            // `keys` are the same function object as `values`.
            if intrinsic == Intrinsic::SetPrototypeValues {
                heap.define_own_named(
                    holder,
                    WellKnownSymbol::Iterator.key(),
                    function,
                    builtin_data(),
                )?;
                let keys = PropertyKey::String(heap.strings.intern("keys")?);
                heap.define_own_named(holder, keys, function, builtin_data())?;
            }
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
        // 20.5.6.2 gives each native error constructor `%Error%` as its
        // [[Prototype]], where every other intrinsic function has
        // %Function.prototype%.
        let parent = Self::rooted(
            heap,
            *intrinsics
                .get(Intrinsic::ErrorConstructor.index())
                .ok_or(HeapError::InvalidReference)?,
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
            let constructor = Self::rooted(
                heap,
                *intrinsics
                    .get(kind.constructor().index())
                    .ok_or(HeapError::InvalidReference)?,
            )?
            .as_object()
            .ok_or(HeapError::InvalidReference)?;
            heap.set_object_prototype(constructor, parent)?;
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

    /// `%Number.prototype%` of 21.1.3.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn number_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.number_prototype)
    }

    /// `%Boolean.prototype%` of 20.3.3.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn boolean_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.boolean_prototype)
    }

    /// %RegExp.prototype%.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] for a stale root.
    pub fn regexp_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.regexp_prototype)
    }

    /// `%Symbol.prototype%` (20.4.3).
    ///
    /// # Errors
    ///
    /// Returns [`HeapError`] when the root no longer names the object.
    pub fn symbol_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.symbol_prototype)
    }

    /// `%Promise.prototype%` of 27.2.5.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root is gone.
    pub fn map_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.map_prototype)
    }

    /// `%Date.prototype%`, 21.4.4.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn date_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.date_prototype)
    }

    /// `%ArrayBuffer.prototype%`, 25.1.6.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn array_buffer_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.array_buffer_prototype)
    }

    /// `%DataView.prototype%`, 25.3.4.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn data_view_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.data_view_prototype)
    }

    /// `%TypedArray.prototype%`, 23.2.3.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn typed_array_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.typed_array_prototype)
    }

    /// Installs the host object of the conformance suite on the global object.
    ///
    /// `$262` is no part of the specification: the embedding asks for it, and
    /// a Realm that is not asked carries none. It holds the global object, the
    /// `DetachArrayBuffer` of 25.1.3.4 and the collection hint of the suite;
    /// `createRealm`, `evalScript`, `agent` and `AbstractModuleSource` are
    /// named gaps, so a Script that reads one is told so.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when a root was discarded.
    pub fn install_host_object(&self, heap: &mut GenerationalHeap) -> Result<(), HeapError> {
        let global = self.global.global_object(heap)?;
        let ordinary = Self::rooted(heap, self.object_prototype)?;
        let shape = heap.shapes.root_shape();
        let host = heap.allocate_immortal_object(shape, ordinary)?;
        heap.set_object_kind(host, super::object::ObjectKind::Host262)?;
        let key = PropertyKey::String(heap.strings.intern("global")?);
        heap.define_own_named(host, key, Value::from_object(global), builtin_data())?;
        for intrinsic in [Intrinsic::HostDetachArrayBuffer, Intrinsic::HostGc] {
            let function = self.intrinsic(heap, intrinsic)?;
            let key = PropertyKey::String(heap.strings.intern(intrinsic.name())?);
            heap.define_own_named(host, key, function, builtin_data())?;
        }
        let key = PropertyKey::String(heap.strings.intern("$262")?);
        heap.define_own_named(global, key, Value::from_object(host), builtin_data())?;
        Ok(())
    }

    /// `%Iterator.prototype%`, 27.1.2.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn iterator_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.iterator_prototype)
    }

    /// `%GeneratorPrototype%`, 27.5.1.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn generator_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.generator_prototype)
    }

    /// `%GeneratorFunction.prototype%`, 27.3.3.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn generator_function_prototype(
        &self,
        heap: &GenerationalHeap,
    ) -> Result<Value, HeapError> {
        Self::rooted(heap, self.generator_function_prototype)
    }

    /// `%AsyncGeneratorPrototype%`, 27.6.1.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn async_generator_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.async_generator_prototype)
    }

    /// `%AsyncGeneratorFunction.prototype%`, 27.4.3.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn async_generator_function_prototype(
        &self,
        heap: &GenerationalHeap,
    ) -> Result<Value, HeapError> {
        Self::rooted(heap, self.async_generator_function_prototype)
    }

    /// `%AsyncIteratorPrototype%`, 27.1.4.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn async_iterator_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.async_iterator_prototype)
    }

    /// `%AsyncFromSyncIteratorPrototype%`, 27.1.4.2.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn async_from_sync_iterator_prototype(
        &self,
        heap: &GenerationalHeap,
    ) -> Result<Value, HeapError> {
        Self::rooted(heap, self.async_from_sync_iterator_prototype)
    }

    /// `%BigInt.prototype%`, 21.2.3.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn bigint_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.bigint_prototype)
    }

    /// `%SharedArrayBuffer.prototype%`, 25.2.5.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn shared_array_buffer_prototype(
        &self,
        heap: &GenerationalHeap,
    ) -> Result<Value, HeapError> {
        Self::rooted(heap, self.shared_array_buffer_prototype)
    }

    /// The prototype 23.2.7 gives the row of table 71 the index names.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root or for an
    /// index outside table 71.
    pub fn typed_array_prototype_of(
        &self,
        kind: u8,
        heap: &GenerationalHeap,
    ) -> Result<Value, HeapError> {
        let root = *self
            .typed_array_prototypes
            .get(usize::from(kind))
            .ok_or(HeapError::InvalidReference)?;
        Self::rooted(heap, root)
    }

    /// `%WeakMap.prototype%`, 24.3.3.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn weak_map_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.weak_map_prototype)
    }

    /// `%WeakSet.prototype%`, 24.4.3.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] for a stale root.
    pub fn weak_set_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.weak_set_prototype)
    }

    /// `%MapIteratorPrototype%`, 24.1.5.2.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn map_iterator_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.map_iterator_prototype)
    }

    /// `%SetIteratorPrototype%`, 24.2.5.2.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn set_iterator_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.set_iterator_prototype)
    }

    /// `%Set.prototype%`, 24.2.3.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn set_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.set_prototype)
    }

    /// `%Map.prototype%`, 24.1.3.
    ///
    /// # Errors
    ///
    /// Returns [`HeapError::InvalidReference`] when the root was discarded.
    pub fn promise_prototype(&self, heap: &GenerationalHeap) -> Result<Value, HeapError> {
        Self::rooted(heap, self.promise_prototype)
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
