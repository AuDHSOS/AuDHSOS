// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a thread may do next, as a table.
//!
//! Invariants: every legal transition is exactly one row of
//! [`TRANSITIONS`]; a pair of state and event the table does not name is an
//! error and never a panic; [`ThreadState::Exited`] is final, so no row
//! leaves it.

use audhsos_abi::{Error, ThreadState};

/// What happens to a thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// `thread_start`: the thread becomes runnable for the first time.
    Start,
    /// The scheduler picked the thread.
    Schedule,
    /// A thread of higher priority took the processor, or the time slice
    /// ran out.
    Preempt,
    /// The thread gave up the rest of its time slice.
    Yield,
    /// The thread waits for a receiver on an endpoint.
    BlockSend,
    /// The thread waits for a sender on an endpoint.
    BlockRecv,
    /// The thread waits for the reply to a call.
    BlockReply,
    /// The thread waits for signal bits.
    BlockNotification,
    /// What the thread waited for happened, or the object it waited on was
    /// destroyed.
    Wake,
    /// `thread_suspend`.
    Suspend,
    /// `thread_resume`.
    Resume,
    /// The thread faulted and no handler took the fault.
    Fault,
    /// The thread ended, by `thread_exit` or by `thread_kill`.
    Exit,
}

impl Event {
    /// Every event, in table order.
    pub const ALL: &[Event] = &[
        Event::Start,
        Event::Schedule,
        Event::Preempt,
        Event::Yield,
        Event::BlockSend,
        Event::BlockRecv,
        Event::BlockReply,
        Event::BlockNotification,
        Event::Wake,
        Event::Suspend,
        Event::Resume,
        Event::Fault,
        Event::Exit,
    ];

    /// The name of the event.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Event::Start => "Start",
            Event::Schedule => "Schedule",
            Event::Preempt => "Preempt",
            Event::Yield => "Yield",
            Event::BlockSend => "BlockSend",
            Event::BlockRecv => "BlockRecv",
            Event::BlockReply => "BlockReply",
            Event::BlockNotification => "BlockNotification",
            Event::Wake => "Wake",
            Event::Suspend => "Suspend",
            Event::Resume => "Resume",
            Event::Fault => "Fault",
            Event::Exit => "Exit",
        }
    }

    /// The state a thread waits in after this event, for the four events
    /// that block one.
    #[must_use]
    pub const fn blocked_state(self) -> Option<ThreadState> {
        match self {
            Event::BlockSend => Some(ThreadState::BlockedSend),
            Event::BlockRecv => Some(ThreadState::BlockedRecv),
            Event::BlockReply => Some(ThreadState::BlockedReply),
            Event::BlockNotification => Some(ThreadState::BlockedNotification),
            _ => None,
        }
    }
}

/// Every legal transition: the state a thread is in, what happens, and the
/// state it is in afterwards.
pub const TRANSITIONS: &[(ThreadState, Event, ThreadState)] = &[
    // A thread that was created and not started yet.
    (ThreadState::Inactive, Event::Start, ThreadState::Ready),
    (
        ThreadState::Inactive,
        Event::Suspend,
        ThreadState::Suspended,
    ),
    (ThreadState::Inactive, Event::Exit, ThreadState::Exited),
    // Waiting for the processor.
    (ThreadState::Ready, Event::Schedule, ThreadState::Running),
    (ThreadState::Ready, Event::Suspend, ThreadState::Suspended),
    (ThreadState::Ready, Event::Exit, ThreadState::Exited),
    // On the processor.
    (ThreadState::Running, Event::Preempt, ThreadState::Ready),
    (ThreadState::Running, Event::Yield, ThreadState::Ready),
    (
        ThreadState::Running,
        Event::BlockSend,
        ThreadState::BlockedSend,
    ),
    (
        ThreadState::Running,
        Event::BlockRecv,
        ThreadState::BlockedRecv,
    ),
    (
        ThreadState::Running,
        Event::BlockReply,
        ThreadState::BlockedReply,
    ),
    (
        ThreadState::Running,
        Event::BlockNotification,
        ThreadState::BlockedNotification,
    ),
    (ThreadState::Running, Event::Suspend, ThreadState::Suspended),
    (ThreadState::Running, Event::Fault, ThreadState::Faulted),
    (ThreadState::Running, Event::Exit, ThreadState::Exited),
    // Waiting for something that happens elsewhere. Every blocked state
    // wakes into `Ready`, is suspendable, and can be killed.
    (ThreadState::BlockedSend, Event::Wake, ThreadState::Ready),
    (
        ThreadState::BlockedSend,
        Event::Suspend,
        ThreadState::Suspended,
    ),
    (ThreadState::BlockedSend, Event::Exit, ThreadState::Exited),
    (ThreadState::BlockedRecv, Event::Wake, ThreadState::Ready),
    (
        ThreadState::BlockedRecv,
        Event::Suspend,
        ThreadState::Suspended,
    ),
    (ThreadState::BlockedRecv, Event::Exit, ThreadState::Exited),
    (ThreadState::BlockedReply, Event::Wake, ThreadState::Ready),
    (
        ThreadState::BlockedReply,
        Event::Suspend,
        ThreadState::Suspended,
    ),
    (ThreadState::BlockedReply, Event::Exit, ThreadState::Exited),
    (
        ThreadState::BlockedNotification,
        Event::Wake,
        ThreadState::Ready,
    ),
    (
        ThreadState::BlockedNotification,
        Event::Suspend,
        ThreadState::Suspended,
    ),
    (
        ThreadState::BlockedNotification,
        Event::Exit,
        ThreadState::Exited,
    ),
    // Stopped by its creator.
    (ThreadState::Suspended, Event::Resume, ThreadState::Ready),
    (ThreadState::Suspended, Event::Exit, ThreadState::Exited),
    // Stopped by a fault nobody handled. Resuming it is what a creator
    // does after repairing whatever the thread fell over.
    (ThreadState::Faulted, Event::Resume, ThreadState::Ready),
    (ThreadState::Faulted, Event::Exit, ThreadState::Exited),
];

/// The state a thread in `state` reaches when `event` happens.
///
/// # Errors
///
/// [`Error::InvalidState`] when the table does not name the pair, which is
/// what an illegal transition is: resuming a thread that is not suspended,
/// starting one twice, or anything at all after it exited.
pub fn next(state: ThreadState, event: Event) -> Result<ThreadState, Error> {
    TRANSITIONS
        .iter()
        .find(|(from, on, _)| *from == state && *on == event)
        .map(|(_, _, to)| *to)
        .ok_or(Error::InvalidState)
}

/// `true` if `event` may happen to a thread in `state`.
#[must_use]
pub fn is_legal(state: ThreadState, event: Event) -> bool {
    next(state, event).is_ok()
}
