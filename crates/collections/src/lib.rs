// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod array_vec;
pub mod bitset;
pub mod error;
pub mod index_list;
pub mod index_map;
pub mod ring;
#[cfg(any(test, feature = "test-strategies"))]
pub mod strategies;

pub use array_vec::ArrayVec;
pub use bitset::BitSet;
pub use error::CollectionError;
pub use index_list::{IndexList, Link, NONE};
pub use index_map::IndexMap;
pub use ring::RingBuffer;

#[cfg(test)]
mod tests;
