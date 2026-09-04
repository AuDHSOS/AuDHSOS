// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Kernel object types and the rights each type accepts.
//!
//! Invariants: every type has a unique, non-zero code; the rights mask of
//! a type always includes `DUPLICATE` and `TRANSFER`.

use crate::{Error, Rights};

/// Declares the object table once and derives the enum, the code lookup,
/// the name, and the rights mask from it.
macro_rules! object_types {
    ($($variant:ident = $code:literal => [$($right:ident),*]),+ $(,)?) => {
        /// The type of a kernel object.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u32)]
        pub enum ObjectType {
            $(
                #[doc = concat!("The `", stringify!($variant), "` object.")]
                $variant = $code,
            )+
        }

        impl ObjectType {
            /// Every type, in table order.
            pub const ALL: &[ObjectType] = &[$(ObjectType::$variant),+];

            /// The stable numeric code of this type.
            #[must_use]
            #[expect(clippy::as_conversions, reason = "discriminant of a repr(u32) enum in a const fn")]
            pub const fn code(self) -> u32 {
                self as u32
            }

            /// Decodes a numeric code.
            #[must_use]
            pub const fn from_code(code: u32) -> Option<Self> {
                match code {
                    $($code => Some(ObjectType::$variant),)+
                    _ => None,
                }
            }

            /// The name of the type.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(ObjectType::$variant => stringify!($variant),)+
                }
            }

            /// The rights a capability to an object of this type may carry:
            /// the type-specific rights plus `DUPLICATE` and `TRANSFER`.
            #[must_use]
            pub const fn rights_mask(self) -> Rights {
                let specific = match self {
                    $(ObjectType::$variant => Rights::EMPTY $(.union(Rights::$right))*,)+
                };
                specific.union(Rights::DUPLICATE).union(Rights::TRANSFER)
            }
        }
    };
}

object_types! {
    Process = 1 => [MANAGE, MAP, INSTALL],
    Thread = 2 => [MANAGE],
    MemoryObject = 3 => [READ, WRITE, EXECUTE, MAP, INFO],
    Endpoint = 4 => [SEND, RECV, BADGE],
    Reply = 5 => [],
    Notification = 6 => [SIGNAL, WAIT, BIND],
    Interrupt = 7 => [MANAGE],
    IoPortRange = 8 => [READ, WRITE],
    SystemControl = 9 => [MANAGE],
}

impl TryFrom<u32> for ObjectType {
    type Error = Error;

    /// Decodes a numeric code; an unknown code yields `InvalidArgument`.
    fn try_from(code: u32) -> Result<Self, Self::Error> {
        Self::from_code(code).ok_or(Error::InvalidArgument)
    }
}
