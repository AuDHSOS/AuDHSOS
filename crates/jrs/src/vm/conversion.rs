// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `OrdinaryToPrimitive` and expression conversions execute user hooks in order.
//! Both operands remain rooted while either conversion can run JavaScript.

use super::{Execution, binary, unary};
use crate::{
    Error, Value,
    parser::{Binary, Unary},
};
use alloc::rc::Rc;

impl Execution<'_> {
    pub(super) fn math_extreme(
        &mut self,
        builtin: crate::bytecode::Builtin,
        args: &[Value],
    ) -> Result<Value, Error> {
        let max = builtin == crate::bytecode::Builtin::MathMax;
        let mut result = if max {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
        let mut nan = false;
        for value in args {
            self.charge(1)?;
            let number = self.numeric(value)?;
            nan |= number.is_nan();
            if (max && number > result)
                || (!max && number < result)
                || (number == 0.0 && result == 0.0 && number.is_sign_negative() != max)
            {
                result = number;
            }
        }
        Ok(Value::Number(if nan { f64::NAN } else { result }))
    }
    pub(super) fn primitive(&mut self, value: &Value, string_hint: bool) -> Result<Value, Error> {
        self.primitive_hint(value, if string_hint { "string" } else { "number" })
    }
    fn primitive_hint(&mut self, value: &Value, hint: &str) -> Result<Value, Error> {
        if !matches!(value, Value::Object(_) | Value::Function(_)) {
            return Ok(value.clone());
        }
        let roots = self.native_roots.len();
        self.native_roots.push(value.clone());
        let result = (|| {
            let key = Value::Symbol(self.well_known("toPrimitive")?);
            let exotic = self.get_key(value, &key)?;
            if !matches!(exotic, Value::Null | Value::Undefined) {
                if !matches!(exotic, Value::Function(_)) {
                    return Err(Error::Type {
                        message: "Symbol.toPrimitive is not callable",
                    });
                }
                let result = self.call_sync(exotic, value.clone(), &[Value::string(hint)])?;
                if matches!(result, Value::Object(_) | Value::Function(_)) {
                    return Err(Error::Type {
                        message: "Symbol.toPrimitive returned an object",
                    });
                }
                return Ok(result);
            }
            let names = if hint == "string" {
                ["toString", "valueOf"]
            } else {
                ["valueOf", "toString"]
            };
            for name in names {
                self.charge(1)?;
                let method = self.get(value, &Value::string(name).units())?;
                if matches!(method, Value::Function(_)) {
                    let result = self.call_sync(method, value.clone(), &[])?;
                    if !matches!(result, Value::Object(_) | Value::Function(_)) {
                        return Ok(result);
                    }
                }
            }
            Err(Error::Type {
                message: "cannot convert object to primitive",
            })
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn numeric(&mut self, value: &Value) -> Result<f64, Error> {
        let value = self.primitive(value, false)?;
        if matches!(value, Value::Symbol(_)) {
            return Err(Error::Type {
                message: "cannot convert Symbol to Number",
            });
        }
        Ok(value.to_number())
    }
    pub(super) fn string_units(&mut self, value: &Value) -> Result<Rc<[u16]>, Error> {
        let value = self.primitive(value, true)?;
        if matches!(value, Value::Symbol(_)) {
            return Err(Error::Type {
                message: "cannot convert Symbol to String",
            });
        }
        Ok(value.units())
    }
    pub(super) fn convert_unary(&mut self, op: Unary, value: Value) -> Result<Value, Error> {
        let value = if matches!(op, Unary::Plus | Unary::Minus | Unary::BitNot) {
            Value::Number(self.numeric(&value)?)
        } else {
            value
        };
        Ok(unary(op, &value))
    }
    pub(super) fn convert_binary(
        &mut self,
        op: Binary,
        left: &Value,
        right: &Value,
    ) -> Result<Value, Error> {
        // Primitive arithmetic cannot reenter JavaScript or collect. Keep the
        // numeric hot path free of root-vector writes and duplicate clones.
        if !matches!(op, Binary::In | Binary::InstanceOf)
            && !matches!(left, Value::Object(_) | Value::Function(_))
            && !matches!(right, Value::Object(_) | Value::Function(_))
        {
            return binary(op, left, right, self.limits);
        }
        let roots = self.native_roots.len();
        self.native_roots.extend([left.clone(), right.clone()]);
        let result = self.binary_rooted(op, left, right);
        self.native_roots.truncate(roots);
        result
    }
    fn binary_rooted(&mut self, op: Binary, left: &Value, right: &Value) -> Result<Value, Error> {
        if matches!(
            op,
            Binary::Pow
                | Binary::Sub
                | Binary::Mul
                | Binary::Div
                | Binary::Rem
                | Binary::BitAnd
                | Binary::BitOr
                | Binary::BitXor
                | Binary::Shl
                | Binary::Shr
                | Binary::Ushr
        ) {
            let left = Value::Number(self.numeric(left)?);
            let right = Value::Number(self.numeric(right)?);
            return binary(op, &left, &right, self.limits);
        }
        if matches!(op, Binary::InstanceOf) {
            return Ok(Value::Boolean(self.has_instance(right, left, true)?));
        }
        if matches!(op, Binary::In) {
            if !matches!(right, Value::Object(_) | Value::Function(_)) {
                return Err(Error::Type {
                    message: "in requires an object",
                });
            }
            let key = self.property_key(left)?;
            return Ok(Value::Boolean(self.has_key(right, &key)?));
        }
        let objects = |v: &Value| matches!(v, Value::Object(_) | Value::Function(_));
        if matches!(op, Binary::StrictEq | Binary::StrictNe) {
            return binary(op, left, right, self.limits);
        }
        if matches!(op, Binary::Eq | Binary::Ne)
            && (objects(left) == objects(right)
                || matches!(left, Value::Null | Value::Undefined)
                || matches!(right, Value::Null | Value::Undefined))
        {
            return binary(op, left, right, self.limits);
        }
        let hint = if matches!(op, Binary::Add | Binary::Eq | Binary::Ne) {
            "default"
        } else {
            "number"
        };
        let left = self.primitive_hint(left, hint)?;
        self.native_roots.push(left.clone());
        let right = self.primitive_hint(right, hint)?;
        binary(op, &left, &right, self.limits)
    }

    pub(super) fn array_length_value(&mut self, value: &Value) -> Result<Value, Error> {
        let a = Value::Number(self.numeric(value)?).to_uint32();
        let b = Value::Number(self.numeric(value)?);
        if !b.strictly_equals(&Value::Number(f64::from(a))) {
            return Err(Error::Range {
                message: "invalid array length",
            });
        }
        Ok(Value::Number(f64::from(a)))
    }
}
