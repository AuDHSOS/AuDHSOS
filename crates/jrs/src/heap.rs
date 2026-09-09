// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Non-moving mark/sweep heap with generation-checked indices. Nodes own no
//! other nodes: cycles in captured bindings are traced and can be reclaimed.

use crate::{
    Error, Value,
    bytecode::{FunctionCode, Slot},
    object::Object,
};
use alloc::{collections::BTreeMap, rc::Rc, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Handle {
    index: usize,
    generation: u64,
}

pub(crate) enum Node {
    Symbol,
    HostFunction {
        behavior: HostBehavior,
        name: alloc::rc::Rc<[u16]>,
        object: Object,
    },
    Cell {
        slot: Slot,
        value: Option<Value>,
    },
    Function {
        code: Rc<FunctionCode>,
        captures: Vec<Handle>,
        lexical_this: Option<Value>,
        object: Object,
    },
    Object(Object),
    Bound {
        target: Value,
        constructible: bool,
        receiver: Value,
        arguments: Vec<Value>,
        object: Object,
    },
    Resolving {
        target: Value,
        used: bool,
        object: Object,
    },
    Suspended(Option<alloc::boxed::Box<crate::vm::promises::Suspension>>),
}

#[derive(Clone, Copy)]
pub(crate) enum HostBehavior {
    AsyncFunctionConstructor,
    Capability(u32),
    QueueMicrotask,
    CollectGarbage,
    StringPrint,
    EvalScript,
    Intrinsic(crate::bytecode::Builtin),
    EventGet(crate::event::Field),
    EventSet(crate::event::Field),
    DomExceptionGet(crate::event::ExceptionField),
    AbortGet(crate::event::AbortField),
    AbortSet,
    RegExpGet(crate::regexp::Field),
}

struct Entry {
    generation: u64,
    marked: bool,
    node: Option<Node>,
}

#[derive(Default)]
pub(crate) struct Heap {
    entries: Vec<Entry>,
    free: Vec<usize>,
    work: Vec<Handle>,
}

impl Heap {
    /// Live async resources after sweeping, including pending reactions.
    pub(crate) fn async_usage(&self) -> (usize, usize, usize, usize) {
        let (mut frames, mut slots, mut operands, mut reactions) = (0usize, 0usize, 0usize, 0usize);
        for entry in &self.entries {
            match entry.node.as_ref() {
                Some(Node::Suspended(Some(state))) => {
                    let (bindings, stack) = state.sizes();
                    frames = frames.saturating_add(1);
                    slots = slots.saturating_add(bindings);
                    operands = operands.saturating_add(stack);
                }
                Some(Node::Object(object)) => {
                    if let Some(promise) = &object.promise {
                        reactions = reactions.saturating_add(promise.reactions.len());
                    }
                }
                _ => {}
            }
        }
        (frames, slots, operands, reactions)
    }
    pub(crate) fn allocate(&mut self, node: Node, limit: usize) -> Result<Handle, Error> {
        if let Some(index) = self.free.pop() {
            let entry = self.entries.get_mut(index).ok_or(Error::InvalidBytecode)?;
            entry.node = Some(node);
            return Ok(Handle {
                index,
                generation: entry.generation,
            });
        }
        if self.entries.len() >= limit {
            return Err(Error::Limit {
                resource: "heap entries",
            });
        }
        let index = self.entries.len();
        self.entries.push(Entry {
            generation: 0,
            marked: false,
            node: Some(node),
        });
        Ok(Handle {
            index,
            generation: 0,
        })
    }
    pub(crate) const fn full(&self, limit: usize) -> bool {
        self.free.is_empty() && self.entries.len() >= limit
    }
    pub(crate) fn get(&self, handle: Handle) -> Result<&Node, Error> {
        self.entries
            .get(handle.index)
            .filter(|e| e.generation == handle.generation)
            .and_then(|e| e.node.as_ref())
            .ok_or(Error::InvalidBytecode)
    }
    pub(crate) fn get_mut(&mut self, handle: Handle) -> Result<&mut Node, Error> {
        self.entries
            .get_mut(handle.index)
            .filter(|e| e.generation == handle.generation)
            .and_then(|e| e.node.as_mut())
            .ok_or(Error::InvalidBytecode)
    }
    pub(crate) fn mark(&mut self, handle: Handle) {
        self.work.push(handle);
    }
    pub(crate) fn mark_value(&mut self, value: &Value) {
        if let Some(handle) = value.heap_handle() {
            self.mark(handle);
        }
    }
    pub(crate) fn collect(&mut self, fuel: &mut u64) -> Result<(), Error> {
        // Each ephemeron is registered once when its map becomes live. Marking
        // a key activates its pending values. No repeated full-heap fixpoint.
        let mut pending: BTreeMap<Handle, Vec<Handle>> = BTreeMap::new();
        let mut maps = Vec::new();
        while let Some(handle) = self.work.pop() {
            charge(fuel, 1)?;
            let entry = self
                .entries
                .get_mut(handle.index)
                .filter(|e| e.generation == handle.generation)
                .ok_or(Error::InvalidBytecode)?;
            if entry.marked {
                continue;
            }
            entry.marked = true;
            if let Some(values) = pending.remove(&handle) {
                self.work.extend(values);
            }
            match entry.node.as_ref().ok_or(Error::InvalidBytecode)? {
                Node::Symbol => {}
                Node::HostFunction { object, .. } => trace_object(object, &mut self.work),
                Node::Cell { value, .. } => self
                    .work
                    .extend(value.iter().filter_map(Value::heap_handle)),
                Node::Function {
                    captures,
                    lexical_this,
                    object,
                    ..
                } => {
                    self.work.extend(captures.iter().copied());
                    self.work
                        .extend(lexical_this.iter().filter_map(Value::heap_handle));
                    trace_object(object, &mut self.work);
                }
                Node::Object(object) => {
                    trace_object(object, &mut self.work);
                    if let Some(map) = &object.weakmap {
                        maps.push(handle);
                        for (key, value) in &map.entries {
                            charge(fuel, 1)?;
                            let Some(value) = value.heap_handle() else {
                                continue;
                            };
                            match key {
                                crate::weakmap::Key::Native(_) => self.work.push(value),
                                crate::weakmap::Key::Heap(key, _) => {
                                    pending.entry(*key).or_default().push(value);
                                }
                            }
                        }
                    }
                }
                Node::Bound {
                    target,
                    receiver,
                    arguments,
                    object,
                    ..
                } => {
                    self.work.extend(target.heap_handle());
                    self.work.extend(receiver.heap_handle());
                    for value in arguments {
                        self.work.extend(value.heap_handle());
                    }
                    trace_object(object, &mut self.work);
                }
                Node::Resolving { target, object, .. } => {
                    self.work.extend(target.heap_handle());
                    trace_object(object, &mut self.work);
                }
                Node::Suspended(state) => {
                    if let Some(state) = state {
                        state.trace(&mut self.work);
                    }
                }
            }
            self.activate_marked_keys(handle, &mut pending)?;
            self.trace_abort_dependents(handle, fuel)?;
        }
        self.sweep_weakmaps(&maps, fuel)?;
        charge(fuel, self.entries.len())?;
        for (index, entry) in self.entries.iter_mut().enumerate() {
            if entry.marked {
                entry.marked = false;
            } else if entry.node.take().is_some() {
                // Never reuse a generation after overflow.
                if let Some(next) = entry.generation.checked_add(1) {
                    entry.generation = next;
                    self.free.push(index);
                }
            }
        }
        Ok(())
    }

    fn activate_marked_keys(
        &mut self,
        handle: Handle,
        pending: &mut BTreeMap<Handle, Vec<Handle>>,
    ) -> Result<(), Error> {
        // A map can be reached after its key. Activate already marked keys
        // from this map without rescanning other maps or pending entries.
        if let Node::Object(object) = self.get(handle)?
            && let Some(map) = &object.weakmap
        {
            let keys: Vec<_> = map
                .entries
                .keys()
                .filter_map(|key| match key {
                    crate::weakmap::Key::Heap(key, _) => Some(*key),
                    crate::weakmap::Key::Native(_) => None,
                })
                .collect();
            for key in keys {
                if self.marked(key)
                    && let Some(values) = pending.remove(&key)
                {
                    self.work.extend(values);
                }
            }
        }
        Ok(())
    }

    fn trace_abort_dependents(&mut self, handle: Handle, fuel: &mut u64) -> Result<(), Error> {
        let Node::Object(object) = self.get(handle)? else {
            return Ok(());
        };
        let Some(signal) = &object.abort_signal else {
            return Ok(());
        };
        charge(fuel, signal.dependents.len())?;
        let mut live = Vec::new();
        for dependent in &signal.dependents {
            if let Ok(Node::Object(object)) = self.get(*dependent) {
                charge(
                    fuel,
                    object
                        .listeners
                        .as_ref()
                        .map_or(0, audhsos_event_target::Listeners::len),
                )?;
            }
            if let Ok(Node::Object(object)) = self.get(*dependent)
                && let Some(signal) = &object.abort_signal
                && !signal.aborted()
                && (!signal.removals.is_empty()
                    || object.listeners.as_ref().is_some_and(|ls| {
                        ls.iter()
                            .any(|l| l.event_type.as_ref() == [97, 98, 111, 114, 116])
                    }))
            {
                live.push(*dependent);
            }
        }
        self.work.extend(live);
        Ok(())
    }

    fn sweep_weakmaps(&mut self, maps: &[Handle], fuel: &mut u64) -> Result<(), Error> {
        for map in maps {
            let Node::Object(object) = self.get(*map)? else {
                return Err(Error::InvalidBytecode);
            };
            let entries = &object
                .weakmap
                .as_ref()
                .ok_or(Error::InvalidBytecode)?
                .entries;
            charge(fuel, entries.len())?;
            let dead: Vec<_> = entries.keys().filter(|key| {
                matches!(key, crate::weakmap::Key::Heap(handle, _) if !self.marked(*handle))
            }).copied().collect();
            let Node::Object(object) = self.get_mut(*map)? else {
                return Err(Error::InvalidBytecode);
            };
            for key in dead {
                object
                    .weakmap
                    .as_mut()
                    .ok_or(Error::InvalidBytecode)?
                    .entries
                    .remove(&key);
            }
        }
        Ok(())
    }

    fn marked(&self, handle: Handle) -> bool {
        self.entries
            .get(handle.index)
            .is_some_and(|e| e.generation == handle.generation && e.marked)
    }

    pub(crate) fn weak_entries(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|entry| match entry.node.as_ref() {
                Some(Node::Object(object)) => object.weakmap.as_ref(),
                _ => None,
            })
            .fold(0usize, |n, map| n.saturating_add(map.entries.len()))
    }
}

fn charge(fuel: &mut u64, amount: usize) -> Result<(), Error> {
    *fuel = fuel
        .checked_sub(u64::try_from(amount).unwrap_or(u64::MAX))
        .ok_or(Error::Limit {
            resource: "garbage collection work",
        })?;
    Ok(())
}

fn trace_object(object: &Object, work: &mut Vec<Handle>) {
    work.extend(
        object
            .controller_signal
            .as_ref()
            .and_then(Value::heap_handle),
    );
    work.extend(
        object
            .listener_signals
            .values()
            .filter_map(Value::heap_handle),
    );
    if let Some(signal) = &object.abort_signal {
        work.extend(signal.reason.heap_handle());
        work.extend(signal.handler.heap_handle());
        work.extend(
            signal
                .removals
                .iter()
                .filter_map(|(target, _)| target.heap_handle()),
        );
    }
    if let Some(event) = &object.event {
        work.extend(event.target.heap_handle());
        work.extend(event.current.heap_handle());
        work.extend(event.detail.as_ref().and_then(Value::heap_handle));
    }
    if let Some(listeners) = &object.listeners {
        for listener in listeners.iter() {
            work.extend(listener.callback.heap_handle());
        }
    }
    work.extend(object.home.as_ref().and_then(Value::heap_handle));
    work.extend(object.lexical_this_cell);
    work.extend(object.lexical_new_target.heap_handle());
    work.extend(object.lexical_super.as_ref().and_then(Value::heap_handle));
    if let Some(state) = &object.iterator {
        work.extend(state.source.heap_handle());
    }
    for (symbol, property) in object.symbols.values() {
        work.push(symbol.handle);
        work.extend(property.value.heap_handle());
        if let Some((get, set)) = &property.accessor {
            work.extend(get.heap_handle());
            work.extend(set.heap_handle());
        }
    }
    work.extend(object.primitive.as_ref().and_then(Value::heap_handle));
    work.extend(object.argument_map.values().copied());
    if let Some(promise) = &object.promise {
        match &promise.state {
            crate::promise::State::Pending => {}
            crate::promise::State::Fulfilled(v) | crate::promise::State::Rejected(v) => {
                work.extend(v.heap_handle());
            }
        }
        for reaction in &promise.reactions {
            work.extend(reaction.target.heap_handle());
            work.extend(reaction.resolve.heap_handle());
            work.extend(reaction.reject.heap_handle());
            work.extend(reaction.fulfilled.heap_handle());
            work.extend(reaction.rejected.heap_handle());
            work.extend(reaction.resume);
        }
    }
    work.extend(object.prototype.heap_handle());
    work.extend(
        object
            .properties
            .values()
            .filter_map(|property| property.value.heap_handle()),
    );
    for property in object.properties.values() {
        if let Some((get, set)) = &property.accessor {
            work.extend(get.heap_handle());
            work.extend(set.heap_handle());
        }
    }
}
