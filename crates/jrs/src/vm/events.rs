// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Standalone DOM `EventTarget` adapter; no tree propagation or shadow retargeting.
use super::Execution;
use crate::{
    Error, Value,
    bytecode::Builtin,
    event::{Event, Field},
    heap::HostBehavior,
    object::Property,
    value::FunctionValue,
};
use alloc::vec::Vec;
use audhsos_event_target::{Listeners, Options};

const EVENT_METHODS: &[(&str, Builtin, u32)] = &[
    ("preventDefault", Builtin::EventPrevent, 0),
    ("stopPropagation", Builtin::EventStop, 0),
    ("stopImmediatePropagation", Builtin::EventStopImmediate, 0),
    ("composedPath", Builtin::EventPath, 0),
    ("initEvent", Builtin::EventInit, 1),
];
const TARGET_METHODS: &[(&str, Builtin, u32)] = &[
    ("addEventListener", Builtin::EventAdd, 2),
    ("removeEventListener", Builtin::EventRemove, 2),
    ("dispatchEvent", Builtin::EventDispatch, 1),
];
const FIELDS: &[(&str, Field)] = &[
    ("type", Field::Type),
    ("target", Field::Target),
    ("srcElement", Field::Target),
    ("currentTarget", Field::Current),
    ("eventPhase", Field::Phase),
    ("bubbles", Field::Bubbles),
    ("cancelable", Field::Cancelable),
    ("composed", Field::Composed),
    ("defaultPrevented", Field::Canceled),
    ("cancelBubble", Field::CancelBubble),
    ("returnValue", Field::ReturnValue),
    ("timeStamp", Field::Timestamp),
];

impl Execution<'_> {
    fn custom_event_prototype(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.custom_event_proto {
            return Ok(value.clone());
        }
        let proto = self.event_prototype(false)?;
        let value = self.allocate_object(proto)?;
        self.custom_event_proto = Some(value.clone());
        self.install_tag(&value, "CustomEvent")?;
        self.define(
            &value,
            Value::string("constructor").units(),
            Property {
                enumerable: false,
                ..Property::data(native(Builtin::CustomEvent))
            },
        )?;
        self.event_accessor(&value, "detail", Field::Detail, true)?;
        self.define(
            &value,
            Value::string("initCustomEvent").units(),
            Property::data(native(Builtin::CustomEventInit)),
        )?;
        Ok(value)
    }
    pub(super) fn event_prototype(&mut self, target: bool) -> Result<Value, Error> {
        if let Some(value) = if target {
            &self.event_target_proto
        } else {
            &self.event_proto
        } {
            return Ok(value.clone());
        }
        let parent = self.prototype()?;
        let value = self.allocate_object(parent)?;
        if target {
            self.event_target_proto = Some(value.clone());
        } else {
            self.event_proto = Some(value.clone());
        }
        self.install_tag(&value, if target { "EventTarget" } else { "Event" })?;
        self.define(
            &value,
            Value::string("constructor").units(),
            Property {
                enumerable: false,
                ..Property::data(native(if target {
                    Builtin::EventTarget
                } else {
                    Builtin::Event
                }))
            },
        )?;
        for (name, kind, _) in if target {
            TARGET_METHODS
        } else {
            EVENT_METHODS
        } {
            self.define(
                &value,
                Value::string(name).units(),
                Property::data(native(*kind)),
            )?;
        }
        if !target {
            for (name, field) in FIELDS {
                self.event_accessor(&value, name, *field, true)?;
            }
            for (name, n) in [
                ("NONE", 0),
                ("CAPTURING_PHASE", 1),
                ("AT_TARGET", 2),
                ("BUBBLING_PHASE", 3),
            ] {
                self.define(
                    &value,
                    Value::string(name).units(),
                    constant(Value::Number(f64::from(n))),
                )?;
            }
        }
        Ok(value)
    }
    fn event_accessor(
        &mut self,
        object: &Value,
        name: &str,
        field: Field,
        configurable: bool,
    ) -> Result<(), Error> {
        let getter = if matches!(field, Field::Trusted) && self.event_trusted_getter.is_some() {
            self.event_trusted_getter
                .clone()
                .ok_or(Error::InvalidBytecode)?
        } else {
            self.new_host_behavior(
                HostBehavior::EventGet(field),
                &alloc::format!("get {name}"),
                0,
            )?
        };
        if matches!(field, Field::Trusted) {
            self.event_trusted_getter = Some(getter.clone());
        }
        self.native_roots.push(getter.clone());
        let setter = if matches!(field, Field::CancelBubble | Field::ReturnValue) {
            let v = self.new_host_behavior(
                HostBehavior::EventSet(field),
                &alloc::format!("set {name}"),
                1,
            )?;
            self.native_roots.push(v.clone());
            v
        } else {
            Value::Undefined
        };
        self.define(
            object,
            Value::string(name).units(),
            Property {
                value: Value::Undefined,
                writable: false,
                enumerable: true,
                configurable,
                accessor: Some((getter, setter)),
            },
        )
    }
    pub(super) fn event_property(
        &mut self,
        builtin: Builtin,
        key: &[u16],
    ) -> Result<Option<Property>, Error> {
        if builtin == Builtin::CustomEvent && key == Value::string("prototype").units().as_ref() {
            return Ok(Some(Property {
                enumerable: false,
                ..constant(self.custom_event_prototype()?)
            }));
        }
        if matches!(builtin, Builtin::Event | Builtin::EventTarget)
            && key == Value::string("prototype").units().as_ref()
        {
            return Ok(Some(Property {
                enumerable: false,
                ..constant(self.event_prototype(builtin == Builtin::EventTarget)?)
            }));
        }
        if builtin == Builtin::Event {
            for (name, n) in [
                ("NONE", 0),
                ("CAPTURING_PHASE", 1),
                ("AT_TARGET", 2),
                ("BUBBLING_PHASE", 3),
            ] {
                if key == Value::string(name).units().as_ref() {
                    return Ok(Some(constant(Value::Number(f64::from(n)))));
                }
            }
        }
        let (name, length) = if builtin == Builtin::CustomEvent {
            ("CustomEvent", 1)
        } else if builtin == Builtin::CustomEventInit {
            ("initCustomEvent", 1)
        } else if builtin == Builtin::Event {
            ("Event", 1)
        } else if builtin == Builtin::EventTarget {
            ("EventTarget", 0)
        } else if let Some((name, _, length)) = EVENT_METHODS
            .iter()
            .chain(TARGET_METHODS)
            .find(|(_, id, _)| *id == builtin)
        {
            (*name, *length)
        } else {
            return Ok(None);
        };
        let value = if key == Value::string("name").units().as_ref() {
            Value::string(name)
        } else if key == Value::string("length").units().as_ref() {
            Value::Number(f64::from(length))
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
    pub(super) fn construct_event(
        &mut self,
        builtin: Builtin,
        args: &[Value],
        new_target: &Value,
    ) -> Result<Value, Error> {
        if builtin == Builtin::EventTarget {
            let default = self.event_prototype(true)?;
            let proto = self.constructor_prototype(new_target, default)?;
            let object = self.allocate_object(proto)?;
            self.object_mut(&object)?.listeners = Some(Listeners::new());
            return Ok(object);
        }
        let first = args.first().ok_or(Error::Type {
            message: "Event requires a type",
        })?;
        let name = self.string_units(first)?;
        let options = args.get(1).unwrap_or(&Value::Undefined);
        let (mut bubbles, mut cancelable, mut composed) = (false, false, false);
        if !matches!(options, Value::Null | Value::Undefined) {
            self.object_ref(options)?;
            bubbles = self
                .get(options, &Value::string("bubbles").units())?
                .to_boolean();
            cancelable = self
                .get(options, &Value::string("cancelable").units())?
                .to_boolean();
            composed = self
                .get(options, &Value::string("composed").units())?
                .to_boolean();
        }
        let detail = if builtin == Builtin::CustomEvent {
            Some(if matches!(options, Value::Null | Value::Undefined) {
                Value::Null
            } else {
                let value = self.get(options, &Value::string("detail").units())?;
                if matches!(value, Value::Undefined) {
                    Value::Null
                } else {
                    value
                }
            })
        } else {
            None
        };
        if let Some(value) = &detail {
            self.native_roots.push(value.clone());
        }
        let default = if builtin == Builtin::CustomEvent {
            self.custom_event_prototype()?
        } else {
            self.event_prototype(false)?
        };
        let proto = self.constructor_prototype(new_target, default)?;
        let object = self.allocate_object(proto)?;
        self.native_roots.push(object.clone());
        let timestamp = self.host.event_timestamp();
        if !timestamp.is_finite() || timestamp < 0.0 {
            return Err(Error::Host);
        }
        self.object_mut(&object)?.event = Some(Event {
            state: audhsos_event_target::Event::new(name, bubbles, cancelable, composed),
            target: Value::Null,
            current: Value::Null,
            timestamp,
            detail,
            trusted: false,
        });
        self.event_accessor(&object, "isTrusted", Field::Trusted, false)?;
        Ok(object)
    }
    fn event_ref(&self, value: &Value) -> Result<&Event, Error> {
        self.object_ref(value)?.event.as_ref().ok_or(Error::Type {
            message: "Event receiver required",
        })
    }
    fn event_mut(&mut self, value: &Value) -> Result<&mut Event, Error> {
        self.object_mut(value)?.event.as_mut().ok_or(Error::Type {
            message: "Event receiver required",
        })
    }
    pub(super) fn event_get(&self, value: &Value, field: Field) -> Result<Value, Error> {
        let event = self.event_ref(value)?;
        Ok(match field {
            Field::Detail => event.detail.clone().ok_or(Error::Type {
                message: "CustomEvent receiver required",
            })?,
            Field::Type => Value::String(event.state.event_type()),
            Field::Target => event.target.clone(),
            Field::Current => event.current.clone(),
            Field::Phase => Value::Number(if matches!(event.current, Value::Null) {
                0.0
            } else {
                2.0
            }),
            Field::Bubbles => Value::Boolean(event.state.bubbles()),
            Field::Cancelable => Value::Boolean(event.state.cancelable()),
            Field::Composed => Value::Boolean(event.state.composed()),
            Field::Canceled => Value::Boolean(event.state.canceled()),
            Field::CancelBubble => Value::Boolean(event.state.stopped()),
            Field::ReturnValue => Value::Boolean(!event.state.canceled()),
            Field::Trusted => Value::Boolean(event.trusted),
            Field::Timestamp => Value::Number(event.timestamp),
        })
    }
    pub(super) fn event_set(
        &mut self,
        value: &Value,
        field: Field,
        arg: &Value,
    ) -> Result<(), Error> {
        let event = self.event_mut(value)?;
        match field {
            Field::CancelBubble => {
                if arg.to_boolean() {
                    event.state.stop(false);
                }
            }
            Field::ReturnValue => {
                if !arg.to_boolean() {
                    event.state.prevent_default();
                }
            }
            _ => return Err(Error::InvalidBytecode),
        }
        Ok(())
    }
    pub(super) fn event_call(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        if matches!(
            builtin,
            Builtin::Event | Builtin::CustomEvent | Builtin::EventTarget
        ) {
            return Err(Error::Type {
                message: "event constructor requires new",
            });
        }
        if builtin != Builtin::CustomEventInit
            && !EVENT_METHODS
                .iter()
                .chain(TARGET_METHODS)
                .any(|(_, id, _)| *id == builtin)
        {
            return Ok(None);
        }
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        self.native_roots.extend_from_slice(args);
        let result = self.event_call_rooted(builtin, receiver, args);
        self.native_roots.truncate(roots);
        result.map(Some)
    }
    fn event_call_rooted(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Value, Error> {
        match builtin {
            Builtin::EventAdd | Builtin::EventRemove => {
                self.listener_change(builtin, receiver, args)?;
            }
            Builtin::EventDispatch => {
                return self.event_dispatch(
                    receiver,
                    args.first().ok_or(Error::Type {
                        message: "dispatchEvent requires an event",
                    })?,
                );
            }
            Builtin::EventPrevent => self.event_mut(receiver)?.state.prevent_default(),
            Builtin::EventStop | Builtin::EventStopImmediate => self
                .event_mut(receiver)?
                .state
                .stop(builtin == Builtin::EventStopImmediate),
            Builtin::EventPath => {
                let event = self.event_ref(receiver)?;
                let values = if event.state.dispatching() {
                    alloc::vec![event.target.clone()]
                } else {
                    Vec::new()
                };
                return self.array_from_values(&values);
            }
            Builtin::EventInit | Builtin::CustomEventInit => {
                self.event_ref(receiver)?;
                if builtin == Builtin::CustomEventInit && self.event_ref(receiver)?.detail.is_none()
                {
                    return Err(Error::Type {
                        message: "CustomEvent receiver required",
                    });
                }
                let kind = self.string_units(args.first().ok_or(Error::Type {
                    message: "initEvent requires a type",
                })?)?;
                let bubbles = args.get(1).is_some_and(Value::to_boolean);
                let cancelable = args.get(2).is_some_and(Value::to_boolean);
                let event = self.event_mut(receiver)?;
                if !event.state.dispatching() {
                    event.state.initialize(kind, bubbles, cancelable);
                    event.target = Value::Null;
                    event.trusted = false;
                    if builtin == Builtin::CustomEventInit {
                        event.detail = Some(
                            args.get(3)
                                .filter(|v| !matches!(v, Value::Undefined))
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                    }
                }
            }
            _ => return Err(Error::InvalidBytecode),
        }
        Ok(Value::Undefined)
    }
    fn listener_change(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<(), Error> {
        if self.object_ref(receiver)?.listeners.is_none() {
            return Err(Error::Type {
                message: "EventTarget receiver required",
            });
        }
        if args.len() < 2 {
            return Err(Error::Type {
                message: "listener operation requires type and callback",
            });
        }
        let kind = self.string_units(args.first().ok_or(Error::InvalidBytecode)?)?;
        let callback = args.get(1).ok_or(Error::InvalidBytecode)?;
        if !matches!(
            callback,
            Value::Null | Value::Undefined | Value::Object(_) | Value::Function(_)
        ) {
            return Err(Error::Type {
                message: "EventListener must be an object or null",
            });
        }
        let (options, signal) = self.listener_options(
            args.get(2).unwrap_or(&Value::Undefined),
            builtin == Builtin::EventAdd,
        )?;
        if let Some(signal) = &signal {
            self.native_roots.push(signal.clone());
            if self.signal_ref(signal)?.aborted() {
                return Ok(());
            }
        }
        if matches!(callback, Value::Null | Value::Undefined) {
            return Ok(());
        }
        let limit = self.limits.properties;
        let units = self.limits.string_units;
        let count = self
            .object_ref(receiver)?
            .listeners
            .as_ref()
            .ok_or(Error::InvalidBytecode)?
            .len();
        self.charge(u64::try_from(count).unwrap_or(u64::MAX))?;
        let listeners = self
            .object_mut(receiver)?
            .listeners
            .as_mut()
            .ok_or(Error::InvalidBytecode)?;
        if builtin == Builtin::EventAdd {
            let id = listeners.next_id();
            let added = listeners
                .add(kind, callback.clone(), options, limit, units)
                .map_err(|_| Error::Limit {
                    resource: "event listeners",
                })?;
            if added && let Some(signal) = signal {
                self.attach_listener_signal(&signal, receiver, id)?;
            }
        } else {
            let id = listeners
                .iter()
                .find(|l| {
                    l.event_type == kind
                        && l.callback == *callback
                        && l.options.capture == options.capture
                })
                .map(|l| l.id);
            if let Some(id) = id {
                self.remove_listener_id(receiver, id)?;
            }
        }
        Ok(())
    }
    fn listener_options(
        &mut self,
        value: &Value,
        add: bool,
    ) -> Result<(Options, Option<Value>), Error> {
        let mut options = Options::default();
        let mut abort = None;
        if matches!(value, Value::Object(_) | Value::Function(_)) {
            options.capture = self
                .get(value, &Value::string("capture").units())?
                .to_boolean();
            if add {
                options.once = self
                    .get(value, &Value::string("once").units())?
                    .to_boolean();
                options.passive = self
                    .get(value, &Value::string("passive").units())?
                    .to_boolean();
                let signal = self.get(value, &Value::string("signal").units())?;
                if !matches!(signal, Value::Undefined) {
                    self.signal_ref(&signal)?;
                    abort = Some(signal);
                }
            }
        } else {
            options.capture = value.to_boolean();
        }
        Ok((options, abort))
    }
    fn event_dispatch(&mut self, target: &Value, event: &Value) -> Result<Value, Error> {
        if self.object_ref(target)?.listeners.is_none() {
            return Err(Error::Type {
                message: "EventTarget receiver required",
            });
        }
        if self.event_ref(event)?.state.dispatching() {
            return Err(self.invalid_event_state()?);
        }
        self.event_mut(event)?.trusted = false;
        self.dispatch_trusted_event(target, event)
    }
    pub(super) fn dispatch_trusted_event(
        &mut self,
        target: &Value,
        event: &Value,
    ) -> Result<Value, Error> {
        if self.object_ref(target)?.listeners.is_none() {
            return Err(Error::Type {
                message: "EventTarget receiver required",
            });
        }
        if self.event_ref(event)?.state.dispatching() {
            return Err(self.invalid_event_state()?);
        }
        if self.event_depth >= 12 {
            return Err(Error::Limit {
                resource: "event dispatch reentry",
            });
        }
        self.event_mut(event)?.state.begin();
        self.event_mut(event)?.target = target.clone();
        self.event_depth = self.event_depth.saturating_add(1);
        let result = (|| {
            for capture in [true, false] {
                if self.event_ref(event)?.state.stopped() {
                    break;
                }
                self.event_mut(event)?.current = target.clone();
                let kind = self.event_ref(event)?.state.event_type();
                let listeners = self
                    .object_ref(target)?
                    .listeners
                    .as_ref()
                    .ok_or(Error::InvalidBytecode)?;
                let ids = listeners.snapshot(&kind, capture);
                self.charge(u64::try_from(listeners.len()).unwrap_or(u64::MAX))?;
                for id in ids {
                    self.charge(1)?;
                    let listener = self
                        .object_mut(target)?
                        .listeners
                        .as_mut()
                        .ok_or(Error::InvalidBytecode)?
                        .take_for_invoke(id);
                    let Some(listener) = listener else {
                        continue;
                    };
                    if listener.options.once {
                        self.remove_listener_id(target, id)?;
                    }
                    self.event_mut(event)?
                        .state
                        .passive(listener.options.passive);
                    let roots = self.native_roots.len();
                    self.native_roots.push(listener.callback.clone());
                    let result = self.invoke_listener(listener.callback, event, target);
                    self.event_mut(event)?.state.passive(false);
                    if let Err(error) = result {
                        self.report_listener_error(error)?;
                    }
                    self.native_roots.truncate(roots);
                    if self.event_ref(event)?.state.immediate() {
                        break;
                    }
                }
            }
            Ok(Value::Boolean(!self.event_ref(event)?.state.canceled()))
        })();
        self.event_depth = self.event_depth.saturating_sub(1);
        let state = self.event_mut(event)?;
        state.state.finish();
        state.current = Value::Null;
        result
    }
    fn invoke_listener(
        &mut self,
        callback: Value,
        event: &Value,
        target: &Value,
    ) -> Result<(), Error> {
        if matches!(callback, Value::Function(_)) {
            self.call_sync(callback, target.clone(), core::slice::from_ref(event))?;
        } else {
            let method = self.get(&callback, &Value::string("handleEvent").units())?;
            self.call_sync(method, callback, core::slice::from_ref(event))?;
        }
        Ok(())
    }
    fn report_listener_error(&mut self, error: Error) -> Result<(), Error> {
        let value = self.exception_value(error)?;
        self.retain_result(value.clone())?;
        let result = self
            .host
            .report_exception(&value)
            .map(|()| Value::Undefined);
        self.validate_host_result(&result)?;
        if let Err(error) = result {
            if !super::iterators::language_error(&error) {
                return Err(error);
            }
            if self.reported_exceptions.len() >= self.limits.jobs {
                return Err(Error::Limit {
                    resource: "reported event exceptions",
                });
            }
            if let Error::Thrown { value } = &error {
                self.retain_result(value.clone())?;
            }
            self.reported_exceptions.push(error);
        }
        Ok(())
    }
}
const fn native(kind: Builtin) -> Value {
    Value::Function(FunctionValue::native(kind))
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
