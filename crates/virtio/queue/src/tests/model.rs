// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The free set against a reference model: over generated sequences of
//! adding, completing, and reaping, no descriptor is handed out twice and
//! none is left behind.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use test_support::generators::{BoxGen, Generator, range};
use test_support::model::{ModelTest, run_model_test};

use crate::device::Device;
use crate::doubles::RamQueue;
use crate::error::QueueError;
use crate::queue::Queue;
use crate::tests::support::{chain_of, live_device, to_device};

/// Descriptors of the queue under test.
const SIZE: u16 = 8;

/// One step of a sequence.
#[derive(Clone, Debug)]
enum Op {
    /// The driver adds a chain of this many buffers. Zero and more than
    /// [`SIZE`] are the two refusals.
    Add(u8),
    /// The device gives back one outstanding chain, chosen by this
    /// number, so that chains come back out of order.
    Complete(u8),
    /// The driver takes one used element.
    Reap,
}

/// The queue under test, with its memory and its device.
struct Sut {
    /// A device that has reached `DRIVER_OK`.
    device: Device,
    /// The three regions.
    memory: RamQueue,
    /// The queue.
    queue: Queue<1>,
}

/// What the queue ought to be doing.
struct Model {
    /// Descriptors that are not in any chain.
    free: BTreeSet<u16>,
    /// The descriptors of every chain that is out, by head.
    chains: BTreeMap<u16, Vec<u16>>,
    /// Heads added and not yet given back by the device.
    outstanding: VecDeque<u16>,
    /// Heads in the used ring and not yet taken by the driver.
    returned: VecDeque<u16>,
    /// What this sequence arrived at.
    reached: BTreeSet<&'static str>,
}

/// The free set of a queue that has done nothing.
struct FreeSetModel;

impl ModelTest for FreeSetModel {
    type Op = Op;
    type Sut = Sut;
    type Model = Model;

    fn generator(&self) -> BoxGen<Op> {
        range(0u16..=95)
            .map(|value| {
                let payload = u8::try_from(value / 4).unwrap_or(0);
                match value % 4 {
                    0 | 1 => Op::Add(payload % 10),
                    2 => Op::Complete(payload),
                    _ => Op::Reap,
                }
            })
            .boxed()
    }

    fn new_sut(&self) -> Sut {
        let (device, _) = live_device();
        let mut memory = RamQueue::new(SIZE);
        let queue = Queue::<1>::new(&mut memory, SIZE).expect("a queue of eight");
        Sut {
            device,
            memory,
            queue,
        }
    }

    fn new_model(&self) -> Model {
        Model {
            free: (0..SIZE).collect(),
            chains: BTreeMap::new(),
            outstanding: VecDeque::new(),
            returned: VecDeque::new(),
            reached: BTreeSet::new(),
        }
    }

    fn step(&self, sut: &mut Sut, model: &mut Model, op: &Op) -> Result<(), String> {
        match op {
            Op::Add(count) => add(sut, model, usize::from(*count))?,
            Op::Complete(pick) => complete(sut, model, usize::from(*pick)),
            Op::Reap => reap(sut, model)?,
        }
        let free = usize::from(sut.queue.free_count());
        if free != model.free.len() {
            return Err(format!(
                "the queue has {free} free descriptors and the model {}",
                model.free.len()
            ));
        }
        let in_flight = usize::from(sut.queue.in_flight());
        let expected = model.outstanding.len().saturating_add(model.returned.len());
        if in_flight != expected {
            return Err(format!(
                "the queue has {in_flight} chains outstanding and the model {expected}"
            ));
        }
        Ok(())
    }

    fn required(&self) -> &'static [&'static str] {
        &[
            "a chain was added",
            "a chain of three",
            "the free set ran out",
            "a chain came back",
            "a chain came back out of order",
            "no buffers refused",
            "too many buffers refused",
            "nothing to reap",
        ]
    }

    fn reached(&self, model: &Model) -> Vec<&'static str> {
        model.reached.iter().copied().collect()
    }
}

/// The driver adds a chain of `count` buffers.
fn add(sut: &mut Sut, model: &mut Model, count: usize) -> Result<(), String> {
    let buffers = to_device(count);
    let result = sut.queue.add(&sut.device, &mut sut.memory, &buffers);

    if count == 0 {
        model.reached.insert("no buffers refused");
        return expect(&result, &Err(QueueError::EmptyChain));
    }
    if count > usize::from(SIZE) {
        model.reached.insert("too many buffers refused");
        return expect(
            &result,
            &Err(QueueError::ChainTooLong {
                buffers: count,
                size: SIZE,
            }),
        );
    }
    if count > model.free.len() {
        model.reached.insert("the free set ran out");
        let free = u16::try_from(model.free.len()).unwrap_or(u16::MAX);
        return expect(
            &result,
            &Err(QueueError::QueueFull {
                free,
                needed: count,
            }),
        );
    }

    let head = result.map_err(|error| format!("a chain of {count} was refused: {error}"))?;
    let chain = chain_of(&sut.memory, head);
    if chain.len() != count {
        return Err(format!(
            "a chain of {count} buffers became {} descriptors",
            chain.len()
        ));
    }
    for index in &chain {
        if !model.free.remove(index) {
            return Err(format!("the descriptor {index} was handed out twice"));
        }
    }
    model.chains.insert(head, chain);
    model.outstanding.push_back(head);
    model.reached.insert("a chain was added");
    if count == 3 {
        model.reached.insert("a chain of three");
    }
    Ok(())
}

/// The device gives one outstanding chain back.
fn complete(sut: &mut Sut, model: &mut Model, pick: usize) {
    let Some(at) = pick.checked_rem(model.outstanding.len()) else {
        return;
    };
    let Some(head) = model.outstanding.remove(at) else {
        return;
    };
    if at > 0 {
        model.reached.insert("a chain came back out of order");
    }
    // Every chain the model builds is device-readable, so a device that
    // is done with one reports no bytes written.
    sut.memory.complete(u32::from(head), 0);
    model.returned.push_back(head);
}

/// The driver takes one used element.
fn reap(sut: &mut Sut, model: &mut Model) -> Result<(), String> {
    let result = sut.queue.next_used(&sut.device, &sut.memory);
    let Some(expected) = model.returned.pop_front() else {
        model.reached.insert("nothing to reap");
        return match result {
            Ok(None) => Ok(()),
            other => Err(format!("an empty used ring answered {other:?}")),
        };
    };
    let completion = match result {
        Ok(Some(completion)) => completion,
        other => return Err(format!("the chain {expected} came back as {other:?}")),
    };
    if completion.head != expected {
        return Err(format!(
            "the used ring named {} where {expected} was next",
            completion.head
        ));
    }
    let chain = model
        .chains
        .remove(&expected)
        .ok_or_else(|| format!("the chain {expected} was taken back twice"))?;
    for index in chain {
        if !model.free.insert(index) {
            return Err(format!("the descriptor {index} was freed twice"));
        }
    }
    model.reached.insert("a chain came back");
    Ok(())
}

/// Compares what the queue answered against what it owed.
fn expect(got: &Result<u16, QueueError>, owed: &Result<u16, QueueError>) -> Result<(), String> {
    if got == owed {
        Ok(())
    } else {
        Err(format!(
            "the queue answered {got:?} where {owed:?} was owed"
        ))
    }
}

#[test]
fn no_sequence_leaks_a_descriptor_or_hands_out_one_twice() {
    run_model_test("virtqueue free set", &FreeSetModel, 32);
}
