// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Generators: values with integrated shrinking.
//!
//! Invariants: a generator produces values inside the bounds it was built
//! with, and every shrink candidate lies inside the same bounds.

use std::fmt::Debug;
use std::ops::RangeInclusive;
use std::rc::Rc;

use crate::rng::Rng;
use crate::tree::{Tree, sequence, zip};

/// Produces values with shrink candidates.
pub trait Generator {
    /// The generated type.
    type Value: Clone + Debug + 'static;

    /// Draws one value and its shrink tree.
    fn generate(&self, rng: &mut Rng) -> Tree<Self::Value>;

    /// Transforms every generated value (and every candidate) with `f`.
    fn map<U, F>(self, f: F) -> Map<Self, U>
    where
        Self: Sized,
        U: Clone + Debug + 'static,
        F: Fn(Self::Value) -> U + 'static,
    {
        Map {
            inner: self,
            f: Rc::new(f),
        }
    }

    /// Keeps only values for which `keep` holds; candidates that fail
    /// `keep` are dropped.
    fn filter<P>(self, keep: P) -> Filter<Self>
    where
        Self: Sized,
        P: Fn(&Self::Value) -> bool + 'static,
    {
        Filter {
            inner: self,
            keep: Rc::new(keep),
        }
    }

    /// Erases the concrete generator type.
    fn boxed(self) -> BoxGen<Self::Value>
    where
        Self: Sized + 'static,
    {
        BoxGen(Rc::new(self))
    }
}

/// A type-erased generator.
pub struct BoxGen<T>(Rc<dyn Generator<Value = T>>);

impl<T> Clone for BoxGen<T> {
    fn clone(&self) -> Self {
        BoxGen(Rc::clone(&self.0))
    }
}

impl<T: Clone + Debug + 'static> Generator for BoxGen<T> {
    type Value = T;

    fn generate(&self, rng: &mut Rng) -> Tree<T> {
        self.0.generate(rng)
    }
}

impl<T> Debug for BoxGen<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BoxGen")
    }
}

/// A shared mapping function.
type MapFn<T, U> = Rc<dyn Fn(T) -> U>;

/// The result of [`Generator::map`].
pub struct Map<G: Generator, U> {
    inner: G,
    f: MapFn<G::Value, U>,
}

impl<G: Generator + Clone, U> Clone for Map<G, U> {
    fn clone(&self) -> Self {
        Map {
            inner: self.inner.clone(),
            f: Rc::clone(&self.f),
        }
    }
}

impl<G: Generator, U: Clone + Debug + 'static> Generator for Map<G, U> {
    type Value = U;

    fn generate(&self, rng: &mut Rng) -> Tree<U> {
        self.inner.generate(rng).map(Rc::clone(&self.f))
    }
}

/// A shared predicate.
type Predicate<T> = Rc<dyn Fn(&T) -> bool>;

/// The result of [`Generator::filter`].
pub struct Filter<G: Generator> {
    inner: G,
    keep: Predicate<G::Value>,
}

impl<G: Generator + Clone> Clone for Filter<G> {
    fn clone(&self) -> Self {
        Filter {
            inner: self.inner.clone(),
            keep: Rc::clone(&self.keep),
        }
    }
}

/// Number of consecutive rejections after which a filter gives up.
pub const FILTER_ATTEMPTS: u32 = 1000;

impl<G: Generator> Generator for Filter<G> {
    type Value = G::Value;

    /// # Panics
    ///
    /// Panics when [`FILTER_ATTEMPTS`] consecutive values are rejected; a
    /// filter that restrictive must be replaced by a narrower generator.
    #[expect(clippy::panic, reason = "a rejected generator is a test-writing error")]
    fn generate(&self, rng: &mut Rng) -> Tree<G::Value> {
        for _ in 0..FILTER_ATTEMPTS {
            if let Some(tree) = self.inner.generate(rng).filter(Rc::clone(&self.keep)) {
                return tree;
            }
        }
        panic!("filter rejected {FILTER_ATTEMPTS} consecutive values");
    }
}

/// An integer type usable with [`range`].
pub trait Int: Copy + Debug + Ord + 'static {
    /// The value widened to `i128`.
    fn to_i128(self) -> i128;
    /// The value narrowed from an `i128` that is inside the type's range.
    fn from_i128(value: i128) -> Self;
}

macro_rules! int_impls {
    ($($ty:ty),+) => {
        $(
            impl Int for $ty {
                fn to_i128(self) -> i128 {
                    i128::from(self)
                }

                fn from_i128(value: i128) -> Self {
                    Self::try_from(value).unwrap_or(Self::MIN)
                }
            }
        )+
    };
}

int_impls!(u8, u16, u32, u64, i8, i16, i32, i64);

impl Int for usize {
    fn to_i128(self) -> i128 {
        i128::try_from(self).unwrap_or(i128::MAX)
    }

    fn from_i128(value: i128) -> Self {
        Self::try_from(value).unwrap_or(Self::MIN)
    }
}

/// Uniform integers in an inclusive range, shrinking toward the value
/// closest to zero inside the range.
#[derive(Clone, Debug)]
pub struct Range<T> {
    low: T,
    high: T,
}

/// Uniform integers in `bounds`, shrinking toward the value closest to zero
/// inside `bounds`.
///
/// # Panics
///
/// Panics if the range is empty.
#[must_use]
pub fn range<T: Int>(bounds: RangeInclusive<T>) -> Range<T> {
    let (low, high) = bounds.into_inner();
    assert!(low <= high, "range is empty");
    Range { low, high }
}

fn int_tree<T: Int>(value: i128, target: i128) -> Tree<T> {
    Tree::new(T::from_i128(value), move || {
        let mut candidates = Vec::new();
        let mut step = value.saturating_sub(target);
        while step != 0 {
            candidates.push(int_tree(value.saturating_sub(step), target));
            step /= 2;
        }
        candidates
    })
}

impl<T: Int> Generator for Range<T> {
    type Value = T;

    fn generate(&self, rng: &mut Rng) -> Tree<T> {
        let low = self.low.to_i128();
        let high = self.high.to_i128();
        let span = u64::try_from(high.saturating_sub(low)).unwrap_or(u64::MAX);
        let offset = i128::from(rng.range(0, span));
        let value = low.saturating_add(offset);
        let target = 0i128.clamp(low, high);
        int_tree(value, target)
    }
}

/// A generator that always yields the same value.
#[derive(Clone, Debug)]
pub struct Just<T>(T);

/// Always `value`, without shrink candidates.
pub const fn just<T: Clone + Debug + 'static>(value: T) -> Just<T> {
    Just(value)
}

impl<T: Clone + Debug + 'static> Generator for Just<T> {
    type Value = T;

    fn generate(&self, _rng: &mut Rng) -> Tree<T> {
        Tree::leaf(self.0.clone())
    }
}

/// One of a fixed list of values, shrinking toward the first entry.
#[derive(Clone, Debug)]
pub struct OneOf<T>(Rc<Vec<T>>);

/// One of `values`, shrinking toward the first entry.
///
/// # Panics
///
/// Panics if `values` is empty.
#[must_use]
pub fn one_of<T: Clone + Debug + 'static>(values: Vec<T>) -> OneOf<T> {
    assert!(!values.is_empty(), "one_of needs at least one value");
    OneOf(Rc::new(values))
}

impl<T: Clone + Debug + 'static> Generator for OneOf<T> {
    type Value = T;

    fn generate(&self, rng: &mut Rng) -> Tree<T> {
        let values = Rc::clone(&self.0);
        let last_index = values.len().saturating_sub(1);
        let pick = move |index: usize| {
            values
                .get(index)
                .or_else(|| values.first())
                .cloned()
                .unwrap_or_else(unreachable_value)
        };
        range(0..=last_index).generate(rng).map(Rc::new(pick))
    }
}

/// Never called: `one_of` rejects empty lists, so the pick always succeeds.
#[expect(clippy::panic, reason = "unreachable by construction; see one_of")]
fn unreachable_value<T>() -> T {
    panic!("one_of picked from an empty list")
}

/// Booleans, shrinking toward `false`.
#[derive(Clone, Debug, Default)]
pub struct Bool;

/// Uniform booleans, shrinking toward `false`.
#[must_use]
pub const fn bool() -> Bool {
    Bool
}

impl Generator for Bool {
    type Value = bool;

    fn generate(&self, rng: &mut Rng) -> Tree<bool> {
        if rng.bool() {
            Tree::new(true, || vec![Tree::leaf(false)])
        } else {
            Tree::leaf(false)
        }
    }
}

/// Two generators combined; shrinks the left value first.
#[derive(Clone, Debug)]
pub struct Pair<A, B> {
    left: A,
    right: B,
}

/// Pairs of values from `left` and `right`.
#[must_use]
pub const fn pair<A: Generator, B: Generator>(left: A, right: B) -> Pair<A, B> {
    Pair { left, right }
}

impl<A: Generator, B: Generator> Generator for Pair<A, B> {
    type Value = (A::Value, B::Value);

    fn generate(&self, rng: &mut Rng) -> Tree<Self::Value> {
        zip(self.left.generate(rng), self.right.generate(rng))
    }
}

/// Vectors of generated elements with a length in a range.
#[derive(Clone, Debug)]
pub struct VecGen<G> {
    element: G,
    len: RangeInclusive<usize>,
}

/// Vectors with a length in `len`, shrinking by removing elements down to
/// the minimum length and by shrinking single elements.
///
/// # Panics
///
/// Panics if `len` is empty.
#[must_use]
pub fn vec<G: Generator>(element: G, len: RangeInclusive<usize>) -> VecGen<G> {
    assert!(len.start() <= len.end(), "length range is empty");
    VecGen { element, len }
}

impl<G: Generator> Generator for VecGen<G> {
    type Value = Vec<G::Value>;

    fn generate(&self, rng: &mut Rng) -> Tree<Self::Value> {
        let low = u64::try_from(*self.len.start()).unwrap_or(u64::MAX);
        let high = u64::try_from(*self.len.end()).unwrap_or(u64::MAX);
        let count = usize::try_from(rng.range(low, high)).unwrap_or(usize::MAX);
        let elements = (0..count).map(|_| self.element.generate(rng)).collect();
        sequence(elements, *self.len.start())
    }
}

/// Byte vectors with a length in `len`.
#[must_use]
pub fn bytes(len: RangeInclusive<usize>) -> VecGen<Range<u8>> {
    vec(range(0..=u8::MAX), len)
}

/// `Option` values; `None` with probability one in four, `Some` shrinking
/// to `None` first.
#[derive(Clone, Debug)]
pub struct OptionGen<G>(G);

/// Optional values from `inner`.
pub const fn option<G: Generator>(inner: G) -> OptionGen<G> {
    OptionGen(inner)
}

impl<G: Generator> Generator for OptionGen<G> {
    type Value = Option<G::Value>;

    fn generate(&self, rng: &mut Rng) -> Tree<Self::Value> {
        if rng.chance(1, 4) {
            return Tree::leaf(None);
        }
        let inner = self.0.generate(rng);
        let some = inner.map(Rc::new(Some));
        let value = some.value().clone();
        Tree::new(value, move || {
            std::iter::once(Tree::leaf(None))
                .chain(some.shrinks())
                .collect()
        })
    }
}
