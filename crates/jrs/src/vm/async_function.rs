// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Realm-local `AsyncFunction` constructor and ordinary prototype (ECMA-262 27.10).
use super::Execution;
use crate::{
    Error, Value, bytecode::Builtin, heap::HostBehavior, object::Property, parser::AsyncKind,
    value::FunctionValue,
};
impl Execution<'_> {
    pub(super) fn callable_prototype(&mut self, kind: AsyncKind) -> Result<Value, Error> {
        if kind == AsyncKind::Sync {
            return self.function_prototype();
        }
        if let Some(value) = &self.async_function_proto {
            return Ok(value.clone());
        }
        let parent = self.function_prototype()?;
        let prototype = self.allocate_object(parent)?;
        self.async_function_proto = Some(prototype.clone());
        let ctor =
            self.new_host_behavior(HostBehavior::AsyncFunctionConstructor, "AsyncFunction", 1)?;
        self.async_function_ctor = Some(ctor.clone());
        self.object_mut(&ctor)?.prototype =
            Value::Function(FunctionValue::native(Builtin::Function));
        self.define(
            &ctor,
            Value::string("prototype").units(),
            Property {
                value: prototype.clone(),
                writable: false,
                enumerable: false,
                configurable: false,
                accessor: None,
            },
        )?;
        self.define(
            &prototype,
            Value::string("constructor").units(),
            Property {
                value: ctor,
                writable: false,
                enumerable: false,
                configurable: true,
                accessor: None,
            },
        )?;
        self.install_tag(&prototype, "AsyncFunction")?;
        Ok(prototype)
    }
}
