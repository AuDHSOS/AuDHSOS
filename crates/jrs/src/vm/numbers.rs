// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Numeric text builtins with shared intrinsic identity and ordered coercions.
use super::Execution;
use crate::{Error, Value, bytecode::Builtin, heap::HostBehavior, object::Property};
use alloc::string::String;

impl Execution<'_> {
    pub(super) fn math_pow(&mut self, args: &[Value]) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots.extend_from_slice(args);
        let result = (|| {
            let base = self.numeric(args.first().unwrap_or(&Value::Undefined))?;
            let exponent = self.numeric(args.get(1).unwrap_or(&Value::Undefined))?;
            Ok(Value::Number(audhsos_math::pow(base, exponent)))
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn number_parser(&mut self, integer: bool) -> Result<Value, Error> {
        if self.number_parsers.is_none() {
            let roots = self.native_roots.len();
            let result = (|| {
                let int = self.new_host_behavior(
                    HostBehavior::Intrinsic(Builtin::ParseInt),
                    "parseInt",
                    2,
                )?;
                self.native_roots.push(int.clone());
                let float = self.new_host_behavior(
                    HostBehavior::Intrinsic(Builtin::ParseFloat),
                    "parseFloat",
                    1,
                )?;
                self.number_parsers = Some((int, float));
                Ok(())
            })();
            self.native_roots.truncate(roots);
            result?;
        }
        let (int, float) = self.number_parsers.as_ref().ok_or(Error::InvalidBytecode)?;
        Ok(if integer { int.clone() } else { float.clone() })
    }

    pub(super) fn number_constructor(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.number_constructor_storage {
            return Ok(value.clone());
        }
        let proto = self.function_prototype()?;
        let value = self.allocate_object(proto)?;
        self.number_constructor_storage = Some(value.clone());
        for (name, item, configurable) in [
            ("length", Value::Number(1.0), true),
            ("name", Value::string("Number"), true),
            (
                "prototype",
                self.primitive_prototype(Builtin::Number)?,
                false,
            ),
        ] {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    value: item,
                    writable: false,
                    enumerable: false,
                    configurable,
                    accessor: None,
                },
            )?;
        }
        for (name, constant) in super::boxing::NUMBER_CONSTANTS {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    value: Value::Number(*constant),
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    accessor: None,
                },
            )?;
        }
        for (name, integer) in [("parseInt", true), ("parseFloat", false)] {
            let function = self.number_parser(integer)?;
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(function)
                },
            )?;
        }
        for (name, kind) in [
            ("isFinite", Builtin::NumberIsFinite),
            ("isInteger", Builtin::NumberIsInteger),
            ("isNaN", Builtin::NumberIsNaN),
            ("isSafeInteger", Builtin::NumberIsSafeInteger),
        ] {
            let function = self.new_host_behavior(HostBehavior::Intrinsic(kind), name, 1)?;
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(function)
                },
            )?;
        }
        Ok(value)
    }

    pub(super) fn parse_integer(&mut self, input: &Value, radix: &Value) -> Result<Value, Error> {
        let units = self.string_units(input)?;
        let radix = Value::Number(self.numeric(radix)?).to_int32();
        if radix != 0 && !(2..=36).contains(&radix) {
            return Ok(Value::Number(f64::NAN));
        }
        self.charge(u64::try_from(units.len()).unwrap_or(u64::MAX))?;
        let mut text = units.as_ref();
        while text.first().is_some_and(|u| space(*u)) {
            text = text.get(1..).unwrap_or(&[]);
        }
        let negative = text.first() == Some(&45);
        if matches!(text.first(), Some(43 | 45)) {
            text = text.get(1..).unwrap_or(&[]);
        }
        let mut radix = u32::try_from(radix).unwrap_or(0);
        if (radix == 0 || radix == 16) && matches!(text.get(..2), Some([48, 88 | 120])) {
            text = text.get(2..).unwrap_or(&[]);
            radix = 16;
        }
        if radix == 0 {
            radix = 10;
        }
        let mut integer = audhsos_math::RadixInteger::new(radix).ok_or(Error::InvalidBytecode)?;
        let mut found = false;
        for unit in text {
            let Some(digit) = char::from_u32(u32::from(*unit)).and_then(|c| c.to_digit(radix))
            else {
                break;
            };
            integer.push(digit);
            found = true;
        }
        let n = if found { integer.to_f64() } else { f64::NAN };
        Ok(Value::Number(if negative { -n } else { n }))
    }

    pub(super) fn parse_float(&mut self, input: &Value) -> Result<Value, Error> {
        let units = self.string_units(input)?;
        self.charge(u64::try_from(units.len()).unwrap_or(u64::MAX))?;
        let mut text = units.as_ref();
        while text.first().is_some_and(|u| space(*u)) {
            text = text.get(1..).unwrap_or(&[]);
        }
        let mut end = usize::from(matches!(text.first(), Some(43 | 45)));
        let infinity = [73, 110, 102, 105, 110, 105, 116, 121];
        if text.get(end..).is_some_and(|t| t.starts_with(&infinity)) {
            return Ok(Value::Number(if text.first() == Some(&45) {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }));
        }
        let start = end;
        while text.get(end).is_some_and(|u| digit(*u)) {
            end = end.saturating_add(1);
        }
        let mut digits = end.saturating_sub(start);
        if text.get(end) == Some(&46) {
            end = end.saturating_add(1);
            let start = end;
            while text.get(end).is_some_and(|u| digit(*u)) {
                end = end.saturating_add(1);
            }
            digits = digits.saturating_add(end.saturating_sub(start));
        }
        if digits == 0 {
            return Ok(Value::Number(f64::NAN));
        }
        if matches!(text.get(end), Some(69 | 101)) {
            let mark = end;
            end = end.saturating_add(1);
            if matches!(text.get(end), Some(43 | 45)) {
                end = end.saturating_add(1);
            }
            let start = end;
            while text.get(end).is_some_and(|u| digit(*u)) {
                end = end.saturating_add(1);
            }
            if start == end {
                end = mark;
            }
        }
        let ascii: String = text
            .get(..end)
            .unwrap_or(&[])
            .iter()
            .map(|u| char::from(u8::try_from(*u).unwrap_or(0)))
            .collect();
        Ok(Value::Number(ascii.parse().unwrap_or(f64::NAN)))
    }
}
fn space(unit: u16) -> bool {
    char::from_u32(u32::from(unit))
        .is_some_and(|c| crate::value::whitespace(c) || crate::value::line_terminator(c))
}
fn digit(unit: u16) -> bool {
    (48..=57).contains(&unit)
}

/// Number predicates never coerce: wrappers, Symbols and objects are false.
pub(super) fn predicate(kind: Builtin, value: &Value) -> Value {
    let Value::Number(n) = value else {
        return Value::Boolean(false);
    };
    let result = match kind {
        Builtin::NumberIsFinite => n.is_finite(),
        Builtin::NumberIsNaN => n.is_nan(),
        Builtin::NumberIsInteger => integral(*n),
        Builtin::NumberIsSafeInteger => integral(*n) && n.abs() <= 9_007_199_254_740_991.0,
        _ => false,
    };
    Value::Boolean(result)
}

fn integral(n: f64) -> bool {
    let bits = n.to_bits() & 0x7fff_ffff_ffff_ffff;
    if bits == 0 {
        return true;
    }
    let exponent = (bits >> 52) & 0x7ff;
    if !(1023..2047).contains(&exponent) {
        return false;
    }
    if exponent >= 1075 {
        return true;
    }
    let fractional = 1075u64.saturating_sub(exponent);
    bits & ((1u64 << fractional).saturating_sub(1)) == 0
}

#[cfg(test)]
#[path = "../tests/number_predicates.rs"]
mod tests;
