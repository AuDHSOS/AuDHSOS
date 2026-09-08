// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The input server: the one program that reaches the PS/2 controller.
//!
//! It runs two threads, as the console driver does and for the same reason:
//! a thread of this kernel waits on exactly one thing, and this server waits
//! on its endpoint and on the interrupts of two lines. The second thread
//! owns the ports, drains the controller, and sends what it read to the
//! first over the same endpoint under a badge of its own; the first owns the
//! decoders, the subscribers, and their rings. Nothing is held by both, so
//! nothing needs a lock.
//!
//! Both interrupts are bound to one notification, each on a bit of its own
//! (D-108). The controller has one output buffer and two lines, and the byte
//! standing in it belongs to whichever device the status register names, so
//! one thread drains it and lets both lines go whichever of them woke it.
//!
//! A client subscribes once and is given a ring of one page. Everything
//! after that is shared memory and a signal, so a keystroke costs no message
//! at all. A client whose notification can no longer be signalled has ended,
//! and its subscription and its ring go with it, which is why this server
//! watches nobody.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds eleven programs and each uses a different part of what
// it depends on; these are the crates this one does not.
use driver_uart16550 as _;
use gfx as _;
use server_console as _;
use server_display as _;
use server_memory as _;
use server_name as _;
use user_loader as _;

use core::sync::atomic::{AtomicU64, Ordering};

use audhsos_abi::layout::{MAX_MESSAGE_WORDS, PAGE_SIZE};
use audhsos_abi::{Error, Handle};
use driver_i8042::controller::{COMMAND_PORT, Controller, DATA_PORT, Devices, Ports};
use server_input::service::{Line, Lines, service, unpack};
use server_input::state::{Clients, Input};
use user_programs::client::{allocate, register, release, write_line};
use user_programs::mapping::Mapping;
use user_programs::serve::{Serving, receive};
use user_proto::input::{Event, Reply, Request, RingWriter};
use user_rt::{
    EndpointHandle, InterruptHandle, IoPortHandle, MemoryHandle, NotificationHandle, Startup, Typed,
};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the server registers itself under.
const NAME: &[u8] = b"input";

/// How many clients this process holds rings for.
const CLIENTS: usize = 4;

/// Where the ring of the first client is mapped.
const RINGS: u64 = 0x2000_0000;

/// The badge the draining thread sends its bytes under, which is how the
/// serving thread tells them from a client's request.
const INTERRUPT_BADGE: u64 = 0x1_4042;

/// The label it sends them under.
const BYTES_LABEL: u64 = 1;

/// The bit of the notification the keyboard line signals.
const KEYBOARD_BIT: u64 = 0;

/// The bit the mouse line signals.
const MOUSE_BIT: u64 = 1;

/// The bits this server sets on a subscriber's notification: one, and there
/// is nothing else to say than that something is waiting.
const EVENT_BITS: u64 = 1;

/// The top of the stack of the draining thread, and how many pages it gets.
const DRAIN_STACK_TOP: u64 = 0x0080_0000;
const DRAIN_STACK_PAGES: u64 = 4;

/// What the draining thread has to be told, in a place it can reach: it
/// starts with nothing but the address of its own IPC buffer, and this is a
/// process without a heap.
static SHARED: Shared = Shared::new();

/// Serves the two devices until the endpoint is gone.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let (Some(endpoint), Some(ports)) = (startup.own_endpoint, startup.io_ports) else {
        gate.thread_exit()
    };
    if let Some(names) = startup.name_server {
        let outcome = register(&mut gate, names, NAME, endpoint);
        if let Err(error) = outcome {
            say(&mut gate, &startup, "cannot register", error);
        }
    }

    // The controller comes up on this thread, before there is a second one:
    // nothing may read the output buffer while the devices are answering
    // their reset.
    let mut controller = Controller::new(PortRegisters::new(gate, ports));
    let devices = controller.init().unwrap_or_default();
    let mut gate = controller.into_ports().gate;
    say_devices(&mut gate, &startup, devices);

    if let Err(error) = start_drain_thread(&mut gate, &startup, endpoint) {
        say(&mut gate, &startup, "no draining thread", error);
    }

    let mut input: Input<CLIENTS> = Input::new(devices.mouse_id);
    let mut held: [Option<Held>; CLIENTS] = [const { None }; CLIENTS];
    let mut serving = Serving::default();
    loop {
        if receive(&mut gate, endpoint, &mut serving).is_err() {
            gate.thread_exit()
        }
        // The draining thread sends bytes, not requests, and it sends them
        // with `ipc_send`, so there is nothing to reply to.
        if serving.badge == INTERRUPT_BADGE {
            arrived(&mut gate, &startup, &mut input, &mut held);
            continue;
        }
        let answer = match Request::decode(gate.reader()) {
            Ok(request) => handle(
                &mut gate,
                &startup,
                &mut input,
                &mut held,
                serving.badge,
                request,
            ),
            Err(error) => Reply::Unsubscribed(Err(Error::from(error))),
        };
        let _written = answer.encode(&mut gate.writer());
    }
}

/// The ring of one client, and what the server holds of that client.
struct Held {
    /// The memory object of its ring.
    memory: MemoryHandle,
    /// The notification the client waits on.
    notification: NotificationHandle,
    /// Where the ring is mapped in this process.
    mapping: Mapping,
}

/// The rings, and the way to wake the clients that hold them.
struct Rings<'a> {
    gate: &'a mut Gate,
    held: &'a mut [Option<Held>; CLIENTS],
}

impl Clients for Rings<'_> {
    fn ring(&mut self, slot: usize) -> Option<&mut [u8]> {
        let held = self.held.get_mut(slot)?.as_mut()?;
        // SAFETY: the mapping was made when the client subscribed, nothing
        // has unmapped it, and this is the only reference to it while the
        // event is appended.
        Some(unsafe { held.mapping.bytes() })
    }

    fn wake(&mut self, slot: usize) -> bool {
        let Some(Some(held)) = self.held.get(slot) else {
            return false;
        };
        let notification = held.notification;
        self.gate
            .notification_signal(notification, EVENT_BITS)
            .is_ok()
    }
}

/// What the server answers to one request.
fn handle(
    gate: &mut Gate,
    startup: &Startup,
    input: &mut Input<CLIENTS>,
    held: &mut [Option<Held>; CLIENTS],
    badge: u64,
    request: Request,
) -> Reply {
    match request {
        Request::Subscribe { notification } => Reply::Subscribed(subscribe(
            gate,
            startup,
            input,
            held,
            badge,
            NotificationHandle::from_handle(notification),
        )),
        Request::Unsubscribe => Reply::Unsubscribed(unsubscribe(gate, startup, input, held, badge)),
    }
}

/// Makes a ring for a client: memory out of the memory server, mapped here
/// so that the events can be written into it, and the handle to it in the
/// answer.
fn subscribe(
    gate: &mut Gate,
    startup: &Startup,
    input: &mut Input<CLIENTS>,
    held: &mut [Option<Held>; CLIENTS],
    badge: u64,
    notification: NotificationHandle,
) -> Result<Handle, Error> {
    let (Some(process), Some(server)) = (startup.own_process, startup.memory_server) else {
        return Err(Error::NotFound);
    };
    let slot = input.subscribe(badge)?;
    let outcome = make_ring(gate, process, server, notification, held, slot);
    match outcome {
        Ok(_given) => say_line(
            gate,
            startup,
            user_rt::Line::<96>::of(format_args!("[input] client {badge} listens\n")).as_bytes(),
        ),
        Err(_error) => {
            let _gone = input.unsubscribe(badge);
            let _closed = gate.handle_close(notification.handle());
        }
    }
    outcome
}

/// The memory, the mapping, and the header of one ring.
fn make_ring(
    gate: &mut Gate,
    process: user_rt::ProcessHandle,
    server: EndpointHandle,
    notification: NotificationHandle,
    held: &mut [Option<Held>; CLIENTS],
    slot: usize,
) -> Result<Handle, Error> {
    let memory = allocate(gate, server, PAGE_SIZE, PAGE_SIZE)?;
    let mut mapping = match Mapping::new(gate, process, memory, window_of(slot), PAGE_SIZE) {
        Ok(mapping) => mapping,
        Err(error) => {
            let _released = release(gate, server, memory);
            return Err(error);
        }
    };
    {
        // SAFETY: the mapping was made just now, it is still standing, and
        // nothing else in this program holds a reference to it.
        let bytes = unsafe { mapping.bytes() };
        RingWriter::create(bytes).ok_or(Error::InvalidArgument)?;
    }
    // The handle goes to the client as it stands. A memory object of this
    // system carries no `DUPLICATE` — the root task withholds it and every
    // object the memory server hands out comes out of one it was given — so
    // a handle with fewer rights cannot be made from it. The client needs
    // `READ` and `WRITE` in any case: the reader advances the read sequence
    // and clears the count of what was dropped, and both of those are
    // writes into the page.
    let given = memory.handle();
    if let Some(place) = held.get_mut(slot) {
        *place = Some(Held {
            memory,
            notification,
            mapping,
        });
    }
    Ok(given)
}

/// Lets a client go: the ring is unmapped and given back.
fn unsubscribe(
    gate: &mut Gate,
    startup: &Startup,
    input: &mut Input<CLIENTS>,
    held: &mut [Option<Held>; CLIENTS],
    badge: u64,
) -> Result<(), Error> {
    let slot = input.unsubscribe(badge)?;
    give_back(gate, startup, held, slot);
    Ok(())
}

/// Unmaps a ring, gives its memory back, and lets go of the notification.
fn give_back(gate: &mut Gate, startup: &Startup, held: &mut [Option<Held>; CLIENTS], slot: usize) {
    let Some(taken) = held.get_mut(slot).and_then(Option::take) else {
        return;
    };
    if let Some(process) = startup.own_process {
        let _unmapped = taken.mapping.unmap(gate, process);
    }
    if let Some(server) = startup.memory_server {
        let _released = release(gate, server, taken.memory);
    }
    let _closed = gate.handle_close(taken.notification.handle());
}

/// Takes the bytes the draining thread sent, decodes them, and hands what
/// they made to everyone who listens.
fn arrived(
    gate: &mut Gate,
    startup: &Startup,
    input: &mut Input<CLIENTS>,
    held: &mut [Option<Held>; CLIENTS],
) {
    let mut words = [0u64; MAX_MESSAGE_WORDS];
    let count = {
        let reader = gate.reader();
        let Ok(message) = reader.message() else {
            return;
        };
        let mut taken = 0usize;
        for (slot, index) in words.iter_mut().zip(0..message.word_count) {
            let Some(word) = reader.word(index) else {
                break;
            };
            *slot = word;
            taken = taken.wrapping_add(1);
        }
        taken
    };
    for word in words.get(..count).unwrap_or(&[]) {
        let (byte, aux) = unpack(*word);
        let produced = input.feed(byte, aux);
        for event in &produced {
            deliver(gate, startup, input, held, *event);
        }
    }
}

/// Puts one event into every ring and gives back what the clients that are
/// gone held.
fn deliver(
    gate: &mut Gate,
    startup: &Startup,
    input: &mut Input<CLIENTS>,
    held: &mut [Option<Held>; CLIENTS],
    event: Event,
) {
    let gone = {
        let mut rings = Rings { gate, held };
        input.deliver(event, &mut rings)
    };
    for subscriber in &gone {
        say_line(
            gate,
            startup,
            user_rt::Line::<96>::of(format_args!(
                "[input] client {} is gone: ring released\n",
                subscriber.badge
            ))
            .as_bytes(),
        );
        give_back(gate, startup, held, subscriber.slot);
    }
}

/// Makes the draining thread and starts it: its own stack, one notification
/// with both lines bound to it, and a badged capability to send bytes
/// through.
fn start_drain_thread(
    gate: &mut Gate,
    startup: &Startup,
    endpoint: EndpointHandle,
) -> Result<(), Error> {
    let (Some(own), Some(memory), Some(ports)) =
        (startup.own_process, startup.memory_server, startup.io_ports)
    else {
        return Err(Error::NotFound);
    };
    let notification = gate.notification_create()?;
    if let Some(keyboard) = startup.interrupt {
        gate.interrupt_bind(keyboard, notification, KEYBOARD_BIT)?;
    }
    if let Some(mouse) = startup.aux_interrupt {
        gate.interrupt_bind(mouse, notification, MOUSE_BIT)?;
    }
    let badged = gate.endpoint_badge(endpoint, INTERRUPT_BADGE)?;

    let bytes = DRAIN_STACK_PAGES.wrapping_mul(PAGE_SIZE);
    let object = allocate(gate, memory, bytes, PAGE_SIZE)?;
    let base = DRAIN_STACK_TOP.wrapping_sub(bytes);
    // The mapping stays: the stack of a thread that never ends is never
    // taken back.
    let _stack = Mapping::new(gate, own, object, base, bytes)?;

    SHARED.set(
        badged,
        notification,
        ports,
        startup.interrupt,
        startup.aux_interrupt,
    );
    let thread = gate.thread_create(
        own,
        entry_address(),
        DRAIN_STACK_TOP,
        u64::from(user_programs::priority::DRIVER),
        u64::from(user_programs::priority::DRIVER),
        None,
    )?;
    gate.thread_start(thread)
}

/// Where the draining thread begins.
#[expect(
    clippy::as_conversions,
    reason = "a function has to become an address for `thread_create`, and there is no other way to write it"
)]
fn entry_address() -> u64 {
    let pointer: unsafe extern "sysv64" fn(u64) -> ! = drain;
    pointer as usize as u64
}

/// The draining thread: wait, empty the buffer, send what was in it, let
/// both lines go.
///
/// # Safety
///
/// The kernel starts this once, with the address of the thread's IPC buffer
/// in the first argument register.
unsafe extern "sysv64" fn drain(ipc_buffer: u64) -> ! {
    // SAFETY: the kernel started this thread with the address of its own
    // buffer, and this is the only gate over it.
    let gate = unsafe { Gate::adopt(ipc_buffer) };
    let Some((endpoint, notification, ports)) = SHARED.get() else {
        let mut gate = gate;
        gate.thread_exit()
    };
    let mut controller = Controller::new(PortRegisters {
        gate,
        ports,
        keyboard: SHARED.keyboard(),
        mouse: SHARED.mouse(),
    });
    loop {
        if controller
            .ports()
            .gate
            .notification_wait(notification)
            .is_err()
        {
            controller.ports().gate.thread_exit()
        }
        let mut words = [0u64; MAX_MESSAGE_WORDS];
        let taken = service(&mut controller, &mut words);
        if taken == 0 {
            continue;
        }
        let mut writer = user_rt::message::Writer::new();
        {
            let mut buffer = controller.ports().gate.writer();
            for word in words.get(..taken).unwrap_or(&[]) {
                let _written = writer.word(&mut buffer, *word);
            }
            let _finished = writer.finish(&mut buffer, BYTES_LABEL);
        }
        let _sent = controller.ports().gate.ipc_send(endpoint);
    }
}

/// Says which devices came up, on the console the root task gave this
/// program.
fn say_devices(gate: &mut Gate, startup: &Startup, devices: Devices) {
    let line = user_rt::Line::<128>::of(format_args!(
        "[input] keyboard={} mouse={} id={}\n",
        devices.keyboard, devices.mouse, devices.mouse_id
    ));
    say_line(gate, startup, line.as_bytes());
}

/// Says what went wrong, in one line.
fn say(gate: &mut Gate, startup: &Startup, what: &str, error: Error) {
    let line = user_rt::Line::<128>::of(format_args!("[input] {what}: {}\n", error.message()));
    say_line(gate, startup, line.as_bytes());
}

/// Writes one line to the console the root task gave this program, and
/// nowhere when it was given none.
fn say_line(gate: &mut Gate, startup: &Startup, line: &[u8]) {
    if let Some(console) = startup.log {
        let _said = write_line(gate, console, line);
    }
}

/// The window of the address space slot `slot` maps its ring in.
fn window_of(slot: usize) -> u64 {
    let step = u64::try_from(slot).unwrap_or(0).saturating_mul(PAGE_SIZE);
    RINGS.saturating_add(step)
}

/// The two ports of the controller and the two lines it asserts, over the
/// system calls that reach them. Both go through one gate, which is why one
/// type carries both.
struct PortRegisters {
    gate: Gate,
    ports: IoPortHandle,
    keyboard: Option<InterruptHandle>,
    mouse: Option<InterruptHandle>,
}

impl PortRegisters {
    /// The ports without the lines, which is what the thread that brings
    /// the controller up holds: nothing is bound yet.
    const fn new(gate: Gate, ports: IoPortHandle) -> Self {
        PortRegisters {
            gate,
            ports,
            keyboard: None,
            mouse: None,
        }
    }

    /// One byte out of `port`, or zero when the call is refused.
    fn read(&mut self, port: u64) -> u8 {
        self.gate
            .ioport_read(self.ports, port, 1)
            .map_or(0, |value| u8::try_from(value & 0xFF).unwrap_or(0))
    }

    /// One byte into `port`.
    fn write(&mut self, port: u64, value: u8) {
        let _written = self
            .gate
            .ioport_write(self.ports, port, 1, u64::from(value));
    }
}

impl Ports for PortRegisters {
    fn read_data(&mut self) -> u8 {
        self.read(u64::from(DATA_PORT))
    }

    fn read_status(&mut self) -> u8 {
        self.read(u64::from(COMMAND_PORT))
    }

    fn write_data(&mut self, value: u8) {
        self.write(u64::from(DATA_PORT), value);
    }

    fn write_command(&mut self, value: u8) {
        self.write(u64::from(COMMAND_PORT), value);
    }
}

impl Lines for PortRegisters {
    fn acknowledge(&mut self, line: Line) {
        let handle = match line {
            Line::Keyboard => self.keyboard,
            Line::Mouse => self.mouse,
        };
        if let Some(handle) = handle {
            let _acknowledged = self.gate.interrupt_ack(handle);
        }
    }
}

/// What the draining thread has to be told, in a place it can reach.
struct Shared {
    endpoint: AtomicU64,
    notification: AtomicU64,
    ports: AtomicU64,
    keyboard: AtomicU64,
    mouse: AtomicU64,
}

impl Shared {
    const fn new() -> Self {
        Shared {
            endpoint: AtomicU64::new(0),
            notification: AtomicU64::new(0),
            ports: AtomicU64::new(0),
            keyboard: AtomicU64::new(0),
            mouse: AtomicU64::new(0),
        }
    }

    fn set(
        &self,
        endpoint: EndpointHandle,
        notification: NotificationHandle,
        ports: IoPortHandle,
        keyboard: Option<InterruptHandle>,
        mouse: Option<InterruptHandle>,
    ) {
        self.endpoint.store(endpoint.raw(), Ordering::SeqCst);
        self.notification
            .store(notification.raw(), Ordering::SeqCst);
        self.ports.store(ports.raw(), Ordering::SeqCst);
        self.keyboard
            .store(keyboard.map_or(0, Typed::raw), Ordering::SeqCst);
        self.mouse
            .store(mouse.map_or(0, Typed::raw), Ordering::SeqCst);
    }

    fn get(&self) -> Option<(EndpointHandle, NotificationHandle, IoPortHandle)> {
        Some((
            EndpointHandle::from_raw(self.endpoint.load(Ordering::SeqCst))?,
            NotificationHandle::from_raw(self.notification.load(Ordering::SeqCst))?,
            IoPortHandle::from_raw(self.ports.load(Ordering::SeqCst))?,
        ))
    }

    fn keyboard(&self) -> Option<InterruptHandle> {
        InterruptHandle::from_raw(self.keyboard.load(Ordering::SeqCst))
    }

    fn mouse(&self) -> Option<InterruptHandle> {
        InterruptHandle::from_raw(self.mouse.load(Ordering::SeqCst))
    }
}
