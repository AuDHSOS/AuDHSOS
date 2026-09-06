// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One handle type per kind of object, so that a call that wants an
//! endpoint cannot be handed a memory object.
//!
//! A handle is a number, and every system call of this interface takes one
//! as its first argument. The kernel checks the type behind it and answers
//! [`Error::WrongObjectType`](audhsos_abi::Error::WrongObjectType) for a
//! handle of the wrong kind — after a round trip through the gate. The
//! types here refuse the same mistake where it is written.
//!
//! The types are written out one by one rather than derived from
//! [`ObjectType`], because each carries its own documentation of what the
//! object is and what a program does with it, and a table would have
//! nowhere to put that.
//!
//! None of them closes itself when it goes out of scope. `handle_close` is
//! a system call, a system call needs the IPC buffer of the calling thread,
//! and `Drop::drop` is handed nothing but the value. The types are
//! therefore `#[must_use]`, and a program closes what it is done with
//! through the gate.
//!
//! Invariant: a typed handle holds exactly the [`Handle`] it was built
//! from, so a program that gets a handle out of a message and puts it into
//! a call passes on what it received and nothing else.

use audhsos_abi::{Handle, ObjectType};

/// A capability to a process: its address space, its threads, and the
/// handles installed in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub struct ProcessHandle(Handle);

/// A capability to a thread: its state, its priority, and its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub struct ThreadHandle(Handle);

/// A capability to a memory object: a range of frames that can be split and
/// mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub struct MemoryHandle(Handle);

/// A capability to an endpoint: the address messages are sent to and
/// received from. It may carry a badge, which is what the receiver sees of
/// the sender.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub struct EndpointHandle(Handle);

/// A capability to a reply object: the right to answer one call, once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub struct ReplyHandle(Handle);

/// A capability to a notification: sixty-four signal bits a thread can wait
/// for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub struct NotificationHandle(Handle);

/// A capability to an interrupt: one line of the controller, which can be
/// bound to a bit of a notification and acknowledged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub struct InterruptHandle(Handle);

/// A capability to a range of I/O ports, which is the whole of what a
/// userland driver is allowed to reach of the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub struct IoPortHandle(Handle);

/// A capability to system control: the right to create interrupts, port
/// ranges, and device memory. The root task holds the only one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub struct SystemControlHandle(Handle);

/// What every typed handle can do: say which object type it names, be built
/// from a raw handle, and give the raw handle back for a system call.
pub trait Typed: Copy {
    /// The object type a handle of this kind names.
    const TYPE: ObjectType;

    /// A typed handle over `handle`. Nothing is checked here; the kernel
    /// checks the type at the call, and a handle that came out of a startup
    /// message or a reply is the type the sender said it was.
    fn from_handle(handle: Handle) -> Self;

    /// The handle behind the type.
    fn handle(self) -> Handle;

    /// The handle as the word a system call argument carries.
    #[must_use]
    fn raw(self) -> u64 {
        self.handle().raw()
    }

    /// A typed handle over a raw word, or `None` for a word that is no
    /// handle.
    fn from_raw(raw: u64) -> Option<Self> {
        Handle::from_raw(raw).map(Self::from_handle)
    }
}

/// Writes the trait implementation of one handle type. It is the same six
/// lines every time, and the documentation that distinguishes the types
/// stands above them at the type itself.
macro_rules! typed {
    ($name:ident => $object:ident) => {
        impl Typed for $name {
            const TYPE: ObjectType = ObjectType::$object;

            fn from_handle(handle: Handle) -> Self {
                $name(handle)
            }

            fn handle(self) -> Handle {
                self.0
            }
        }

        impl From<$name> for Handle {
            fn from(typed: $name) -> Handle {
                typed.0
            }
        }
    };
}

typed!(ProcessHandle => Process);
typed!(ThreadHandle => Thread);
typed!(MemoryHandle => MemoryObject);
typed!(EndpointHandle => Endpoint);
typed!(ReplyHandle => Reply);
typed!(NotificationHandle => Notification);
typed!(InterruptHandle => Interrupt);
typed!(IoPortHandle => IoPortRange);
typed!(SystemControlHandle => SystemControl);
