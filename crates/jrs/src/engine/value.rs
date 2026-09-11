// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "NaN-boxed 64-bit value bit-level manipulation"
)]

//! 64-bit NaN-boxed value representation.
//!
//! Every JavaScript value fits in a single 64-bit word (`Value(u64)`).
//! All normal binary64 IEEE-754 numbers are represented directly. Non-number
//! values are encoded inside the quiet NaN bit space (`0xFFF8_0000_0000_0000`
//! and above), allowing zero-allocation and single-word register storage.

use core::fmt;

/// Opaque index reference to an object in the heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectRef(pub u32);

const OLD_GENERATION_BIT: u32 = 1 << 31;

impl ObjectRef {
    /// Creates a reference into the active Nursery semispace.
    #[must_use]
    pub const fn young(index: u32) -> Self {
        Self(index & !OLD_GENERATION_BIT)
    }

    /// Creates a reference into the Old Generation.
    #[must_use]
    pub const fn old(index: u32) -> Self {
        Self(OLD_GENERATION_BIT | (index & !OLD_GENERATION_BIT))
    }

    /// Returns `true` when this reference addresses the Old Generation.
    #[must_use]
    pub const fn is_old(self) -> bool {
        self.0 & OLD_GENERATION_BIT != 0
    }

    /// Returns `true` when this reference addresses the Nursery.
    #[must_use]
    pub const fn is_young(self) -> bool {
        !self.is_old()
    }

    /// Returns the generation-local object index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0 & !OLD_GENERATION_BIT
    }
}

/// Opaque index reference to a string in the string arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StringRef(pub u32);

/// Opaque index reference to a symbol in the symbol arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolRef(pub u32);

/// Opaque index reference to a `BigInt` in the `BigInt` arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BigIntRef(pub u32);

/// 64-bit NaN-boxed value.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Value(pub u64);

// High 16 bits tags for NaN-boxing.
// IEEE-754 double quiet NaNs have exponent 0x7FF and mantissa bit 51 set.
// When sign bit is 1, quiet NaNs span 0xFFF8_0000_0000_0000 .. 0xFFFF_FFFF_FFFF_FFFF.
const TAG_MASK: u64 = 0xFFFF_0000_0000_0000;
const TAG_PREFIX: u64 = 0xFFF8_0000_0000_0000;

/// Tag for 32-bit signed small integers (Smis).
pub const TAG_SMI: u64 = 0xFFF9_0000_0000_0000;
/// Tag for heap-allocated objects.
pub const TAG_OBJECT: u64 = 0xFFFA_0000_0000_0000;
/// Tag for heap-allocated strings.
pub const TAG_STRING: u64 = 0xFFFB_0000_0000_0000;
/// Tag for unique symbols.
pub const TAG_SYMBOL: u64 = 0xFFFC_0000_0000_0000;
/// Tag for `BigInts`.
pub const TAG_BIGINT: u64 = 0xFFFD_0000_0000_0000;
/// Tag for small inline Latin-1 strings (up to 5 bytes).
pub const TAG_SSO_STRING: u64 = 0xFFFE_0000_0000_0000;
/// Tag for language singletons (undefined, null, booleans).
pub const TAG_SPECIAL: u64 = 0xFFFF_0000_0000_0000;

/// Bit pattern for `undefined`.
pub const VALUE_UNDEFINED: Value = Value(TAG_SPECIAL);
/// Bit pattern for `null`.
pub const VALUE_NULL: Value = Value(TAG_SPECIAL | 1);
/// Bit pattern for `false`.
pub const VALUE_FALSE: Value = Value(TAG_SPECIAL | 2);
/// Bit pattern for `true`.
pub const VALUE_TRUE: Value = Value(TAG_SPECIAL | 3);

/// Canonical quiet NaN used for floating-point NaN results.
pub const CANONICAL_NAN_BITS: u64 = 0x7FF8_0000_0000_0000;
/// Canonical quiet NaN value.
pub const VALUE_NAN: Value = Value(CANONICAL_NAN_BITS);

const PAYLOAD_MASK: u64 = 0x0000_0000_FFFF_FFFF;

impl Value {
    /// Encodes a small signed 32-bit integer.
    #[must_use]
    pub const fn from_smi(value: i32) -> Self {
        #[expect(
            clippy::as_conversions,
            reason = "transmuting i32 to lower 32 bits of u64 is lossless"
        )]
        let payload = (value as u32) as u64;
        Self(TAG_SMI | payload)
    }

    /// Encodes an IEEE-754 binary64 number.
    #[must_use]
    pub const fn from_f64(number: f64) -> Self {
        if number.is_nan() {
            VALUE_NAN
        } else {
            Self(number.to_bits())
        }
    }

    /// Encodes a boolean value.
    #[must_use]
    pub const fn from_bool(boolean: bool) -> Self {
        if boolean { VALUE_TRUE } else { VALUE_FALSE }
    }

    /// Encodes an object reference.
    #[must_use]
    pub const fn from_object(object: ObjectRef) -> Self {
        #[expect(
            clippy::as_conversions,
            reason = "widening u32 index to u64 payload is lossless"
        )]
        let payload = object.0 as u64;
        Self(TAG_OBJECT | payload)
    }

    /// Encodes a string reference.
    #[must_use]
    pub const fn from_string(string: StringRef) -> Self {
        #[expect(
            clippy::as_conversions,
            reason = "widening u32 index to u64 payload is lossless"
        )]
        let payload = string.0 as u64;
        Self(TAG_STRING | payload)
    }

    /// Encodes a symbol reference.
    #[must_use]
    pub const fn from_symbol(symbol: SymbolRef) -> Self {
        #[expect(
            clippy::as_conversions,
            reason = "widening u32 index to u64 payload is lossless"
        )]
        let payload = symbol.0 as u64;
        Self(TAG_SYMBOL | payload)
    }

    /// Encodes a `BigInt` reference.
    #[must_use]
    pub const fn from_bigint(bigint: BigIntRef) -> Self {
        #[expect(
            clippy::as_conversions,
            reason = "widening u32 index to u64 payload is lossless"
        )]
        let payload = bigint.0 as u64;
        Self(TAG_BIGINT | payload)
    }

    /// Encodes an inline Small String Optimization (SSO) string (up to 5 ASCII/Latin-1 bytes).
    #[must_use]
    pub fn from_sso(bytes: &[u8]) -> Option<Self> {
        let len = bytes.len();
        if len > 5 {
            return None;
        }
        let mut payload = u64::try_from(len).unwrap_or(0) << 40;
        for (i, &byte) in bytes.iter().enumerate() {
            let shift = 4usize.saturating_sub(i).saturating_mul(8);
            let byte_u64 = u64::from(byte);
            payload |= byte_u64 << shift;
        }
        Some(Self(TAG_SSO_STRING | payload))
    }

    /// Returns `true` if the value is an IEEE-754 double (not a tagged pointer/special/smi).
    #[must_use]
    pub const fn is_double(self) -> bool {
        (self.0 & TAG_PREFIX) != TAG_PREFIX
    }

    /// Returns `true` if the value is a number (either Smi or double).
    #[must_use]
    pub const fn is_number(self) -> bool {
        self.is_double() || self.is_smi()
    }

    /// Returns `true` if the value is a small integer (Smi).
    #[must_use]
    pub const fn is_smi(self) -> bool {
        (self.0 & TAG_MASK) == TAG_SMI
    }

    /// Extracts the small integer if this is a Smi.
    #[must_use]
    pub const fn as_smi(self) -> Option<i32> {
        if self.is_smi() {
            #[expect(
                clippy::as_conversions,
                reason = "low 32 bits represent exact i32 two's complement"
            )]
            Some((self.0 as u32) as i32)
        } else {
            None
        }
    }

    /// Returns the numeric value as `f64`.
    #[must_use]
    pub fn as_f64(self) -> Option<f64> {
        if self.is_double() {
            Some(f64::from_bits(self.0))
        } else {
            self.as_smi().map(f64::from)
        }
    }

    /// Returns `true` if the value is `undefined`.
    #[must_use]
    pub const fn is_undefined(self) -> bool {
        self.0 == VALUE_UNDEFINED.0
    }

    /// Returns `true` if the value is `null`.
    #[must_use]
    pub const fn is_null(self) -> bool {
        self.0 == VALUE_NULL.0
    }

    /// Returns `true` if the value is `null` or `undefined`.
    #[must_use]
    pub const fn is_null_or_undefined(self) -> bool {
        self.is_undefined() || self.is_null()
    }

    /// Returns `true` if the value is a boolean.
    #[must_use]
    pub const fn is_boolean(self) -> bool {
        self.0 == VALUE_FALSE.0 || self.0 == VALUE_TRUE.0
    }

    /// Extracts the boolean value if this is a boolean.
    #[must_use]
    pub const fn as_boolean(self) -> Option<bool> {
        if self.0 == VALUE_TRUE.0 {
            Some(true)
        } else if self.0 == VALUE_FALSE.0 {
            Some(false)
        } else {
            None
        }
    }

    /// Returns `true` if the value is a heap object reference.
    #[must_use]
    pub const fn is_object(self) -> bool {
        (self.0 & TAG_MASK) == TAG_OBJECT
    }

    /// Extracts the heap object reference if this is an object.
    #[must_use]
    pub const fn as_object(self) -> Option<ObjectRef> {
        if self.is_object() {
            #[expect(
                clippy::as_conversions,
                reason = "payload mask ensures truncation fits u32 index"
            )]
            Some(ObjectRef((self.0 & PAYLOAD_MASK) as u32))
        } else {
            None
        }
    }

    /// Returns `true` if the value is a heap-allocated string reference.
    #[must_use]
    pub const fn is_heap_string(self) -> bool {
        (self.0 & TAG_MASK) == TAG_STRING
    }

    /// Extracts the heap string reference if this is a heap string.
    #[must_use]
    pub const fn as_heap_string(self) -> Option<StringRef> {
        if self.is_heap_string() {
            #[expect(
                clippy::as_conversions,
                reason = "payload mask ensures truncation fits u32 index"
            )]
            Some(StringRef((self.0 & PAYLOAD_MASK) as u32))
        } else {
            None
        }
    }

    /// Returns `true` if the value is an inline SSO string.
    #[must_use]
    pub const fn is_sso_string(self) -> bool {
        (self.0 & TAG_MASK) == TAG_SSO_STRING
    }

    /// Extracts bytes from an inline SSO string.
    #[must_use]
    pub fn as_sso_string(self, buffer: &mut [u8; 5]) -> Option<usize> {
        if !self.is_sso_string() {
            return None;
        }
        let len = usize::try_from((self.0 >> 40) & 0x0F).unwrap_or(0);
        if len > 5 {
            return None;
        }
        for (i, slot) in buffer.iter_mut().take(len).enumerate() {
            let shift = 4usize.saturating_sub(i).saturating_mul(8);
            #[expect(clippy::as_conversions, reason = "masking lowest 8 bits extracts byte")]
            let byte = ((self.0 >> shift) & 0xFF) as u8;
            *slot = byte;
        }
        Some(len)
    }

    /// Returns `true` if the value is a string (heap or SSO).
    #[must_use]
    pub const fn is_string(self) -> bool {
        self.is_heap_string() || self.is_sso_string()
    }

    /// Returns `true` if the value is a symbol reference.
    #[must_use]
    pub const fn is_symbol(self) -> bool {
        (self.0 & TAG_MASK) == TAG_SYMBOL
    }

    /// Extracts the symbol reference if this is a symbol.
    #[must_use]
    pub const fn as_symbol(self) -> Option<SymbolRef> {
        if self.is_symbol() {
            #[expect(
                clippy::as_conversions,
                reason = "payload mask ensures truncation fits u32 index"
            )]
            Some(SymbolRef((self.0 & PAYLOAD_MASK) as u32))
        } else {
            None
        }
    }

    /// Returns `true` if the value is a `BigInt` reference.
    #[must_use]
    pub const fn is_bigint(self) -> bool {
        (self.0 & TAG_MASK) == TAG_BIGINT
    }

    /// Extracts the `BigInt` reference if this is a `BigInt`.
    #[must_use]
    pub const fn as_bigint(self) -> Option<BigIntRef> {
        if self.is_bigint() {
            #[expect(
                clippy::as_conversions,
                reason = "payload mask ensures truncation fits u32 index"
            )]
            Some(BigIntRef((self.0 & PAYLOAD_MASK) as u32))
        } else {
            None
        }
    }

    /// ECMAScript `ToBoolean` (ECMA-262 §7.1.2) for primitives and handles.
    #[must_use]
    pub fn to_boolean(self) -> bool {
        if self.is_undefined() || self.is_null() || self.0 == VALUE_FALSE.0 {
            false
        } else if self.0 == VALUE_TRUE.0 {
            true
        } else if let Some(smi) = self.as_smi() {
            smi != 0
        } else if let Some(double) = self.as_f64() {
            double != 0.0 && !double.is_nan()
        } else if self.is_sso_string() {
            let len = usize::try_from((self.0 >> 40) & 0x0F).unwrap_or(0);
            len > 0
        } else {
            // Objects, symbols, BigInts, and heap strings (non-empty checked by caller)
            true
        }
    }

    /// Strict equality check (ECMA-262 §7.2.15).
    #[must_use]
    pub fn strictly_equals(self, other: Self) -> bool {
        if self.0 == other.0 {
            // Note: In IEEE-754, NaN === NaN is false.
            if self.is_double() && f64::from_bits(self.0).is_nan() {
                return false;
            }
            return true;
        }
        if self.is_number()
            && other.is_number()
            && let (Some(a), Some(b)) = (self.as_f64(), other.as_f64())
        {
            #[expect(clippy::float_cmp, reason = "ECMAScript strict equality")]
            return a == b;
        }
        false
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_undefined() {
            f.write_str("undefined")
        } else if self.is_null() {
            f.write_str("null")
        } else if let Some(b) = self.as_boolean() {
            write!(f, "{b}")
        } else if let Some(smi) = self.as_smi() {
            write!(f, "{smi}")
        } else if let Some(dbl) = self.as_f64() {
            write!(f, "{dbl}")
        } else if let Some(obj) = self.as_object() {
            write!(f, "[Object #{}]", obj.0)
        } else if let Some(s) = self.as_heap_string() {
            write!(f, "[String #{}]", s.0)
        } else if self.is_sso_string() {
            let mut buf = [0u8; 5];
            let len = self.as_sso_string(&mut buf).unwrap_or(0);
            #[expect(
                clippy::indexing_slicing,
                reason = "as_sso_string bounds len to buf.len()"
            )]
            let s = core::str::from_utf8(&buf[..len]).unwrap_or("<invalid sso>");
            write!(f, "\"{s}\"")
        } else if let Some(sym) = self.as_symbol() {
            write!(f, "Symbol(#{})", sym.0)
        } else if let Some(bi) = self.as_bigint() {
            write!(f, "{}n", bi.0)
        } else {
            write!(f, "<Value 0x{:016X}>", self.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_size_is_8_bytes() {
        assert_eq!(core::mem::size_of::<Value>(), 8);
    }

    #[test]
    fn primitives_encoding_and_detection() {
        assert!(VALUE_UNDEFINED.is_undefined());
        assert!(!VALUE_UNDEFINED.is_null());
        assert!(!VALUE_UNDEFINED.to_boolean());

        assert!(VALUE_NULL.is_null());
        assert!(!VALUE_NULL.is_undefined());
        assert!(!VALUE_NULL.to_boolean());

        assert_eq!(VALUE_TRUE.as_boolean(), Some(true));
        assert!(VALUE_TRUE.to_boolean());
        assert_eq!(VALUE_FALSE.as_boolean(), Some(false));
        assert!(!VALUE_FALSE.to_boolean());

        let smi = Value::from_smi(42);
        assert!(smi.is_smi());
        assert!(smi.is_number());
        assert_eq!(smi.as_smi(), Some(42));
        assert_eq!(smi.as_f64(), Some(42.0));
        assert!(smi.to_boolean());

        let zero_smi = Value::from_smi(0);
        assert!(!zero_smi.to_boolean());

        let negative_smi = Value::from_smi(-100);
        assert_eq!(negative_smi.as_smi(), Some(-100));

        let dbl = Value::from_f64(1.2345);
        assert!(dbl.is_double());
        assert!(dbl.is_number());
        assert_eq!(dbl.as_f64(), Some(1.2345));
        assert!(dbl.to_boolean());

        let nan_val = Value::from_f64(f64::NAN);
        assert!(nan_val.is_double());
        assert!(nan_val.as_f64().is_some_and(f64::is_nan));
        assert!(!nan_val.to_boolean());
    }

    #[test]
    fn handles_and_sso_encoding() {
        let obj = Value::from_object(ObjectRef(1234));
        assert!(obj.is_object());
        assert_eq!(obj.as_object(), Some(ObjectRef(1234)));

        let str_ref = Value::from_string(StringRef(5678));
        assert!(str_ref.is_heap_string());
        assert!(str_ref.is_string());
        assert_eq!(str_ref.as_heap_string(), Some(StringRef(5678)));

        let sso = Value::from_sso(b"hello").unwrap();
        assert!(sso.is_sso_string());
        assert!(sso.is_string());
        let mut buf = [0u8; 5];
        let len = sso.as_sso_string(&mut buf).unwrap();
        assert_eq!(len, 5);
        assert_eq!(&buf[..5], b"hello");

        let empty_sso = Value::from_sso(b"").unwrap();
        assert!(empty_sso.is_sso_string());
        assert!(!empty_sso.to_boolean());
    }

    #[test]
    fn strict_equality_matches_ecma() {
        assert!(Value::from_smi(42).strictly_equals(Value::from_smi(42)));
        assert!(Value::from_smi(42).strictly_equals(Value::from_f64(42.0)));
        assert!(!Value::from_smi(42).strictly_equals(Value::from_smi(43)));
        assert!(!VALUE_NAN.strictly_equals(VALUE_NAN));
        assert!(VALUE_UNDEFINED.strictly_equals(VALUE_UNDEFINED));
        assert!(!VALUE_UNDEFINED.strictly_equals(VALUE_NULL));
    }
}
