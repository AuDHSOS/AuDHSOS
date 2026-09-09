// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! FIFO Promise jobs share script fuel and the real host. Resolving function
//! pairs share a heap latch; pending adoption cannot be overridden by rejection.

use super::{Binding, Execution, Frame, exceptions::Completion};
use crate::{
    Error, Value,
    bytecode::Builtin,
    heap::{Handle, Node},
    object::{Object, Property},
    promise::{Capability, Job, Promise, Reaction, State},
    value::{Callable, FunctionValue},
};
use alloc::vec::Vec;

pub(crate) struct Suspension {
    frame: Frame,
    stack: Vec<Value>,
}
impl Suspension {
    pub(crate) const fn sizes(&self) -> (usize, usize) {
        (self.frame.locals.len(), self.stack.len())
    }
    pub(crate) fn trace(&self, work: &mut Vec<Handle>) {
        for value in &self.stack {
            work.extend(value.heap_handle());
        }
        for binding in &self.frame.locals {
            match binding {
                Binding::Cell(h) => work.push(*h),
                Binding::Direct(Some(v)) => work.extend(v.heap_handle()),
                Binding::Direct(None) => {}
            }
        }
        work.extend(self.frame.captures.iter().copied());
        work.extend(self.frame.this_cell);
        work.extend(self.frame.new_target.heap_handle());
        for value in [
            &self.frame.result,
            &self.frame.callee,
            &self.frame.this_value,
        ] {
            work.extend(value.heap_handle());
        }
        if let Some(value) = &self.frame.async_promise {
            work.extend(value.heap_handle());
        }
        for value in &self.frame.arguments {
            work.extend(value.heap_handle());
        }
        for enumeration in self.frame.enumerations.iter().flatten() {
            work.extend(enumeration.object.heap_handle());
            if let Some(record) = &enumeration.iterator {
                work.extend(record.object.heap_handle());
                work.extend(record.next.heap_handle());
            }
        }
        for handler in &self.frame.handlers {
            handler.trace_handles(work);
        }
    }
}

impl Job {
    pub(crate) fn roots(&self) -> Vec<&Value> {
        match self {
            Self::Microtask(callback) => alloc::vec![callback],
            Self::Reaction {
                reaction, value, ..
            } => alloc::vec![
                &reaction.fulfilled,
                &reaction.rejected,
                &reaction.target,
                &reaction.resolve,
                &reaction.reject,
                value
            ],
            Self::Thenable {
                target,
                object,
                then,
            } => alloc::vec![target, object, then],
        }
    }
}

impl Execution<'_> {
    fn new_capability(&mut self, ctor: &Value) -> Result<Capability, Error> {
        if !self.is_constructor(ctor) {
            return Err(Error::Type {
                message: "Promise capability requires a constructor",
            });
        }
        let cap = if ctor.strictly_equals(&native(Builtin::Promise)) {
            let promise = self.new_promise()?;
            self.native_roots.push(promise.clone());
            let (resolve, reject) = self.resolving_functions(&promise)?;
            Capability {
                promise,
                resolve,
                reject,
            }
        } else {
            let source = "(function(C,fail){'use strict';let resolve,reject;let promise=new C(function(r,j){if(resolve!==undefined||reject!==undefined)fail();resolve=r;reject=j;});if(typeof resolve!=='function'||typeof reject!=='function')fail();return {promise,resolve,reject};})";
            let result = self.promise_helper(
                Builtin::PromiseCapability,
                source,
                &[ctor.clone(), native(Builtin::ThrowTypeError)],
            )?;
            self.native_roots.push(result.clone());
            Capability {
                promise: self.get(&result, &Value::string("promise").units())?,
                resolve: self.get(&result, &Value::string("resolve").units())?,
                reject: self.get(&result, &Value::string("reject").units())?,
            }
        };
        self.native_roots
            .extend([cap.promise.clone(), cap.resolve.clone(), cap.reject.clone()]);
        Ok(cap)
    }
    fn promise_species(&mut self, promise: &Value) -> Result<Value, Error> {
        let ctor = self.get(promise, &Value::string("constructor").units())?;
        if matches!(ctor, Value::Undefined) {
            return Ok(native(Builtin::Promise));
        }
        if !matches!(ctor, Value::Object(_) | Value::Function(_)) {
            return Err(Error::Type {
                message: "invalid species constructor",
            });
        }
        self.native_roots.push(ctor.clone());
        let key = Value::Symbol(self.well_known("species")?);
        let species = self.get_key(&ctor, &key)?;
        if matches!(species, Value::Undefined | Value::Null) {
            return Ok(native(Builtin::Promise));
        }
        if !self.is_constructor(&species) {
            return Err(Error::Type {
                message: "species is not a constructor",
            });
        }
        Ok(species)
    }
    fn promise_helper(
        &mut self,
        id: Builtin,
        source: &str,
        args: &[Value],
    ) -> Result<Value, Error> {
        let code = if let Some((_, code)) = self.intrinsic_code.iter().find(|(kind, _)| *kind == id)
        {
            code.clone()
        } else {
            let program = crate::compile(source, crate::Limits::default())?;
            let mut code = program
                .functions
                .first()
                .ok_or(Error::InvalidBytecode)?
                .clone();
            // The abstract capability executor is callable, not constructible.
            // It closes over the helper's private resolve/reject slots, but
            // must not acquire an ordinary function's prototype property.
            if id == Builtin::PromiseCapability {
                let executor = alloc::rc::Rc::make_mut(&mut code)
                    .program
                    .functions
                    .first_mut()
                    .ok_or(Error::InvalidBytecode)?;
                alloc::rc::Rc::make_mut(executor).constructible = false;
            }
            self.intrinsic_code.push((id, code.clone()));
            code
        };
        self.reserve()?;
        let handle = self.heap.allocate(
            Node::Function {
                code,
                captures: Vec::new(),
                lexical_this: None,
                object: Object::new(Value::Null),
            },
            self.limits.heap_entries,
        )?;
        self.call_sync(
            Value::Function(FunctionValue(Callable::Script {
                handle,
                owner: self.owner.clone(),
            })),
            Value::Undefined,
            args,
        )
    }
    pub(super) fn construct_promise(
        &mut self,
        args: &[Value],
        proto: Value,
    ) -> Result<Value, Error> {
        let executor = args.first().cloned().unwrap_or(Value::Undefined);
        if !matches!(executor, Value::Function(_)) {
            return Err(Error::Type {
                message: "Promise executor is not callable",
            });
        }
        let promise = self.allocate_object(proto)?;
        self.object_mut(&promise)?.promise = Some(Promise::new());
        self.native_roots.push(promise.clone());
        let (resolve, reject) = self.resolving_functions(&promise)?;
        self.native_roots.extend([resolve.clone(), reject.clone()]);
        if let Err(error) = self.call_sync(executor, Value::Undefined, &[resolve, reject.clone()]) {
            let reason = self.exception_value(error)?;
            self.call_sync(reject, Value::Undefined, &[reason])?;
        }
        Ok(promise)
    }
    pub(super) fn promise_prototype(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.promise_proto {
            return Ok(value.clone());
        }
        let proto = self.prototype()?;
        let value = self.allocate_object(proto)?;
        self.promise_proto = Some(value.clone());
        self.install_tag(&value, "Promise")?;
        for (name, builtin) in [
            ("then", Builtin::PromiseThen),
            ("catch", Builtin::PromiseCatch),
            ("finally", Builtin::PromiseFinally),
            ("constructor", Builtin::Promise),
        ] {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(native(builtin))
                },
            )?;
        }
        Ok(value)
    }
    pub(super) fn new_promise(&mut self) -> Result<Value, Error> {
        let proto = self.promise_prototype()?;
        let value = self.allocate_object(proto)?;
        self.object_mut(&value)?.promise = Some(Promise::new());
        Ok(value)
    }
    fn is_promise(&self, value: &Value) -> bool {
        self.object_ref(value).is_ok_and(|o| o.promise.is_some())
    }
    pub(super) fn enqueue(&mut self, job: Job) -> Result<(), Error> {
        if self.jobs.len() >= self.limits.jobs {
            return Err(Error::Limit {
                resource: "Promise jobs",
            });
        }
        self.jobs.push_back(job);
        Ok(())
    }
    pub(super) fn enqueue_microtask(&mut self, callback: Value) -> Result<(), Error> {
        if !matches!(callback, Value::Function(_)) {
            return Err(Error::Type {
                message: "queueMicrotask callback must be callable",
            });
        }
        self.enqueue(Job::Microtask(callback))
    }
    fn resolving_functions(&mut self, target: &Value) -> Result<(Value, Value), Error> {
        self.reserve()?;
        let handle = self.heap.allocate(
            Node::Resolving {
                target: target.clone(),
                used: false,
                object: Object::new(Value::Null),
            },
            self.limits.heap_entries,
        )?;
        Ok((
            Value::Function(FunctionValue(Callable::Resolver {
                handle,
                owner: self.owner.clone(),
                reject: false,
            })),
            Value::Function(FunctionValue(Callable::Resolver {
                handle,
                owner: self.owner.clone(),
                reject: true,
            })),
        ))
    }
    pub(super) fn resolve_once(
        &mut self,
        handle: Handle,
        value: Value,
        reject: bool,
    ) -> Result<(), Error> {
        let Node::Resolving { target, used, .. } = self.heap.get_mut(handle)? else {
            return Err(Error::InvalidBytecode);
        };
        if *used {
            return Ok(());
        }
        *used = true;
        let target = target.clone();
        if reject {
            self.settle(&target, value, true)
        } else {
            self.resolve_promise(&target, value)
        }
    }
    pub(super) fn resolve_promise(&mut self, target: &Value, value: Value) -> Result<(), Error> {
        let roots = self.native_roots.len();
        self.native_roots.extend([target.clone(), value.clone()]);
        let result = self.resolve_rooted(target, value);
        self.native_roots.truncate(roots);
        result
    }
    fn resolve_rooted(&mut self, target: &Value, value: Value) -> Result<(), Error> {
        if target.strictly_equals(&value) {
            let error = self.error_object(&Error::Type {
                message: "Promise self resolution",
            })?;
            return self.settle(target, error, true);
        }
        if !matches!(value, Value::Object(_) | Value::Function(_)) {
            return self.settle(target, value, false);
        }
        let then = match self.get(&value, &Value::string("then").units()) {
            Ok(then) => then,
            Err(error) => {
                let reason = self.exception_value(error)?;
                return self.settle(target, reason, true);
            }
        };
        if !matches!(then, Value::Function(_)) {
            return self.settle(target, value, false);
        }
        self.enqueue(Job::Thenable {
            target: target.clone(),
            object: value,
            then,
        })
    }
    pub(super) fn settle(
        &mut self,
        target: &Value,
        value: Value,
        reject: bool,
    ) -> Result<(), Error> {
        let promise = self
            .object_mut(target)?
            .promise
            .as_mut()
            .ok_or(Error::Type {
                message: "Promise receiver required",
            })?;
        if !matches!(promise.state, State::Pending) {
            return Ok(());
        }
        let job_value = value.clone();
        promise.state = if reject {
            State::Rejected(value)
        } else {
            State::Fulfilled(value)
        };
        let reactions = core::mem::take(&mut promise.reactions);
        let unhandled = reject && !promise.handled;
        self.reaction_count = self.reaction_count.saturating_sub(reactions.len());
        if unhandled {
            if self.rejections.len() >= self.limits.jobs {
                return Err(Error::Limit {
                    resource: "unhandled rejections",
                });
            }
            self.rejections.push(target.clone());
        }
        for reaction in reactions {
            self.enqueue(Job::Reaction {
                reaction,
                value: job_value.clone(),
                reject,
            })?;
        }
        Ok(())
    }
    fn react(&mut self, promise: &Value, reaction: Reaction) -> Result<(), Error> {
        let limit = self.limits.jobs;
        let total = self.reaction_count;
        let data = self
            .object_mut(promise)?
            .promise
            .as_mut()
            .ok_or(Error::Type {
                message: "Promise receiver required",
            })?;
        data.handled = true;
        let (value, reject) = match &data.state {
            State::Pending => {
                if total >= limit {
                    return Err(Error::Limit {
                        resource: "Promise reactions",
                    });
                }
                data.reactions.push(reaction);
                self.reaction_count = self.reaction_count.saturating_add(1);
                return Ok(());
            }
            State::Fulfilled(v) => (v.clone(), false),
            State::Rejected(v) => (v.clone(), true),
        };
        self.enqueue(Job::Reaction {
            reaction,
            value,
            reject,
        })
    }
    pub(super) fn exception_value(&mut self, error: Error) -> Result<Value, Error> {
        match error {
            Error::Thrown { value } => Ok(value),
            error @ (Error::Type { .. }
            | Error::Reference { .. }
            | Error::Range { .. }
            | Error::Syntax { .. }) => self.error_object(&error),
            other => Err(other),
        }
    }
    fn then(&mut self, promise: &Value, fulfilled: Value, rejected: Value) -> Result<Value, Error> {
        if !self.is_promise(promise) {
            return Err(Error::Type {
                message: "Promise receiver required",
            });
        }
        let ctor = self.promise_species(promise)?;
        self.native_roots.push(ctor.clone());
        let cap = self.new_capability(&ctor)?;
        self.react(
            promise,
            Reaction {
                fulfilled,
                rejected,
                target: cap.promise.clone(),
                resolve: cap.resolve,
                reject: cap.reject,
                resume: None,
            },
        )?;
        Ok(cap.promise)
    }
    pub(super) fn promise_resolve(&mut self, value: Value) -> Result<Value, Error> {
        self.promise_resolve_ctor(&native(Builtin::Promise), value)
    }
    fn promise_resolve_ctor(&mut self, ctor: &Value, value: Value) -> Result<Value, Error> {
        if !matches!(ctor, Value::Object(_) | Value::Function(_)) {
            return Err(Error::Type {
                message: "Promise.resolve receiver is not an object",
            });
        }
        if self.is_promise(&value)
            && self
                .get(&value, &Value::string("constructor").units())?
                .strictly_equals(ctor)
        {
            return Ok(value);
        }
        let roots = self.native_roots.len();
        self.native_roots.push(value.clone());
        self.native_roots.push(ctor.clone());
        let result = (|| {
            let cap = self.new_capability(ctor)?;
            self.call_sync(cap.resolve, Value::Undefined, &[value])?;
            Ok(cap.promise)
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn promise_call(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        if !matches!(
            builtin,
            Builtin::Promise
                | Builtin::PromiseResolve
                | Builtin::PromiseReject
                | Builtin::PromiseThen
                | Builtin::PromiseCatch
                | Builtin::PromiseFinally
                | Builtin::PromiseAll
                | Builtin::PromiseAllSettled
                | Builtin::PromiseRace
                | Builtin::InternalPromiseResolve
                | Builtin::PromiseWithResolvers
                | Builtin::PromiseSpecies
                | Builtin::InternalCallable
        ) {
            return Ok(None);
        }
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        self.native_roots.extend_from_slice(args);
        let result = self.promise_call_rooted(builtin, receiver, args);
        self.native_roots.truncate(roots);
        result.map(Some)
    }
    fn promise_call_rooted(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Value, Error> {
        let first = args.first().cloned().unwrap_or(Value::Undefined);
        let second = args.get(1).cloned().unwrap_or(Value::Undefined);
        match builtin {
            Builtin::PromiseAll | Builtin::PromiseAllSettled | Builtin::PromiseRace => {
                self.combinator(builtin, receiver, first)
            }
            Builtin::InternalPromiseResolve => self.promise_resolve_ctor(&first, second),
            Builtin::PromiseSpecies => Ok(receiver.clone()),
            Builtin::InternalCallable => {
                if matches!(first, Value::Function(_)) {
                    Ok(first)
                } else {
                    Err(Error::Type {
                        message: "Promise resolve is not callable",
                    })
                }
            }
            Builtin::PromiseFinally => self.promise_finally(receiver, first),
            Builtin::Promise => Err(Error::Type {
                message: "Promise requires new",
            }),
            Builtin::PromiseResolve | Builtin::PromiseReject | Builtin::PromiseWithResolvers => {
                if builtin == Builtin::PromiseResolve {
                    return self.promise_resolve_ctor(receiver, first);
                }
                let cap = self.new_capability(receiver)?;
                if builtin == Builtin::PromiseReject {
                    self.call_sync(cap.reject, Value::Undefined, &[first])?;
                    return Ok(cap.promise);
                }
                let proto = self.prototype()?;
                let result = self.allocate_object(proto)?;
                for (name, value) in [
                    ("promise", cap.promise),
                    ("resolve", cap.resolve),
                    ("reject", cap.reject),
                ] {
                    self.define(&result, Value::string(name).units(), Property::data(value))?;
                }
                Ok(result)
            }
            Builtin::PromiseThen => self.then(receiver, first, second),
            Builtin::PromiseCatch => {
                let then = self.get(receiver, &Value::string("then").units())?;
                self.call_sync(then, receiver.clone(), &[Value::Undefined, first])
            }
            _ => Err(Error::InvalidBytecode),
        }
    }
    pub(super) fn drain_jobs(&mut self) -> Result<(), Error> {
        let mut reported = None;
        while let Some(job) = self.jobs.pop_front() {
            self.charge(1)?;
            let roots = self.native_roots.len();
            for value in job.roots() {
                self.native_roots.push(value.clone());
            }
            let microtask = matches!(job, Job::Microtask(_));
            let result = self.run_job(job);
            if let Err(error) = &result
                && microtask
                && super::iterators::language_error(error)
            {
                if let Error::Thrown { value } = error {
                    self.native_roots.push(value.clone());
                    self.retain_result(value.clone())?;
                }
                if reported.is_none() {
                    reported = Some(error.clone());
                }
            }
            self.native_roots.truncate(roots);
            if let Err(error) = result
                && (!microtask || !super::iterators::language_error(&error))
            {
                return Err(error);
            }
        }
        for promise in core::mem::take(&mut self.rejections) {
            if let Some(data) = &self.object_ref(&promise)?.promise
                && !data.handled
                && let State::Rejected(value) = &data.state
            {
                let value = value.clone();
                self.retain_result(value.clone())?;
                let result = self
                    .host
                    .unhandled_rejection(&value)
                    .map(|()| Value::Undefined);
                self.validate_host_result(&result)?;
                if let Err(error) = result {
                    if !super::iterators::language_error(&error) {
                        return Err(error);
                    }
                    if reported.is_none() {
                        reported = Some(error);
                    }
                }
            }
        }
        reported.map_or(Ok(()), Err)
    }
    fn run_job(&mut self, job: Job) -> Result<(), Error> {
        match job {
            Job::Microtask(callback) => {
                if let Err(error) = self.call_sync(callback, Value::Undefined, &[]) {
                    let reason = self.exception_value(error)?;
                    self.retain_result(reason.clone())?;
                    let report = self
                        .host
                        .report_exception(&reason)
                        .map(|()| Value::Undefined);
                    self.validate_host_result(&report)?;
                    report?;
                }
                Ok(())
            }
            Job::Reaction {
                reaction,
                value,
                reject,
            } => {
                if let Some(handle) = reaction.resume {
                    return self.resume_async(handle, value, reject);
                }
                let handler = if reject {
                    reaction.rejected
                } else {
                    reaction.fulfilled
                };
                if !matches!(handler, Value::Function(_)) {
                    let settle = if reject {
                        reaction.reject
                    } else {
                        reaction.resolve
                    };
                    self.call_sync(settle, Value::Undefined, &[value])?;
                    return Ok(());
                }
                let (settle, value) = match self.call_sync(handler, Value::Undefined, &[value]) {
                    Ok(result) => (reaction.resolve, result),
                    Err(error) => {
                        let reason = self.exception_value(error)?;
                        (reaction.reject, reason)
                    }
                };
                self.call_sync(settle, Value::Undefined, &[value])?;
                Ok(())
            }
            Job::Thenable {
                target,
                object,
                then,
            } => {
                let (resolve, reject) = self.resolving_functions(&target)?;
                self.native_roots.extend([resolve.clone(), reject.clone()]);
                if let Err(error) = self.call_sync(then, object, &[resolve, reject.clone()]) {
                    let reason = self.exception_value(error)?;
                    self.call_sync(reject, Value::Undefined, &[reason])?;
                }
                Ok(())
            }
        }
    }

    fn combinator(&mut self, builtin: Builtin, ctor: &Value, input: Value) -> Result<Value, Error> {
        let cap = self.new_capability(ctor)?;
        let code = if let Some((_, code)) =
            self.intrinsic_code.iter().find(|(id, _)| *id == builtin)
        {
            code.clone()
        } else {
            let action = match builtin {
                Builtin::PromiseRace => "p.then(resolve,reject);",
                Builtin::PromiseAll => {
                    "p.then(function(value){if(called)return;called=true;define(values,i,value);remaining--;if(remaining===0)resolve(values);},reject);"
                }
                _ => {
                    "p.then(function(value){if(called)return;called=true;define(values,i,{status:'fulfilled',value});remaining--;if(remaining===0)resolve(values);},function(reason){if(called)return;called=true;define(values,i,{status:'rejected',reason});remaining--;if(remaining===0)resolve(values);});"
                }
            };
            let finish = if builtin == Builtin::PromiseRace {
                ""
            } else {
                "remaining--;if(remaining===0)resolve(values);"
            };
            let initialize = if builtin == Builtin::PromiseRace {
                ""
            } else {
                "let values=[];let remaining=1;let index=0;"
            };
            let element = if builtin == Builtin::PromiseRace {
                ""
            } else {
                "let i=index++;define(values,i,undefined);remaining++;let called=false;"
            };
            let source = alloc::format!(
                "(function(input,P,define,resolve,reject,invoke,callable){{'use strict';try{{{initialize}let resolver=callable(P.resolve);for(let item of input){{{element}let p=invoke(resolver,P,item);{action}}}{finish}}}catch(e){{invoke(reject,undefined,e)}}}})"
            );
            let program = crate::compile(&source, crate::Limits::default())?;
            let code = program
                .functions
                .first()
                .ok_or(Error::InvalidBytecode)?
                .clone();
            self.intrinsic_code.push((builtin, code.clone()));
            code
        };
        self.reserve()?;
        let handle = self.heap.allocate(
            Node::Function {
                code,
                captures: Vec::new(),
                lexical_this: None,
                object: Object::new(Value::Null),
            },
            self.limits.heap_entries,
        )?;
        let function = Value::Function(FunctionValue(Callable::Script {
            handle,
            owner: self.owner.clone(),
        }));
        self.call_sync(
            function,
            Value::Undefined,
            &[
                input,
                ctor.clone(),
                native(Builtin::InternalDefine),
                cap.resolve,
                cap.reject,
                native(Builtin::InternalCall),
                native(Builtin::InternalCallable),
            ],
        )?;
        Ok(cap.promise)
    }

    fn promise_finally(&mut self, promise: &Value, callback: Value) -> Result<Value, Error> {
        if !matches!(promise, Value::Object(_) | Value::Function(_)) {
            return Err(Error::Type {
                message: "Promise.finally receiver is not an object",
            });
        }
        let ctor = self.promise_species(promise)?;
        self.native_roots.push(ctor.clone());
        if !matches!(callback, Value::Function(_)) {
            let then = self.get(promise, &Value::string("then").units())?;
            return self.call_sync(then, promise.clone(), &[callback.clone(), callback]);
        }
        let code = if let Some((_, code)) = self
            .intrinsic_code
            .iter()
            .find(|(id, _)| *id == Builtin::PromiseFinally)
        {
            code.clone()
        } else {
            let source = "(function(p,callback,invoke,resolve,C){'use strict';return p.then(function(value){let result=invoke(callback,undefined);return resolve(C,result).then(function(){return value;});},function(reason){let result=invoke(callback,undefined);return resolve(C,result).then(function(){throw reason;});});})";
            let program = crate::compile(source, crate::Limits::default())?;
            let code = program
                .functions
                .first()
                .ok_or(Error::InvalidBytecode)?
                .clone();
            self.intrinsic_code
                .push((Builtin::PromiseFinally, code.clone()));
            code
        };
        self.reserve()?;
        let handle = self.heap.allocate(
            Node::Function {
                code,
                captures: Vec::new(),
                lexical_this: None,
                object: Object::new(Value::Null),
            },
            self.limits.heap_entries,
        )?;
        let function = Value::Function(FunctionValue(Callable::Script {
            handle,
            owner: self.owner.clone(),
        }));
        self.call_sync(
            function,
            Value::Undefined,
            &[
                promise.clone(),
                callback,
                native(Builtin::InternalCall),
                native(Builtin::InternalPromiseResolve),
                ctor,
            ],
        )
    }
}

impl Execution<'_> {
    pub(super) fn suspend_await(&mut self, value: Value, pc: usize) -> Result<(), Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(value.clone());
        let result = (|| {
            let awaited = self.promise_resolve(value)?;
            self.native_roots.push(awaited.clone());
            self.reserve()?;
            let mut frame = self.frames.pop().ok_or(Error::InvalidBytecode)?;
            let promise = frame.async_promise.clone().ok_or(Error::InvalidBytecode)?;
            let base = frame.base;
            frame.pc = pc;
            let stack = self.stack.split_off(base);
            self.suspended_slots = self
                .suspended_slots
                .checked_add(stack.len())
                .filter(|n| *n <= self.limits.stack)
                .ok_or(Error::Limit {
                    resource: "suspended operand stack",
                })?;
            for handler in &mut frame.handlers {
                handler.rebase(base, 0);
            }
            frame.base = 0;
            let handle = self.heap.allocate(
                Node::Suspended(Some(alloc::boxed::Box::new(Suspension { frame, stack }))),
                self.limits.heap_entries,
            )?;
            self.suspended_frames = self.suspended_frames.saturating_add(1);
            self.react(
                &awaited,
                Reaction {
                    fulfilled: Value::Undefined,
                    rejected: Value::Undefined,
                    target: promise.clone(),
                    resolve: Value::Undefined,
                    reject: Value::Undefined,
                    resume: Some(handle),
                },
            )?;
            self.push(promise)
        })();
        self.native_roots.truncate(roots);
        result
    }
    fn resume_async(&mut self, handle: Handle, value: Value, reject: bool) -> Result<(), Error> {
        let Node::Suspended(slot) = self.heap.get_mut(handle)? else {
            return Err(Error::InvalidBytecode);
        };
        let Some(state) = slot.take() else {
            return Err(Error::InvalidBytecode);
        };
        let Suspension { mut frame, stack } = *state;
        self.suspended_slots = self.suspended_slots.saturating_sub(stack.len());
        self.suspended_frames = self.suspended_frames.saturating_sub(1);
        let boundary = self.frames.len();
        let base = self.stack.len();
        for handler in &mut frame.handlers {
            handler.rebase(0, base);
        }
        frame.base = base;
        let mut pc = frame.pc;
        self.frames.push(frame);
        self.stack.extend(stack);
        let old_boundary = self.unwind_boundary;
        self.unwind_boundary = boundary;
        let program = self.top_program.clone().ok_or(Error::InvalidBytecode)?;
        let result = (|| {
            if reject {
                self.abrupt_until(
                    Completion::Throw(Error::Thrown { value }),
                    &mut pc,
                    boundary,
                )?;
                if self.frames.len() > boundary {
                    self.frames.last_mut().ok_or(Error::InvalidBytecode)?.pc = pc;
                }
            } else {
                self.push(value)?;
            }
            if self.frames.len() > boundary {
                self.execute(&program, boundary)?;
            } else {
                self.pop()?;
            }
            Ok(())
        })();
        self.unwind_boundary = old_boundary;
        self.stack.truncate(base);
        result
    }
}

const fn native(builtin: Builtin) -> Value {
    Value::Function(FunctionValue::native(builtin))
}
