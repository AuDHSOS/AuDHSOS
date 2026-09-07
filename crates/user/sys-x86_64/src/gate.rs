// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The gate of a thread: the page the kernel mapped for it and the
//! instruction that hands the page over.
//!
//! A system call is three steps — write the call number and the arguments
//! into the page, execute `int 0x80`, read the status word back — and the
//! forty-two methods here are those three steps with the names and the
//! types of the call table. Everything they need of the machine is the
//! address of the page, which the kernel put in the first argument register
//! when it started the thread.
//!
//! The address belongs to the thread and not to the process: the buffer of
//! the thread in slot `n` lies `n` pages below `IPC_BUFFER_TOP`, so a
//! process with two threads has two of them. A gate is therefore a value a
//! thread carries and never a global; there is no thread-local storage in
//! this system to put one in.
//!
//! The methods are written out one by one rather than generated from the
//! call table. A macro would put the argument list in the signature and in
//! the argument array from one place and could not confuse them, which is
//! the mistake that is easiest to make here and hardest to see; what it
//! would cost is that the shape of every call disappears behind an
//! expansion. The test `every_call_has_a_wrapper` holds the list to the
//! table instead.
//!
//! Invariants: a method writes every argument word the call reads and zero
//! into the rest, so no word of an earlier call is read as an argument of
//! this one; a method that returns an error has read the status word and
//! nothing else.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE, Status};
use audhsos_abi::layout::MAX_SYSCALL_ARGUMENTS;
use audhsos_abi::{
    Error, Fault, FaultKind, Framebuffer, FramebufferFormat, Handle, Rights, Syscall, ThreadState,
};
use user_rt::handle::{
    EndpointHandle, InterruptHandle, IoPortHandle, MemoryHandle, NotificationHandle, ProcessHandle,
    ReplyHandle, SystemControlHandle, ThreadHandle, Typed,
};

use crate::syscall;

/// What `thread_info` says about a thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThreadInfo {
    /// What the thread is doing.
    pub state: ThreadState,
    /// The fault it stopped on, for a thread that faulted.
    pub fault: Option<Fault>,
}

/// What `memory_info` says about a memory object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemoryInfo {
    /// The physical address the object starts at.
    pub start: u64,
    /// How many bytes it covers.
    pub length: u64,
}

/// How many words `system_info` writes into the message area.
pub const SYSTEM_INFO_WORDS: usize = 26;

/// Where the six words about the framebuffer begin.
const FRAMEBUFFER_WORD: usize = 20;

/// What `system_info` says about the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SystemInfo {
    /// Capacity and live count of each object pool, in pool order.
    pub pools: [(u64, u64); 9],
    /// Timer ticks per second.
    pub ticks_per_second: u64,
    /// The physical address of the root system description pointer.
    pub acpi: u64,
    /// The framebuffer of the machine, if it has one.
    pub framebuffer: Option<Framebuffer>,
}

/// What a message says of its sender and of the answer it expects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Received {
    /// The badge of the capability the sender used, or zero.
    pub badge: u64,
    /// The reply object, for a message that was a call.
    pub reply: Option<ReplyHandle>,
}

/// Every call the gate has a method for, in the order of the system call
/// table.
///
/// What this proves. A call added to `syscalls!` and forgotten here fails
/// the build, because the lengths differ; a call removed from the table
/// takes its variant with it, so an entry here for it stops compiling. The
/// order has to match too, which makes the two lists readable side by
/// side.
///
/// What it cannot prove is that a method exists for each entry and that
/// each method passes the entry it belongs to: this is a list of values,
/// and Rust gives no way to enumerate the methods of a type without a
/// macro, which [D-92](../../../../docs/09-decisions.md) rules out for
/// this seam. That half is proved by running them: the `wrappers` test
/// image calls all forty-two in the order of this list and holds the
/// numbers the kernel saw against the table, so a method missing from the
/// run, or one passing another call of the same shape, fails there
/// (D-98).
const COVERED: [Syscall; 42] = [
    Syscall::ProcessCreate,
    Syscall::ProcessInstallHandle,
    Syscall::ProcessSetFaultHandler,
    Syscall::ProcessKill,
    Syscall::ThreadCreate,
    Syscall::ThreadStart,
    Syscall::ThreadSuspend,
    Syscall::ThreadResume,
    Syscall::ThreadKill,
    Syscall::ThreadSetPriority,
    Syscall::ThreadInfo,
    Syscall::ThreadExit,
    Syscall::ThreadYield,
    Syscall::MemorySplit,
    Syscall::MemoryMap,
    Syscall::MemoryUnmap,
    Syscall::MemoryProtect,
    Syscall::MemoryInfo,
    Syscall::HandleDuplicate,
    Syscall::HandleClose,
    Syscall::EndpointCreate,
    Syscall::EndpointBadge,
    Syscall::IpcCall,
    Syscall::IpcSend,
    Syscall::IpcRecv,
    Syscall::IpcTryRecv,
    Syscall::IpcReply,
    Syscall::IpcReplyRecv,
    Syscall::NotificationCreate,
    Syscall::NotificationSignal,
    Syscall::NotificationWait,
    Syscall::NotificationPoll,
    Syscall::InterruptCreate,
    Syscall::InterruptBind,
    Syscall::InterruptAck,
    Syscall::IoPortCreate,
    Syscall::IoPortRead,
    Syscall::IoPortWrite,
    Syscall::MemoryCreateDevice,
    Syscall::SystemInfo,
    Syscall::DebugLog,
    Syscall::MemoryMerge,
];

/// `true` when [`COVERED`] is the system call table, in its order.
///
/// The two lists are indexed rather than walked with `get`, which is not a
/// `const fn`. Indexing is safe here in the strongest sense available: the
/// function runs at compile time, so an index outside either list would be
/// an error of the build and not a panic of a running system.
#[expect(
    clippy::indexing_slicing,
    reason = "a const fn cannot call `slice::get`, and an index out of range here fails the build"
)]
const fn covers_the_table() -> bool {
    if COVERED.len() != Syscall::ALL.len() {
        return false;
    }
    let mut index = 0;
    while index < COVERED.len() {
        if COVERED[index].number() != Syscall::ALL[index].number() {
            return false;
        }
        index = index.wrapping_add(1);
    }
    true
}

const _: () = assert!(
    covers_the_table(),
    "the gate and the system call table disagree"
);

/// The gate of one thread.
///
/// It is the address of that thread's IPC buffer and nothing else; the size
/// of the type is the size of an address.
#[derive(Debug)]
pub struct Gate {
    buffer: u64,
}

impl Gate {
    /// The gate over the buffer at `address`.
    ///
    /// # Safety
    ///
    /// `address` must be what the kernel put in the first argument register
    /// at the start of this thread, and no other gate over the same address
    /// may exist: the gate is the only path to the page, and two of them
    /// would be two writers of one buffer.
    #[must_use]
    pub const unsafe fn adopt(address: u64) -> Self {
        Gate { buffer: address }
    }

    /// The address of the buffer, which a program needs when it hands the
    /// page to something that reads it directly.
    #[must_use]
    pub const fn address(&self) -> u64 {
        self.buffer
    }

    /// The buffer, to read a message out of.
    #[must_use]
    pub const fn reader(&self) -> Buffer<'_> {
        let pointer = core::ptr::without_provenance::<[u8; SIZE]>(self.page());
        // SAFETY: the kernel mapped one page there, read and write, for
        // this thread alone, and `adopt` promised the address is that page
        // and that this gate is the only one over it. The borrow of `self`
        // keeps the reference from outliving the gate.
        Buffer::new(unsafe { &*pointer })
    }

    /// The buffer, to build a message in.
    #[must_use]
    pub const fn writer(&mut self) -> BufferMut<'_> {
        let pointer = core::ptr::without_provenance_mut::<[u8; SIZE]>(self.page());
        // SAFETY: as `reader`, and the exclusive borrow of `self` is what
        // makes this the only reference to the page while it lives.
        BufferMut::new(unsafe { &mut *pointer })
    }

    /// The address of the page as a pointer-sized number.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "the target is 64-bit, so an address fits a usize; try_from would add a branch no build of this crate can take"
    )]
    const fn page(&self) -> usize {
        self.buffer as usize
    }

    /// Writes the call and its arguments, makes the call, and reads the
    /// status word.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered, and
    /// [`Error::InvalidArgument`] for a status word that is none, which is
    /// a buffer no kernel of this system wrote.
    fn request(&mut self, call: Syscall, arguments: &[u64]) -> Result<Status, Error> {
        {
            let mut writer = self.writer();
            writer.set_syscall_number(u64::from(call.number()));
            for index in 0..MAX_SYSCALL_ARGUMENTS {
                writer.set_argument(index, arguments.get(index).copied().unwrap_or(0));
            }
        }
        // SAFETY: the buffer of this thread now holds a call number and its
        // arguments, which is what the gate of the interrupt table reads.
        unsafe {
            syscall();
        }
        let status = self.reader().status()?;
        match status.error() {
            Some(error) => Err(error),
            None => Ok(status),
        }
    }

    /// A call that answers nothing.
    fn done(&mut self, call: Syscall, arguments: &[u64]) -> Result<(), Error> {
        self.request(call, arguments)?;
        Ok(())
    }

    /// A call that answers one word.
    fn value(&mut self, call: Syscall, arguments: &[u64]) -> Result<u64, Error> {
        self.request(call, arguments)?;
        Ok(self.reader().return_word(0).unwrap_or(0))
    }

    /// A call that answers two words.
    fn values(&mut self, call: Syscall, arguments: &[u64]) -> Result<(u64, u64), Error> {
        self.request(call, arguments)?;
        let view = self.reader();
        Ok((
            view.return_word(0).unwrap_or(0),
            view.return_word(1).unwrap_or(0),
        ))
    }

    /// A call that answers a handle.
    fn capability<T: Typed>(&mut self, call: Syscall, arguments: &[u64]) -> Result<T, Error> {
        let word = self.value(call, arguments)?;
        Handle::from_raw(word)
            .map(T::from_handle)
            .ok_or(Error::InvalidHandle)
    }

    /// A call that answers a message, and what the receiving side of it
    /// learned about the sender.
    fn received(&mut self, call: Syscall, arguments: &[u64]) -> Result<Received, Error> {
        let (badge, reply) = self.values(call, arguments)?;
        Ok(Received {
            badge,
            reply: Handle::from_raw(reply).map(ReplyHandle::from_handle),
        })
    }

    // --- processes -------------------------------------------------------

    /// `process_create`: a process with an address space of its own, a
    /// handle capacity, and quotas taken out of what `process` holds.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn process_create(
        &mut self,
        process: ProcessHandle,
        capacity: u64,
        frames: u64,
        objects: u64,
    ) -> Result<ProcessHandle, Error> {
        self.capability(
            Syscall::ProcessCreate,
            &[process.raw(), capacity, frames, objects, 0],
        )
    }

    /// `process_install_handle`: puts a capability of the caller into the
    /// table of `process`, with `rights` and no more than it had.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn process_install_handle(
        &mut self,
        process: ProcessHandle,
        handle: Handle,
        rights: Rights,
    ) -> Result<Handle, Error> {
        let word = self.value(
            Syscall::ProcessInstallHandle,
            &[process.raw(), handle.raw(), u64::from(rights.bits())],
        )?;
        Handle::from_raw(word).ok_or(Error::InvalidHandle)
    }

    /// `process_set_fault_handler`: the endpoint the faults of `process`
    /// are reported to. `None` clears it.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn process_set_fault_handler(
        &mut self,
        process: ProcessHandle,
        endpoint: Option<EndpointHandle>,
    ) -> Result<(), Error> {
        let handler = endpoint.map_or(0, Typed::raw);
        self.done(Syscall::ProcessSetFaultHandler, &[process.raw(), handler])
    }

    /// `process_kill`: ends the process, its threads, and everything they
    /// held.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn process_kill(&mut self, process: ProcessHandle) -> Result<(), Error> {
        self.done(Syscall::ProcessKill, &[process.raw()])
    }

    // --- threads ---------------------------------------------------------

    /// `thread_create`: a thread of `process` that will begin at `entry` on
    /// `stack`. It is inactive until `thread_start`.
    ///
    /// `buffer` is the page its IPC buffer goes in. A creator that supplies
    /// one can write the startup message into it before the thread runs,
    /// which is the only way a userland parent has of telling a child what
    /// its handles are; `None` lets the kernel take a frame out of its own
    /// reserve, which is what the root task's own threads get (D-91).
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn thread_create(
        &mut self,
        process: ProcessHandle,
        entry: u64,
        stack: u64,
        priority: u64,
        maximum_priority: u64,
        buffer: Option<MemoryHandle>,
    ) -> Result<ThreadHandle, Error> {
        self.capability(
            Syscall::ThreadCreate,
            &[
                process.raw(),
                entry,
                stack,
                priority,
                maximum_priority,
                buffer.map_or(0, Typed::raw),
            ],
        )
    }

    /// `thread_start`: puts the thread in a run queue.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn thread_start(&mut self, thread: ThreadHandle) -> Result<(), Error> {
        self.done(Syscall::ThreadStart, &[thread.raw()])
    }

    /// `thread_suspend`: takes the thread off the processor and out of
    /// every queue.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn thread_suspend(&mut self, thread: ThreadHandle) -> Result<(), Error> {
        self.done(Syscall::ThreadSuspend, &[thread.raw()])
    }

    /// `thread_resume`: puts a suspended thread back.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn thread_resume(&mut self, thread: ThreadHandle) -> Result<(), Error> {
        self.done(Syscall::ThreadResume, &[thread.raw()])
    }

    /// `thread_kill`: ends the thread wherever it stands.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn thread_kill(&mut self, thread: ThreadHandle) -> Result<(), Error> {
        self.done(Syscall::ThreadKill, &[thread.raw()])
    }

    /// `thread_set_priority`: a priority at or below the maximum the thread
    /// was created with.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn thread_set_priority(
        &mut self,
        thread: ThreadHandle,
        priority: u64,
    ) -> Result<(), Error> {
        self.done(Syscall::ThreadSetPriority, &[thread.raw(), priority])
    }

    /// `thread_info`: what the thread is doing, and what it faulted on.
    ///
    /// The two return words carry the state and the fault kind; the three
    /// words of the message area carry the address, the instruction
    /// pointer, and the error code, and are there only for a thread that
    /// faulted.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered, and [`Error::InvalidArgument`] for a
    /// state or a fault kind that names none.
    pub fn thread_info(&mut self, thread: ThreadHandle) -> Result<ThreadInfo, Error> {
        let (state, kind) = self.values(Syscall::ThreadInfo, &[thread.raw()])?;
        let code = u32::try_from(state).map_err(|_| Error::InvalidArgument)?;
        let state = ThreadState::from_code(code).ok_or(Error::InvalidArgument)?;
        if kind == 0 {
            return Ok(ThreadInfo { state, fault: None });
        }
        let code = u32::try_from(kind).map_err(|_| Error::InvalidArgument)?;
        let kind = FaultKind::from_code(code).ok_or(Error::InvalidArgument)?;
        let view = self.reader();
        Ok(ThreadInfo {
            state,
            fault: Some(Fault {
                kind,
                address: view.word(0).unwrap_or(0),
                instruction_pointer: view.word(1).unwrap_or(0),
                error_code: view.word(2).unwrap_or(0),
            }),
        })
    }

    /// `thread_exit`: ends the calling thread. It does not return, because
    /// the kernel never puts the thread back on the processor.
    pub fn thread_exit(&mut self) -> ! {
        loop {
            let _ended = self.done(Syscall::ThreadExit, &[]);
        }
    }

    /// `thread_yield`: gives the rest of the time slice away.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn thread_yield(&mut self) -> Result<(), Error> {
        self.done(Syscall::ThreadYield, &[])
    }

    // --- memory ----------------------------------------------------------

    /// `memory_split`: divides the object at `offset` and answers with the
    /// upper half. The caller keeps the lower half under the handle it
    /// already had.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn memory_split(
        &mut self,
        memory: MemoryHandle,
        offset: u64,
    ) -> Result<MemoryHandle, Error> {
        self.capability(Syscall::MemorySplit, &[memory.raw(), offset])
    }

    /// `memory_merge`: one object out of two that lie side by side in
    /// physical memory. `lower` grows to cover both and `upper` ceases to
    /// exist, so the handle to it names nothing afterwards.
    ///
    /// This is what lets memory recover: without it an object can only ever
    /// become smaller, and a server that hands memory out and takes it back
    /// would grind its objects down to single pages (D-90).
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn memory_merge(&mut self, lower: MemoryHandle, upper: MemoryHandle) -> Result<(), Error> {
        self.done(Syscall::MemoryMerge, &[lower.raw(), upper.raw()])
    }

    /// `memory_map`: maps `length` bytes of `memory`, from `offset`, at
    /// `address` in `process`, with `permissions`.
    ///
    /// Answers with how many pages were mapped, which is fewer than were
    /// asked for when the status says partial progress; the caller then
    /// calls again for the rest.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn memory_map(
        &mut self,
        process: ProcessHandle,
        memory: MemoryHandle,
        address: u64,
        offset: u64,
        length: u64,
        permissions: u64,
    ) -> Result<u64, Error> {
        self.value(
            Syscall::MemoryMap,
            &[
                process.raw(),
                memory.raw(),
                address,
                offset,
                length,
                permissions,
            ],
        )
    }

    /// `memory_unmap`: takes `length` bytes at `address` out of `process`.
    /// Answers with how many pages were unmapped.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn memory_unmap(
        &mut self,
        process: ProcessHandle,
        address: u64,
        length: u64,
    ) -> Result<u64, Error> {
        self.value(Syscall::MemoryUnmap, &[process.raw(), address, length])
    }

    /// `memory_protect`: changes the permissions of `length` bytes at
    /// `address` in `process`. Answers with how many pages were changed.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn memory_protect(
        &mut self,
        process: ProcessHandle,
        address: u64,
        length: u64,
        permissions: u64,
    ) -> Result<u64, Error> {
        self.value(
            Syscall::MemoryProtect,
            &[process.raw(), address, length, permissions],
        )
    }

    /// `memory_info`: where the object lies and how large it is.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn memory_info(&mut self, memory: MemoryHandle) -> Result<MemoryInfo, Error> {
        let (start, length) = self.values(Syscall::MemoryInfo, &[memory.raw()])?;
        Ok(MemoryInfo { start, length })
    }

    // --- handles ---------------------------------------------------------

    /// `handle_duplicate`: a second handle to the same object, with rights
    /// at or below those of the first.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn handle_duplicate(&mut self, handle: Handle, rights: Rights) -> Result<Handle, Error> {
        let word = self.value(
            Syscall::HandleDuplicate,
            &[handle.raw(), u64::from(rights.bits())],
        )?;
        Handle::from_raw(word).ok_or(Error::InvalidHandle)
    }

    /// `handle_close`: gives the capability up. The object goes with it
    /// when this was the last handle to it.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn handle_close(&mut self, handle: Handle) -> Result<(), Error> {
        self.done(Syscall::HandleClose, &[handle.raw()])
    }

    // --- endpoints and messages ------------------------------------------

    /// `endpoint_create`: an endpoint of the caller's own.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn endpoint_create(&mut self) -> Result<EndpointHandle, Error> {
        self.capability(Syscall::EndpointCreate, &[])
    }

    /// `endpoint_badge`: a capability to the same endpoint that marks
    /// everything sent through it with `badge`.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn endpoint_badge(
        &mut self,
        endpoint: EndpointHandle,
        badge: u64,
    ) -> Result<EndpointHandle, Error> {
        self.capability(Syscall::EndpointBadge, &[endpoint.raw(), badge])
    }

    /// `ipc_call`: sends the message that stands in the buffer and waits
    /// for the answer, which stands in the buffer when the call returns.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn ipc_call(&mut self, endpoint: EndpointHandle) -> Result<(), Error> {
        self.done(Syscall::IpcCall, &[endpoint.raw()])
    }

    /// `ipc_send`: sends the message that stands in the buffer and waits
    /// for nothing.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn ipc_send(&mut self, endpoint: EndpointHandle) -> Result<(), Error> {
        self.done(Syscall::IpcSend, &[endpoint.raw()])
    }

    /// `ipc_recv`: waits for a message, which stands in the buffer when the
    /// call returns.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn ipc_recv(&mut self, endpoint: EndpointHandle) -> Result<Received, Error> {
        self.received(Syscall::IpcRecv, &[endpoint.raw()])
    }

    /// `ipc_try_recv`: takes a message that is already waiting, and answers
    /// [`Error::WouldBlock`] when none is.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn ipc_try_recv(&mut self, endpoint: EndpointHandle) -> Result<Received, Error> {
        self.received(Syscall::IpcTryRecv, &[endpoint.raw()])
    }

    /// `ipc_reply`: answers a call with the message that stands in the
    /// buffer. The reply object is used up.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn ipc_reply(&mut self, reply: ReplyHandle) -> Result<(), Error> {
        self.done(Syscall::IpcReply, &[reply.raw()])
    }

    /// `ipc_reply_recv`: answers a call and waits for the next message in
    /// one step, which is the loop of every server of this system.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn ipc_reply_recv(
        &mut self,
        reply: ReplyHandle,
        endpoint: EndpointHandle,
    ) -> Result<Received, Error> {
        self.received(Syscall::IpcReplyRecv, &[reply.raw(), endpoint.raw()])
    }

    // --- notifications ---------------------------------------------------

    /// `notification_create`: sixty-four signal bits of the caller's own.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn notification_create(&mut self) -> Result<NotificationHandle, Error> {
        self.capability(Syscall::NotificationCreate, &[])
    }

    /// `notification_signal`: sets `bits` and wakes whoever waits.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn notification_signal(
        &mut self,
        notification: NotificationHandle,
        bits: u64,
    ) -> Result<(), Error> {
        self.done(Syscall::NotificationSignal, &[notification.raw(), bits])
    }

    /// `notification_wait`: waits until a bit is set and answers with the
    /// bits, which it clears.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn notification_wait(&mut self, notification: NotificationHandle) -> Result<u64, Error> {
        self.value(Syscall::NotificationWait, &[notification.raw()])
    }

    /// `notification_poll`: the bits that are set, cleared, without
    /// waiting. Zero when none are.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn notification_poll(&mut self, notification: NotificationHandle) -> Result<u64, Error> {
        self.value(Syscall::NotificationPoll, &[notification.raw()])
    }

    // --- devices ---------------------------------------------------------

    /// `interrupt_create`: an interrupt object for `line`.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn interrupt_create(
        &mut self,
        system: SystemControlHandle,
        line: u64,
    ) -> Result<InterruptHandle, Error> {
        self.capability(Syscall::InterruptCreate, &[system.raw(), line])
    }

    /// `interrupt_bind`: an arriving interrupt sets `bit` of
    /// `notification`.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn interrupt_bind(
        &mut self,
        interrupt: InterruptHandle,
        notification: NotificationHandle,
        bit: u64,
    ) -> Result<(), Error> {
        self.done(
            Syscall::InterruptBind,
            &[interrupt.raw(), notification.raw(), bit],
        )
    }

    /// `interrupt_ack`: unmasks the line, which the kernel masked when the
    /// interrupt arrived. Until this is called, no further one comes.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn interrupt_ack(&mut self, interrupt: InterruptHandle) -> Result<(), Error> {
        self.done(Syscall::InterruptAck, &[interrupt.raw()])
    }

    /// `ioport_create`: the right to reach `count` ports from `first`.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn ioport_create(
        &mut self,
        system: SystemControlHandle,
        first: u64,
        count: u64,
    ) -> Result<IoPortHandle, Error> {
        self.capability(Syscall::IoPortCreate, &[system.raw(), first, count])
    }

    /// `ioport_read`: reads `width` bytes from `port`.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn ioport_read(
        &mut self,
        ports: IoPortHandle,
        port: u64,
        width: u64,
    ) -> Result<u64, Error> {
        self.value(Syscall::IoPortRead, &[ports.raw(), port, width])
    }

    /// `ioport_write`: writes `width` bytes of `value` to `port`.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn ioport_write(
        &mut self,
        ports: IoPortHandle,
        port: u64,
        width: u64,
        value: u64,
    ) -> Result<(), Error> {
        self.done(Syscall::IoPortWrite, &[ports.raw(), port, width, value])
    }

    /// `memory_create_device`: a memory object over frames that are not
    /// memory — a register window or a framebuffer.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn memory_create_device(
        &mut self,
        system: SystemControlHandle,
        first_frame: u64,
        frames: u64,
    ) -> Result<MemoryHandle, Error> {
        self.capability(
            Syscall::MemoryCreateDevice,
            &[system.raw(), first_frame, frames],
        )
    }

    /// `system_info`: the capacity and the live count of every object pool,
    /// the tick rate, the root system description pointer, and the
    /// framebuffer of the machine when it has one.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn system_info(&mut self, system: SystemControlHandle) -> Result<SystemInfo, Error> {
        self.request(Syscall::SystemInfo, &[system.raw()])?;
        let view = self.reader();
        let mut pools = [(0u64, 0u64); 9];
        for (index, slot) in pools.iter_mut().enumerate() {
            let at = index.wrapping_mul(2);
            *slot = (
                view.word(at).unwrap_or(0),
                view.word(at.wrapping_add(1)).unwrap_or(0),
            );
        }
        let described = |offset: usize| view.word(FRAMEBUFFER_WORD.saturating_add(offset));
        let framebuffer =
            FramebufferFormat::from_code(u32::try_from(described(5).unwrap_or(0)).unwrap_or(0))
                .map(|format| Framebuffer {
                    phys_start: described(0).unwrap_or(0),
                    len: described(1).unwrap_or(0),
                    width: u32::try_from(described(2).unwrap_or(0)).unwrap_or(0),
                    height: u32::try_from(described(3).unwrap_or(0)).unwrap_or(0),
                    stride: u32::try_from(described(4).unwrap_or(0)).unwrap_or(0),
                    format,
                });
        Ok(SystemInfo {
            pools,
            ticks_per_second: view.word(18).unwrap_or(0),
            acpi: view.word(19).unwrap_or(0),
            framebuffer,
        })
    }

    /// `debug_log`: writes the bytes of the message area to the debug
    /// console of the kernel, in a build that has one and before the
    /// console driver has taken the port over. Answers with how many bytes
    /// were written.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn debug_log(&mut self) -> Result<u64, Error> {
        self.value(Syscall::DebugLog, &[])
    }
}
