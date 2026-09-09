// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Primitive values follow ECMA-262 6.1, 7.1.2, 7.1.4 and 7.2.14.
//! Strings preserve all UTF-16 code units, including lone surrogates.

use alloc::{rc::Rc, string::String, string::ToString};
use core::fmt;

/// Opaque callable identity. Script handles belong to one runtime heap.
/// They cannot be constructed or dereferenced by an embedding.
#[derive(Clone, Debug)]
pub struct FunctionValue(pub(crate) Callable);

/// Opaque ordinary-object identity owned by one execution heap.
#[derive(Clone, Debug)]
pub struct ObjectValue {
    pub(crate) handle: crate::heap::Handle,
    pub(crate) owner: Rc<()>,
}

impl PartialEq for ObjectValue {
    fn eq(&self, other: &Self) -> bool {
        self.handle == other.handle && Rc::ptr_eq(&self.owner, &other.owner)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Callable {
    Native(crate::bytecode::Builtin),
    Host {
        handle: crate::heap::Handle,
        owner: Rc<()>,
    },
    Script {
        handle: crate::heap::Handle,
        owner: Rc<()>,
    },
    Resolver {
        handle: crate::heap::Handle,
        owner: Rc<()>,
        reject: bool,
    },
    Bound {
        handle: crate::heap::Handle,
        owner: Rc<()>,
    },
}

impl FunctionValue {
    pub(crate) const fn native(builtin: crate::bytecode::Builtin) -> Self {
        Self(Callable::Native(builtin))
    }
}

impl PartialEq for FunctionValue {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Callable::Native(a), Callable::Native(b)) => a == b,
            (
                Callable::Host {
                    handle: a,
                    owner: x,
                },
                Callable::Host {
                    handle: b,
                    owner: y,
                },
            )
            | (
                Callable::Bound {
                    handle: a,
                    owner: x,
                },
                Callable::Bound {
                    handle: b,
                    owner: y,
                },
            )
            | (
                Callable::Script {
                    handle: a,
                    owner: x,
                },
                Callable::Script {
                    handle: b,
                    owner: y,
                },
            ) => a == b && Rc::ptr_eq(x, y),
            (
                Callable::Resolver {
                    handle: left,
                    owner: left_owner,
                    reject: left_reject,
                },
                Callable::Resolver {
                    handle: right,
                    owner: right_owner,
                    reject: right_reject,
                },
            ) => {
                left == right && left_reject == right_reject && Rc::ptr_eq(left_owner, right_owner)
            }
            _ => false,
        }
    }
}

/// A currently implemented ECMAScript value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// The sole value of the Undefined type.
    Undefined,
    /// The sole value of the Null type.
    Null,
    /// A Boolean value.
    Boolean(bool),
    /// An IEEE 754 binary64 Number, including NaN and signed zero.
    Number(f64),
    /// Immutable UTF-16 code units. Cloning shares the allocation.
    String(Rc<[u16]>),
    /// A unique Symbol identity, distinct from its description.
    Symbol(crate::SymbolValue),
    /// A script, intrinsic or explicitly installed host callable.
    Function(FunctionValue),
    /// An ordinary object identified by a generation-checked heap handle.
    Object(ObjectValue),
}

impl Value {
    /// Constructs a JavaScript string from Unicode scalar values.
    #[must_use]
    pub fn string(text: &str) -> Self {
        Self::String(text.encode_utf16().collect())
    }

    /// ECMAScript `ToBoolean` for the implemented primitive types.
    #[must_use]
    pub fn to_boolean(&self) -> bool {
        match self {
            Self::Undefined | Self::Null => false,
            Self::Boolean(value) => *value,
            Self::Number(value) => *value != 0.0 && !value.is_nan(),
            Self::String(value) => !value.is_empty(),
            Self::Function(_) | Self::Object(_) | Self::Symbol(_) => true,
        }
    }

    /// ECMAScript `ToNumber` for the implemented primitive types.
    #[must_use]
    pub fn to_number(&self) -> f64 {
        match self {
            Self::Undefined | Self::Function(_) | Self::Object(_) | Self::Symbol(_) => f64::NAN,
            Self::Null => 0.0,
            Self::Boolean(value) => f64::from(u8::from(*value)),
            Self::Number(value) => *value,
            Self::String(units) => {
                String::from_utf16(units).map_or(f64::NAN, |s| string_number(&s))
            }
        }
    }

    /// ECMAScript strict equality (NaN differs from itself; signed zeroes compare equal).
    #[must_use]
    pub fn strictly_equals(&self, other: &Self) -> bool {
        self == other
    }

    /// ECMAScript `ToUint32`, including truncation and reduction modulo 2^32.
    #[must_use]
    pub fn to_uint32(&self) -> u32 {
        number_uint32(self.to_number())
    }

    /// ECMAScript `ToInt32`, interpreting the reduced bits as signed.
    #[must_use]
    pub fn to_int32(&self) -> i32 {
        i32::from_ne_bytes(self.to_uint32().to_ne_bytes())
    }

    #[expect(
        clippy::float_cmp,
        reason = "ECMA-262 7.2.13 requires exact Number equality, not a tolerance"
    )]
    pub(crate) fn loosely_equals(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Undefined) | (Self::Undefined, Self::Null) => true,
            (Self::Boolean(_), _) => Self::Number(self.to_number()).loosely_equals(other),
            (_, Self::Boolean(_)) => self.loosely_equals(&Self::Number(other.to_number())),
            (Self::String(_), Self::Number(_)) | (Self::Number(_), Self::String(_)) => {
                self.to_number() == other.to_number()
            }
            _ => self.strictly_equals(other),
        }
    }

    pub(crate) const fn type_name(&self) -> &'static str {
        match self {
            Self::Undefined => "undefined",
            Self::Null | Self::Object(_) => "object",
            Self::Boolean(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Function(_) => "function",
            Self::Symbol(_) => "symbol",
        }
    }

    pub(crate) fn units(&self) -> Rc<[u16]> {
        if let Self::String(units) = self {
            units.clone()
        } else {
            self.to_string().encode_utf16().collect()
        }
    }

    pub(crate) const fn heap_handle(&self) -> Option<crate::heap::Handle> {
        match self {
            Self::Symbol(symbol) => Some(symbol.handle),
            Self::Object(object) => Some(object.handle),
            Self::Function(FunctionValue(
                Callable::Script { handle, .. }
                | Callable::Host { handle, .. }
                | Callable::Resolver { handle, .. }
                | Callable::Bound { handle, .. },
            )) => Some(*handle),
            _ => None,
        }
    }

    pub(crate) fn retention_key(&self) -> Option<(crate::heap::Handle, bool)> {
        let handle = self.heap_handle()?;
        let reject = matches!(
            self,
            Self::Function(FunctionValue(Callable::Resolver { reject: true, .. }))
        );
        Some((handle, reject))
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undefined => f.write_str("undefined"),
            Self::Null => f.write_str("null"),
            Self::Boolean(value) => write!(f, "{value}"),
            Self::Number(value) if value.is_nan() => f.write_str("NaN"),
            Self::Number(value) if *value == f64::INFINITY => f.write_str("Infinity"),
            Self::Number(value) if *value == f64::NEG_INFINITY => f.write_str("-Infinity"),
            Self::Number(value) if *value == 0.0 => f.write_str("0"),
            Self::Number(value) => crate::number::decimal(*value, f),
            // Display is a host boundary. Internal operations never replace surrogates.
            Self::String(units) => f.write_str(&String::from_utf16_lossy(units)),
            Self::Symbol(symbol) => f.write_str(&String::from_utf16_lossy(&symbol.descriptive())),
            Self::Function(_) => f.write_str("function () { [jrs code] }"),
            Self::Object(_) => f.write_str("[object Object]"),
        }
    }
}

pub(crate) const fn whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\t' | '\u{b}' | '\u{c}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

pub(crate) const fn line_terminator(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// `ToIntegerOrInfinity` after numeric conversion, without a platform libm.
pub(crate) fn integer_or_infinity(n: f64) -> f64 {
    if n.is_nan() || n == 0.0 {
        return 0.0;
    }
    let exponent = i32::try_from((n.to_bits() >> 52) & 0x7ff)
        .unwrap_or(0)
        .saturating_sub(1023);
    if exponent < 0 {
        0.0
    } else if exponent >= 52 {
        n
    } else {
        let shift = u32::try_from(52i32.saturating_sub(exponent)).unwrap_or(0);
        f64::from_bits(n.to_bits() & !((1u64 << shift).saturating_sub(1)))
    }
}

fn string_number(text: &str) -> f64 {
    let text = text.trim_matches(|c| whitespace(c) || line_terminator(c));
    if text.is_empty() {
        return 0.0;
    }
    match text {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(digits) = text.strip_prefix(prefix) {
            return radix_number(digits, radix).unwrap_or(f64::NAN);
        }
    }
    // Rust also accepts `inf`; ECMAScript does not. A decimal contains digits,
    // signs, a decimal point and exponent markers only; parse checks their order.
    if !text
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E'))
    {
        return f64::NAN;
    }
    text.parse().unwrap_or(f64::NAN)
}

pub(crate) fn radix_number(digits: &str, radix: u32) -> Option<f64> {
    if digits.is_empty() {
        return None;
    }
    let mut integer = audhsos_math::RadixInteger::new(radix)?;
    for ch in digits.chars() {
        if !integer.push(ch.to_digit(radix)?) {
            return None;
        }
    }
    Some(integer.to_f64())
}

// ECMA-262 7.1.5, 7.1.7–9. Reading the binary64 significand computes
// truncation modulo 2^32 without a saturating float cast or a libm call.
fn number_uint32(number: f64) -> u32 {
    let bits = number.to_bits();
    let exponent = u32::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
    if !(1023..1107).contains(&exponent) {
        return 0;
    }
    let exponent = exponent.saturating_sub(1023);
    let mantissa = (bits & 0x000f_ffff_ffff_ffff) | (1u64 << 52);
    let truncated = if exponent < 52 {
        mantissa.wrapping_shr(52u32.saturating_sub(exponent))
    } else {
        mantissa.wrapping_shl(exponent.saturating_sub(52))
    };
    let value = u32::try_from(truncated & u64::from(u32::MAX)).unwrap_or(0);
    if number.is_sign_negative() {
        value.wrapping_neg()
    } else {
        value
    }
}
