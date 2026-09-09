// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Persistent single-realm script evaluation. Declaration validation precedes
//! publication; lexical bindings and global object properties remain distinct.

use super::{Execution, Frame, Host};
use crate::{
    Error, Limits, Program, Value,
    bytecode::{GlobalKind, Slot},
    heap::Node,
    object::Property,
    value::FunctionValue,
};
use alloc::rc::Rc;

/// A persistent single realm bound to an explicit host capability.
/// Scripts are parsed independently and share globals, intrinsics and jobs.
/// Fuel is cumulative for the lifetime of the realm. Returned heap values and
/// thrown values are retained (up to `Limits.properties`) until the realm drops;
/// no ambient filesystem, clock or networking capability is supplied.
pub struct Realm<'host> {
    execution: Execution<'host>,
    failed: bool,
}

impl<'host> Realm<'host> {
    /// Installs optional HTML-style timeout/interval functions and enables
    /// AbortSignal.timeout when events are installed. Host must supply active
    /// monotonic time. Timers run only through `run_timer`, not evaluate.
    ///
    /// # Errors
    /// Missing clock, poisoned realm, rejected globals or exhausted quotas.
    pub fn install_timers(&mut self) -> Result<(), Error> {
        self.operation(&[], |e| {
            e.install_timers()?;
            Ok(Value::Undefined)
        })
        .map(|_| ())
    }
    /// Nanoseconds until the next timer deadline, without running JavaScript.
    /// None means no pending timers. The host may wait, then call `run_timer`.
    ///
    /// # Errors
    /// Poisoned realm, missing/nonmonotonic clock or host clock failure.
    pub fn timer_wait(&mut self) -> Result<Option<u128>, Error> {
        self.available()?;
        let Some(deadline) = self.execution.timers.queue.next_deadline() else {
            return Ok(None);
        };
        match self.execution.timer_clock() {
            Ok(now) => Ok(Some(deadline.saturating_sub(now))),
            Err(error) => self.outcome(Err(error)).map(|_| None),
        }
    }
    /// Runs at most one due timer task and its microtask checkpoint. Returns
    /// false when none is due. Exceptions use `Host::report_exception`; fuel is
    /// cumulative, and timer tasks never run inside another JavaScript call.
    ///
    /// # Errors
    /// Reported exception, poisoned realm or host/resource failure.
    pub fn run_timer(&mut self) -> Result<bool, Error> {
        let result = self.operation(&[], |e| {
            let result = e.run_due_timer().map(Value::Boolean);
            // The timer task has ended before the host-turn microtask checkpoint.
            e.timers.nesting = 0;
            result
        });
        result.map(|v| v == Value::Boolean(true))
    }
    /// Creates a test-host `evalScript` callback, bound to this realm. It applies
    /// `ToString` and evaluates an independent global Script synchronously, not
    /// lexical `eval` code. Jobs wait for the enclosing host-turn checkpoint.
    ///
    /// # Errors
    /// Poisoned realm or resource exhaustion while creating the capability.
    pub fn eval_script_function(&mut self) -> Result<Value, Error> {
        self.operation(&[], |e| {
            e.new_host_behavior(crate::heap::HostBehavior::EvalScript, "evalScript", 1)
        })
    }
    /// Creates a collectible native callback exposing explicit GC to test hosts.
    /// This capability is not installed in ordinary JavaScript globals.
    ///
    /// # Errors
    /// Poisoned realm or resource exhaustion.
    pub fn gc_function(&mut self) -> Result<Value, Error> {
        self.operation(&[], |e| {
            e.new_host_behavior(crate::heap::HostBehavior::CollectGarbage, "gc", 0)
        })
    }
    /// Creates a test-host print callback which applies `ToString` to its first
    /// argument and ignores extra arguments before calling `Host::print`.
    ///
    /// # Errors
    /// Poisoned realm or resource exhaustion.
    pub fn string_print_function(&mut self) -> Result<Value, Error> {
        self.operation(&[], |e| {
            e.new_host_behavior(crate::heap::HostBehavior::StringPrint, "print", 1)
        })
    }
    /// Executes a precompiled global Script and performs its job checkpoint.
    /// Declaration instantiation errors belong to execution, not compilation.
    /// Compilation limits must equal the limits of this realm.
    ///
    /// # Errors
    /// Limit mismatch, poisoned realm, declaration conflicts or execution errors.
    pub fn evaluate_compiled(&mut self, script: &crate::Script) -> Result<Value, Error> {
        self.available()?;
        if script.limits != self.execution.limits {
            return Err(Error::Type {
                message: "compiled Script limits differ from realm limits",
            });
        }
        let result = self.execution.evaluate_script(&script.program);
        self.outcome(result)
    }
    /// Installs standalone Event/EventTarget and AbortController/AbortSignal
    /// interfaces (timeout additionally requires `install_timers`). This does not make the
    /// global object a Window or `EventTarget`, nor create DOM parent paths.
    ///
    /// # Errors
    /// Rejected definitions, poisoned realm or resource exhaustion.
    pub fn install_events(&mut self) -> Result<(), Error> {
        self.operation(&[], |execution| {
            let global = execution.global_object()?;
            for (name, kind) in [
                ("Event", crate::bytecode::Builtin::Event),
                ("CustomEvent", crate::bytecode::Builtin::CustomEvent),
                ("DOMException", crate::bytecode::Builtin::DomException),
                ("EventTarget", crate::bytecode::Builtin::EventTarget),
                ("AbortController", crate::bytecode::Builtin::AbortController),
                ("AbortSignal", crate::bytecode::Builtin::AbortSignal),
            ] {
                execution.define(
                    &global,
                    Value::string(name).units(),
                    Property {
                        enumerable: false,
                        ..Property::data(Value::Function(FunctionValue::native(kind)))
                    },
                )?;
            }
            Ok(Value::Undefined)
        })
        .map(|_| ())
    }
    /// Installs the optional HTML `queueMicrotask` function as a writable,
    /// configurable global. It shares the Promise FIFO job queue and reports
    /// thrown values through `Host::report_exception`. This does not install
    /// Window, Worker or DOM APIs. Reinstalling creates a new function identity.
    ///
    /// # Errors
    /// Poisoned realm, rejected global definition or resource failure.
    pub fn install_queue_microtask(&mut self) -> Result<(), Error> {
        self.operation(&[], |execution| {
            let global = execution.global_object()?;
            let function = execution.new_host_behavior(
                crate::heap::HostBehavior::QueueMicrotask,
                "queueMicrotask",
                1,
            )?;
            execution.define(
                &global,
                Value::string("queueMicrotask").units(),
                Property::data(function),
            )?;
            Ok(Value::Undefined)
        })
        .map(|_| ())
    }
    /// Queues a callback from Rust without running it or triggering a checkpoint.
    /// The queue itself roots the callback, so host retention can be released.
    /// Execution occurs at the next evaluate/call/checkpoint operation.
    ///
    /// # Errors
    /// Non-callable, foreign/stale value, poisoned realm or job quota exhaustion.
    pub fn queue_microtask(&mut self, callback: &Value) -> Result<(), Error> {
        self.available()?;
        let result = self
            .execution
            .validate_host_value(callback)
            .and_then(|()| self.execution.enqueue_microtask(callback.clone()))
            .map(|()| Value::Undefined);
        self.outcome(result).map(|_| ())
    }
    /// Drains the current Promise/microtask queue without evaluating a Script.
    /// Nested enqueues join the same FIFO checkpoint; host errors are observable.
    ///
    /// # Errors
    /// Reported callback exceptions, poisoned realm or host/resource failures.
    pub fn checkpoint(&mut self) -> Result<(), Error> {
        self.operation(&[], |_| Ok(Value::Undefined)).map(|_| ())
    }
    const fn available(&self) -> Result<(), Error> {
        if self.failed {
            Err(Error::Limit {
                resource: "realm is poisoned",
            })
        } else {
            Ok(())
        }
    }
    fn outcome(&mut self, result: Result<Value, Error>) -> Result<Value, Error> {
        if result
            .as_ref()
            .is_err_and(|e| !super::iterators::language_error(e))
        {
            self.failed = true;
        }
        result
    }
    fn operation(
        &mut self,
        inputs: &[Value],
        action: impl FnOnce(&mut Execution<'host>) -> Result<Value, Error>,
    ) -> Result<Value, Error> {
        self.available()?;
        let result = (|| {
            for value in inputs {
                self.execution.validate_host_value(value)?;
            }
            let program = Program::empty();
            self.execution.start_script_frame(&program)?;
            self.execution.native_roots.extend_from_slice(inputs);
            let result = action(&mut self.execution);
            self.execution.finish_script_turn(&program, result)
        })();
        self.outcome(result)
    }

    /// Creates a non-constructible function dispatching to `Host::call(id, ...)`.
    /// Each creation has distinct identity and ordinary mutable properties.
    /// The returned value is retained; install it with `set_global` or `set`.
    ///
    /// # Errors
    /// Returns poisoned-realm, allocation, property or string-quota errors.
    pub fn host_function(&mut self, id: u32, name: &str, length: u32) -> Result<Value, Error> {
        self.host_name(name)?;
        self.operation(&[], |execution| {
            execution.new_host_function(id, name, length)
        })
    }
    /// Reads a global binding, including lexical bindings, and runs a checkpoint.
    ///
    /// # Errors
    /// Returns reference/getter exceptions or host/resource failures.
    pub fn get_global(&mut self, name: &str) -> Result<Value, Error> {
        self.host_name(name)?;
        self.operation(&[], |execution| execution.global_get(name, false))
    }
    /// Sets a global object property (not a lexical binding), invoking setters.
    ///
    /// # Errors
    /// Foreign/stale values, rejected writes, getter/setter or resource errors.
    pub fn set_global(&mut self, name: &str, value: &Value) -> Result<(), Error> {
        self.host_name(name)?;
        self.operation(&[Value::string(name), value.clone()], |execution| {
            let global = execution.global_object()?;
            execution.set(&global, Value::string(name).units(), value.clone(), true)?;
            Ok(Value::Undefined)
        })
        .map(|_| ())
    }
    /// Defines a property using the same descriptor conversion and validation as
    /// `Object.defineProperty`. This can install host-function getters/setters;
    /// descriptor getters may execute JavaScript. A checkpoint follows.
    ///
    /// # Errors
    /// Foreign/stale values, invalid descriptors, rejected definitions, or quotas.
    pub fn define_property(
        &mut self,
        object: &Value,
        key: &Value,
        descriptor: &Value,
    ) -> Result<(), Error> {
        self.operation(
            &[object.clone(), key.clone(), descriptor.clone()],
            |execution| {
                execution.native_call(
                    crate::bytecode::Builtin::DefineProperty,
                    &Value::Undefined,
                    &[object.clone(), key.clone(), descriptor.clone()],
                )?;
                Ok(Value::Undefined)
            },
        )
        .map(|_| ())
    }
    /// Converts a value to a primitive String, invoking conversion hooks and then
    /// a checkpoint. Unlike diagnostic Display, this preserves UTF-16 and throws
    /// for Symbol operands just like the abstract `ToString` operation.
    ///
    /// # Errors
    /// Foreign/stale values, conversion exceptions, or host/resource failures.
    pub fn to_string(&mut self, value: &Value) -> Result<Value, Error> {
        self.operation(core::slice::from_ref(value), |execution| {
            Ok(Value::String(execution.string_units(value)?))
        })
    }
    /// Converts a value to a Number using the VM's fallible `ToNumber` operation
    /// and runs a checkpoint. Object hooks and Symbol errors are observed.
    ///
    /// # Errors
    /// Foreign/stale values, conversion exceptions, or host/resource failures.
    pub fn to_number(&mut self, value: &Value) -> Result<Value, Error> {
        self.operation(core::slice::from_ref(value), |execution| {
            Ok(Value::Number(execution.numeric(value)?))
        })
    }
    /// Reads a property using ordinary key conversion and accessor semantics.
    ///
    /// # Errors
    /// Invalid/foreign values, JavaScript exceptions or resource errors.
    pub fn get(&mut self, object: &Value, key: &Value) -> Result<Value, Error> {
        self.operation(&[object.clone(), key.clone()], |execution| {
            let key = execution.property_key(key)?;
            execution.get_key(object, &key)
        })
    }
    /// Writes a property with strict assignment semantics and a checkpoint.
    ///
    /// # Errors
    /// Invalid/foreign values, rejected writes, JavaScript or resource errors.
    pub fn set(&mut self, object: &Value, key: &Value, value: &Value) -> Result<(), Error> {
        self.operation(&[object.clone(), key.clone(), value.clone()], |execution| {
            let key = execution.property_key(key)?;
            execution.set_key(object, key, value.clone(), true)?;
            Ok(Value::Undefined)
        })
        .map(|_| ())
    }
    /// Allocates an ordinary object with the realm's Object prototype.
    ///
    /// # Errors
    /// Poisoned-realm or resource failures.
    pub fn object(&mut self) -> Result<Value, Error> {
        self.operation(&[], |execution| {
            let proto = execution.prototype()?;
            execution.allocate_object(proto)
        })
    }
    /// Invokes a retained callback with the exact receiver and arguments, then
    /// drains Promise jobs. This is an explicit host turn, not a browser event.
    ///
    /// # Errors
    /// Non-callable, foreign/stale arguments, script exceptions or host/resource errors.
    pub fn call(
        &mut self,
        function: &Value,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Value, Error> {
        self.available()?;
        if args.len().saturating_add(2) > self.execution.limits.stack {
            return self.outcome(Err(Error::Limit {
                resource: "host call arguments",
            }));
        }
        let mut inputs = alloc::vec![function.clone(), receiver.clone()];
        inputs.extend_from_slice(args);
        self.operation(&inputs, |execution| {
            execution.call_sync(function.clone(), receiver.clone(), args)
        })
    }
    /// Releases this value's implicit host retention. Copies are not separately
    /// rooted; after release the host must not use any copy unless it obtains a
    /// newly retained value. A stale handle is rejected, never dereferenced.
    ///
    /// # Errors
    /// Poisoned-realm, foreign/stale identity or oversized string input.
    pub fn release(&mut self, value: &Value) -> Result<(), Error> {
        self.available()?;
        let result = self.execution.validate_host_value(value);
        if let Err(error) = result {
            return self.outcome(Err(error)).map(|_| ());
        }
        if let Some(key) = value.retention_key() {
            self.execution.retained.remove(&key);
        }
        Ok(())
    }
    fn host_name(&mut self, name: &str) -> Result<(), Error> {
        self.available()?;
        // Reject oversized Rust strings before allocating their UTF-16 form.
        if name
            .encode_utf16()
            .take(self.execution.limits.string_units.saturating_add(1))
            .count()
            > self.execution.limits.string_units
        {
            return self
                .outcome(Err(Error::Limit {
                    resource: "host property name",
                }))
                .map(|_| ());
        }
        Ok(())
    }
    /// Creates a fresh realm with the implemented standard globals.
    ///
    /// # Errors
    /// Returns a resource error if intrinsic/global initialization exceeds limits.
    pub fn new(limits: Limits, host: &'host mut impl Host) -> Result<Self, Error> {
        let mut execution = Execution::new(host, limits);
        execution.fuel = limits.fuel;
        execution.initialize_globals()?;
        Ok(Self {
            execution,
            failed: false,
        })
    }
    /// Evaluates one independent Script and then drains its Promise jobs.
    /// Earlier script effects survive syntax errors and JavaScript exceptions.
    /// Resource, host and VM-invariant failures poison the realm; further calls
    /// then fail without executing user code.
    ///
    /// # Errors
    /// Compilation/declaration conflicts, JavaScript exceptions, or host/resource errors.
    pub fn evaluate(&mut self, source: &str) -> Result<Value, Error> {
        if self.failed {
            return Err(Error::Limit {
                resource: "realm is poisoned",
            });
        }
        let program = match crate::bytecode::compile_realm(source, self.execution.limits) {
            Ok(program) => program,
            Err(error) => {
                if !super::iterators::language_error(&error) {
                    self.failed = true;
                }
                return Err(error);
            }
        };
        let result = self.execution.evaluate_script(&program);
        if result
            .as_ref()
            .is_err_and(|e| !super::iterators::language_error(e))
        {
            self.failed = true;
        }
        result
    }
}

impl Execution<'_> {
    pub(super) fn eval_global_source(&mut self, input: &Value) -> Result<Value, Error> {
        if self.native_depth >= 12 {
            return Err(Error::Limit {
                resource: "nested Script reentry",
            });
        }
        self.native_depth = self.native_depth.saturating_add(1);
        let roots = self.native_roots.len();
        self.native_roots.push(input.clone());
        let result = (|| {
            let units = self.string_units(input)?;
            self.charge(u64::try_from(units.len()).unwrap_or(u64::MAX))?;
            let source =
                alloc::string::String::from_utf16(&units).map_err(|_| Error::Unsupported {
                    feature: "lone-surrogate Script source",
                })?;
            let program = crate::bytecode::compile_realm(&source, self.limits)?;
            self.charge(u64::try_from(program.instruction_count()).unwrap_or(u64::MAX))?;
            self.execute_nested_script(program)
        })();
        self.native_roots.truncate(roots);
        self.native_depth = self.native_depth.saturating_sub(1);
        result
    }
    fn execute_nested_script(&mut self, program: Program) -> Result<Value, Error> {
        self.check_frame_limit()?;
        self.instantiate_globals(&program)?;
        self.reserve_bindings(program.slots.len())?;
        let frames = self.frames.len();
        let stack = self.stack.len();
        let boundary = self.unwind_boundary;
        let program = Rc::new(program);
        let previous = self.top_program.replace(program.clone());
        self.unwind_boundary = frames;
        self.frames.push(Frame::script(&program, stack));
        let result = self.execute(&program, frames);
        while self.frames.len() > frames {
            let frame = self.frames.pop().ok_or(Error::InvalidBytecode)?;
            self.binding_slots = self.binding_slots.saturating_sub(frame.locals.len());
        }
        self.stack.truncate(stack);
        self.unwind_boundary = boundary;
        self.top_program = previous;
        result
    }
    pub(super) fn initialize_globals(&mut self) -> Result<(), Error> {
        if self.globals_initialized {
            return Ok(());
        }
        let global = self.global_object()?;
        for name in [
            "print",
            "Number",
            "String",
            "Boolean",
            "Object",
            "Array",
            "RegExp",
            "Promise",
            "WeakMap",
            "Symbol",
            "Function",
            "Error",
            "TypeError",
            "RangeError",
            "ReferenceError",
            "SyntaxError",
            "EvalError",
            "URIError",
            "isNaN",
            "isFinite",
        ] {
            let builtin = crate::bytecode::builtin(name).ok_or(Error::InvalidBytecode)?;
            self.define(
                &global,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(Value::Function(FunctionValue::native(builtin)))
                },
            )?;
        }
        let math = self.math_object()?;
        let reflect = self.reflect_object()?;
        self.define(
            &global,
            Value::string("Reflect").units(),
            Property {
                enumerable: false,
                ..Property::data(reflect)
            },
        )?;
        let json = self.json_object()?;
        self.define(
            &global,
            Value::string("JSON").units(),
            Property {
                enumerable: false,
                ..Property::data(json)
            },
        )?;
        self.define(
            &global,
            Value::string("Math").units(),
            Property {
                enumerable: false,
                ..Property::data(math)
            },
        )?;
        for (name, value) in [
            ("undefined", Value::Undefined),
            ("NaN", Value::Number(f64::NAN)),
            ("Infinity", Value::Number(f64::INFINITY)),
        ] {
            self.define(
                &global,
                Value::string(name).units(),
                Property {
                    value,
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    accessor: None,
                },
            )?;
        }
        self.globals_initialized = true;
        Ok(())
    }
    fn instantiate_globals(&mut self, program: &Program) -> Result<(), Error> {
        let global = self.global_object()?;
        for decl in &program.globals {
            self.charge(1)?;
            let property = self.own(&global, &Value::string(&decl.name).units())?;
            if self.global_lexicals.contains_key(&decl.name)
                || matches!(decl.kind, GlobalKind::Lexical(_))
                    && property.as_ref().is_some_and(|p| !p.configurable)
            {
                return Err(Error::Syntax {
                    offset: 0,
                    message: "conflicting global declaration",
                });
            }
        }
        // All lexical collisions/restricted properties precede function/var
        // definability checks, irrespective of source declaration order.
        for decl in &program.globals {
            self.charge(1)?;
            let property = self.own(&global, &Value::string(&decl.name).units())?;
            if matches!(decl.kind, GlobalKind::Var | GlobalKind::Function) {
                if property.is_none() && !self.object_ref(&global)?.extensible {
                    return Err(Error::Type {
                        message: "global object is not extensible",
                    });
                }
                if decl.kind == GlobalKind::Function
                    && property.as_ref().is_some_and(|p| {
                        !p.configurable && (p.accessor.is_some() || !p.writable || !p.enumerable)
                    })
                {
                    return Err(Error::Type {
                        message: "global function cannot be declared",
                    });
                }
            }
        }
        let lexical_count = program
            .globals
            .iter()
            .filter(|d| matches!(d.kind, GlobalKind::Lexical(_)))
            .count();
        if self.global_lexicals.len().saturating_add(lexical_count) > self.limits.binding_slots {
            return Err(Error::Limit {
                resource: "global bindings",
            });
        }
        for decl in &program.globals {
            match decl.kind {
                GlobalKind::Lexical(mutable) => {
                    self.reserve()?;
                    let h = self.heap.allocate(
                        Node::Cell {
                            slot: Slot {
                                name: decl.name.clone(),
                                mutable,
                                captured: true,
                            },
                            value: None,
                        },
                        self.limits.heap_entries,
                    )?;
                    self.global_lexicals.insert(decl.name.clone(), h);
                }
                GlobalKind::Var | GlobalKind::Function => {
                    let key = Value::string(&decl.name).units();
                    if self.own(&global, &key)?.is_none() {
                        self.define(
                            &global,
                            key,
                            Property {
                                configurable: false,
                                ..Property::data(Value::Undefined)
                            },
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
    fn evaluate_script(&mut self, program: &Program) -> Result<Value, Error> {
        self.instantiate_globals(program)?;
        self.start_script_frame(program)?;
        let result = self.execute(program, 0);
        self.finish_script_turn(program, result)
    }
    fn start_script_frame(&mut self, program: &Program) -> Result<(), Error> {
        if !self.frames.is_empty() || !self.stack.is_empty() {
            return Err(Error::InvalidBytecode);
        }
        self.binding_slots = program
            .slots
            .len()
            .saturating_add(self.heap.async_usage().1);
        if self
            .binding_slots
            .saturating_add(self.global_lexicals.len())
            > self.limits.binding_slots
        {
            return Err(Error::Limit {
                resource: "binding slots",
            });
        }
        self.frames.push(Frame::script(program, 0));
        self.top_program = Some(Rc::new(program.clone()));
        Ok(())
    }
    fn finish_script_turn(
        &mut self,
        program: &Program,
        mut result: Result<Value, Error>,
    ) -> Result<Value, Error> {
        if let Ok(value) | Err(Error::Thrown { value }) = &result {
            self.retain_result(value.clone())?;
        }
        self.stack.clear();
        self.frames.truncate(1);
        if let Some(frame) = self.frames.first_mut() {
            frame.handlers.clear();
            frame.locals.clear();
            frame.result = Value::Undefined;
            frame.pc = program.code.len();
        }
        if (result.is_ok() || result.as_ref().is_err_and(super::iterators::language_error))
            && let Err(error) = self.drain_jobs()
            && (result.is_ok() || !super::iterators::language_error(&error))
        {
            result = Err(error);
        }
        if let Some(error) = self.reported_exceptions.first().cloned()
            && result.is_ok()
        {
            result = Err(error);
        }
        self.stack.clear();
        self.frames.clear();
        if let Err(Error::Thrown { value }) = &result {
            self.retain_result(value.clone())?;
        }
        self.reported_exceptions.clear();
        self.native_roots.clear();
        self.top_program = None;
        self.binding_slots = self.heap.async_usage().1;
        result
    }
    pub(super) fn retain_result(&mut self, value: Value) -> Result<(), Error> {
        if let Some(key) = value.retention_key()
            && !self.retained.contains_key(&key)
        {
            if self.retained.len() >= self.limits.properties {
                return Err(Error::Limit {
                    resource: "retained realm results",
                });
            }
            self.retained.insert(key, value);
        }
        Ok(())
    }
    pub(super) fn global_delete(&mut self, name: &str) -> Result<bool, Error> {
        if self.global_lexicals.contains_key(name) {
            return Ok(false);
        }
        let global = self.global_object()?;
        Ok(self
            .object_mut(&global)?
            .delete(&Value::string(name).units()))
    }
    pub(super) fn global_get(&mut self, name: &str, typeof_operand: bool) -> Result<Value, Error> {
        if let Some(handle) = self.global_lexicals.get(name) {
            let Node::Cell { value, .. } = self.heap.get(*handle)? else {
                return Err(Error::InvalidBytecode);
            };
            return value.clone().ok_or(Error::Reference { name: name.into() });
        }
        let global = self.global_object()?;
        let key = Value::string(name).units();
        if !typeof_operand && !self.has_property(&global, &key)? {
            return Err(Error::Reference { name: name.into() });
        }
        self.get(&global, &key)
    }
    pub(super) fn global_set(
        &mut self,
        name: &str,
        value: Value,
        strict: bool,
        initialize: bool,
    ) -> Result<(), Error> {
        if let Some(handle) = self.global_lexicals.get(name) {
            let Node::Cell { value: old, slot } = self.heap.get_mut(*handle)? else {
                return Err(Error::InvalidBytecode);
            };
            if !initialize {
                if old.is_none() {
                    return Err(Error::Reference { name: name.into() });
                }
                if !slot.mutable {
                    return Err(Error::Type {
                        message: "assignment to a constant global binding",
                    });
                }
            }
            *old = Some(value);
            return Ok(());
        }
        let roots = self.native_roots.len();
        self.native_roots.push(value.clone());
        let result = (|| {
            let global = self.global_object()?;
            let key = Value::string(name).units();
            if initialize {
                return self.define(
                    &global,
                    key,
                    Property {
                        configurable: false,
                        ..Property::data(value)
                    },
                );
            }
            if strict && !self.has_property(&global, &key)? {
                return Err(Error::Reference { name: name.into() });
            }
            self.set(&global, key, value, strict)
        })();
        self.native_roots.truncate(roots);
        result
    }
}
