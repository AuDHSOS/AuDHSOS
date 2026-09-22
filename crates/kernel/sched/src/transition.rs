// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! O(1) state transitions, derived from the legal rows in [`TRANSITIONS`].

use audhsos_abi::{Error, ThreadState};

/// What happens to a thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// `thread_start`: the thread becomes runnable for the first time.
    Start,
    /// The scheduler picked the thread.
    Schedule,
    /// The time slice of the thread ran out.
    SliceExpired,
    /// A thread of higher priority took the processor while the slice of
    /// this one still had ticks.
    Displaced,
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
        Event::SliceExpired,
        Event::Displaced,
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
            Event::SliceExpired => "SliceExpired",
            Event::Displaced => "Displaced",
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
    (
        ThreadState::Running,
        Event::SliceExpired,
        ThreadState::Ready,
    ),
    (ThreadState::Running, Event::Displaced, ThreadState::Ready),
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
    // Blocked threads may wake, suspend, or exit.
    (ThreadState::BlockedSend, Event::Wake, ThreadState::Ready),
    (
        ThreadState::BlockedSend,
        Event::Suspend,
        ThreadState::Suspended,
    ),
    (ThreadState::BlockedSend, Event::Exit, ThreadState::Exited),
    // A queued call moves directly from waiting for a receiver to a reply.
    (
        ThreadState::BlockedSend,
        Event::BlockReply,
        ThreadState::BlockedReply,
    ),
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

// ThreadState codes start at one; Event discriminants start at zero.
const STATES: usize = ThreadState::ALL.len() + 1;
const EVENTS: usize = Event::ALL.len();

#[expect(
    clippy::indexing_slicing,
    clippy::as_conversions,
    reason = "const evaluation checks all enum indices and transition rows"
)]
const LOOKUP: [[Option<ThreadState>; EVENTS]; STATES] = {
    let mut table = [[None; EVENTS]; STATES];
    let mut index = 0;
    while index < ThreadState::ALL.len() {
        assert!((ThreadState::ALL[index] as usize) < STATES);
        index += 1;
    }
    index = 0;
    while index < EVENTS {
        assert!((Event::ALL[index] as usize) == index);
        index += 1;
    }
    index = 0;
    while index < TRANSITIONS.len() {
        let (from, event, to) = TRANSITIONS[index];
        assert!(table[from as usize][event as usize].is_none());
        table[from as usize][event as usize] = Some(to);
        index += 1;
    }
    table
};

/// The state reached by `event`, looked up in O(1).
///
/// # Errors
/// [`Error::InvalidState`] for a pair absent from [`TRANSITIONS`].
#[expect(
    clippy::as_conversions,
    reason = "enum discriminants fit usize on kernel targets"
)]
pub fn next(state: ThreadState, event: Event) -> Result<ThreadState, Error> {
    LOOKUP
        .get(state as usize)
        .and_then(|row| row.get(event as usize))
        .copied()
        .flatten()
        .ok_or(Error::InvalidState)
}

/// `true` if `event` may happen to a thread in `state`.
#[must_use]
pub fn is_legal(state: ThreadState, event: Event) -> bool {
    next(state, event).is_ok()
}
