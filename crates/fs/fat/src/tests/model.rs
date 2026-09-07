// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The free count against a reference model: over generated sequences of
//! taking chains and giving them back, no cluster is handed out twice,
//! none is left behind, and the count is what is left.

use std::collections::BTreeSet;

use test_support::generators::{BoxGen, Generator, range};
use test_support::model::{ModelTest, run_model_test};

use crate::doubles::RamDisk;
use crate::error::Error;
use crate::fs::FileSystem;
use crate::tests::support::{SECTORS, options};

/// One step of a sequence.
#[derive(Clone, Debug)]
enum Op {
    /// Take a chain of this many clusters, one to four.
    Take(u8),
    /// Give back the chain this number picks out of the ones that are
    /// out, so that chains come back in an order of their own.
    Give(u8),
}

/// What the volume ought to be doing.
struct Model {
    /// How many clusters are free.
    free: u32,
    /// The chains that are out, each as its clusters in order.
    chains: Vec<Vec<u32>>,
    /// Every cluster that is out, so that one handed out twice is seen.
    taken: BTreeSet<u32>,
    /// What this sequence arrived at.
    reached: BTreeSet<&'static str>,
}

/// The free count of a volume that has done nothing yet.
struct FreeCount;

impl ModelTest for FreeCount {
    type Op = Op;
    type Sut = FileSystem<RamDisk>;
    type Model = Model;

    fn generator(&self) -> BoxGen<Op> {
        range(0u16..=39)
            .map(|value| {
                let payload = u8::try_from(value / 4).unwrap_or(0);
                if value % 4 == 3 {
                    Op::Give(payload)
                } else {
                    Op::Take(payload % 4)
                }
            })
            .boxed()
    }

    fn new_sut(&self) -> FileSystem<RamDisk> {
        FileSystem::format(RamDisk::new(SECTORS), &options()).expect("format")
    }

    fn new_model(&self) -> Model {
        Model {
            free: 0,
            chains: Vec::new(),
            taken: BTreeSet::new(),
            reached: BTreeSet::new(),
        }
    }

    fn step(&self, sut: &mut Self::Sut, model: &mut Model, op: &Op) -> Result<(), String> {
        if model.chains.is_empty() && model.taken.is_empty() && model.free == 0 {
            model.free = sut.free_clusters();
        }
        match *op {
            Op::Take(count) => take(sut, model, u32::from(count).saturating_add(1)),
            Op::Give(pick) => give(sut, model, usize::from(pick)),
        }?;
        if sut.free_clusters() != model.free {
            return Err(format!(
                "the volume reports {} free where {} are",
                sut.free_clusters(),
                model.free
            ));
        }
        Ok(())
    }

    fn required(&self) -> &'static [&'static str] {
        &[
            "a chain of more than one cluster",
            "a chain came back",
            "a chain came back out of order",
            "nothing to give back",
        ]
    }

    fn reached(&self, model: &Model) -> Vec<&'static str> {
        model.reached.iter().copied().collect()
    }
}

/// Takes a chain of `count` clusters.
fn take(sut: &mut FileSystem<RamDisk>, model: &mut Model, count: u32) -> Result<(), String> {
    let first = sut.allocate().map_err(|error| format!("{error}"))?;
    let mut chain = vec![first];
    let mut last = first;
    for _ in 1..count {
        let next = sut.extend(last).map_err(|error| format!("{error}"))?;
        chain.push(next);
        last = next;
    }
    for cluster in &chain {
        if !model.taken.insert(*cluster) {
            return Err(format!("the cluster {cluster} was handed out twice"));
        }
    }
    model.free = model.free.saturating_sub(count);
    if count > 1 {
        model.reached.insert("a chain of more than one cluster");
    }
    let length = sut
        .chain_length(first)
        .map_err(|error| format!("{error}"))?;
    if length != count {
        return Err(format!("a chain of {count} reads back as {length}"));
    }
    model.chains.push(chain);
    Ok(())
}

/// Gives back the chain `pick` names.
fn give(sut: &mut FileSystem<RamDisk>, model: &mut Model, pick: usize) -> Result<(), String> {
    if model.chains.is_empty() {
        model.reached.insert("nothing to give back");
        return Ok(());
    }
    let at = pick.checked_rem(model.chains.len()).unwrap_or(0);
    if at > 0 {
        model.reached.insert("a chain came back out of order");
    }
    let chain = model.chains.remove(at);
    let first = *chain.first().ok_or_else(|| "an empty chain".to_owned())?;
    let freed = sut.free_chain(first).map_err(|error| format!("{error}"))?;
    let owed = u32::try_from(chain.len()).unwrap_or(0);
    if freed != owed {
        return Err(format!("giving back {owed} clusters returned {freed}"));
    }
    for cluster in &chain {
        if !model.taken.remove(cluster) {
            return Err(format!("the cluster {cluster} came back twice"));
        }
    }
    model.free = model.free.saturating_add(owed);
    model.reached.insert("a chain came back");
    if sut.chain_length(first) != Err(Error::FreeInChain(first)) {
        return Err(format!("the chain at {first} is still a chain"));
    }
    Ok(())
}

#[test]
fn no_sequence_leaks_a_cluster_or_hands_out_one_twice() {
    run_model_test("fat32 free count", &FreeCount, 16);
}
