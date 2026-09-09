// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! DOM abort algorithms and HTML onabort handler, sharing real event dispatch.
use super::Execution;
use crate::{
    Error, Value,
    bytecode::Builtin,
    event::{AbortField, AbortSignal},
    heap::{Handle, HostBehavior, Node},
    object::Property,
    value::{FunctionValue, ObjectValue},
};
use alloc::{collections::BTreeSet, vec::Vec};
use audhsos_event_target::{Listeners, Options};

impl Execution<'_> {
    fn abort_prototype(&mut self, signal: bool) -> Result<Value, Error> {
        if let Some(v) = if signal {
            &self.abort_signal_proto
        } else {
            &self.abort_controller_proto
        } {
            return Ok(v.clone());
        }
        let parent = if signal {
            self.event_prototype(true)?
        } else {
            self.prototype()?
        };
        let object = self.allocate_object(parent)?;
        if signal {
            self.abort_signal_proto = Some(object.clone());
        } else {
            self.abort_controller_proto = Some(object.clone());
        }
        self.install_tag(
            &object,
            if signal {
                "AbortSignal"
            } else {
                "AbortController"
            },
        )?;
        self.define(
            &object,
            Value::string("constructor").units(),
            Property {
                enumerable: false,
                ..Property::data(native(if signal {
                    Builtin::AbortSignal
                } else {
                    Builtin::AbortController
                }))
            },
        )?;
        let fields: &[(&str, AbortField)] = if signal {
            &[
                ("aborted", AbortField::Aborted),
                ("reason", AbortField::Reason),
                ("onabort", AbortField::Handler),
            ]
        } else {
            &[("signal", AbortField::Signal)]
        };
        for (name, field) in fields {
            let get = self.new_host_behavior(
                HostBehavior::AbortGet(*field),
                &alloc::format!("get {name}"),
                0,
            )?;
            self.native_roots.push(get.clone());
            let set = if matches!(field, AbortField::Handler) {
                self.new_host_behavior(HostBehavior::AbortSet, "set onabort", 1)?
            } else {
                Value::Undefined
            };
            self.define(
                &object,
                Value::string(name).units(),
                Property {
                    value: Value::Undefined,
                    writable: false,
                    enumerable: true,
                    configurable: true,
                    accessor: Some((get, set)),
                },
            )?;
        }
        self.define(
            &object,
            Value::string(if signal { "throwIfAborted" } else { "abort" }).units(),
            Property::data(native(if signal {
                Builtin::SignalThrow
            } else {
                Builtin::Abort
            })),
        )?;
        Ok(object)
    }
    pub(super) fn abort_property(
        &mut self,
        kind: Builtin,
        key: &[u16],
    ) -> Result<Option<Property>, Error> {
        let (name, length) = match kind {
            Builtin::AbortController => ("AbortController", 0),
            Builtin::AbortSignal => ("AbortSignal", 0),
            Builtin::Abort | Builtin::SignalAbort => ("abort", 0),
            Builtin::SignalAny => ("any", 1),
            Builtin::SignalThrow => ("throwIfAborted", 0),
            _ => return Ok(None),
        };
        let property = if key == Value::string("prototype").units().as_ref()
            && matches!(kind, Builtin::AbortController | Builtin::AbortSignal)
        {
            Property {
                value: self.abort_prototype(kind == Builtin::AbortSignal)?,
                writable: false,
                enumerable: false,
                configurable: false,
                accessor: None,
            }
        } else if kind == Builtin::AbortSignal && key == Value::string("abort").units().as_ref() {
            Property::data(native(Builtin::SignalAbort))
        } else if kind == Builtin::AbortSignal && key == Value::string("any").units().as_ref() {
            Property::data(native(Builtin::SignalAny))
        } else if kind == Builtin::AbortSignal
            && key == Value::string("timeout").units().as_ref()
            && let Some(function) = &self.timers.timeout_function
        {
            Property::data(function.clone())
        } else if key == Value::string("name").units().as_ref()
            || key == Value::string("length").units().as_ref()
        {
            Property {
                value: if key == Value::string("name").units().as_ref() {
                    Value::string(name)
                } else {
                    Value::Number(f64::from(length))
                },
                writable: false,
                enumerable: false,
                configurable: true,
                accessor: None,
            }
        } else {
            return Ok(None);
        };
        Ok(Some(property))
    }
    pub(super) fn new_signal(&mut self) -> Result<Value, Error> {
        let proto = self.abort_prototype(true)?;
        let object = self.allocate_object(proto)?;
        let data = self.object_mut(&object)?;
        data.listeners = Some(Listeners::new());
        data.abort_signal = Some(AbortSignal::new());
        Ok(object)
    }
    pub(super) fn construct_abort(
        &mut self,
        kind: Builtin,
        target: &Value,
    ) -> Result<Value, Error> {
        if kind != Builtin::AbortController {
            return Err(Error::Type {
                message: "AbortSignal is not constructible",
            });
        }
        let default = self.abort_prototype(false)?;
        let proto = self.constructor_prototype(target, default)?;
        self.native_roots.push(proto.clone());
        let signal = self.new_signal()?;
        self.native_roots.push(signal.clone());
        let controller = self.allocate_object(proto)?;
        self.object_mut(&controller)?.controller_signal = Some(signal);
        Ok(controller)
    }
    pub(super) fn signal_ref(&self, value: &Value) -> Result<&AbortSignal, Error> {
        self.object_ref(value)?
            .abort_signal
            .as_ref()
            .ok_or(Error::Type {
                message: "AbortSignal required",
            })
    }
    fn signal_mut(&mut self, value: &Value) -> Result<&mut AbortSignal, Error> {
        self.object_mut(value)?
            .abort_signal
            .as_mut()
            .ok_or(Error::Type {
                message: "AbortSignal required",
            })
    }
    pub(super) fn abort_get(&self, value: &Value, field: AbortField) -> Result<Value, Error> {
        if matches!(field, AbortField::Signal) {
            return self
                .object_ref(value)?
                .controller_signal
                .clone()
                .ok_or(Error::Type {
                    message: "AbortController required",
                });
        }
        let signal = self.signal_ref(value)?;
        Ok(match field {
            AbortField::Aborted => Value::Boolean(signal.aborted()),
            AbortField::Reason => signal.reason.clone(),
            AbortField::Handler => signal.handler.clone(),
            AbortField::Signal => return Err(Error::InvalidBytecode),
        })
    }
    pub(super) fn abort_call(
        &mut self,
        kind: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        if !matches!(
            kind,
            Builtin::AbortController
                | Builtin::AbortSignal
                | Builtin::Abort
                | Builtin::SignalAbort
                | Builtin::SignalAny
                | Builtin::SignalThrow
                | Builtin::AbortHandler
        ) {
            return Ok(None);
        }
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        self.native_roots.extend_from_slice(args);
        let result = (|| match kind {
            Builtin::AbortController | Builtin::AbortSignal => Err(Error::Type {
                message: "abort interface requires new",
            }),
            Builtin::Abort => {
                let signal = self.abort_get(receiver, AbortField::Signal)?;
                self.native_roots.push(signal.clone());
                self.signal_abort(&signal, args.first().unwrap_or(&Value::Undefined))?;
                Ok(Value::Undefined)
            }
            Builtin::SignalAbort => {
                let reason = self.abort_reason(args.first().unwrap_or(&Value::Undefined))?;
                self.native_roots.push(reason.clone());
                let signal = self.new_signal()?;
                self.signal_mut(&signal)?.reason = reason;
                Ok(signal)
            }
            Builtin::SignalAny => self.signal_any(args.first().ok_or(Error::Type {
                message: "AbortSignal.any requires a sequence",
            })?),
            Builtin::SignalThrow => {
                let signal = self.signal_ref(receiver)?;
                if signal.aborted() {
                    Err(Error::Thrown {
                        value: signal.reason.clone(),
                    })
                } else {
                    Ok(Value::Undefined)
                }
            }
            Builtin::AbortHandler => {
                let callback = self.signal_ref(receiver)?.handler.clone();
                if matches!(callback, Value::Function(_)) {
                    let result = self.call_sync(callback, receiver.clone(), args)?;
                    if result == Value::Boolean(false) {
                        self.event_set(
                            args.first().ok_or(Error::InvalidBytecode)?,
                            crate::event::Field::ReturnValue,
                            &Value::Boolean(false),
                        )?;
                    }
                }
                Ok(Value::Undefined)
            }
            _ => Err(Error::InvalidBytecode),
        })();
        self.native_roots.truncate(roots);
        result.map(Some)
    }
    fn abort_reason(&mut self, reason: &Value) -> Result<Value, Error> {
        if !matches!(reason, Value::Undefined) {
            return Ok(reason.clone());
        }
        self.construct_dom_exception(
            &[
                Value::string("The operation was aborted"),
                Value::string("AbortError"),
            ],
            &native(Builtin::DomException),
        )
    }
    pub(super) fn abort_handler_set(&mut self, object: &Value, value: &Value) -> Result<(), Error> {
        let id = self.signal_ref(object)?.handler_id;
        let value = if matches!(value, Value::Object(_) | Value::Function(_)) {
            value.clone()
        } else {
            Value::Null
        };
        if matches!(value, Value::Null) {
            if let Some(id) = id {
                self.remove_listener_id(object, id)?;
            }
            let signal = self.signal_mut(object)?;
            signal.handler = Value::Null;
            signal.handler_id = None;
        } else {
            if id.is_none() {
                let cap = self.limits.properties;
                let units = self.limits.string_units;
                let listeners = self
                    .object_mut(object)?
                    .listeners
                    .as_mut()
                    .ok_or(Error::InvalidBytecode)?;
                let id = listeners.next_id();
                listeners
                    .add(
                        Value::string("abort").units(),
                        native(Builtin::AbortHandler),
                        Options::default(),
                        cap,
                        units,
                    )
                    .map_err(|_| Error::Limit {
                        resource: "event listeners",
                    })?;
                self.signal_mut(object)?.handler_id = Some(id);
            }
            self.signal_mut(object)?.handler = value;
        }
        Ok(())
    }
    pub(super) fn attach_listener_signal(
        &mut self,
        signal: &Value,
        target: &Value,
        id: u64,
    ) -> Result<(), Error> {
        if self.signal_ref(signal)?.removals.len() >= self.limits.properties {
            return Err(Error::Limit {
                resource: "abort algorithms",
            });
        }
        self.signal_mut(signal)?.removals.push((target.clone(), id));
        self.object_mut(target)?
            .listener_signals
            .insert(id, signal.clone());
        Ok(())
    }
    pub(super) fn remove_listener_id(&mut self, target: &Value, id: u64) -> Result<(), Error> {
        self.object_mut(target)?
            .listeners
            .as_mut()
            .ok_or(Error::InvalidBytecode)?
            .remove_id(id);
        if let Some(signal) = self.object_mut(target)?.listener_signals.remove(&id) {
            self.charge(
                u64::try_from(self.signal_ref(&signal)?.removals.len()).unwrap_or(u64::MAX),
            )?;
            self.signal_mut(&signal)?
                .removals
                .retain(|(t, i)| t != target || *i != id);
        }
        Ok(())
    }
    pub(super) fn signal_abort(&mut self, signal: &Value, reason: &Value) -> Result<(), Error> {
        if self.signal_ref(signal)?.aborted() {
            return Ok(());
        }
        let reason = self.abort_reason(reason)?;
        self.native_roots.push(reason.clone());
        self.signal_mut(signal)?.reason = reason.clone();
        let dependents = self.signal_ref(signal)?.dependents.clone();
        let mut pending = Vec::new();
        for handle in dependents {
            self.charge(1)?;
            if matches!(self.heap.get(handle),Ok(Node::Object(object)) if object.abort_signal.as_ref().is_some_and(|s|!s.aborted()))
            {
                let value = self.signal_value(handle);
                self.signal_mut(&value)?.reason = reason.clone();
                self.native_roots.push(value.clone());
                pending.push(value);
            }
        }
        self.run_abort_steps(signal)?;
        for signal in pending {
            self.run_abort_steps(&signal)?;
        }
        Ok(())
    }
    fn run_abort_steps(&mut self, signal: &Value) -> Result<(), Error> {
        // No user code runs while removing listeners. All reasons were set first.
        let removals = core::mem::take(&mut self.signal_mut(signal)?.removals);
        for (target, id) in removals {
            self.charge(1)?;
            self.remove_listener_id(&target, id)?;
        }
        let event = self.construct_event(
            Builtin::Event,
            &[Value::string("abort")],
            &native(Builtin::Event),
        )?;
        self.native_roots.push(event.clone());
        self.object_mut(&event)?
            .event
            .as_mut()
            .ok_or(Error::InvalidBytecode)?
            .trusted = true;
        self.dispatch_trusted_event(signal, &event)?;
        Ok(())
    }
    fn signal_any(&mut self, input: &Value) -> Result<Value, Error> {
        if !matches!(input, Value::Object(_) | Value::Function(_)) {
            return Err(Error::Type {
                message: "AbortSignal sequence must be an object",
            });
        }
        let mut iterator = self.get_iterator(input)?;
        self.native_roots
            .extend([iterator.object.clone(), iterator.next.clone()]);
        let mut signals = Vec::new();
        while let Some(signal) = self.iterator_step(&mut iterator)? {
            self.signal_ref(&signal)?;
            if signals.len() >= self.limits.properties {
                return Err(Error::Limit {
                    resource: "abort signal sequence",
                });
            }
            self.native_roots.push(signal.clone());
            signals.push(signal);
        }
        let result = self.new_signal()?;
        self.native_roots.push(result.clone());
        for signal in &signals {
            if self.signal_ref(signal)?.aborted() {
                self.signal_mut(&result)?.reason = self.signal_ref(signal)?.reason.clone();
                return Ok(result);
            }
        }
        let mut sources = Vec::new();
        let mut seen = BTreeSet::new();
        for signal in signals {
            let data = self.signal_ref(&signal)?;
            let handles = if data.dependent {
                data.sources.clone()
            } else {
                alloc::vec![signal.heap_handle().ok_or(Error::InvalidBytecode)?]
            };
            for handle in handles {
                self.charge(1)?;
                if self.heap.get(handle).is_ok() && seen.insert(handle) {
                    if sources.len() >= self.limits.properties {
                        return Err(Error::Limit {
                            resource: "abort source signals",
                        });
                    }
                    sources.push(handle);
                }
            }
        }
        self.signal_mut(&result)?.dependent = true;
        let handle = result.heap_handle().ok_or(Error::InvalidBytecode)?;
        for source in &sources {
            let value = self.signal_value(*source);
            if self.signal_ref(&value)?.dependents.len() >= self.limits.properties {
                self.prune_signal_dependents(&value)?;
            }
            if self.signal_ref(&value)?.dependents.len() >= self.limits.properties {
                return Err(Error::Limit {
                    resource: "dependent abort signals",
                });
            }
            self.signal_mut(&value)?.dependents.push(handle);
        }
        self.signal_mut(&result)?.sources = sources;
        Ok(result)
    }
    fn prune_signal_dependents(&mut self, signal: &Value) -> Result<(), Error> {
        let old = core::mem::take(&mut self.signal_mut(signal)?.dependents);
        let mut live = Vec::new();
        for handle in old {
            self.charge(1)?;
            if matches!(self.heap.get(handle),Ok(Node::Object(o)) if o.abort_signal.as_ref().is_some_and(|s|!s.aborted()))
            {
                live.push(handle);
            }
        }
        self.signal_mut(signal)?.dependents = live;
        Ok(())
    }
    fn signal_value(&self, handle: Handle) -> Value {
        Value::Object(ObjectValue {
            handle,
            owner: self.owner.clone(),
        })
    }
}
const fn native(kind: Builtin) -> Value {
    Value::Function(FunctionValue::native(kind))
}
