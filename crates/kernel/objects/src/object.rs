// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The objects the kernel holds and the ids that name them.
//!
//! Invariants: every object type of the ABI that this kernel holds has
//! exactly one structure here and one [`Object`] implementation, so an id
//! and a type always agree; an [`AnyObjectId`] carries the type of the
//! object it names, and turning one back into a typed id checks that type.

use audhsos_abi::layout::{
    PRIORITY_COUNT, REGIONS_PER_PROCESS, THREADS_PER_PROCESS, WATCHERS_PER_PROCESS,
};
use audhsos_abi::{Error, Fault, ObjectType, ThreadState};
use kernel_mm::address_space::RegionTable;
use kernel_types::{CachePolicy, PhysFrame, PhysFrameRange, VirtAddr};

use crate::handle_table::HandleList;
use crate::pool::ObjectId;
use crate::quota::Quota;
use crate::wait_queue::WaitQueue;

/// An object the kernel holds in a pool. The associated type code is what
/// lets an [`AnyObjectId`] be checked against the type an operation needs.
pub trait Object {
    /// The type of the object in the ABI.
    const TYPE: ObjectType;
}

/// The name of a process.
pub type ProcessId = ObjectId<Process>;

/// The name of a thread.
pub type ThreadId = ObjectId<Thread>;

/// The name of a memory object.
pub type MemoryObjectId = ObjectId<MemoryObject>;

/// The name of an endpoint.
pub type EndpointId = ObjectId<Endpoint>;

/// The name of a reply object.
pub type ReplyId = ObjectId<Reply>;

/// The name of a notification.
pub type NotificationId = ObjectId<Notification>;

/// The name of an interrupt object.
pub type InterruptId = ObjectId<Interrupt>;

/// The name of an I/O port range.
pub type IoPortRangeId = ObjectId<IoPortRange>;

/// The name of the system control capability. Nothing looks it up: the
/// index and the generation it carries name no slot of any pool.
pub type SystemControlId = ObjectId<SystemControl>;

/// An object id with the type of its object, as a handle table entry holds
/// it. The typed id is recovered with [`AnyObjectId::typed`], which is
/// where the type of an operation is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnyObjectId {
    object_type: ObjectType,
    index: u32,
    generation: u32,
}

impl AnyObjectId {
    /// The untyped form of `id`.
    #[must_use]
    pub const fn of<T: Object>(id: ObjectId<T>) -> Self {
        AnyObjectId {
            object_type: T::TYPE,
            index: id.index(),
            generation: id.generation(),
        }
    }

    /// The type of the object this id names.
    #[must_use]
    pub const fn object_type(self) -> ObjectType {
        self.object_type
    }

    /// The typed id, if the object has the type `T`.
    ///
    /// # Errors
    ///
    /// [`Error::WrongObjectType`] when the object has another type.
    pub const fn typed<T: Object>(self) -> Result<ObjectId<T>, Error> {
        if self.object_type.code() == T::TYPE.code() {
            Ok(ObjectId::new(self.index, self.generation))
        } else {
            Err(Error::WrongObjectType)
        }
    }

    /// The slot index of the object, which says nothing about what the
    /// object is.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// The generation of the object's pool slot.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

/// What a memory object is backed by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MemoryKind {
    /// Ordinary memory the root task handed out.
    #[default]
    Ram,
    /// A device aperture the root task created from `SystemControl`.
    Device,
}

impl MemoryKind {
    /// The name of the kind.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            MemoryKind::Ram => "Ram",
            MemoryKind::Device => "Device",
        }
    }
}

/// A range of physical memory a process may map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemoryObject {
    /// The frames the object covers.
    pub frames: PhysFrameRange,
    /// What the frames are.
    pub kind: MemoryKind,
    /// How the processor may cache the frames.
    pub cache: CachePolicy,
}

impl Object for MemoryObject {
    const TYPE: ObjectType = ObjectType::MemoryObject;
}

impl MemoryObject {
    /// A memory object over `frames`.
    #[must_use]
    pub const fn new(frames: PhysFrameRange, kind: MemoryKind, cache: CachePolicy) -> Self {
        MemoryObject {
            frames,
            kind,
            cache,
        }
    }

    /// How many frames the object covers.
    #[must_use]
    pub const fn frame_count(&self) -> u64 {
        self.frames.count()
    }
}

/// Somebody waiting for the end of a process: which notification is
/// signalled, and which of its sixty-four bits.
///
/// A watch holds no reference to the notification. A notification that is
/// destroyed before the process ends cannot be signalled, and the watch
/// goes with the process it stands on; a watcher that wants its
/// notification to outlive the process keeps its own handle to it, which is
/// what it needs anyway to wait on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Watch {
    /// The notification the end signals.
    pub notification: NotificationId,
    /// Which of its bits.
    pub bit: u8,
}

/// An address space, a handle list, threads, and quotas.
///
/// The address space is the page-table root and the region table, both
/// fields of this structure: a process has exactly one address space,
/// nothing shares one, and there is therefore no address-space object and
/// no id for one (D-65).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Process {
    /// The page-table root of the address space.
    pub root: PhysFrame,
    /// What the address space maps, and what backs each range.
    pub regions: RegionTable<MemoryObjectId, REGIONS_PER_PROCESS>,
    /// The handles the process holds and the ceiling its creator granted.
    pub handles: HandleList,
    /// The threads of the process.
    pub threads: [Option<ThreadId>; THREADS_PER_PROCESS],
    /// The frames of the kernel reserve the process may still take.
    pub quota: Quota,
    /// The kernel objects the process may still create.
    pub kernel_object_quota: Quota,
    /// The endpoint faults of this process are reported on, and the badge
    /// of the capability that named it. A process with none stops the
    /// thread that faulted, which is what a fault nobody takes ends in.
    ///
    /// The badge is kept because a handler serves more than one process:
    /// it is the only thing in the message that says whose fault this is,
    /// exactly as it is for every other message a server receives.
    pub fault_handler: Option<(EndpointId, u64)>,
    /// Who is to be told when the process ends.
    watchers: [Option<Watch>; WATCHERS_PER_PROCESS],
    /// Whether the end has already been told. A process ends once, so the
    /// watchers are signalled once, however often the kernel walks past
    /// the fact afterwards.
    ended: bool,
}

impl Object for Process {
    const TYPE: ObjectType = ObjectType::Process;
}

impl Process {
    /// A process with the given address space root and quotas, and no
    /// thread yet.
    #[must_use]
    pub const fn new(
        root: PhysFrame,
        handles: HandleList,
        quota: Quota,
        kernel_object_quota: Quota,
    ) -> Self {
        Process {
            root,
            regions: RegionTable::new(),
            handles,
            threads: [None; THREADS_PER_PROCESS],
            quota,
            kernel_object_quota,
            fault_handler: None,
            watchers: [None; WATCHERS_PER_PROCESS],
            ended: false,
        }
    }

    /// Records that `watch` is to be told when the process ends.
    ///
    /// # Errors
    ///
    /// [`Error::AlreadyExists`] when that notification already watches this
    /// process on that bit; [`Error::QuotaExceeded`] when
    /// [`WATCHERS_PER_PROCESS`] of them do.
    pub fn add_watcher(&mut self, watch: Watch) -> Result<(), Error> {
        if self.watchers().any(|held| held == watch) {
            return Err(Error::AlreadyExists);
        }
        let slot = self
            .watchers
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(Error::QuotaExceeded)?;
        *slot = Some(watch);
        Ok(())
    }

    /// Everyone waiting for the end of this process.
    pub fn watchers(&self) -> impl Iterator<Item = Watch> + '_ {
        self.watchers.iter().flatten().copied()
    }

    /// Removes exactly this watch. Already delivered notification bits
    /// are not affected; the owner must validate a delayed delivery.
    pub fn remove_watcher(&mut self, watch: Watch) {
        for slot in &mut self.watchers {
            if *slot == Some(watch) {
                *slot = None;
            }
        }
    }

    /// Whether the end of this process has been told.
    #[must_use]
    pub const fn has_ended(&self) -> bool {
        self.ended
    }

    /// Records that the end has been told, so that it is told once.
    pub const fn mark_ended(&mut self) {
        self.ended = true;
    }

    /// Records `thread` as a thread of this process and returns the slot
    /// it took, which is what says where its IPC buffer is mapped.
    ///
    /// # Errors
    ///
    /// [`Error::QuotaExceeded`] when the process already holds
    /// [`THREADS_PER_PROCESS`] threads.
    pub fn add_thread(&mut self, thread: ThreadId) -> Result<usize, Error> {
        let (index, slot) = self
            .threads
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
            .ok_or(Error::QuotaExceeded)?;
        *slot = Some(thread);
        Ok(index)
    }

    /// Forgets `thread`. Returns `false` when the process does not hold it.
    pub fn remove_thread(&mut self, thread: ThreadId) -> bool {
        let Some(slot) = self.threads.iter_mut().find(|slot| **slot == Some(thread)) else {
            return false;
        };
        *slot = None;
        true
    }

    /// The threads of the process, in slot order.
    pub fn threads(&self) -> impl Iterator<Item = ThreadId> + '_ {
        self.threads.iter().flatten().copied()
    }

    /// How many threads the process holds.
    #[must_use]
    pub fn thread_count(&self) -> usize {
        self.threads().count()
    }

    /// Whether the region table of the process has room for `count` more
    /// regions.
    #[must_use]
    pub const fn has_region_room(&self, count: usize) -> bool {
        self.regions.free_slots() >= count
    }
}

/// The links that chain a thread into one scheduler queue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Links {
    /// The next thread of the queue, if any.
    pub next: Option<ThreadId>,
    /// The previous thread of the queue, if any.
    pub previous: Option<ThreadId>,
}

impl Links {
    /// A thread that is in no queue.
    pub const UNLINKED: Links = Links {
        next: None,
        previous: None,
    };

    /// `true` if the thread is in no queue.
    #[must_use]
    pub const fn is_unlinked(self) -> bool {
        self.next.is_none() && self.previous.is_none()
    }
}

/// A thread of a process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thread {
    /// The process the thread belongs to.
    pub process: ProcessId,
    /// What the thread is doing.
    pub state: ThreadState,
    /// The priority the scheduler runs it at.
    pub priority: u8,
    /// The highest priority the thread may ever be given.
    pub max_priority: u8,
    /// Ticks left of its time slice.
    pub time_slice: u32,
    /// The slot of the kernel stack area the thread's stack occupies.
    pub kernel_stack: u32,
    /// The frame holding the thread's IPC buffer.
    pub ipc_buffer: PhysFrame,
    /// The memory object the buffer frame belongs to, for a thread whose
    /// creator supplied one. `None` means the frame came out of the kernel
    /// reserve and goes back there when the thread ends.
    pub buffer_object: Option<MemoryObjectId>,
    /// Where that frame is mapped in the address space of the process,
    /// which is where the thread finds it and what the kernel hands it in
    /// its first register.
    pub ipc_address: VirtAddr,
    /// Where the thread starts, in its own address space.
    pub entry: VirtAddr,
    /// The stack pointer the thread starts with, in its own address space.
    pub user_stack: VirtAddr,
    /// The kernel stack pointer of the thread while it is not running.
    /// One word is the whole saved context (D-67); everything else the
    /// switch has to keep lies on the stack this points at.
    pub context: VirtAddr,
    /// The run queue the scheduler has it in.
    pub queue_links: Links,
    /// The wait queue of an endpoint or a notification it is in. A thread
    /// is in at most one of the two kinds of queue at a time, because a
    /// thread in a run queue is `Ready` and a thread in a wait queue is
    /// blocked (D-74).
    pub wait_links: Links,
    /// What the thread waits on, so that a cancellation finds the queue
    /// without searching every endpoint.
    pub wait: Wait,
    /// The microseconds since boot at which a wait with a deadline ends, or
    /// `None` for a wait without one. A plain word and not an `Instant`: no
    /// kernel crate depends on `audhsos-time`, and comparing two integers
    /// needs no type.
    pub deadline: Option<u64>,
    /// The list of threads that wait with a deadline, ordered by it. A
    /// thread is in it exactly while it is `BlockedNotification` with a
    /// deadline.
    pub deadline_links: Links,
    /// What the thread stopped on, for `thread_info` and for the message a
    /// fault handler receives.
    pub fault: Option<Fault>,
}

impl Object for Thread {
    const TYPE: ObjectType = ObjectType::Thread;
}

impl Thread {
    /// A thread that has not started yet.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] when `priority` is above `max_priority`
    /// or `max_priority` is not a priority of this system.
    pub const fn new(
        process: ProcessId,
        priority: u8,
        max_priority: u8,
        kernel_stack: u32,
        ipc_buffer: PhysFrame,
    ) -> Result<Self, Error> {
        if max_priority >= PRIORITY_COUNT || priority > max_priority {
            return Err(Error::InvalidArgument);
        }
        Ok(Thread {
            process,
            state: ThreadState::Inactive,
            priority,
            max_priority,
            time_slice: 0,
            kernel_stack,
            ipc_buffer,
            buffer_object: None,
            ipc_address: VirtAddr::ZERO,
            entry: VirtAddr::ZERO,
            user_stack: VirtAddr::ZERO,
            context: VirtAddr::ZERO,
            queue_links: Links::UNLINKED,
            wait_links: Links::UNLINKED,
            wait: Wait::Nothing,
            deadline: None,
            deadline_links: Links::UNLINKED,
            fault: None,
        })
    }

    /// The same thread, with its IPC buffer taken out of `object` rather
    /// than out of the kernel reserve.
    #[must_use]
    pub const fn with_buffer_object(self, object: MemoryObjectId) -> Self {
        Thread {
            buffer_object: Some(object),
            ..self
        }
    }

    /// The same thread, starting at `entry` on `user_stack` with its IPC
    /// buffer at `ipc_address`.
    #[must_use]
    pub const fn starting_at(
        self,
        entry: VirtAddr,
        user_stack: VirtAddr,
        ipc_address: VirtAddr,
    ) -> Self {
        Thread {
            ipc_address,
            entry,
            user_stack,
            ..self
        }
    }

    /// `true` if the scheduler may pick the thread.
    #[must_use]
    pub const fn is_runnable(&self) -> bool {
        self.state.is_runnable()
    }
}

/// A synchronous rendezvous point: a queue of threads that want to send
/// and a queue of threads that want to receive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// The threads waiting for a receiver, `ipc_send` and `ipc_call` alike.
    pub senders: WaitQueue,
    /// The threads waiting for a sender.
    pub receivers: WaitQueue,
}

impl Object for Endpoint {
    const TYPE: ObjectType = ObjectType::Endpoint;
}

impl Endpoint {
    /// An endpoint nobody waits on.
    pub const EMPTY: Endpoint = Endpoint {
        senders: WaitQueue::EMPTY,
        receivers: WaitQueue::EMPTY,
    };

    /// An endpoint nobody waits on.
    #[must_use]
    pub const fn new() -> Self {
        Endpoint::EMPTY
    }

    /// `true` if neither queue holds anyone.
    #[must_use]
    pub const fn is_quiet(&self) -> bool {
        self.senders.is_empty() && self.receivers.is_empty()
    }
}

/// The one-shot right to answer a specific caller, created by `ipc_recv`
/// for a sender that used `ipc_call`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Reply {
    /// The thread that waits for the answer.
    pub caller: ThreadId,
    /// Whether the answer has been given, after which the object refuses a
    /// second one.
    pub consumed: bool,
}

impl Object for Reply {
    const TYPE: ObjectType = ObjectType::Reply;
}

impl Reply {
    /// A reply object for `caller`, not yet answered.
    #[must_use]
    pub const fn new(caller: ThreadId) -> Self {
        Reply {
            caller,
            consumed: false,
        }
    }
}

/// Sixty-four signal bits and at most one waiter.
///
/// Which interrupts signal into it is not recorded here: an interrupt
/// carries the notification and the bit it sets, and several of them may
/// name one notification, each on a bit of its own (D-108).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Notification {
    /// The bits that have been signalled and not yet consumed.
    pub word: u64,
    /// The one thread that may wait; a second gets `Busy`.
    pub waiter: Option<ThreadId>,
}

impl Object for Notification {
    const TYPE: ObjectType = ObjectType::Notification;
}

impl Notification {
    /// A notification with nothing signalled and nobody waiting.
    pub const EMPTY: Notification = Notification {
        word: 0,
        waiter: None,
    };

    /// A notification with nothing signalled and nobody waiting.
    #[must_use]
    pub const fn new() -> Self {
        Notification::EMPTY
    }

    /// Takes the bits that are present and clears the word.
    pub const fn consume(&mut self) -> u64 {
        let word = self.word;
        self.word = 0;
        word
    }
}

/// A hardware interrupt line the kernel forwards to a notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Interrupt {
    /// The line, as the interrupt controller numbers it, or `None` for a
    /// message interrupt: a device that writes its vector itself has no
    /// line at any controller the kernel could mask.
    pub line: Option<u8>,
    /// The vector the plan of `kernel_x86_tables::vectors` gives that line,
    /// or the one the vector allocator handed out for a message interrupt.
    pub vector: u8,
    /// The notification the kernel signals, and which bit of it.
    pub notification: Option<(NotificationId, u8)>,
    /// Whether delivery is held off, which it is from the moment an
    /// interrupt arrives until `interrupt_ack`. For a line that is the mask
    /// at the controller; for a message interrupt it is a flag and nothing
    /// more, because the mask bit lies in the device's own table, which is
    /// mapped in the driver and not in the kernel (D-111).
    pub masked: bool,
}

impl Object for Interrupt {
    const TYPE: ObjectType = ObjectType::Interrupt;
}

impl Interrupt {
    /// An interrupt object for `line`, routed to `vector`, bound to
    /// nothing and unmasked.
    #[must_use]
    pub const fn new(line: u8, vector: u8) -> Self {
        Interrupt {
            line: Some(line),
            vector,
            notification: None,
            masked: false,
        }
    }

    /// An interrupt object for a message interrupt on `vector`, which has
    /// no line.
    #[must_use]
    pub const fn message(vector: u8) -> Self {
        Interrupt {
            line: None,
            vector,
            notification: None,
            masked: false,
        }
    }
}

/// The permission to read and write a range of x86 I/O ports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IoPortRange {
    /// The lowest port of the range.
    pub first: u16,
    /// How many ports it covers, which is never zero.
    pub count: u16,
}

impl Object for IoPortRange {
    const TYPE: ObjectType = ObjectType::IoPortRange;
}

impl IoPortRange {
    /// The range of `count` ports at `first`.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] for a count of zero and for a range that
    /// would run past `0xFFFF`.
    pub const fn new(first: u16, count: u16) -> Result<Self, Error> {
        if count == 0 {
            return Err(Error::InvalidArgument);
        }
        match first.checked_add(count.wrapping_sub(1)) {
            Some(_) => Ok(IoPortRange { first, count }),
            None => Err(Error::InvalidArgument),
        }
    }

    /// The highest port of the range.
    #[must_use]
    pub const fn last(&self) -> u16 {
        self.first.wrapping_add(self.count.wrapping_sub(1))
    }

    /// `true` if `port` is in the range.
    #[must_use]
    pub const fn contains(&self, port: u16) -> bool {
        port >= self.first && port <= self.last()
    }

    /// `true` if an access of `width` bytes at `port` stays inside the
    /// range.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "widening a byte width to a port number in a const fn, where From is not yet const"
    )]
    pub const fn holds(&self, port: u16, width: u8) -> bool {
        if !self.contains(port) {
            return false;
        }
        match port.checked_add(width.wrapping_sub(1) as u16) {
            Some(end) => end <= self.last(),
            None => false,
        }
    }

    /// `true` if the two ranges share a port.
    #[must_use]
    pub const fn overlaps(&self, other: &IoPortRange) -> bool {
        self.first <= other.last() && other.first <= self.last()
    }
}

/// The root authority to create interrupt objects, port ranges, and device
/// memory objects.
///
/// It holds nothing: a capability to it is the whole of that right. The
/// four calls that take one check the type the handle names and never look
/// an object up, so it needs no pool, and the index and the generation its
/// [`AnyObjectId`] carries name nothing. The root task receives the one
/// handle to it at boot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SystemControl;

impl Object for SystemControl {
    const TYPE: ObjectType = ObjectType::SystemControl;
}

impl SystemControl {
    /// The id every handle to the system control capability names. Nothing
    /// looks it up; a generation of one keeps it from looking like an id
    /// that was never handed out.
    pub const ID: SystemControlId = ObjectId::new(0, 1);
}

/// Which queue of an endpoint a thread waits in, and what it asked for.
///
/// The rendezvous is completed by whichever side arrives second, so the
/// record has to say what the side that arrived first wanted: a thread
/// that used `ipc_send` is done when its message is taken, and one that
/// used `ipc_call` waits for the answer afterwards. Both wait in the
/// senders queue, and a cancellation treats them alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Queue {
    /// Waiting for a receiver, with no answer expected (`ipc_send`).
    Senders,
    /// Waiting for a receiver and then for the answer (`ipc_call`).
    Callers,
    /// Waiting for a sender (`ipc_recv`).
    Receivers,
}

impl Queue {
    /// `true` for the two that wait in the senders queue of the endpoint.
    #[must_use]
    pub const fn is_sender(self) -> bool {
        matches!(self, Queue::Senders | Queue::Callers)
    }

    /// `true` when the thread expects an answer after its message is
    /// taken.
    #[must_use]
    pub const fn wants_reply(self) -> bool {
        matches!(self, Queue::Callers)
    }
}

/// What a thread waits on. A cancellation reads this to find the queue the
/// thread is in, so that no operation has to search every endpoint (D-74).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Wait {
    /// The thread waits on nothing.
    #[default]
    Nothing,
    /// The thread waits in one of the two queues of an endpoint.
    Endpoint {
        /// The endpoint.
        endpoint: EndpointId,
        /// Which queue, and what it asked for.
        queue: Queue,
        /// The badge of the capability a queued sender used, which is what
        /// the receiver that meets it later sees. Zero for a receiver, which
        /// has no capability of anyone else's in its hand.
        badge: u64,
    },
    /// The thread waits for the answer to a call.
    Reply {
        /// The reply object the receiver holds.
        reply: ReplyId,
    },
    /// The thread waits for signal bits.
    Notification {
        /// The notification.
        notification: NotificationId,
    },
}

impl Wait {
    /// `true` if the thread waits on nothing.
    #[must_use]
    pub const fn is_nothing(self) -> bool {
        matches!(self, Wait::Nothing)
    }
}
