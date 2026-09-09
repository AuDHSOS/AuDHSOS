// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! ECMAScript JSON hooks and object materialization over the isolated JSON codec.
use super::Execution;
use crate::{
    Error, Value,
    bytecode::Builtin,
    heap::HostBehavior,
    object::{Property, same_value},
};
use alloc::{collections::BTreeMap, rc::Rc, vec::Vec};
mod stringify;

impl Execution<'_> {
    pub(super) fn json_object(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.json {
            return Ok(value.clone());
        }
        let proto = self.prototype()?;
        let value = self.allocate_object(proto)?;
        self.json = Some(value.clone());
        self.install_tag(&value, "JSON")?;
        for (name, kind, length) in [
            ("parse", Builtin::JsonParse, 2),
            ("stringify", Builtin::JsonStringify, 3),
            ("rawJSON", Builtin::JsonRaw, 1),
            ("isRawJSON", Builtin::JsonIsRaw, 1),
        ] {
            let function = self.new_host_behavior(HostBehavior::Intrinsic(kind), name, length)?;
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
    pub(super) fn json_property(builtin: Builtin, key: &[u16]) -> Option<Property> {
        let (name, length) = match builtin {
            Builtin::JsonParse => ("parse", 2),
            Builtin::JsonStringify => ("stringify", 3),
            Builtin::JsonRaw => ("rawJSON", 1),
            Builtin::JsonIsRaw => ("isRawJSON", 1),
            _ => return None,
        };
        let value = if key == Value::string("name").units().as_ref() {
            Value::string(name)
        } else if key == Value::string("length").units().as_ref() {
            Value::Number(f64::from(length))
        } else {
            return None;
        };
        Some(Property {
            value,
            writable: false,
            enumerable: false,
            configurable: true,
            accessor: None,
        })
    }
    pub(super) fn json_call(
        &mut self,
        builtin: Builtin,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        if !matches!(
            builtin,
            Builtin::JsonParse | Builtin::JsonStringify | Builtin::JsonRaw | Builtin::JsonIsRaw
        ) {
            return Ok(None);
        }
        let roots = self.native_roots.len();
        self.native_roots.extend_from_slice(args);
        let first = args.first().unwrap_or(&Value::Undefined);
        let result = match builtin {
            Builtin::JsonStringify => self.json_stringify(
                first,
                args.get(1).unwrap_or(&Value::Undefined),
                args.get(2).unwrap_or(&Value::Undefined),
            ),
            Builtin::JsonParse => self.json_parse(first, args.get(1).unwrap_or(&Value::Undefined)),
            Builtin::JsonRaw => self.json_raw(first),
            _ => Ok(Value::Boolean(
                self.object_ref(first).is_ok_and(|o| o.raw_json.is_some()),
            )),
        };
        self.native_roots.truncate(roots);
        result.map(Some)
    }
    fn parse_document(&mut self, text: &[u16]) -> Result<audhsos_json::Document, Error> {
        let limits = audhsos_json::Limits {
            input: self.limits.string_units,
            nodes: self.limits.instructions,
            depth: 48,
            strings: self.limits.string_units,
        };
        audhsos_json::parse(text, limits, &mut self.fuel).map_err(json_error)
    }
    fn json_raw(&mut self, input: &Value) -> Result<Value, Error> {
        let text = self.string_units(input)?;
        let doc = self.parse_document(&text)?;
        let node = doc.nodes.get(doc.root).ok_or(Error::InvalidBytecode)?;
        if node.source.start != 0
            || node.source.end != text.len()
            || matches!(
                node.kind,
                audhsos_json::Kind::Array(_) | audhsos_json::Kind::Object(_)
            )
        {
            return Err(Error::Syntax {
                offset: 0,
                message: "rawJSON requires one primitive without surrounding whitespace",
            });
        }
        let object = self.allocate_object(Value::Null)?;
        self.define(
            &object,
            Value::string("rawJSON").units(),
            Property {
                value: Value::String(text.clone()),
                writable: false,
                enumerable: true,
                configurable: false,
                accessor: None,
            },
        )?;
        let data = self.object_mut(&object)?;
        data.raw_json = Some(text);
        data.extensible = false;
        Ok(object)
    }
    fn json_parse(&mut self, input: &Value, reviver: &Value) -> Result<Value, Error> {
        let text = self.string_units(input)?;
        let doc = self.parse_document(&text)?;
        let mut values = Vec::with_capacity(doc.nodes.len());
        for node in &doc.nodes {
            self.charge(1)?;
            let value = match &node.kind {
                audhsos_json::Kind::Null => Value::Null,
                audhsos_json::Kind::Boolean(v) => Value::Boolean(*v),
                audhsos_json::Kind::Number(v) => Value::Number(*v),
                audhsos_json::Kind::String(v) => Value::String(v.clone()),
                audhsos_json::Kind::Array(items) => {
                    let array =
                        self.new_array(u32::try_from(items.len()).map_err(|_| Error::Limit {
                            resource: "JSON array length",
                        })?)?;
                    for (i, child) in items.iter().enumerate() {
                        self.charge(1)?;
                        self.define(
                            &array,
                            index_key(i),
                            Property::data(
                                values.get(*child).cloned().ok_or(Error::InvalidBytecode)?,
                            ),
                        )?;
                    }
                    array
                }
                audhsos_json::Kind::Object(items) => {
                    let proto = self.prototype()?;
                    let object = self.allocate_object(proto)?;
                    for (key, child) in items {
                        self.charge(1)?;
                        self.define(
                            &object,
                            key.clone(),
                            Property::data(
                                values.get(*child).cloned().ok_or(Error::InvalidBytecode)?,
                            ),
                        )?;
                    }
                    object
                }
            };
            self.native_roots.push(value.clone());
            values.push(value);
        }
        let value = values
            .get(doc.root)
            .cloned()
            .ok_or(Error::InvalidBytecode)?;
        if !matches!(reviver, Value::Function(_)) {
            return Ok(value);
        }
        let proto = self.prototype()?;
        let holder = self.allocate_object(proto)?;
        self.native_roots.push(holder.clone());
        let name = Value::string("").units();
        self.define(&holder, name.clone(), Property::data(value))?;
        self.revive(
            &holder,
            &name,
            reviver,
            Some(doc.root),
            &ParseSnapshot {
                doc: &doc,
                values: &values,
                text: &text,
            },
            0,
        )
    }
    fn revive(
        &mut self,
        holder: &Value,
        name: &Rc<[u16]>,
        reviver: &Value,
        record: Option<usize>,
        snapshot: &ParseSnapshot<'_>,
        depth: usize,
    ) -> Result<Value, Error> {
        if self.json_depth >= 48 {
            return Err(Error::Limit {
                resource: "JSON reviver nesting",
            });
        }
        self.charge(1)?;
        self.json_depth = self.json_depth.saturating_add(1);
        let roots = self.native_roots.len();
        self.native_roots.push(holder.clone());
        let result = (|| {
            let value = self.get(holder, name)?;
            self.native_roots.push(value.clone());
            let proto = self.prototype()?;
            let context = self.allocate_object(proto)?;
            self.native_roots.push(context.clone());
            let record = record.filter(|i| {
                snapshot
                    .values
                    .get(*i)
                    .is_some_and(|v| same_value(v, &value))
            });
            if let Some(i) = record
                && !matches!(value, Value::Object(_) | Value::Function(_))
            {
                let range = snapshot
                    .doc
                    .nodes
                    .get(i)
                    .ok_or(Error::InvalidBytecode)?
                    .source
                    .clone();
                self.define(
                    &context,
                    Value::string("source").units(),
                    Property::data(Value::String(Rc::from(
                        snapshot.text.get(range).ok_or(Error::InvalidBytecode)?,
                    ))),
                )?;
            }
            if matches!(value, Value::Object(_) | Value::Function(_)) {
                let kind = record
                    .and_then(|i| snapshot.doc.nodes.get(i))
                    .map(|n| &n.kind);
                if self.is_array(&value) {
                    let length = self.array_length(&value)?;
                    for i in 0..length {
                        let index = usize::try_from(i).map_err(|_| Error::InvalidBytecode)?;
                        let child = if let Some(audhsos_json::Kind::Array(items)) = kind {
                            items.get(index).copied()
                        } else {
                            None
                        };
                        let key = index_key(index);
                        let new = self.revive(
                            &value,
                            &key,
                            reviver,
                            child,
                            snapshot,
                            depth.saturating_add(1),
                        )?;
                        self.revive_define(&value, key, new)?;
                    }
                } else {
                    let mut records = BTreeMap::new();
                    if let Some(audhsos_json::Kind::Object(items)) = kind {
                        for (key, child) in items {
                            records.insert(key.clone(), *child);
                        }
                    }
                    let keys = self.json_enumerable_keys(&value)?;
                    for key in keys {
                        let child = records.get(&key).copied();
                        let new = self.revive(
                            &value,
                            &key,
                            reviver,
                            child,
                            snapshot,
                            depth.saturating_add(1),
                        )?;
                        self.revive_define(&value, key, new)?;
                    }
                }
            }
            self.call_sync(
                reviver.clone(),
                holder.clone(),
                &[Value::String(name.clone()), value, context],
            )
        })();
        self.native_roots.truncate(roots);
        self.json_depth = self.json_depth.saturating_sub(1);
        result
    }
    fn revive_define(&mut self, object: &Value, key: Rc<[u16]>, value: Value) -> Result<(), Error> {
        if matches!(value, Value::Undefined) {
            self.object_mut(object)?.delete(&key);
        } else {
            let limit = self.limits.properties;
            self.object_mut(object)?
                .define(key, Property::data(value), limit)?;
        }
        Ok(())
    }
    fn json_enumerable_keys(&mut self, object: &Value) -> Result<Vec<Rc<[u16]>>, Error> {
        let mut keys = Vec::new();
        for key in self.own_keys(object)? {
            if self.own(object, &key)?.is_some_and(|p| p.enumerable) {
                keys.push(key);
            }
        }
        Ok(keys)
    }
}
struct ParseSnapshot<'a> {
    doc: &'a audhsos_json::Document,
    values: &'a [Value],
    text: &'a [u16],
}
fn index_key(index: usize) -> Rc<[u16]> {
    Value::string(&alloc::format!("{index}")).units()
}
const fn json_error(error: audhsos_json::Error) -> Error {
    match error {
        audhsos_json::Error::Syntax(_) => Error::Syntax {
            offset: 0,
            message: "invalid JSON text",
        },
        audhsos_json::Error::Limit => Error::Limit {
            resource: "JSON codec work or storage",
        },
    }
}
