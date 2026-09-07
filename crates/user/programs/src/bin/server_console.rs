// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The console driver: bytes out on the serial line, and the bytes that
//! arrive on it.
//!
//! It runs two threads, because a thread of this kernel waits on exactly
//! one thing: `Wait` names an endpoint or a notification, never both. One
//! thread waits on the endpoint and owns the controller and the ring; the
//! other waits on the interrupt, takes the byte the controller has, and
//! sends it to the first over the same endpoint under a badge of its own.
//!
//! Nothing is held by both, so nothing needs a lock. What the second thread
//! touches of the controller is the receive path and what the first touches
//! is the transmit path, and a 16550 keeps those in different registers.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds nine programs and each uses a different part of
// what it depends on; these are the crates this one does not.
use gfx as _;
use server_display as _;
use server_memory as _;
use server_name as _;
use user_loader as _;

use audhsos_abi::Error;
use audhsos_abi::layout::PAGE_SIZE;
use driver_uart16550::{Register, Registers};
use server_console::Console;
use user_programs::client::allocate;
use user_programs::mapping::Mapping;
use user_programs::serve::{Serving, receive};
use user_proto::console::{Chunk, Reply, Request};
use user_rt::{EndpointHandle, InterruptHandle, IoPortHandle, NotificationHandle, Startup};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The badge the interrupt thread sends its bytes under, which is how the
/// serving thread tells them from a client's request.
const INTERRUPT_BADGE: u64 = 0xC0_1DE;

/// The label the interrupt thread sends a byte under.
const BYTE_LABEL: u64 = 1;

/// The bit of the notification the interrupt sets.
const INTERRUPT_BIT: u64 = 0;

/// The base port of the first serial controller.
const COM1: u64 = 0x3F8;

/// The name the driver registers itself under.
const NAME: &[u8] = b"console";

/// Where the stack of the second thread goes.
const SECOND_STACK_TOP: u64 = 0x0080_0000;

/// How many pages that stack gets.
const STACK_PAGES: u64 = 4;

/// What the interrupt thread needs, put where it can find it: it starts
/// with nothing but the address of its own IPC buffer.
static SHARED: Shared = Shared::new();

/// Serves the console until the endpoint is gone.
///
/// The gate is moved into the register block, which is where it stays: the
/// controller is reached through system calls, so the thing that speaks to
/// the controller has to own the thing that makes them. Everything else
/// borrows it back out through `console.uart().registers().gate()`, which
/// is one owner and one borrow at a time.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let (Some(endpoint), Some(ports), Some(interrupt)) =
        (startup.own_endpoint, startup.io_ports, startup.interrupt)
    else {
        gate.thread_exit()
    };
    SHARED.set_ports(ports);

    // The name goes up before the controller comes over. Until this driver
    // touches a port of it the kernel still owns the line, so a failure
    // here can still be said out loud; afterwards the only voice on the
    // line is this program's own.
    match startup.name_server {
        Some(names) => {
            if let Err(error) = register(&mut gate, names, endpoint) {
                let mut line = user_rt::Line::<128>::new();
                line.put(b"[console] cannot register: ");
                line.put(error.message().as_bytes());
                line.put(b"\n");
                let _said = sys::write_line(&mut gate, None, line.as_bytes());
            }
        }
        None => {
            let _said = sys::write_line(&mut gate, None, b"[console] no name server\n");
        }
    }

    let mut console = Console::new(PortRegisters { gate, ports });
    if let Err(error) = start_second_thread(&mut console, &startup, endpoint, interrupt) {
        complain(&mut console, b"no interrupt thread", error);
    }

    let mut serving = Serving::default();
    loop {
        if receive(held(&mut console), endpoint, &mut serving).is_err() {
            held(&mut console).thread_exit()
        }
        if serving.badge == INTERRUPT_BADGE {
            take_arrived(&mut console);
            continue;
        }
        let request = Request::decode(held(&mut console).reader());
        let answer = match request {
            Ok(request) => handle(&mut console, &request),
            Err(error) => Reply::Written(Err(Error::from(error))),
        };
        let _written = answer.encode(&mut held(&mut console).writer());
    }
}

/// Registers the driver under its name.
///
/// The endpoint travels as a handle in the message, so the name server
/// ends up holding a capability to this driver and can hand copies of it
/// to whoever asks for `console`.
fn register(
    gate: &mut Gate,
    names: user_rt::EndpointHandle,
    endpoint: EndpointHandle,
) -> Result<(), Error> {
    use user_proto::name;
    use user_rt::Typed as _;
    let request = name::Request::Register {
        name: name::Name::new(NAME)?,
        endpoint: endpoint.handle(),
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(names)?;
    match name::Reply::decode(gate.reader())? {
        name::Reply::Registered(outcome) => outcome,
        name::Reply::Found(_) => Err(Error::InvalidArgument),
    }
}

/// Runs one step and says on the line which one did not work.
fn step<T>(
    console: &mut Console<PortRegisters>,
    what: &'static [u8],
    body: impl FnOnce(&mut Gate) -> Result<T, Error>,
) -> Result<T, Error> {
    let outcome = body(held(console));
    if let Err(error) = outcome.as_ref() {
        complain(console, what, *error);
    }
    outcome
}

/// Says what went wrong, on the line the driver owns.
fn complain(console: &mut Console<PortRegisters>, what: &[u8], error: Error) {
    let mut line = user_rt::Line::<128>::new();
    line.put(b"[console] ");
    line.put(what);
    line.put(b": ");
    line.put(error.message().as_bytes());
    line.put(b"\n");
    let _said = console.write(line.as_bytes());
}

/// The gate the console holds.
const fn held(console: &mut Console<PortRegisters>) -> &mut Gate {
    &mut console.uart().registers().gate
}

/// What the console does with one request.
fn handle(console: &mut Console<PortRegisters>, request: &Request) -> Reply {
    match request {
        Request::Write { bytes } => Reply::Written(
            console
                .write(bytes.as_bytes())
                .map(|written| u64::try_from(written).unwrap_or(0))
                .map_err(|_| Error::Busy),
        ),
        Request::Read { max } => {
            let mut into = [0u8; user_proto::console::MAX_CHUNK];
            let bound = usize::try_from(*max).unwrap_or(into.len());
            let taken = console.read(bound, &mut into);
            Reply::Read(
                Chunk::new(into.get(..taken).unwrap_or(&[])).map_err(|_| Error::BufferTooSmall),
            )
        }
    }
}

/// Puts the bytes the interrupt thread sent into the ring.
fn take_arrived(console: &mut Console<PortRegisters>) {
    let mut arrived = [0u8; audhsos_abi::layout::MAX_MESSAGE_WORDS];
    let count = {
        let reader = held(console).reader();
        let Ok(message) = reader.message() else {
            return;
        };
        let mut taken = 0usize;
        for (slot, index) in arrived.iter_mut().zip(0..message.word_count) {
            let Some(word) = reader.word(index) else {
                break;
            };
            *slot = u8::try_from(word & 0xFF).unwrap_or(0);
            taken = taken.wrapping_add(1);
        }
        taken
    };
    for byte in arrived.get(..count).unwrap_or(&[]) {
        let _fitted = console.receive(*byte);
    }
}

/// Makes the second thread and starts it: its own stack, its own notification
/// bound to the interrupt, and a badged capability to send bytes through.
fn start_second_thread(
    console: &mut Console<PortRegisters>,
    startup: &Startup,
    endpoint: EndpointHandle,
    interrupt: InterruptHandle,
) -> Result<(), Error> {
    let (Some(own), Some(memory)) = (startup.own_process, startup.memory_server) else {
        return Err(Error::NotFound);
    };
    let notification = step(console, b"notification", Gate::notification_create)?;
    step(console, b"bind", |gate| {
        gate.interrupt_bind(interrupt, notification, INTERRUPT_BIT)
    })?;
    let badged = step(console, b"badge", |gate| {
        gate.endpoint_badge(endpoint, INTERRUPT_BADGE)
    })?;

    let bytes = STACK_PAGES.wrapping_mul(PAGE_SIZE);
    let object = step(console, b"stack memory", |gate| {
        allocate(gate, memory, bytes, PAGE_SIZE)
    })?;
    let base = SECOND_STACK_TOP.wrapping_sub(bytes);
    // The mapping stays: the stack of a thread that never ends is never
    // taken back.
    let _stack = step(console, b"stack map", |gate| {
        Mapping::new(gate, own, object, base, bytes)
    })?;

    SHARED.set(badged, notification, interrupt);
    let gate = held(console);
    let thread = gate.thread_create(
        own,
        entry_address(),
        SECOND_STACK_TOP,
        u64::from(user_programs::priority::DRIVER),
        u64::from(user_programs::priority::DRIVER),
        None,
    )?;
    gate.thread_start(thread)
}

/// Where the second thread begins.
#[expect(
    clippy::as_conversions,
    reason = "a function has to become an address for `thread_create`, and there is no other way to write it"
)]
fn entry_address() -> u64 {
    let pointer: unsafe extern "sysv64" fn(u64) -> ! = second;
    pointer as usize as u64
}

/// The interrupt thread: wait, take the byte, send it, acknowledge.
///
/// # Safety
///
/// The kernel starts this once, with the address of the thread's IPC
/// buffer in the first argument register.
unsafe extern "sysv64" fn second(ipc_buffer: u64) -> ! {
    // SAFETY: the kernel started this thread with the address of its own
    // buffer, and this is the only gate over it.
    let mut gate = unsafe { Gate::adopt(ipc_buffer) };
    let Some((endpoint, notification, interrupt)) = SHARED.get() else {
        gate.thread_exit()
    };
    let mut ports = PortRegisters {
        gate,
        ports: SHARED.ports(),
    };
    loop {
        if ports.gate.notification_wait(notification).is_err() {
            ports.gate.thread_exit()
        }
        // The controller says whether a byte is there; a 16550 raises one
        // interrupt for several reasons.
        if ports.read(Register::LineStatus) & 1 != 0 {
            let byte = ports.read(Register::Data);
            let mut writer = user_rt::message::Writer::new();
            {
                let mut buffer = ports.gate.writer();
                let _written = writer.word(&mut buffer, u64::from(byte));
                let _finished = writer.finish(&mut buffer, BYTE_LABEL);
            }
            let _sent = ports.gate.ipc_send(endpoint);
        }
        let _acknowledged = ports.gate.interrupt_ack(interrupt);
    }
}

/// The register block of the controller, over the port system calls.
struct PortRegisters {
    gate: Gate,
    ports: IoPortHandle,
}

impl Registers for PortRegisters {
    fn read(&mut self, register: Register) -> u8 {
        let port = COM1.wrapping_add(u64::try_from(register.index()).unwrap_or(0));
        self.gate
            .ioport_read(self.ports, port, 1)
            .map_or(0, |value| u8::try_from(value & 0xFF).unwrap_or(0))
    }

    fn write(&mut self, register: Register, value: u8) {
        let port = COM1.wrapping_add(u64::try_from(register.index()).unwrap_or(0));
        let _written = self
            .gate
            .ioport_write(self.ports, port, 1, u64::from(value));
    }
}

/// What the second thread has to be told, in a place it can reach: it
/// starts with nothing but the address of its buffer, and this is a
/// process without a heap.
struct Shared {
    endpoint: core::sync::atomic::AtomicU64,
    notification: core::sync::atomic::AtomicU64,
    interrupt: core::sync::atomic::AtomicU64,
    ports: core::sync::atomic::AtomicU64,
}

impl Shared {
    const fn new() -> Self {
        use core::sync::atomic::AtomicU64;
        Shared {
            endpoint: AtomicU64::new(0),
            notification: AtomicU64::new(0),
            interrupt: AtomicU64::new(0),
            ports: AtomicU64::new(0),
        }
    }

    fn set(
        &self,
        endpoint: EndpointHandle,
        notification: NotificationHandle,
        interrupt: InterruptHandle,
    ) {
        use core::sync::atomic::Ordering;
        use user_rt::Typed;
        self.endpoint.store(endpoint.raw(), Ordering::SeqCst);
        self.notification
            .store(notification.raw(), Ordering::SeqCst);
        self.interrupt.store(interrupt.raw(), Ordering::SeqCst);
    }

    fn set_ports(&self, ports: IoPortHandle) {
        use core::sync::atomic::Ordering;
        use user_rt::Typed;
        self.ports.store(ports.raw(), Ordering::SeqCst);
    }

    fn ports(&self) -> IoPortHandle {
        use core::sync::atomic::Ordering;
        use user_rt::Typed;
        IoPortHandle::from_raw(self.ports.load(Ordering::SeqCst))
            .unwrap_or(IoPortHandle::from_handle(audhsos_abi::Handle::MAX))
    }

    fn get(&self) -> Option<(EndpointHandle, NotificationHandle, InterruptHandle)> {
        use core::sync::atomic::Ordering;
        use user_rt::Typed;
        Some((
            EndpointHandle::from_raw(self.endpoint.load(Ordering::SeqCst))?,
            NotificationHandle::from_raw(self.notification.load(Ordering::SeqCst))?,
            InterruptHandle::from_raw(self.interrupt.load(Ordering::SeqCst))?,
        ))
    }
}
