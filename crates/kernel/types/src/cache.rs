// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! How the processor may cache a range of physical memory.
//!
//! Invariant: the policy belongs to the memory, not to the table that maps
//! it, so a mapping and a memory object name the same policy for the same
//! frames.

/// How the processor may cache a mapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CachePolicy {
    /// Normal memory.
    #[default]
    WriteBack,
    /// Device memory: never cached, never reordered into a cache line.
    Uncached,
}

impl CachePolicy {
    /// Every policy, in table order.
    pub const ALL: &[CachePolicy] = &[CachePolicy::WriteBack, CachePolicy::Uncached];

    /// The name of the policy.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            CachePolicy::WriteBack => "WriteBack",
            CachePolicy::Uncached => "Uncached",
        }
    }

    /// `true` if the memory must not be cached.
    #[must_use]
    pub const fn is_uncached(self) -> bool {
        matches!(self, CachePolicy::Uncached)
    }
}
