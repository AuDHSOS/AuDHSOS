// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What `thread_info` reports: the state of a thread and, when it stopped
//! on a fault, what the fault was.
//!
//! Invariants: every code is unique and never `0`, so a return word that
//! was never written names no state and no fault kind; the codes are part
//! of the ABI and never change.

use crate::Error;

/// Declares the thread state table once and derives the enum, the code
/// lookup, the name, and the list of all states from it.
macro_rules! thread_states {
    ($($variant:ident = $code:literal => $doc:literal),+ $(,)?) => {
        /// What a thread is doing, as `thread_info` reports it.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u32)]
        pub enum ThreadState {
            $(
                #[doc = $doc]
                $variant = $code,
            )+
        }

        impl ThreadState {
            /// Every state, in table order.
            pub const ALL: &[ThreadState] = &[$(ThreadState::$variant),+];

            /// The stable numeric code of this state.
            #[must_use]
            #[expect(clippy::as_conversions, reason = "discriminant of a repr(u32) enum in a const fn")]
            pub const fn code(self) -> u32 {
                self as u32
            }

            /// Decodes a numeric code.
            #[must_use]
            pub const fn from_code(code: u32) -> Option<Self> {
                match code {
                    $($code => Some(ThreadState::$variant),)+
                    _ => None,
                }
            }

            /// The name of the state.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(ThreadState::$variant => stringify!($variant),)+
                }
            }
        }
    };
}

thread_states! {
    Inactive = 1 => "created, not started",
    Ready = 2 => "in a run queue",
    Running = 3 => "on the processor",
    BlockedSend = 4 => "waiting for a receiver on an endpoint",
    BlockedRecv = 5 => "waiting for a sender on an endpoint",
    BlockedReply = 6 => "waiting for the reply to a call",
    BlockedNotification = 7 => "waiting for signal bits",
    Suspended = 8 => "stopped by `thread_suspend`",
    Faulted = 9 => "stopped after a fault with no handler",
    Exited = 10 => "finished; the slot is released when the last reference drops",
}

impl ThreadState {
    /// `true` if the thread waits for something that has to happen
    /// elsewhere before it can run again.
    #[must_use]
    pub const fn is_blocked(self) -> bool {
        matches!(
            self,
            ThreadState::BlockedSend
                | ThreadState::BlockedRecv
                | ThreadState::BlockedReply
                | ThreadState::BlockedNotification
        )
    }

    /// `true` if the scheduler may pick the thread.
    #[must_use]
    pub const fn is_runnable(self) -> bool {
        matches!(self, ThreadState::Ready | ThreadState::Running)
    }

    /// `true` if the thread will never run again.
    #[must_use]
    pub const fn is_dead(self) -> bool {
        matches!(self, ThreadState::Exited)
    }
}

impl TryFrom<u32> for ThreadState {
    type Error = Error;

    /// Decodes a numeric code; an unknown code yields
    /// [`Error::InvalidArgument`].
    fn try_from(code: u32) -> Result<Self, Error> {
        Self::from_code(code).ok_or(Error::InvalidArgument)
    }
}

/// Declares the fault table once and derives the enum, the code lookup,
/// the name, and the list of all kinds from it.
macro_rules! fault_kinds {
    ($($variant:ident = $code:literal => $doc:literal),+ $(,)?) => {
        /// What made a thread fault.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u32)]
        pub enum FaultKind {
            $(
                #[doc = $doc]
                $variant = $code,
            )+
        }

        impl FaultKind {
            /// Every kind, in table order.
            pub const ALL: &[FaultKind] = &[$(FaultKind::$variant),+];

            /// The stable numeric code of this kind.
            #[must_use]
            #[expect(clippy::as_conversions, reason = "discriminant of a repr(u32) enum in a const fn")]
            pub const fn code(self) -> u32 {
                self as u32
            }

            /// Decodes a numeric code.
            #[must_use]
            pub const fn from_code(code: u32) -> Option<Self> {
                match code {
                    $($code => Some(FaultKind::$variant),)+
                    _ => None,
                }
            }

            /// The name of the kind.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(FaultKind::$variant => stringify!($variant),)+
                }
            }
        }
    };
}

fault_kinds! {
    PageFault = 1 => "a memory access the address space does not allow",
    GeneralProtection = 2 => "an operation the privilege level does not allow",
    InvalidOpcode = 3 => "an instruction the processor does not know",
    DivideError = 4 => "a division by zero or an overflow in a division",
    Breakpoint = 5 => "a breakpoint instruction",
    AlignmentCheck = 6 => "an access that is not aligned as the mode requires",
}

impl TryFrom<u32> for FaultKind {
    type Error = Error;

    /// Decodes a numeric code; an unknown code yields
    /// [`Error::InvalidArgument`].
    fn try_from(code: u32) -> Result<Self, Error> {
        Self::from_code(code).ok_or(Error::InvalidArgument)
    }
}

/// What a thread stopped on, as `thread_info` reports it beside the state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fault {
    /// What made the thread fault.
    pub kind: FaultKind,
    /// The address the fault names: the address of the access for a page
    /// fault, the instruction pointer otherwise.
    pub address: u64,
    /// The instruction pointer at the fault.
    pub instruction_pointer: u64,
    /// The error code the processor pushed, or `0` for a fault that has
    /// none.
    pub error_code: u64,
}
