// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why an operation on a container did not happen.

use core::fmt;

/// What a container refused, and why.
///
/// A container of this crate never panics and never grows. Everything that
/// a growing container would do silently is one of these instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CollectionError {
    /// The container is at its capacity. Nothing was stored.
    Full,
    /// An index at or beyond the size of the container.
    Index(usize),
    /// A node handed to a list that is not in that list. It may be in
    /// another one, or in none.
    NotLinked(u32),
    /// A node handed to a list that is already in a list. Linking it twice
    /// would tear both.
    AlreadyLinked(u32),
    /// A list was built with [`NONE`](crate::index_list::NONE) as its
    /// identifier, which is the value that marks a node as belonging to no
    /// list and therefore cannot name one.
    ListId,
}

impl fmt::Display for CollectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CollectionError::Full => f.write_str("the container is full"),
            CollectionError::Index(at) => write!(f, "the index {at} is outside the container"),
            CollectionError::NotLinked(node) => {
                write!(f, "the node {node} is not in this list")
            }
            CollectionError::AlreadyLinked(node) => {
                write!(f, "the node {node} is already in a list")
            }
            CollectionError::ListId => f.write_str("a list cannot be identified by `NONE`"),
        }
    }
}
