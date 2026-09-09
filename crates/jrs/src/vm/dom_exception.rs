// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Web IDL `DOMException` brands, readonly accessors and legacy code mapping.
use super::Execution;
use crate::{
    Error, Value,
    bytecode::Builtin,
    event::{DomException, ExceptionField},
    heap::HostBehavior,
    object::Property,
    value::FunctionValue,
};

// Empty names denote historical constants that are not current exception names.
const LEGACY: &[(&str, &str)] = &[
    ("IndexSizeError", "INDEX_SIZE_ERR"),
    ("", "DOMSTRING_SIZE_ERR"),
    ("HierarchyRequestError", "HIERARCHY_REQUEST_ERR"),
    ("WrongDocumentError", "WRONG_DOCUMENT_ERR"),
    ("InvalidCharacterError", "INVALID_CHARACTER_ERR"),
    ("", "NO_DATA_ALLOWED_ERR"),
    ("NoModificationAllowedError", "NO_MODIFICATION_ALLOWED_ERR"),
    ("NotFoundError", "NOT_FOUND_ERR"),
    ("NotSupportedError", "NOT_SUPPORTED_ERR"),
    ("InUseAttributeError", "INUSE_ATTRIBUTE_ERR"),
    ("InvalidStateError", "INVALID_STATE_ERR"),
    ("SyntaxError", "SYNTAX_ERR"),
    ("InvalidModificationError", "INVALID_MODIFICATION_ERR"),
    ("NamespaceError", "NAMESPACE_ERR"),
    ("InvalidAccessError", "INVALID_ACCESS_ERR"),
    ("", "VALIDATION_ERR"),
    ("TypeMismatchError", "TYPE_MISMATCH_ERR"),
    ("SecurityError", "SECURITY_ERR"),
    ("NetworkError", "NETWORK_ERR"),
    ("AbortError", "ABORT_ERR"),
    ("URLMismatchError", "URL_MISMATCH_ERR"),
    ("QuotaExceededError", "QUOTA_EXCEEDED_ERR"),
    ("TimeoutError", "TIMEOUT_ERR"),
    ("InvalidNodeTypeError", "INVALID_NODE_TYPE_ERR"),
    ("DataCloneError", "DATA_CLONE_ERR"),
];
impl Execution<'_> {
    fn dom_exception_prototype(&mut self) -> Result<Value, Error> {
        if let Some(v) = &self.dom_exception_proto {
            return Ok(v.clone());
        }
        let parent = self.error_prototype(Builtin::Error)?;
        let object = self.allocate_object(parent)?;
        self.dom_exception_proto = Some(object.clone());
        self.object_mut(&object)?.error = true;
        self.install_tag(&object, "DOMException")?;
        self.define(
            &object,
            Value::string("constructor").units(),
            Property {
                enumerable: false,
                ..Property::data(Value::Function(FunctionValue::native(
                    Builtin::DomException,
                )))
            },
        )?;
        for (name, field) in [
            ("name", ExceptionField::Name),
            ("message", ExceptionField::Message),
            ("code", ExceptionField::Code),
        ] {
            let get = self.new_host_behavior(
                HostBehavior::DomExceptionGet(field),
                &alloc::format!("get {name}"),
                0,
            )?;
            self.define(
                &object,
                Value::string(name).units(),
                Property {
                    value: Value::Undefined,
                    writable: false,
                    enumerable: true,
                    configurable: true,
                    accessor: Some((get, Value::Undefined)),
                },
            )?;
        }
        for (index, (_, name)) in LEGACY.iter().enumerate() {
            self.define(
                &object,
                Value::string(name).units(),
                constant(number(index.saturating_add(1))),
            )?;
        }
        Ok(object)
    }
    pub(super) fn dom_exception_property(
        &mut self,
        key: &[u16],
    ) -> Result<Option<Property>, Error> {
        if key == Value::string("prototype").units().as_ref() {
            return Ok(Some(Property {
                enumerable: false,
                ..constant(self.dom_exception_prototype()?)
            }));
        }
        for (index, (_, name)) in LEGACY.iter().enumerate() {
            if key == Value::string(name).units().as_ref() {
                return Ok(Some(constant(number(index.saturating_add(1)))));
            }
        }
        let value = if key == Value::string("name").units().as_ref() {
            Value::string("DOMException")
        } else if key == Value::string("length").units().as_ref() {
            Value::Number(0.0)
        } else {
            return Ok(None);
        };
        Ok(Some(Property {
            value,
            writable: false,
            enumerable: false,
            configurable: true,
            accessor: None,
        }))
    }
    pub(super) fn construct_dom_exception(
        &mut self,
        args: &[Value],
        target: &Value,
    ) -> Result<Value, Error> {
        let message = if let Some(v) = args.first().filter(|v| !matches!(v, Value::Undefined)) {
            self.string_units(v)?
        } else {
            Value::string("").units()
        };
        let name = if let Some(v) = args.get(1).filter(|v| !matches!(v, Value::Undefined)) {
            self.string_units(v)?
        } else {
            Value::string("Error").units()
        };
        let default = self.dom_exception_prototype()?;
        let proto = self.constructor_prototype(target, default)?;
        let object = self.allocate_object(proto)?;
        self.object_mut(&object)?.dom_exception = Some(DomException { name, message });
        self.object_mut(&object)?.error = true;
        Ok(object)
    }
    pub(super) fn invalid_event_state(&mut self) -> Result<Error, Error> {
        let ctor = Value::Function(FunctionValue::native(Builtin::DomException));
        let value = self.construct_dom_exception(
            &[
                Value::string("Event is already dispatching"),
                Value::string("InvalidStateError"),
            ],
            &ctor,
        )?;
        Ok(Error::Thrown { value })
    }
    pub(super) fn dom_exception_get(
        &self,
        value: &Value,
        field: ExceptionField,
    ) -> Result<Value, Error> {
        let data = self
            .object_ref(value)?
            .dom_exception
            .as_ref()
            .ok_or(Error::Type {
                message: "DOMException receiver required",
            })?;
        Ok(match field {
            ExceptionField::Name => Value::String(data.name.clone()),
            ExceptionField::Message => Value::String(data.message.clone()),
            ExceptionField::Code => number(
                LEGACY
                    .iter()
                    .position(|(name, _)| {
                        !name.is_empty() && Value::string(name).units() == data.name
                    })
                    .map_or(0, |n| n.saturating_add(1)),
            ),
        })
    }
}
fn number(n: usize) -> Value {
    Value::Number(f64::from(u32::try_from(n).unwrap_or(u32::MAX)))
}
const fn constant(value: Value) -> Property {
    Property {
        value,
        writable: false,
        enumerable: true,
        configurable: false,
        accessor: None,
    }
}
