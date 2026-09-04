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
    shrinks: Rc<dyn Fn() -> Vec<Tree<T>>>,
}

impl<T: Clone + 'static> Tree<T> {
    /// A value without shrink candidates.
    pub fn leaf(value: T) -> Self {
        Tree {
            value,
            shrinks: Rc::new(Vec::new),
        }
    }

    /// A value whose candidates are produced by `shrinks` on demand.
    pub fn new(value: T, shrinks: impl Fn() -> Vec<Tree<T>> + 'static) -> Self {
        Tree {
            value,
            shrinks: Rc::new(shrinks),
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
    #[must_use]
    pub fn shrinks(&self) -> Vec<Tree<T>> {
        (self.shrinks)()
    }

    /// Applies `f` to the value and, lazily, to every candidate.
    pub fn map<U: Clone + 'static>(self, f: Rc<dyn Fn(T) -> U>) -> Tree<U> {
        let value = f(self.value.clone());
        let shrinks = self.shrinks;
        Tree::new(value, move || {
            shrinks()
                .into_iter()
                .map(|child| child.map(Rc::clone(&f)))
                .collect()
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
            shrinks()
                .into_iter()
                .filter_map(|child| child.filter(Rc::clone(&keep)))
                .collect()
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
        let from_left = left.shrinks().into_iter().map(|l| zip(l, right.clone()));
        let from_right = right.shrinks().into_iter().map(|r| zip(left.clone(), r));
        from_left.chain(from_right).collect()
    })
}

/// Combines element trees into a tree of a vector. Candidates first remove
/// chunks of elements (largest chunks first) down to `min_len`, then shrink
/// single elements in place.
#[must_use]
pub fn sequence<T: Clone + 'static>(elements: Vec<Tree<T>>, min_len: usize) -> Tree<Vec<T>> {
    let value: Vec<T> = elements.iter().map(|e| e.value().clone()).collect();
    Tree::new(value, move || {
        let mut candidates = Vec::new();
        let len = elements.len();
        let mut chunk = len / 2;
        while chunk >= 1 {
            if len.saturating_sub(chunk) >= min_len {
                let mut start = 0usize;
                while start.saturating_add(chunk) <= len {
                    let mut remaining = elements.clone();
                    remaining.drain(start..start.saturating_add(chunk));
                    candidates.push(sequence(remaining, min_len));
                    start = start.saturating_add(chunk);
                }
            }
            chunk /= 2;
        }
        for (index, element) in elements.iter().enumerate() {
            for child in element.shrinks() {
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
                candidates.push(sequence(replaced, min_len));
            }
        }
        candidates
    })
}
