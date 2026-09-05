// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A counted allowance.
//!
//! Invariant: `used` never exceeds `limit`; a charge that would exceed it
//! changes nothing.

use core::fmt;

/// A charge that would have exceeded the limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QuotaExceeded {
    /// The limit that would have been exceeded.
    pub limit: u32,
    /// What was already in use.
    pub used: u32,
    /// What the rejected charge asked for.
    pub requested: u32,
}

impl fmt::Display for QuotaExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "charging {} would exceed the limit of {} with {} in use",
            self.requested, self.limit, self.used
        )
    }
}

impl From<QuotaExceeded> for audhsos_abi::Error {
    fn from(_error: QuotaExceeded) -> Self {
        audhsos_abi::Error::QuotaExceeded
    }
}

/// How much of an allowance is in use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Quota {
    limit: u32,
    used: u32,
}

impl Quota {
    /// An unused quota with the given limit.
    #[must_use]
    pub const fn new(limit: u32) -> Self {
        Quota { limit, used: 0 }
    }

    /// The limit.
    #[must_use]
    pub const fn limit(self) -> u32 {
        self.limit
    }

    /// What is in use.
    #[must_use]
    pub const fn used(self) -> u32 {
        self.used
    }

    /// What is still available.
    #[must_use]
    pub const fn remaining(self) -> u32 {
        self.limit.saturating_sub(self.used)
    }

    /// `true` if nothing is in use.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.used == 0
    }

    /// Charges `amount` against the quota.
    ///
    /// # Errors
    ///
    /// [`QuotaExceeded`] if the charge would exceed the limit; the quota is
    /// then unchanged.
    pub const fn charge(&mut self, amount: u32) -> Result<(), QuotaExceeded> {
        match self.used.checked_add(amount) {
            Some(used) if used <= self.limit => {
                self.used = used;
                Ok(())
            }
            _ => Err(QuotaExceeded {
                limit: self.limit,
                used: self.used,
                requested: amount,
            }),
        }
    }

    /// Returns `amount` to the quota; refunding more than is in use leaves
    /// the quota empty.
    pub const fn refund(&mut self, amount: u32) {
        self.used = self.used.saturating_sub(amount);
    }
}
