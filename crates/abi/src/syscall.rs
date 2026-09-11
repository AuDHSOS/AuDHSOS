// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The system call table: the one place that says which calls exist, what
//! they are called, how many arguments they take, and what their first
//! argument names.
//!
//! Invariants: every call has a unique, non-zero number that never changes;
//! no call takes more arguments than the argument area of the IPC buffer
//! holds; the kernel dispatcher and the userland wrappers are built from
//! this table and from nothing else.

use crate::{Error, ObjectType};

/// What the first argument of a system call names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FirstArgument {
    /// The call takes no handle; its first argument is a plain value or it
    /// has none.
    Nothing,
    /// The call takes a handle to an object of any type.
    Any,
    /// The call takes a handle to an object of this type.
    Object(ObjectType),
}

impl FirstArgument {
    /// The object type the first argument must name, or `None` when the
    /// call takes no handle or accepts every type.
    #[must_use]
    pub const fn object_type(self) -> Option<ObjectType> {
        match self {
            FirstArgument::Object(ty) => Some(ty),
            FirstArgument::Nothing | FirstArgument::Any => None,
        }
    }

    /// `true` if the first argument is a handle.
    #[must_use]
    pub const fn is_handle(self) -> bool {
        match self {
            FirstArgument::Nothing => false,
            FirstArgument::Any | FirstArgument::Object(_) => true,
        }
    }
}

/// Turns the table's first-argument column into a [`FirstArgument`].
/// `Nothing` and `Any` name themselves; every other name is an object type.
macro_rules! first_argument {
    (Nothing) => {
        FirstArgument::Nothing
    };
    (Any) => {
        FirstArgument::Any
    };
    ($ty:ident) => {
        FirstArgument::Object(ObjectType::$ty)
    };
}

/// Declares the system call table once and derives the enum, the number
/// lookup, the name, the argument count, and the first argument from it.
macro_rules! syscalls {
    ($($variant:ident = $number:literal => $name:literal ($args:literal, $object:ident)),+ $(,)?) => {
        /// A system call of this interface.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u32)]
        pub enum Syscall {
            $(
                #[doc = concat!("The `", $name, "` system call.")]
                $variant = $number,
            )+
        }

        impl Syscall {
            /// Every call, in table order.
            pub const ALL: &[Syscall] = &[$(Syscall::$variant),+];

            /// The stable number of this call.
            #[must_use]
            #[expect(clippy::as_conversions, reason = "discriminant of a repr(u32) enum in a const fn")]
            pub const fn number(self) -> u32 {
                self as u32
            }

            /// Decodes a call number.
            #[must_use]
            pub const fn from_number(number: u32) -> Option<Self> {
                match number {
                    $($number => Some(Syscall::$variant),)+
                    _ => None,
                }
            }

            /// The name of the call, as the documentation and the userland
            /// wrappers spell it.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Syscall::$variant => $name,)+
                }
            }

            /// How many argument words the call reads.
            #[must_use]
            pub const fn argument_count(self) -> u8 {
                match self {
                    $(Syscall::$variant => $args,)+
                }
            }

            /// What the first argument of the call names.
            #[must_use]
            pub const fn first_argument(self) -> FirstArgument {
                match self {
                    $(Syscall::$variant => first_argument!($object),)+
                }
            }
        }
    };
}

syscalls! {
    ProcessCreate = 1 => "process_create" (5, Process),
    ProcessInstallHandle = 2 => "process_install_handle" (3, Process),
    ProcessSetFaultHandler = 3 => "process_set_fault_handler" (2, Process),
    ProcessKill = 4 => "process_kill" (1, Process),
    ThreadCreate = 5 => "thread_create" (6, Process),
    ThreadStart = 6 => "thread_start" (1, Thread),
    ThreadSuspend = 7 => "thread_suspend" (1, Thread),
    ThreadResume = 8 => "thread_resume" (1, Thread),
    ThreadKill = 9 => "thread_kill" (1, Thread),
    ThreadSetPriority = 10 => "thread_set_priority" (2, Thread),
    ThreadInfo = 11 => "thread_info" (1, Thread),
    ThreadExit = 12 => "thread_exit" (0, Nothing),
    ThreadYield = 13 => "thread_yield" (0, Nothing),
    MemorySplit = 14 => "memory_split" (2, MemoryObject),
    MemoryMap = 15 => "memory_map" (6, Process),
    MemoryUnmap = 16 => "memory_unmap" (3, Process),
    MemoryProtect = 17 => "memory_protect" (4, Process),
    MemoryInfo = 18 => "memory_info" (1, MemoryObject),
    HandleDuplicate = 19 => "handle_duplicate" (2, Any),
    HandleClose = 20 => "handle_close" (1, Any),
    EndpointCreate = 21 => "endpoint_create" (0, Nothing),
    EndpointBadge = 22 => "endpoint_badge" (2, Endpoint),
    IpcCall = 23 => "ipc_call" (1, Endpoint),
    IpcSend = 24 => "ipc_send" (1, Endpoint),
    IpcRecv = 25 => "ipc_recv" (1, Endpoint),
    IpcTryRecv = 26 => "ipc_try_recv" (1, Endpoint),
    IpcReply = 27 => "ipc_reply" (1, Reply),
    IpcReplyRecv = 28 => "ipc_reply_recv" (2, Reply),
    NotificationCreate = 29 => "notification_create" (0, Nothing),
    NotificationSignal = 30 => "notification_signal" (2, Notification),
    NotificationWait = 31 => "notification_wait" (1, Notification),
    NotificationPoll = 32 => "notification_poll" (1, Notification),
    InterruptCreate = 33 => "interrupt_create" (2, SystemControl),
    InterruptBind = 34 => "interrupt_bind" (3, Interrupt),
    InterruptAck = 35 => "interrupt_ack" (1, Interrupt),
    IoPortCreate = 36 => "ioport_create" (3, SystemControl),
    IoPortRead = 37 => "ioport_read" (3, IoPortRange),
    IoPortWrite = 38 => "ioport_write" (4, IoPortRange),
    MemoryCreateDevice = 39 => "memory_create_device" (3, SystemControl),
    SystemInfo = 40 => "system_info" (1, SystemControl),
    DebugLog = 41 => "debug_log" (0, Nothing),
    MemoryMerge = 42 => "memory_merge" (2, MemoryObject),
    ProcessWatch = 43 => "process_watch" (3, Process),
    ProcessUnwatch = 44 => "process_unwatch" (3, Process),
    MemoryReferences = 45 => "memory_references" (1, MemoryObject),
    IoPortWriteString = 46 => "ioport_write_string" (3, IoPortRange),
    ClockNow = 47 => "clock_now" (0, Nothing),
    NotificationWaitUntil = 48 => "notification_wait_until" (2, Notification),
    RandomBytes = 49 => "random_bytes" (0, Nothing),
    InterruptCreateMsi = 50 => "interrupt_create_msi" (1, SystemControl),
}

impl Syscall {
    /// The object type the first argument must name, or `None` when the
    /// call takes no handle or accepts a handle of every type.
    #[must_use]
    pub const fn object_type(self) -> Option<ObjectType> {
        self.first_argument().object_type()
    }

    /// `true` if the first argument of the call is a handle.
    #[must_use]
    pub const fn takes_handle(self) -> bool {
        self.first_argument().is_handle()
    }

    /// Decodes the call number as it appears in the IPC buffer, which is a
    /// full word there.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownSyscall`] for a word that names no call, a number
    /// above the table included.
    pub const fn from_word(word: u64) -> Result<Self, Error> {
        if word > 0xFFFF_FFFF {
            return Err(Error::UnknownSyscall);
        }
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            reason = "the bound above keeps the value inside u32, and the function is const"
        )]
        let number = word as u32;
        match Self::from_number(number) {
            Some(call) => Ok(call),
            None => Err(Error::UnknownSyscall),
        }
    }
}

impl TryFrom<u32> for Syscall {
    type Error = Error;

    /// Decodes a call number; a number that names no call yields
    /// [`Error::UnknownSyscall`].
    fn try_from(number: u32) -> Result<Self, Error> {
        Self::from_number(number).ok_or(Error::UnknownSyscall)
    }
}
