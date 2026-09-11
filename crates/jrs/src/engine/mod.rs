// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! State-of-the-Art Pure ECMAScript Engine Core.
//!
//! This module implements the new architecture layers in their strict
//! dependency order:
//! 1. 64-bit NaN-boxed Value representation (`value`, `string`)
//! 2. Shapes (Hidden Classes) and Elements backing store (`shape`, `elements`, `object`)
//! 3. Generational bump-pointer Nursery and rooting protocol (`heap`)
//! 4. Register-based bytecode format (`bytecode`)
//! 5. Inline Caches and feedback vectors (`feedback`)
//! 6. Fast contiguous register interpreter (`interpreter`)

pub mod bytecode;
pub mod context;
pub mod elements;
pub mod feedback;
pub mod heap;
pub mod interpreter;
pub mod object;
pub mod shape;
pub mod string;
pub mod value;

#[cfg(test)]
mod tests;
