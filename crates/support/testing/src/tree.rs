// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A value together with its shrink candidates, computed lazily.
//!
//! Invariants: the children of a tree are "smaller" than its value in the
//! order the generator defines; shrinking follows children only, so it
//! terminates for finite orders.

use std::fmt;
use std::rc::Rc;

/// A generated value with lazily computed shrink candidates.
pub struct Tree<T> {
    value: T,
    shrinks: Rc<dyn Fn() -> Box<dyn Iterator<Item = Tree<T>>>>,
}

impl<T: Clone + 'static> Tree<T> {
    /// A value without shrink candidates.
    pub fn leaf(value: T) -> Self {
        Tree::new(value, std::iter::empty)
    }

    /// A value whose candidates are produced by `shrinks` on demand.
    pub fn new<I>(value: T, shrinks: impl Fn() -> I + 'static) -> Self
    where
        I: IntoIterator<Item = Tree<T>>,
        I::IntoIter: 'static,
    {
        Tree {
            value,
            shrinks: Rc::new(move || Box::new(shrinks().into_iter())),
        }
    }

    /// The value.
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Consumes the tree and returns the value.
    pub fn into_value(self) -> T {
        self.value
    }

    /// The shrink candidates, smallest first.
    pub fn shrinks(&self) -> Box<dyn Iterator<Item = Tree<T>>> {
        (self.shrinks)()
    }

    /// Applies `f` to the value and, lazily, to every candidate.
    pub fn map<U: Clone + 'static>(self, f: Rc<dyn Fn(T) -> U>) -> Tree<U> {
        let value = f(self.value.clone());
        let shrinks = self.shrinks;
        Tree::new(value, move || {
            let f = Rc::clone(&f);
            shrinks().map(move |child| child.map(Rc::clone(&f)))
        })
    }

    /// Keeps the tree if its value satisfies `keep`, and lazily drops every
    /// candidate that does not.
    pub fn filter(self, keep: Rc<dyn Fn(&T) -> bool>) -> Option<Tree<T>> {
        if !keep(&self.value) {
            return None;
        }
        let shrinks = self.shrinks;
        Some(Tree::new(self.value, move || {
            let keep = Rc::clone(&keep);
            shrinks().filter_map(move |child| child.filter(Rc::clone(&keep)))
        }))
    }
}

impl<T: Clone> Clone for Tree<T> {
    fn clone(&self) -> Self {
        Tree {
            value: self.value.clone(),
            shrinks: Rc::clone(&self.shrinks),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Tree<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tree")
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}

/// Combines two trees; candidates shrink the left value first, then the
/// right one.
#[must_use]
pub fn zip<A: Clone + 'static, B: Clone + 'static>(left: Tree<A>, right: Tree<B>) -> Tree<(A, B)> {
    let value = (left.value().clone(), right.value().clone());
    Tree::new(value, move || {
        let left_for_left = left.clone();
        let right_for_left = right.clone();
        let left_for_right = left.clone();
        let from_left = left_for_left
            .shrinks()
            .map(move |l| zip(l, right_for_left.clone()));
        let from_right = right.shrinks().map(move |r| zip(left_for_right.clone(), r));
        from_left.chain(from_right)
    })
}

/// Combines element trees into a tree of a vector. Candidates first remove
/// chunks of elements (largest chunks first) down to `min_len`, then shrink
/// single elements in place.
#[must_use]
pub fn sequence<T: Clone + 'static>(elements: Vec<Tree<T>>, min_len: usize) -> Tree<Vec<T>> {
    let elements = Rc::new(elements);
    let value: Vec<T> = elements.iter().map(|e| e.value().clone()).collect();
    Tree::new(value, move || {
        let len = elements.len();
        let for_chunks = Rc::clone(&elements);
        let chunks =
            std::iter::successors(Some(len / 2), |&chunk| (chunk > 1).then_some(chunk / 2))
                .take_while(|&chunk| chunk > 0)
                .filter(move |&chunk| len.saturating_sub(chunk) >= min_len)
                .flat_map(move |chunk| {
                    let elements = Rc::clone(&for_chunks);
                    (0..=len.saturating_sub(chunk))
                        .step_by(chunk)
                        .map(move |start| {
                            let mut remaining = elements.as_ref().clone();
                            remaining.drain(start..start.saturating_add(chunk));
                            sequence(remaining, min_len)
                        })
                });
        let for_elements = Rc::clone(&elements);
        let children = (0..len).flat_map(move |index| {
            let elements = Rc::clone(&for_elements);
            let shrinks = elements.get(index).map(Tree::shrinks);
            shrinks.into_iter().flatten().map(move |child| {
                let replaced = elements
                    .iter()
                    .enumerate()
                    .map(|(position, original)| {
                        if position == index {
                            child.clone()
                        } else {
                            original.clone()
                        }
                    })
                    .collect();
                sequence(replaced, min_len)
            })
        });
        chunks.chain(children)
    })
}
