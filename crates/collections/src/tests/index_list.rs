// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `IndexList` over a caller's links: both ends, the middle, a list of
//! one, and a node that belongs elsewhere.

use std::collections::VecDeque;

use test_support::generators::BoxGen;
use test_support::model::{ModelTest, run_model_test};

use crate::error::CollectionError;
use crate::index_list::{IndexList, Link, NONE};
use crate::strategies::{ListOp, any_list_op};

/// Nodes the model test runs over.
const NODES: u32 = 6;

/// A list and the links it runs over.
fn list_of(id: u32, nodes: usize) -> (IndexList, Vec<Link>) {
    (
        IndexList::new(id).expect("a usable identifier"),
        vec![Link::new(); nodes],
    )
}

/// The nodes of a list, head first.
fn order(list: &IndexList, links: &[Link]) -> Vec<u32> {
    list.iter(links).collect()
}

#[test]
fn a_new_list_is_empty_and_carries_its_identifier() {
    let (list, links) = list_of(7, 4);
    assert_eq!(list.id(), 7);
    assert_eq!(list.len(), 0);
    assert!(list.is_empty());
    assert_eq!(list.head(), None);
    assert_eq!(list.tail(), None);
    assert_eq!(order(&list, &links), Vec::<u32>::new());
    assert!(!format!("{list:?}").is_empty());
}

#[test]
fn a_list_cannot_be_identified_by_the_value_that_marks_no_list() {
    assert_eq!(IndexList::new(NONE), Err(CollectionError::ListId));
    assert!(IndexList::new(0).is_ok());
    assert!(IndexList::new(NONE.wrapping_sub(1)).is_ok());
}

#[test]
fn a_new_link_is_in_no_list() {
    let link = Link::new();
    assert_eq!(link.prev(), None);
    assert_eq!(link.next(), None);
    assert_eq!(link.owner(), None);
    assert!(!link.is_linked());
    assert_eq!(Link::default(), link);
    assert!(!format!("{link:?}").is_empty());
}

#[test]
fn pushing_at_the_front_and_the_back_gives_the_insertion_order() {
    let (mut list, mut links) = list_of(1, 5);
    list.push_back(&mut links, 0).expect("a free node");
    list.push_back(&mut links, 1).expect("a free node");
    list.push_front(&mut links, 2).expect("a free node");
    assert_eq!(order(&list, &links), vec![2, 0, 1]);
    assert_eq!(list.head(), Some(2));
    assert_eq!(list.tail(), Some(1));
    assert_eq!(list.len(), 3);
    let node = links.first().copied().expect("a link");
    assert_eq!(node.prev(), Some(2));
    assert_eq!(node.next(), Some(1));
    assert_eq!(node.owner(), Some(1));
}

#[test]
fn a_list_of_one_has_the_same_node_at_both_ends() {
    let (mut list, mut links) = list_of(1, 3);
    list.push_front(&mut links, 1).expect("a free node");
    assert_eq!(list.head(), Some(1));
    assert_eq!(list.tail(), Some(1));
    assert_eq!(order(&list, &links), vec![1]);
    assert_eq!(list.pop_front(&mut links), Some(1));
    assert!(list.is_empty());
    assert_eq!(list.head(), None);
    assert_eq!(list.tail(), None);
    assert!(!links.get(1).copied().expect("a link").is_linked());

    list.push_back(&mut links, 2).expect("a free node");
    assert_eq!(list.pop_back(&mut links), Some(2));
    assert!(list.is_empty());
}

#[test]
fn unlinking_from_the_middle_the_head_and_the_tail_keeps_the_rest() {
    let (mut list, mut links) = list_of(1, 5);
    for node in 0..5 {
        list.push_back(&mut links, node).expect("a free node");
    }
    assert_eq!(list.unlink(&mut links, 2), Ok(()));
    assert_eq!(order(&list, &links), vec![0, 1, 3, 4]);
    assert_eq!(list.unlink(&mut links, 0), Ok(()));
    assert_eq!(order(&list, &links), vec![1, 3, 4]);
    assert_eq!(list.head(), Some(1));
    assert_eq!(list.unlink(&mut links, 4), Ok(()));
    assert_eq!(order(&list, &links), vec![1, 3]);
    assert_eq!(list.tail(), Some(3));
    assert_eq!(list.len(), 2);
}

#[test]
fn unlinking_a_node_that_is_not_in_the_list_is_rejected() {
    let (mut list, mut links) = list_of(1, 4);
    list.push_back(&mut links, 0).expect("a free node");
    assert_eq!(
        list.unlink(&mut links, 1),
        Err(CollectionError::NotLinked(1))
    );
    assert_eq!(list.unlink(&mut links, 0), Ok(()));
    assert_eq!(
        list.unlink(&mut links, 0),
        Err(CollectionError::NotLinked(0))
    );
    assert!(list.is_empty());
}

#[test]
fn a_node_outside_the_slice_is_an_index_error() {
    let (mut list, mut links) = list_of(1, 2);
    assert_eq!(
        list.push_back(&mut links, 2),
        Err(CollectionError::Index(2))
    );
    assert_eq!(
        list.push_front(&mut links, 9),
        Err(CollectionError::Index(9))
    );
    assert_eq!(list.unlink(&mut links, 2), Err(CollectionError::Index(2)));
    assert!(!list.contains(&links, 2));
    assert!(list.is_empty());
}

#[test]
fn a_node_already_in_a_list_is_not_taken_by_another() {
    // The shape of one run queue per priority over one array of threads.
    let mut ready = IndexList::new(1).expect("a usable identifier");
    let mut blocked = IndexList::new(2).expect("a usable identifier");
    let mut links = vec![Link::new(); 4];

    ready.push_back(&mut links, 0).expect("a free node");
    assert_eq!(
        ready.push_back(&mut links, 0),
        Err(CollectionError::AlreadyLinked(0))
    );
    assert_eq!(
        blocked.push_back(&mut links, 0),
        Err(CollectionError::AlreadyLinked(0))
    );
    assert_eq!(
        blocked.unlink(&mut links, 0),
        Err(CollectionError::NotLinked(0))
    );
    assert!(ready.contains(&links, 0));
    assert!(!blocked.contains(&links, 0));

    // It moves only through the list that has it.
    ready.unlink(&mut links, 0).expect("its own node");
    blocked.push_back(&mut links, 0).expect("now free");
    assert!(blocked.contains(&links, 0));
    assert_eq!(order(&blocked, &links), vec![0]);
    assert_eq!(order(&ready, &links), Vec::<u32>::new());
}

#[test]
fn popping_an_empty_list_is_none() {
    let (mut list, mut links) = list_of(1, 2);
    assert_eq!(list.pop_front(&mut links), None);
    assert_eq!(list.pop_back(&mut links), None);
}

#[test]
fn the_walk_stops_after_as_many_steps_as_the_list_is_long() {
    // A caller that corrupts its own slice must not make the walk run
    // forever. The links are rewritten into a cycle behind the list's
    // back; the walk still ends.
    let (mut list, mut links) = list_of(1, 3);
    for node in 0..3 {
        list.push_back(&mut links, node).expect("a free node");
    }
    let head = links.first().copied().expect("a link");
    let tail = links.get_mut(2).expect("a link");
    *tail = head;
    let walked: Vec<u32> = list.iter(&links).collect();
    assert_eq!(walked.len(), 3);
}

/// The list against a `VecDeque` of the nodes it holds.
struct ListModel;

/// The state under test: a list and the links it runs over.
struct ListSut {
    /// The list.
    list: IndexList,
    /// The links of every node.
    links: Vec<Link>,
}

impl ModelTest for ListModel {
    type Op = ListOp;
    type Sut = ListSut;
    type Model = VecDeque<u32>;

    fn generator(&self) -> BoxGen<ListOp> {
        any_list_op(NODES)
    }

    fn new_sut(&self) -> ListSut {
        ListSut {
            list: IndexList::new(1).expect("a usable identifier"),
            links: vec![Link::new(); usize::try_from(NODES).unwrap_or(0)],
        }
    }

    fn new_model(&self) -> VecDeque<u32> {
        VecDeque::new()
    }

    fn step(
        &self,
        sut: &mut ListSut,
        model: &mut VecDeque<u32>,
        op: &ListOp,
    ) -> Result<(), String> {
        match *op {
            ListOp::PushFront(node) | ListOp::PushBack(node) => {
                let front = matches!(*op, ListOp::PushFront(_));
                let result = if front {
                    sut.list.push_front(&mut sut.links, node)
                } else {
                    sut.list.push_back(&mut sut.links, node)
                };
                let expected = if node >= NODES {
                    Err(CollectionError::Index(
                        usize::try_from(node).unwrap_or(usize::MAX),
                    ))
                } else if model.contains(&node) {
                    Err(CollectionError::AlreadyLinked(node))
                } else {
                    Ok(())
                };
                if result != expected {
                    return Err(format!("push {node}: {result:?} against {expected:?}"));
                }
                if result.is_ok() {
                    if front {
                        model.push_front(node);
                    } else {
                        model.push_back(node);
                    }
                }
            }
            ListOp::PopFront => {
                let expected = model.pop_front();
                let got = sut.list.pop_front(&mut sut.links);
                if got != expected {
                    return Err(format!("pop_front {got:?} against {expected:?}"));
                }
            }
            ListOp::PopBack => {
                let expected = model.pop_back();
                let got = sut.list.pop_back(&mut sut.links);
                if got != expected {
                    return Err(format!("pop_back {got:?} against {expected:?}"));
                }
            }
            ListOp::Unlink(node) => {
                let expected = if node >= NODES {
                    Err(CollectionError::Index(
                        usize::try_from(node).unwrap_or(usize::MAX),
                    ))
                } else if model.contains(&node) {
                    Ok(())
                } else {
                    Err(CollectionError::NotLinked(node))
                };
                let result = sut.list.unlink(&mut sut.links, node);
                if result != expected {
                    return Err(format!("unlink {node}: {result:?} against {expected:?}"));
                }
                if result.is_ok() {
                    model.retain(|held| *held != node);
                }
            }
        }
        if usize::try_from(sut.list.len()).unwrap_or(usize::MAX) != model.len() {
            return Err(format!("length {} against {}", sut.list.len(), model.len()));
        }
        let walked: Vec<u32> = sut.list.iter(&sut.links).collect();
        let expected: Vec<u32> = model.iter().copied().collect();
        if walked != expected {
            return Err(format!("{walked:?} against {expected:?}"));
        }
        if sut.list.head() != expected.first().copied()
            || sut.list.tail() != expected.last().copied()
        {
            return Err("the ends disagree with the order".to_owned());
        }
        Ok(())
    }
}

#[test]
fn model_a_list_agrees_with_a_deque_of_nodes() {
    run_model_test("index_list_model", &ListModel, 64);
}
