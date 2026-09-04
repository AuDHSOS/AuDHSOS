// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Rights carried by a capability.
//!
//! Invariants: every named right has a unique bit; a `Rights` value never
//! contains a bit that is not named; rights only ever shrink along
//! duplication and transfer, which the kernel enforces with
//! [`Rights::is_subset_of`].

use core::fmt;
use core::ops::{BitAnd, BitOr};

use crate::Error;

/// Declares the right table once and derives the constants, the name lookup,
/// and the mask of all rights from it.
macro_rules! rights {
    ($($name:ident = $bit:literal => $doc:literal),+ $(,)?) => {
        impl Rights {
            $(
                #[doc = $doc]
                pub const $name: Rights = Rights(1 << $bit);
            )+

            /// Every named right with its name, in table order.
            pub const NAMED: &[(Rights, &str)] = &[$((Rights::$name, stringify!($name))),+];

            /// The union of every named right.
            pub const ALL: Rights = Rights($((1 << $bit))|+);
        }
    };
}

/// A set of rights, stored as one bit per right.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rights(u32);

rights! {
    READ = 0 => "read the contents of a memory object or an I/O port range",
    WRITE = 1 => "write the contents of a memory object or an I/O port range",
    EXECUTE = 2 => "map a memory object executable",
    MAP = 3 => "map a memory object into an address space",
    INFO = 4 => "query the physical range of a memory object",
    SEND = 5 => "send on an endpoint",
    RECV = 6 => "receive on an endpoint",
    BADGE = 7 => "derive a badged send-only capability of an endpoint",
    SIGNAL = 8 => "signal a notification",
    WAIT = 9 => "wait on a notification",
    BIND = 10 => "bind an interrupt to a notification",
    MANAGE = 11 => "change the state of the object (start, kill, set priority, acknowledge, create)",
    INSTALL = 12 => "install handles into a process",
    DUPLICATE = 13 => "create another handle with equal or fewer rights",
    TRANSFER = 14 => "send the handle in a message or install it into another process",
}

impl Rights {
    /// The empty set.
    pub const EMPTY: Rights = Rights(0);

    /// The raw bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Decodes raw bits.
    ///
    /// # Errors
    ///
    /// Bits that do not belong to a named right are rejected with
    /// [`Error::InvalidArgument`].
    pub const fn from_bits(bits: u32) -> Result<Self, Error> {
        if bits & !Self::ALL.0 == 0 {
            Ok(Rights(bits))
        } else {
            Err(Error::InvalidArgument)
        }
    }

    /// `true` if no right is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// `true` if every right of `other` is also in `self`.
    #[must_use]
    pub const fn contains(self, other: Rights) -> bool {
        self.0 & other.0 == other.0
    }

    /// `true` if every right of `self` is also in `other`.
    #[must_use]
    pub const fn is_subset_of(self, other: Rights) -> bool {
        other.contains(self)
    }

    /// The rights in either set.
    #[must_use]
    pub const fn union(self, other: Rights) -> Rights {
        Rights(self.0 | other.0)
    }

    /// The rights in both sets.
    #[must_use]
    pub const fn intersection(self, other: Rights) -> Rights {
        Rights(self.0 & other.0)
    }

    /// The rights of `self` that are not in `other`.
    #[must_use]
    pub const fn difference(self, other: Rights) -> Rights {
        Rights(self.0 & !other.0)
    }
}

impl BitOr for Rights {
    type Output = Rights;

    fn bitor(self, rhs: Rights) -> Rights {
        self.union(rhs)
    }
}

impl BitAnd for Rights {
    type Output = Rights;

    fn bitand(self, rhs: Rights) -> Rights {
        self.intersection(rhs)
    }
}

impl fmt::Debug for Rights {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Rights(")?;
        let mut first = true;
        for &(right, name) in Self::NAMED {
            if self.contains(right) {
                if !first {
                    f.write_str(" | ")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        if first {
            f.write_str("EMPTY")?;
        }
        f.write_str(")")
    }
}
