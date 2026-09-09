// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Normal dynamic Function construction, ECMA-262 20.2.1.1.1. Parameter and
//! body grammars are parsed independently; no wrapper-source execution occurs.
use super::Execution;
use crate::{
    Error, Value,
    heap::Node,
    object::{Object, Property},
    value::{Callable, FunctionValue},
};
use alloc::{rc::Rc, string::String, vec::Vec};
impl Execution<'_> {
    pub(super) fn dynamic_function(
        &mut self,
        args: &[Value],
        new_target: &Value,
    ) -> Result<Value, Error> {
        self.dynamic_function_kind(args, new_target, crate::parser::AsyncKind::Sync)
    }
    pub(super) fn dynamic_function_kind(
        &mut self,
        args: &[Value],
        new_target: &Value,
        kind: crate::parser::AsyncKind,
    ) -> Result<Value, Error> {
        if self.native_depth >= 12 {
            return Err(Error::Limit {
                resource: "dynamic Function reentry",
            });
        }
        self.native_depth = self.native_depth.saturating_add(1);
        let roots = self.native_roots.len();
        self.native_roots.extend_from_slice(args);
        self.native_roots.push(new_target.clone());
        let result = (|| {
            self.initialize_globals()?;
            let mut parameters = Vec::new();
            let count = args.len().saturating_sub(1);
            for (index, arg) in args.iter().take(count).enumerate() {
                let units = self.string_units(arg)?;
                if index > 0 {
                    self.dynamic_append(&mut parameters, &[44])?;
                }
                self.dynamic_append(&mut parameters, &units)?;
            }
            let body = if let Some(body) = args.last() {
                self.string_units(body)?
            } else {
                Rc::from([])
            };
            let policy = self
                .host
                .ensure_can_compile_strings()
                .map(|()| Value::Undefined);
            self.validate_host_result(&policy)?;
            policy?;
            let mut source = Vec::new();
            self.dynamic_append(
                &mut source,
                &Value::string(if kind == crate::parser::AsyncKind::Async {
                    "async function anonymous("
                } else {
                    "function anonymous("
                })
                .units(),
            )?;
            self.dynamic_append(&mut source, &parameters)?;
            self.dynamic_append(&mut source, &Value::string("\n) {\n").units())?;
            self.dynamic_append(&mut source, &body)?;
            self.dynamic_append(&mut source, &Value::string("\n}").units())?;
            let params = utf8(&parameters)?;
            let body = alloc::format!("\n{}\n", utf8(&body)?);
            let source_bytes = utf8(&source)?.len();
            if source_bytes > self.limits.source_bytes {
                return Err(Error::Limit {
                    resource: "dynamic Function source bytes",
                });
            }
            self.charge(u64::try_from(source_bytes).unwrap_or(u64::MAX))?;
            let code = crate::bytecode::compile_dynamic(&params, &body, self.limits, kind)?;
            self.charge(u64::try_from(code.program.instruction_count()).unwrap_or(u64::MAX))?;
            self.allocate_dynamic_function(code, source, new_target, kind)
        })();
        self.native_roots.truncate(roots);
        self.native_depth = self.native_depth.saturating_sub(1);
        result
    }
    fn allocate_dynamic_function(
        &mut self,
        code: Rc<crate::bytecode::FunctionCode>,
        source: Vec<u16>,
        new_target: &Value,
        kind: crate::parser::AsyncKind,
    ) -> Result<Value, Error> {
        if !code.captures.is_empty() {
            return Err(Error::InvalidBytecode);
        }
        let default = self.callable_prototype(kind)?;
        let prototype = self.constructor_prototype(new_target, default)?;
        self.native_roots.push(prototype.clone());
        self.reserve()?;
        let mut object = Object::new(prototype);
        object.dynamic_source = Some(source.into());
        let length = Value::Number(f64::from(
            u32::try_from(code.length).map_err(|_| Error::InvalidBytecode)?,
        ));
        let handle = self.heap.allocate(
            Node::Function {
                code,
                captures: Vec::new(),
                lexical_this: None,
                object,
            },
            self.limits.heap_entries,
        )?;
        let value = Value::Function(FunctionValue(Callable::Script {
            handle,
            owner: self.owner.clone(),
        }));
        self.native_roots.push(value.clone());
        for (name, property_value) in [("length", length), ("name", Value::string("anonymous"))] {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    value: property_value,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    accessor: None,
                },
            )?;
        }
        self.ensure_function_prototype(&value)?;
        Ok(value)
    }
    fn dynamic_append(&mut self, out: &mut Vec<u16>, units: &[u16]) -> Result<(), Error> {
        if out
            .len()
            .checked_add(units.len())
            .is_none_or(|n| n > self.limits.string_units)
        {
            return Err(Error::Limit {
                resource: "dynamic Function source units",
            });
        }
        self.charge(u64::try_from(units.len()).unwrap_or(u64::MAX))?;
        out.extend_from_slice(units);
        Ok(())
    }
}
fn utf8(units: &[u16]) -> Result<String, Error> {
    String::from_utf16(units).map_err(|_| Error::Unsupported {
        feature: "lone-surrogate dynamic Function source",
    })
}
