// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{
    Error, Value,
    bytecode::Slot,
    heap::{Heap, Node},
};

fn cell() -> Node {
    Node::Cell {
        slot: Slot {
            name: "x".to_owned(),
            mutable: true,
            captured: true,
        },
        value: Some(Value::Number(1.0)),
    }
}

#[test]
fn heap_roots_survive_and_old_generations_are_rejected() -> Result<(), Error> {
    let mut heap = Heap::default();
    let mut fuel = u64::MAX;
    let first = heap.allocate(cell(), 1)?;
    heap.mark(first);
    heap.mark(first);
    heap.collect(&mut fuel)?;
    assert!(heap.get(first).is_ok());
    assert!(matches!(heap.allocate(cell(), 1), Err(Error::Limit { .. })));
    heap.collect(&mut fuel)?;
    assert!(heap.get(first).is_err());
    let second = heap.allocate(cell(), 1)?;
    assert_ne!(first, second);
    assert!(heap.get_mut(first).is_err());
    assert!(heap.get(second).is_ok());
    heap.mark(first);
    assert!(heap.collect(&mut fuel).is_err());
    Ok(())
}

fn object_value(handle: crate::heap::Handle) -> Value {
    Value::Object(crate::ObjectValue {
        handle,
        owner: alloc::rc::Rc::new(()),
    })
}

#[test]
fn abort_graph_only_retains_live_observed_dependents_and_drops_weak_backedges() -> Result<(), Error>
{
    use crate::{event::AbortSignal, object::Object};
    for observed in [false, true] {
        for aborted in [false, true] {
            let mut heap = Heap::default();
            let source = heap.allocate(Node::Object(Object::new(Value::Null)), 8)?;
            let dependent = heap.allocate(Node::Object(Object::new(Value::Null)), 8)?;
            let Node::Object(object) = heap.get_mut(source)? else {
                return Err(Error::InvalidBytecode);
            };
            let mut signal = AbortSignal::new();
            signal.dependents.push(dependent);
            object.abort_signal = Some(signal);
            let Node::Object(object) = heap.get_mut(dependent)? else {
                return Err(Error::InvalidBytecode);
            };
            let mut signal = AbortSignal::new();
            signal.dependent = true;
            signal.sources.push(source);
            if aborted {
                signal.reason = Value::Null;
            }
            object.abort_signal = Some(signal);
            let mut listeners = audhsos_event_target::Listeners::new();
            if observed {
                listeners
                    .add(
                        Value::string("abort").units(),
                        Value::Null,
                        audhsos_event_target::Options::default(),
                        4,
                        8,
                    )
                    .unwrap();
            }
            object.listeners = Some(listeners);
            heap.mark(source);
            heap.collect(&mut 100)?;
            assert_eq!(heap.get(dependent).is_ok(), observed && !aborted);
            if observed && !aborted {
                heap.mark(dependent);
            }
            heap.collect(&mut 100)?;
            assert!(heap.get(source).is_err());
            // Stale weak graph edges never alias a reused generation.
            let next = heap.allocate(Node::Object(Object::new(Value::Null)), 8)?;
            assert_ne!(next, source);
            if observed && !aborted {
                heap.mark(dependent);
            }
            heap.collect(&mut 100)?;
            assert!(heap.get(next).is_err());
        }
    }
    Ok(())
}

#[test]
fn ephemeron_chain_activates_in_both_mark_orders_and_reclaims_backedges() -> Result<(), Error> {
    use crate::{
        object::Object,
        weakmap::{Key, WeakMap},
    };
    for key_first in [false, true] {
        let mut heap = Heap::default();
        let keys = (0..128)
            .map(|_| heap.allocate(Node::Object(Object::new(Value::Null)), 256))
            .collect::<Result<Vec<_>, _>>()?;
        let mut map = WeakMap::default();
        for pair in keys.windows(2) {
            let [a, b] = pair else {
                return Err(Error::InvalidBytecode);
            };
            map.entries.insert(Key::Heap(*a, false), object_value(*b));
        }
        let mut object = Object::new(Value::Null);
        object.weakmap = Some(map);
        let map = heap.allocate(Node::Object(object), 256)?;
        let first = *keys.first().ok_or(Error::InvalidBytecode)?;
        let last = *keys.last().ok_or(Error::InvalidBytecode)?;
        if key_first {
            heap.mark(map);
            heap.mark(first);
        } else {
            heap.mark(first);
            heap.mark(map);
        }
        let mut fuel = 2000;
        heap.collect(&mut fuel)?;
        assert!(heap.get(last).is_ok());
        assert_eq!(heap.weak_entries(), 127);
        // No externally rooted key: even a map-held chain must disappear.
        heap.mark(map);
        heap.collect(&mut fuel)?;
        assert_eq!(heap.weak_entries(), 0);
        assert!(heap.get(first).is_err() && heap.get(last).is_err());
        let replacement = heap.allocate(Node::Object(Object::new(Value::Null)), 256)?;
        assert!(!keys.contains(&replacement));
    }
    Ok(())
}

#[test]
fn a_dead_map_does_not_keep_values_alive_even_with_a_live_key() -> Result<(), Error> {
    use crate::{
        object::Object,
        weakmap::{Key, WeakMap},
    };
    let mut heap = Heap::default();
    let key = heap.allocate(cell(), 10)?;
    let value = heap.allocate(cell(), 10)?;
    let mut object = Object::new(Value::Null);
    let mut map = WeakMap::default();
    map.entries
        .insert(Key::Heap(key, false), object_value(value));
    object.weakmap = Some(map);
    let map = heap.allocate(Node::Object(object), 10)?;
    heap.mark(key);
    heap.collect(&mut 100)?;
    assert!(heap.get(key).is_ok());
    assert!(heap.get(value).is_err());
    assert!(heap.get(map).is_err());
    assert_eq!(heap.weak_entries(), 0);
    Ok(())
}

#[test]
fn collection_work_exhaustion_is_an_embedding_error() -> Result<(), Error> {
    let mut heap = Heap::default();
    let key = heap.allocate(cell(), 10)?;
    heap.mark(key);
    assert!(matches!(
        heap.collect(&mut 0),
        Err(Error::Limit {
            resource: "garbage collection work"
        })
    ));
    Ok(())
}

#[test]
fn generated_ephemeron_graphs_match_a_least_fixed_point_oracle() -> Result<(), Error> {
    for seed in 0u32..128 {
        generated_graph(seed)?;
    }
    Ok(())
}

fn generated_graph(seed: u32) -> Result<(), Error> {
    use crate::{
        object::{Object, Property},
        weakmap::{Key, WeakMap},
    };
    let mut state = seed;
    let mut next = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        usize::try_from(state >> 16).unwrap_or(0) % 32
    };
    let mut heap = Heap::default();
    let handles = (0..32)
        .map(|_| heap.allocate(Node::Object(Object::new(Value::Null)), 32))
        .collect::<Result<Vec<_>, _>>()?;
    let mut strong = Vec::new();
    let mut weak = Vec::new();
    for map in 0..8usize {
        let mut data = WeakMap::default();
        for _ in 0..8 {
            let key = next();
            let value = next();
            // Later insertion with the same key replaces the old value.
            data.entries.insert(
                Key::Heap(*handles.get(key).ok_or(Error::InvalidBytecode)?, false),
                object_value(*handles.get(value).ok_or(Error::InvalidBytecode)?),
            );
        }
        for (key, value) in &data.entries {
            let Key::Heap(key, _) = key else {
                return Err(Error::InvalidBytecode);
            };
            let k = handles
                .iter()
                .position(|h| h == key)
                .ok_or(Error::InvalidBytecode)?;
            let v = handles
                .iter()
                .position(|h| Some(*h) == value.heap_handle())
                .ok_or(Error::InvalidBytecode)?;
            weak.push((map, k, v));
        }
        let Node::Object(object) =
            heap.get_mut(*handles.get(map).ok_or(Error::InvalidBytecode)?)?
        else {
            return Err(Error::InvalidBytecode);
        };
        object.weakmap = Some(data);
    }
    for i in 0..32 {
        let from = next();
        let to = next();
        strong.push((from, to));
        let Node::Object(object) =
            heap.get_mut(*handles.get(from).ok_or(Error::InvalidBytecode)?)?
        else {
            return Err(Error::InvalidBytecode);
        };
        object.define(
            Value::string(&format!("e{i}")).units(),
            Property::data(object_value(
                *handles.get(to).ok_or(Error::InvalidBytecode)?,
            )),
            64,
        )?;
    }
    let mut live = [false; 32];
    for _ in 0..4 {
        let root = next();
        *live.get_mut(root).ok_or(Error::InvalidBytecode)? = true;
        heap.mark(*handles.get(root).ok_or(Error::InvalidBytecode)?);
    }
    loop {
        let old = live;
        for (from, to) in &strong {
            if old.get(*from) == Some(&true) {
                *live.get_mut(*to).ok_or(Error::InvalidBytecode)? = true;
            }
        }
        for (map, key, value) in &weak {
            if old.get(*map) == Some(&true) && old.get(*key) == Some(&true) {
                *live.get_mut(*value).ok_or(Error::InvalidBytecode)? = true;
            }
        }
        if live == old {
            break;
        }
    }
    heap.collect(&mut 10_000)?;
    for (index, handle) in handles.iter().enumerate() {
        assert_eq!(
            heap.get(*handle).is_ok(),
            *live.get(index).ok_or(Error::InvalidBytecode)?,
            "seed {seed}, node {index}"
        );
    }
    let expected = weak
        .iter()
        .filter(|(m, k, _)| live.get(*m) == Some(&true) && live.get(*k) == Some(&true))
        .count();
    assert_eq!(heap.weak_entries(), expected, "seed {seed}");
    Ok(())
}
