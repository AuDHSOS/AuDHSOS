// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The Promise objects of 27.2 and the job queue of 9.5 they settle through.
//!
//! Every record clause 27.2 keeps in a specification type lives in an Array of
//! the heap, so the collector traces it like any other object and no structure
//! beside the heap holds a reference:
//!
//! | Record | 27.2 | Layout |
//! | --- | --- | --- |
//! | Capability | 27.2.1.1 | `[promise, resolve, reject]` |
//! | Resolving pair state | 27.2.1.3.1 | `[promise, alreadyResolved]` |
//! | Reaction | 27.2.1.2 | `[handler, capability]` |
//! | Job | 9.5 | `[kind, function, receiver, first, second, capability]` |
//!
//! A reaction is held in the list its type names, so the record carries no
//! `[[Type]]`: the list it was taken from decides the `kind` of the job it
//! becomes.

use super::{
    heap::{GenerationalHeap, HeapError},
    object::ObjectKind,
    realm::{Intrinsic, Realm},
    value::{ObjectRef, VALUE_FALSE, VALUE_TRUE, VALUE_UNDEFINED, Value},
};

/// `[[PromiseState]]` pending.
pub const PENDING: u8 = 0;
/// `[[PromiseState]]` fulfilled.
pub const FULFILLED: u8 = 1;
/// `[[PromiseState]]` rejected.
pub const REJECTED: u8 = 2;

/// A job of 27.2.2.1 whose handler answers into `capability.resolve`.
pub const JOB_FULFILL: i32 = 1;
/// A job of 27.2.2.1 whose handler answers into `capability.reject`.
pub const JOB_REJECT: i32 = 2;
/// The job of 27.2.2.2, which calls `then` with the two resolving functions.
pub const JOB_THENABLE: i32 = 3;

/// Index of `kind` in a job record.
pub const JOB_KIND: u32 = 0;
/// Index of the function a job calls.
pub const JOB_FUNCTION: u32 = 1;
/// Index of the `this` value a job calls with.
pub const JOB_RECEIVER: u32 = 2;
/// Index of the first argument a job calls with.
pub const JOB_FIRST: u32 = 3;
/// Index of the second argument a job calls with.
pub const JOB_SECOND: u32 = 4;
/// Index of the capability a job settles.
pub const JOB_CAPABILITY: u32 = 5;
/// Number of slots in a job record.
pub const JOB_SLOTS: u32 = 6;

/// Index of `[[Promise]]` in a capability record.
pub const CAPABILITY_PROMISE: u32 = 0;
/// Index of `[[Resolve]]` in a capability record.
pub const CAPABILITY_RESOLVE: u32 = 1;
/// Index of `[[Reject]]` in a capability record.
pub const CAPABILITY_REJECT: u32 = 2;

/// Index of `[[Handler]]` in a reaction record.
pub const REACTION_HANDLER: u32 = 0;
/// Index of `[[Capability]]` in a reaction record.
pub const REACTION_CAPABILITY: u32 = 1;

/// Index of `[[Values]]` in the record the element functions of 27.2.4.1
/// share.
pub const GROUP_VALUES: u32 = 0;
/// Index of `[[Capability]]` in that record.
pub const GROUP_CAPABILITY: u32 = 1;
/// Index of `[[RemainingElements]]` in that record.
pub const GROUP_REMAINING: u32 = 2;

/// Index of the shared record in the state of one element function.
pub const ELEMENT_GROUP: u32 = 0;
/// Index of `[[Index]]` in that state.
pub const ELEMENT_INDEX: u32 = 1;
/// Index of `[[AlreadyCalled]]` in that state.
pub const ELEMENT_CALLED: u32 = 2;

/// Index of `[[Promise]]` in the state a pair of resolving functions shares.
pub const STATE_PROMISE: u32 = 0;
/// Index of `[[AlreadyResolved]]` in that state.
pub const STATE_RESOLVED: u32 = 1;

/// One element of an Array of the heap, undefined where the index is absent.
///
/// Every Array this module makes is packed and written only here, so an index
/// below its length always answers a slot.
#[must_use]
pub fn slot(heap: &GenerationalHeap, array: Value, index: u32) -> Value {
    let Some(object) = array.as_object() else {
        return VALUE_UNDEFINED;
    };
    heap.get_object(object)
        .and_then(|object| object.elements)
        .and_then(|elements| heap.get_elements(elements))
        .and_then(|elements| elements.get(index))
        .unwrap_or(VALUE_UNDEFINED)
}

/// Writes one element of such an Array.
///
/// # Errors
///
/// Returns [`HeapError::InvalidReference`] unless `array` is a live Array.
pub fn set_slot(
    heap: &mut GenerationalHeap,
    array: Value,
    index: u32,
    value: Value,
) -> Result<(), HeapError> {
    let object = array.as_object().ok_or(HeapError::InvalidReference)?;
    heap.set_array_element(object, index, value)
}

/// Builds an Array of the heap holding `values`.
///
/// # Errors
///
/// The errors of the allocation.
pub fn record(
    heap: &mut GenerationalHeap,
    realm: &Realm,
    values: &[Value],
) -> Result<Value, HeapError> {
    let length = u32::try_from(values.len()).unwrap_or(0);
    let array = realm.array(heap, length)?;
    for (index, value) in values.iter().enumerate() {
        heap.set_array_element(array, u32::try_from(index).unwrap_or(0), *value)?;
    }
    Ok(Value::from_object(array))
}

/// One element function of 27.2.4.1.3, 27.2.4.2.2 or 27.2.4.2.3, which
/// carries the shared record, its index and `[[AlreadyCalled]]`.
///
/// # Errors
///
/// The errors of the allocation.
pub fn element_function(
    heap: &mut GenerationalHeap,
    realm: &Realm,
    intrinsic: Intrinsic,
    group: Value,
    index: u32,
) -> Result<Value, HeapError> {
    let state = record(
        heap,
        realm,
        &[group, Value::from_smi(index_as_smi(index)), VALUE_FALSE],
    )?;
    let parent = realm.function_prototype(heap)?;
    // 17 gives each of these closures the `length` its own clause names.
    let function = heap.allocate_native(parent, intrinsic.id(), intrinsic.length(), state)?;
    name_the_function(heap, function)?;
    Ok(Value::from_object(function))
}

/// An index as the Number a slot of a record carries.
fn index_as_smi(index: u32) -> i32 {
    i32::try_from(index).unwrap_or(i32::MAX)
}

/// The `[[PromiseState]]` of a value that is a Promise, and nothing for one
/// that is not (27.2.4.4 step 2).
#[must_use]
pub fn state_of(value: Value, heap: &GenerationalHeap) -> Option<u8> {
    let object = value.as_object()?;
    match heap.get_object(object)?.kind {
        ObjectKind::Promise { state, .. } => Some(state),
        _ => None,
    }
}

/// 27.2.4.7.1: a new pending Promise with two empty reaction lists.
///
/// # Errors
///
/// The errors of the allocation.
pub fn create(
    heap: &mut GenerationalHeap,
    realm: &Realm,
    prototype: Value,
) -> Result<ObjectRef, HeapError> {
    let fulfill = Value::from_object(realm.array(heap, 0)?);
    let reject = Value::from_object(realm.array(heap, 0)?);
    heap.allocate_promise(prototype, fulfill, reject)
}

/// 27.2.1.3: the pair of functions that settles `promise`, sharing one state.
///
/// # Errors
///
/// The errors of the allocation.
pub fn resolving_functions(
    heap: &mut GenerationalHeap,
    realm: &Realm,
    promise: Value,
) -> Result<(Value, Value), HeapError> {
    let state = record(heap, realm, &[promise, VALUE_FALSE])?;
    let parent = realm.function_prototype(heap)?;
    let resolve = heap.allocate_native(parent, Intrinsic::PromiseResolveFunction.id(), 1, state)?;
    name_the_function(heap, resolve)?;
    let reject = heap.allocate_native(parent, Intrinsic::PromiseRejectFunction.id(), 1, state)?;
    name_the_function(heap, reject)?;
    Ok((Value::from_object(resolve), Value::from_object(reject)))
}

/// 27.2.1.5 for a constructor that is not `%Promise%`: the empty capability
/// record of 27.2.1.1 and the executor of 27.2.1.5.1 that fills it.
///
/// The constructor takes the executor and calls it with the two functions it
/// settles its own promise through, which the executor writes into the
/// record; the caller reads them off it once the construct has answered.
///
/// # Errors
///
/// The errors of the allocation.
pub fn capability_executor(
    heap: &mut GenerationalHeap,
    realm: &Realm,
) -> Result<(Value, Value), HeapError> {
    let capability = record(
        heap,
        realm,
        &[VALUE_UNDEFINED, VALUE_UNDEFINED, VALUE_UNDEFINED],
    )?;
    let parent = realm.function_prototype(heap)?;
    let executor =
        heap.allocate_native(parent, Intrinsic::CapabilitiesExecutor.id(), 2, capability)?;
    let flags = super::realm::builtin_metadata();
    let key = super::value::PropertyKey::String(heap.strings.intern("length")?);
    heap.define_own_named(executor, key, Value::from_smi(2), flags)?;
    let empty = heap.strings.allocate_str("")?;
    let key = super::value::PropertyKey::String(heap.strings.intern("name")?);
    heap.define_own_named(executor, key, Value::from_string(empty), flags)?;
    Ok((capability, Value::from_object(executor)))
}

/// The `length` and `name` 10.3.3 gives an anonymous built-in function, in the
/// order it creates them.
fn name_the_function(
    heap: &mut GenerationalHeap,
    function: super::value::ObjectRef,
) -> Result<(), HeapError> {
    let flags = super::realm::builtin_metadata();
    let key = super::value::PropertyKey::String(heap.strings.intern("length")?);
    heap.define_own_named(function, key, Value::from_smi(1), flags)?;

    let empty = heap.strings.allocate_str("")?;
    let key = super::value::PropertyKey::String(heap.strings.intern("name")?);
    heap.define_own_named(function, key, Value::from_string(empty), flags)?;
    Ok(())
}

/// 27.2.1.5 for `%Promise%`: a new Promise and the pair that settles it.
///
/// # Errors
///
/// The errors of the allocation.
pub fn capability(heap: &mut GenerationalHeap, realm: &Realm) -> Result<Value, HeapError> {
    let prototype = realm.promise_prototype(heap)?;
    let promise = Value::from_object(create(heap, realm, prototype)?);
    let (resolve, reject) = resolving_functions(heap, realm, promise)?;
    record(heap, realm, &[promise, resolve, reject])
}

/// The reaction list of a settled state, which decides which jobs 27.2.1.8
/// enqueues.
const fn list_of(state: u8, fulfill: Value, reject: Value) -> Value {
    if state == FULFILLED { fulfill } else { reject }
}

/// 27.2.1.4 and 27.2.1.7: settles `promise` and enqueues its reactions.
///
/// The value is held in `[[PromiseResult]]`, both lists are dropped, and every
/// reaction of the list this state names becomes a job.
///
/// # Errors
///
/// The errors of the allocation.
pub fn settle(
    heap: &mut GenerationalHeap,
    realm: &Realm,
    queue: Value,
    promise: Value,
    state: u8,
    value: Value,
) -> Result<(), HeapError> {
    let object = promise.as_object().ok_or(HeapError::InvalidReference)?;
    // 27.2.1.4 step 1 asserts the state is pending: a second settling of the
    // same promise is what [[AlreadyResolved]] and the settled lists prevent,
    // so it reaches nothing here.
    let Some(&ObjectKind::Promise {
        state: PENDING,
        handled: was_handled,
        fulfill,
        reject,
        ..
    }) = heap.get_object(object).map(|object| &object.kind)
    else {
        return Ok(());
    };
    heap.set_object_kind(
        object,
        ObjectKind::Promise {
            state,
            handled: was_handled,
            value,
            fulfill: VALUE_UNDEFINED,
            reject: VALUE_UNDEFINED,
        },
    )?;
    let list = list_of(state, fulfill, reject);
    let kind = if state == FULFILLED {
        JOB_FULFILL
    } else {
        JOB_REJECT
    };
    let count = length_of(heap, list);
    for index in 0..count {
        let reaction = slot(heap, list, index);
        let taken = slot(heap, reaction, REACTION_HANDLER);
        let capability = slot(heap, reaction, REACTION_CAPABILITY);
        let job = record(
            heap,
            realm,
            &[
                Value::from_smi(kind),
                taken,
                VALUE_UNDEFINED,
                value,
                VALUE_UNDEFINED,
                capability,
            ],
        )?;
        enqueue(heap, queue, job)?;
    }
    Ok(())
}

/// The number of elements of an Array of the heap.
#[must_use]
pub fn length_of(heap: &GenerationalHeap, array: Value) -> u32 {
    let Some(object) = array.as_object() else {
        return 0;
    };
    match heap.get_object(object).map(|object| &object.kind) {
        Some(&ObjectKind::Array { length, .. }) => length,
        _ => 0,
    }
}

/// Appends a job record to the queue of 9.5.
///
/// # Errors
///
/// The errors of the write.
pub fn enqueue(heap: &mut GenerationalHeap, queue: Value, job: Value) -> Result<(), HeapError> {
    let index = length_of(heap, queue);
    set_slot(heap, queue, index, job)
}

/// 27.2.1.3.2: resolves `promise` with `value`.
///
/// A value that is no thenable fulfils the promise. One that is enqueues the
/// job of 27.2.2.2, which calls its `then` with the pair of 27.2.1.3.
///
/// # Errors
///
/// The errors of the allocation.
pub fn resolve(
    heap: &mut GenerationalHeap,
    realm: &Realm,
    queue: Value,
    promise: Value,
    value: Value,
    then: Option<Value>,
) -> Result<(), HeapError> {
    // 27.2.1.3.2 step 7: a `then` that is not callable fulfils the promise
    // with the object itself, which is what `None` says here.
    let Some(then) = then else {
        return settle(heap, realm, queue, promise, FULFILLED, value);
    };
    let (resolve, reject) = resolving_functions(heap, realm, promise)?;
    let job = record(
        heap,
        realm,
        &[
            Value::from_smi(JOB_THENABLE),
            then,
            value,
            resolve,
            reject,
            VALUE_UNDEFINED,
        ],
    )?;
    enqueue(heap, queue, job)
}

/// Whether `[[AlreadyResolved]]` of the pair was already set, setting it.
///
/// # Errors
///
/// The errors of the write.
pub fn take_resolution(heap: &mut GenerationalHeap, state: Value) -> Result<bool, HeapError> {
    if slot(heap, state, STATE_RESOLVED) == VALUE_TRUE {
        return Ok(true);
    }
    set_slot(heap, state, STATE_RESOLVED, VALUE_TRUE)?;
    Ok(false)
}

/// 27.2.5.4.1: adds a reaction to a pending promise, or enqueues the job a
/// settled one owes.
///
/// # Errors
///
/// The errors of the allocation.
pub fn react(
    heap: &mut GenerationalHeap,
    realm: &Realm,
    queue: Value,
    promise: Value,
    on_fulfilled: Value,
    on_rejected: Value,
    capability: Value,
) -> Result<(), HeapError> {
    let object = promise.as_object().ok_or(HeapError::InvalidReference)?;
    let Some(&ObjectKind::Promise {
        state,
        value,
        fulfill,
        reject,
        ..
    }) = heap.get_object(object).map(|object| &object.kind)
    else {
        return Err(HeapError::InvalidReference);
    };
    if state == PENDING {
        let fulfill_reaction = record(heap, realm, &[on_fulfilled, capability])?;
        let reject_reaction = record(heap, realm, &[on_rejected, capability])?;
        let index = length_of(heap, fulfill);
        set_slot(heap, fulfill, index, fulfill_reaction)?;
        let index = length_of(heap, reject);
        set_slot(heap, reject, index, reject_reaction)?;
    } else {
        let (kind, handler) = if state == FULFILLED {
            (JOB_FULFILL, on_fulfilled)
        } else {
            (JOB_REJECT, on_rejected)
        };
        let job = record(
            heap,
            realm,
            &[
                Value::from_smi(kind),
                handler,
                VALUE_UNDEFINED,
                value,
                VALUE_UNDEFINED,
                capability,
            ],
        )?;
        enqueue(heap, queue, job)?;
    }
    // 27.2.5.4.1 step 12 marks the promise handled whatever its state is.
    mark_handled(heap, object)
}

/// Sets `[[PromiseIsHandled]]`.
///
/// # Errors
///
/// The errors of the write.
pub fn mark_handled(heap: &mut GenerationalHeap, promise: ObjectRef) -> Result<(), HeapError> {
    let Some(&ObjectKind::Promise {
        state,
        value,
        fulfill,
        reject,
        ..
    }) = heap.get_object(promise).map(|object| &object.kind)
    else {
        return Err(HeapError::InvalidReference);
    };
    heap.set_object_kind(
        promise,
        ObjectKind::Promise {
            state,
            handled: true,
            value,
            fulfill,
            reject,
        },
    )
}
