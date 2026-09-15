// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The network server: the one program that drives the network device and
//! the one that answers the socket protocol.
//!
//! It runs three threads, because a thread of this kernel waits on exactly
//! one thing: `Wait` names an endpoint or a notification, never both.
//!
//! The serving thread owns the device, the driver and the stack, and waits
//! on the endpoint. The interrupt thread waits on the notification the
//! MSI-X vector is bound to and sends to the same endpoint under a badge of
//! its own. The timer thread waits with a deadline and sends a tick under a
//! third badge when the deadline passes. The serving thread tells the three
//! apart by the badge, which is the console driver's arrangement with one
//! thread more.
//!
//! The deadline is one aligned word both the serving thread and the timer
//! thread reach. The two are threads of one process, so the word is a
//! `static` of this program and needs no memory object: one thread writes
//! it and the other reads it, and the notification signalled after the
//! write is what wakes a timer already asleep on a later deadline.
//!
//! A machine that carries no network device starts this program all the
//! same, and it answers `Unavailable` to everything.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds three programs and each uses a different part of what
// it depends on; these are the crates this one does not.
use audhsos_encoding as _;
use audhsos_ssh as _;
use net_http as _;

use core::sync::atomic::{AtomicU64, Ordering};

use audhsos_abi::Error;
use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::startup::Location;
use audhsos_time::Instant;
use crypto_rng::ChaChaRng;
use driver_virtio_net::queues::Side;
use driver_virtio_net::{Net, Vectors};
use net_stack::{Config, FRAME_LEN};
use net_wire::MacAddr;
use server_net::memory::REGION_BYTES;
use server_net::server::{Rings, Server};
use server_net::sockets::MAX_SOCKETS;
use user_net_programs::net_dma::{NetDma, QUEUE_SIZE, REGION_BYTES as DMA_BYTES};
use user_net_programs::net_registers::{NET_DMA, NET_WINDOW, Window};
use user_programs::client::{allocate, register, write_line};
use user_programs::mapping::Mapping;
use user_programs::serve::{Serving, receive};
use user_proto::ring::SOCKET_PAGE_LEN;
use user_proto::socket::{Reply, Request, refuse};
use user_rt::startup::Device;
use user_rt::{
    EndpointHandle, InterruptHandle, Line, MemoryHandle, NotificationHandle, ProcessHandle,
    Startup, Typed as _,
};
use user_sys_x86_64::{self as sys, Gate};
use virtio_queue::Queue;

sys::program!(main);

/// The name the server registers itself under.
const NAME: &[u8] = b"net";

/// The badge the interrupt thread sends under.
const DEVICE_BADGE: u64 = 0x4E45_54DE;

/// The badge the timer thread sends under.
const TICK_BADGE: u64 = 0x4E45_5471;

/// Words of the free set of a queue: one bit per descriptor, which
/// [`QUEUE_SIZE`] of them fit in one.
const QUEUE_WORDS: usize = 1;

/// How many descriptors and buffers the driver keeps track of.
const SLOTS: usize = user_net_programs::net_dma::BUFFERS;

/// The bit of the notification the serving thread signals the timer with.
const TICK_BIT: u64 = 0;

/// Where the stack of the interrupt thread ends.
const INTERRUPT_STACK_TOP: u64 = 0x0080_0000;

/// Where the stack of the timer thread ends.
const TIMER_STACK_TOP: u64 = 0x0090_0000;

/// How many pages each of those stacks gets.
const STACK_PAGES: u64 = 4;

/// Where the socket ring of slot zero is mapped; the next ones follow it.
const RINGS: u64 = 0x0000_6400_0000_0000;

/// Where the memory the stack writes into is mapped.
const BUFFERS: u64 = 0x0000_6C00_0000_0000;

/// How long the server waits for the device to finish a reset, in
/// microseconds.
const RESET_DEADLINE: u64 = 1_000_000;

/// How long a line this program writes.
type Report = Line<160>;

/// What the two other threads need, put where they can find it.
static SHARED: Shared = Shared::new();

/// The instant the timer thread is to wake at, in microseconds since the
/// kernel started. Zero is no deadline at all.
static DEADLINE: AtomicU64 = AtomicU64::new(0);

/// Brings the device up and answers clients.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
#[expect(
    clippy::too_many_lines,
    reason = "every mapping of this program stands in the frame that never returns, so the borrows of them cannot be moved into a function"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let voice = startup.log;
    let Some(endpoint) = startup.own_endpoint else {
        gate.thread_exit()
    };
    if let Some(names) = startup.name_server {
        let _registered = register(&mut gate, names, NAME, endpoint);
    }
    let (Some(process), Some(memory)) = (startup.own_process, startup.memory_server) else {
        stop(&mut gate, voice, Error::AccessDenied)
    };
    let Some(given) = startup.net.as_ref().and_then(Given::of) else {
        say(
            &mut gate,
            voice,
            &Report::of(format_args!("[net] no interface\n")),
        );
        let _refused = refuse_everything(&mut gate, endpoint);
        stop(&mut gate, voice, Error::Unavailable)
    };

    // Every mapping stands in the frame that never returns: the driver and
    // the server hold byte slices over them for the rest of the program,
    // and a borrow cannot outlive the mapping it came from.
    let mut windows = match map_windows(&mut gate, process, memory, &given) {
        Ok(windows) => windows,
        Err(error) => stop(&mut gate, voice, error),
    };
    let physical = windows.physical;
    // SAFETY: every mapping stands for the rest of this program — none is
    // unmapped and `main` does not return — each is memory of this process
    // alone, and the values built here are the only ones that reach those
    // bytes.
    let mut registers = Window::new(unsafe { windows.registers.bytes() }, given.places);
    // SAFETY: as above.
    let Some(mut dma) = NetDma::new(unsafe { windows.region.bytes() }, physical) else {
        stop(&mut gate, voice, Error::Unaligned)
    };
    // SAFETY: as above.
    let buffers = unsafe { windows.buffers.bytes() };
    let mut pages = [None; MAX_SOCKETS];
    for (slot, mapping) in pages.iter_mut().zip(windows.rings.iter()) {
        // SAFETY: as above, and every access to a socket page is atomic on
        // both sides.
        *slot = unsafe { mapping.socket_page() };
    }

    let (mut net, mut receive_queue, transmit_queue) =
        match bring_up(&mut gate, &mut dma, &mut registers, &given) {
            Ok(driver) => driver,
            Err(error) => stop(&mut gate, voice, error),
        };
    let mac = MacAddr::new(net.mac().unwrap_or([0; 6]));
    say(
        &mut gate,
        voice,
        &Report::of(format_args!(
            "[net] device mac={mac} queues={} rx={}\n",
            net.queue(Side::Receive).size,
            net.queue(Side::Transmit).size
        )),
    );

    let mut rings = [None; MAX_SOCKETS];
    for (slot, (page, handle)) in rings
        .iter_mut()
        .zip(pages.iter().zip(windows.objects.iter()))
    {
        *slot = page.map(|page| Rings {
            page,
            object: handle.handle(),
        });
    }
    let Some(rings) = every(&rings) else {
        stop(&mut gate, voice, Error::OutOfMemory)
    };
    let Some(mut server) = Server::new(Config::new(mac), buffers, rings) else {
        stop(&mut gate, voice, Error::BufferTooSmall)
    };

    let mut generator = match seed(&mut gate) {
        Ok(generator) => generator,
        Err(error) => stop(&mut gate, voice, error),
    };
    let now = gate.clock_now().unwrap_or(0);
    server.stack().configure(Instant::from_micros(now));

    if let Err(error) = start_threads(&mut gate, &startup, endpoint, &given) {
        say(
            &mut gate,
            voice,
            &Report::of(format_args!(
                "[net] no second thread: {}\n",
                error.message()
            )),
        );
    }

    // Every receive buffer goes into the available ring before the first
    // frame arrives, and the device is told they are there.
    let filled = net
        .fill(&mut receive_queue, &mut dma.receive, &dma.taken)
        .unwrap_or(0);
    net.notify(&mut registers, Side::Receive);
    if filled == 0 {
        say(
            &mut gate,
            voice,
            &Report::of(format_args!("[net] no receive buffer went in\n")),
        );
    }

    let mut driver = Driver {
        net,
        receive_queue,
        transmit_queue,
        registers,
        dma,
    };
    let outcome = serve(
        &mut gate,
        &mut server,
        &mut driver,
        &mut generator,
        endpoint,
        voice,
    );
    if let Err(error) = outcome {
        say(
            &mut gate,
            voice,
            &Report::of(format_args!("[net] {}\n", error.message())),
        );
    }
    gate.thread_exit()
}

/// The device, the driver, and the two queues.
struct Driver<'a> {
    net: Net<SLOTS>,
    receive_queue: Queue<QUEUE_WORDS>,
    transmit_queue: Queue<QUEUE_WORDS>,
    registers: Window<'a>,
    dma: NetDma<'a>,
}

impl Driver<'_> {
    /// Takes the next frame the device wrote into `into`, or `None` when
    /// there is none.
    fn take<'b>(&mut self, into: &'b mut [u8]) -> Option<&'b [u8]> {
        match self.net.receive(
            &mut self.receive_queue,
            &mut self.dma.receive,
            &self.dma.taken,
            into,
        ) {
            Ok(frame) => {
                self.net.notify(&mut self.registers, Side::Receive);
                frame
            }
            // A refusal says the driver and the device no longer agree on
            // what is in the queue, and a refusal that came before the
            // buffer went back is a buffer the device no longer has. Every
            // buffer this driver holds goes back in, so that eight
            // refusals do not leave the receive queue empty and the server
            // deaf.
            Err(_refused) => {
                self.refill();
                None
            }
        }
    }

    /// Puts every receive buffer the device does not hold back into the
    /// available ring.
    fn refill(&mut self) {
        let _filled = self.net.fill(
            &mut self.receive_queue,
            &mut self.dma.receive,
            &self.dma.taken,
        );
        self.net.notify(&mut self.registers, Side::Receive);
    }

    /// Sends one frame, and answers whether the device took it.
    fn give(&mut self, frame: &[u8]) -> bool {
        match self.net.send(
            &mut self.transmit_queue,
            &mut self.dma.transmit,
            &mut self.dma.given,
            frame,
        ) {
            Ok(notify) => {
                if notify {
                    self.net.notify(&mut self.registers, Side::Transmit);
                }
                true
            }
            Err(_refused) => false,
        }
    }
}

/// Answers clients and drives the stack until the endpoint fails.
fn serve(
    gate: &mut Gate,
    server: &mut Server<'_>,
    driver: &mut Driver<'_>,
    generator: &mut ChaChaRng<Seed>,
    endpoint: EndpointHandle,
    voice: Option<EndpointHandle>,
) -> Result<(), Error> {
    let mut serving = Serving::default();
    let mut reported = false;
    // One round before the first message: the address configuration
    // client has a discover to send, and nothing has asked this server
    // for anything yet.
    let now = Instant::from_micros(gate.clock_now().unwrap_or(0));
    run(server, driver, generator, now);
    tell_the_timer(gate, server, now);
    loop {
        receive(gate, endpoint, &mut serving)?;
        let now = Instant::from_micros(gate.clock_now().unwrap_or(0));
        let decoded = match serving.badge {
            DEVICE_BADGE | TICK_BADGE => None,
            _client => Some(Request::decode(gate.reader())),
        };
        // The buffer that carries a reply is the buffer a line to the
        // console goes through, so the line is written after the request
        // is read and before the reply is put in: one is reported a round
        // later than the lease arrives.
        if !reported {
            reported = say_lease(gate, server, voice);
        }
        // What the device wrote before this message is part of what the
        // answer sees; what the answer gives the stack goes out in the
        // round after it.
        run(server, driver, generator, now);
        if let Some(decoded) = decoded {
            let reply = match decoded {
                Ok(request) => server.answer(serving.badge, &request, now, generator),
                Err(_unreadable) => {
                    server.forget(serving.badge);
                    Reply::Closed(Err(Error::InvalidArgument))
                }
            };
            let _encoded = reply.encode(&mut gate.writer());
        }
        run(server, driver, generator, now);
        tell_the_timer(gate, server, now);
    }
}

/// Writes the instant the stack next has work at and wakes the timer
/// thread, so that a timer asleep on a later deadline re-reads it.
fn tell_the_timer(gate: &mut Gate, server: &Server<'_>, now: Instant) {
    let deadline = server.poll_at(now).map_or(0, Instant::as_micros);
    DEADLINE.store(deadline, Ordering::SeqCst);
    if let Some(notification) = SHARED.timer() {
        let _signalled = gate.notification_signal(notification, 1 << TICK_BIT);
    }
}

/// One round: every frame the device has in, every frame the stack has out.
fn run(
    server: &mut Server<'_>,
    driver: &mut Driver<'_>,
    generator: &mut ChaChaRng<Seed>,
    now: Instant,
) {
    let mut taken = [0u8; FRAME_LEN];
    let mut out = [0u8; FRAME_LEN];
    let mut rounds = 0usize;
    loop {
        rounds = rounds.saturating_add(1);
        if rounds > ROUNDS {
            return;
        }
        let received = driver.take(&mut taken).map(<[u8]>::len);
        let frame = received.and_then(|len| taken.get(..len));
        let Ok(answer) = server.poll(now, frame, &mut out, generator) else {
            return;
        };
        let sent = answer.map(<[u8]>::len);
        if let Some(len) = sent
            && let Some(frame) = out.get(..len)
        {
            let _taken = driver.give(frame);
        }
        if received.is_none() && sent.is_none() {
            return;
        }
    }
}

/// How many times one round of the loop asks the stack before it goes back
/// to the endpoint. A client that never stops talking must not keep the
/// server out of its own loop.
const ROUNDS: usize = 64;

/// Says what the address configuration client reached, once.
fn say_lease(gate: &mut Gate, server: &mut Server<'_>, voice: Option<EndpointHandle>) -> bool {
    let Some(lease) = server.stack().lease() else {
        return false;
    };
    let line = Report::of(format_args!(
        "[net] lease address={} gateway={} servers={}\n",
        lease.network.address(),
        lease
            .router
            .map_or(0, |router| u32::from_be_bytes(router.octets())),
        lease.servers.len()
    ));
    say(gate, voice, &line);
    true
}

/// The loop of a machine that carries no network device: every request is
/// refused and the server stays where it is, because a client that asks for
/// a socket has to hear that there is none.
fn refuse_everything(gate: &mut Gate, endpoint: EndpointHandle) -> Result<(), Error> {
    let mut serving = Serving::default();
    loop {
        receive(gate, endpoint, &mut serving)?;
        let reply = Request::decode(gate.reader())
            .map_or(Reply::Closed(Err(Error::Unavailable)), |request| {
                refuse(&request, Error::Unavailable)
            });
        let _encoded = reply.encode(&mut gate.writer());
    }
}

/// Says why the server stops and ends the thread.
fn stop(gate: &mut Gate, voice: Option<EndpointHandle>, error: Error) -> ! {
    say(
        gate,
        voice,
        &Report::of(format_args!("[net] {}\n", error.message())),
    );
    gate.thread_exit()
}

/// Writes one line, if there is anywhere to write it.
fn say(gate: &mut Gate, voice: Option<EndpointHandle>, line: &Report) {
    let Some(console) = voice else {
        return;
    };
    let _logged = write_line(gate, console, line.as_bytes());
}

/// Every entry of `slots`, or `None` when one of them is missing.
fn every<T: Copy>(slots: &[Option<T>; MAX_SOCKETS]) -> Option<[T; MAX_SOCKETS]> {
    let first = (*slots.first()?)?;
    let mut filled = [first; MAX_SOCKETS];
    for (slot, given) in filled.iter_mut().zip(slots.iter()) {
        *slot = (*given)?;
    }
    Some(filled)
}

/// What the root task said about the device.
struct Given {
    /// The device memory over its registers.
    registers: MemoryHandle,
    /// How many bytes of that window the four structures reach into.
    bytes: u64,
    /// Where each structure lies in the window.
    places: [Location; 4],
    /// The multiplier of virtio 4.1.4.4.
    multiplier: u32,
    /// The notification its message interrupt is bound to.
    notification: NotificationHandle,
    /// The message interrupt itself.
    interrupt: InterruptHandle,
}

impl Given {
    /// What the nine roles of the device say, or nothing when they say it
    /// only in part.
    fn of(device: &Device) -> Option<Self> {
        let places = device.structures()?;
        let mut end = 0u64;
        for place in &places {
            end = end.max(u64::from(place.offset).wrapping_add(u64::from(place.len)));
        }
        Some(Given {
            registers: device.registers,
            bytes: end.next_multiple_of(PAGE_SIZE),
            places,
            multiplier: u32::try_from(device.notify_multiplier?).ok()?,
            notification: device.notification?,
            interrupt: device.interrupt?,
        })
    }
}

/// Every mapping the server works through, and the objects behind the ones
/// it hands to clients.
struct Windows {
    /// The registers of the device.
    registers: Mapping,
    /// The region the device reads and writes.
    region: Mapping,
    /// The memory the stack writes into.
    buffers: Mapping,
    /// The two rings of each socket.
    rings: [Mapping; MAX_SOCKETS],
    /// The objects behind those rings, which travel to the clients.
    objects: [MemoryHandle; MAX_SOCKETS],
    /// Where the region the device reads and writes begins in physical
    /// memory.
    physical: u64,
}

/// Maps everything the server needs and makes the objects it hands out.
fn map_windows(
    gate: &mut Gate,
    process: ProcessHandle,
    memory: EndpointHandle,
    given: &Given,
) -> Result<Windows, Error> {
    let registers = Mapping::new(gate, process, given.registers, NET_WINDOW, given.bytes)?;
    let bytes = u64::try_from(DMA_BYTES)
        .unwrap_or(0)
        .next_multiple_of(PAGE_SIZE);
    let object = allocate(gate, memory, bytes, PAGE_SIZE)?;
    let physical = gate.memory_info(object)?.start;
    let region = Mapping::new(gate, process, object, NET_DMA, bytes)?;
    let stack_bytes = u64::try_from(REGION_BYTES)
        .unwrap_or(0)
        .next_multiple_of(PAGE_SIZE);
    let stack_object = allocate(gate, memory, stack_bytes, PAGE_SIZE)?;
    let buffers = Mapping::new(gate, process, stack_object, BUFFERS, stack_bytes)?;
    let page_bytes = u64::try_from(SOCKET_PAGE_LEN)
        .unwrap_or(0)
        .next_multiple_of(PAGE_SIZE);
    let mut objects = [stack_object; MAX_SOCKETS];
    let mut rings = [const { None }; MAX_SOCKETS];
    for (index, slot) in rings.iter_mut().enumerate() {
        let ring = allocate(gate, memory, page_bytes, PAGE_SIZE)?;
        let at = RINGS.wrapping_add(u64::try_from(index).unwrap_or(0).wrapping_mul(page_bytes));
        *slot = Some(Mapping::new(gate, process, ring, at, page_bytes)?);
        if let Some(object) = objects.get_mut(index) {
            *object = ring;
        }
    }
    let mut mapped = rings.into_iter();
    let rings = core::array::from_fn(|_| mapped.next().flatten());
    let rings: [Mapping; MAX_SOCKETS] = match rings {
        [Some(one), Some(two), Some(three), Some(four)] => [one, two, three, four],
        _incomplete => return Err(Error::OutOfMemory),
    };
    Ok(Windows {
        registers,
        region,
        buffers,
        rings,
        objects,
        physical,
    })
}

/// Resets the device and brings it up: the driver and its two queues.
fn bring_up(
    gate: &mut Gate,
    dma: &mut NetDma<'_>,
    registers: &mut Window<'_>,
    given: &Given,
) -> Result<(Net<SLOTS>, Queue<QUEUE_WORDS>, Queue<QUEUE_WORDS>), Error> {
    let mut net = Net::<SLOTS>::new(given.multiplier);
    net.reset(registers);
    let deadline = gate.clock_now()?.saturating_add(RESET_DEADLINE);
    while !net.is_reset(registers) {
        if gate.clock_now()? > deadline {
            return Err(Error::Unavailable);
        }
    }
    // The rings are zeroed before the device is told where they are, which
    // is what virtio 2.7.10.1 asks of the used ring's flags.
    dma.clear();
    let receive =
        Queue::<QUEUE_WORDS>::new(&mut dma.receive, QUEUE_SIZE).map_err(|_| Error::InvalidState)?;
    let transmit = Queue::<QUEUE_WORDS>::new(&mut dma.transmit, QUEUE_SIZE)
        .map_err(|_| Error::InvalidState)?;
    // One vector for both queues: this driver waits on one notification,
    // and a queue of its own would need a thread of its own.
    net.initialize(registers, &dma.queues(), &Vectors::shared(0))
        .map_err(|_| Error::Unavailable)?;
    Ok((net, receive, transmit))
}

/// The entropy source of this program, which never delivers.
///
/// The seed is drawn once at startup through `random_bytes`, which is what
/// 13.4 asks of a process, and the generator produces everything else. A
/// reseed is due after a mebibyte of drawn bytes; a handshake, a lease and
/// an ephemeral port together are a few hundred, so this server never
/// reaches one. The source answers nothing because the system call behind
/// it needs the gate of the thread, which a source held inside the
/// generator does not have.
struct Seed;

impl crypto_rng::Entropy for Seed {
    fn fill(&mut self, _out: &mut [u8]) -> Result<(), crypto_rng::EntropyError> {
        Err(crypto_rng::EntropyError::Unavailable)
    }
}

/// A generator seeded from the machine.
fn seed(gate: &mut Gate) -> Result<ChaChaRng<Seed>, Error> {
    let words = gate.random_bytes()?;
    let mut bytes = [0u8; 32];
    for (chunk, word) in bytes.chunks_mut(8).zip(words) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    Ok(ChaChaRng::from_seed(&bytes, Seed))
}

/// Starts the interrupt thread and the timer thread.
fn start_threads(
    gate: &mut Gate,
    startup: &Startup,
    endpoint: EndpointHandle,
    given: &Given,
) -> Result<(), Error> {
    let (Some(own), Some(memory)) = (startup.own_process, startup.memory_server) else {
        return Err(Error::NotFound);
    };
    // The interrupt is already bound: the root task bound it to the
    // notification it handed over, as it does for a block device.
    let device = gate.endpoint_badge(endpoint, DEVICE_BADGE)?;
    let tick = gate.endpoint_badge(endpoint, TICK_BADGE)?;
    let timer = gate.notification_create()?;
    SHARED.set(device, tick, given.notification, given.interrupt, timer);

    let bytes = STACK_PAGES.wrapping_mul(PAGE_SIZE);
    for (top, entry) in [
        (INTERRUPT_STACK_TOP, interrupt_entry()),
        (TIMER_STACK_TOP, timer_entry()),
    ] {
        let object = allocate(gate, memory, bytes, PAGE_SIZE)?;
        // The mapping stays: the stack of a thread that never ends is
        // never taken back.
        let _stack = Mapping::new(gate, own, object, top.wrapping_sub(bytes), bytes)?;
        let thread = gate.thread_create(
            own,
            entry,
            top,
            u64::from(user_programs::priority::DRIVER),
            u64::from(user_programs::priority::DRIVER),
            None,
        )?;
        gate.thread_start(thread)?;
    }
    Ok(())
}

/// Where the interrupt thread begins.
#[expect(
    clippy::as_conversions,
    reason = "a function has to become an address for `thread_create`, and there is no other way to write it"
)]
fn interrupt_entry() -> u64 {
    let pointer: unsafe extern "sysv64" fn(u64) -> ! = on_interrupt;
    pointer as usize as u64
}

/// Where the timer thread begins.
#[expect(clippy::as_conversions, reason = "as `interrupt_entry`")]
fn timer_entry() -> u64 {
    let pointer: unsafe extern "sysv64" fn(u64) -> ! = on_deadline;
    pointer as usize as u64
}

/// The interrupt thread: wait, tell the serving thread, acknowledge.
///
/// # Safety
///
/// The kernel starts this once, with the address of the thread's IPC
/// buffer in the first argument register.
unsafe extern "sysv64" fn on_interrupt(ipc_buffer: u64) -> ! {
    // SAFETY: the kernel started this thread with the address of its own
    // buffer, and this is the only gate over it.
    let mut gate = unsafe { Gate::adopt(ipc_buffer) };
    let Some(shared) = SHARED.get() else {
        gate.thread_exit()
    };
    loop {
        if gate.notification_wait(shared.device_notification).is_err() {
            gate.thread_exit()
        }
        let writer = user_rt::message::Writer::new();
        {
            let mut buffer = gate.writer();
            let _finished = writer.finish(&mut buffer, DEVICE_LABEL);
        }
        let _sent = gate.ipc_send(shared.device);
        let _acknowledged = gate.interrupt_ack(shared.interrupt);
    }
}

/// The timer thread: sleep until the deadline the serving thread wrote,
/// and send a tick when it passes.
///
/// # Safety
///
/// As [`on_interrupt`].
unsafe extern "sysv64" fn on_deadline(ipc_buffer: u64) -> ! {
    // SAFETY: as `on_interrupt`.
    let mut gate = unsafe { Gate::adopt(ipc_buffer) };
    let Some(shared) = SHARED.get() else {
        gate.thread_exit()
    };
    loop {
        let deadline = DEADLINE.load(Ordering::SeqCst);
        let woken = if deadline == 0 {
            gate.notification_wait(shared.timer).map(|_bits| ())
        } else {
            gate.notification_wait_until(shared.timer, deadline)
                .map(|_bits| ())
        };
        if woken.is_err() {
            gate.thread_exit()
        }
        // A wake-up before the deadline is the serving thread saying the
        // deadline moved; the loop re-reads it and sleeps again.
        let now = gate.clock_now().unwrap_or(0);
        if deadline != 0 && now >= deadline {
            let writer = user_rt::message::Writer::new();
            {
                let mut buffer = gate.writer();
                let _finished = writer.finish(&mut buffer, TICK_LABEL);
            }
            let _sent = gate.ipc_send(shared.tick);
        }
    }
}

/// The label the interrupt thread sends under.
const DEVICE_LABEL: u64 = 1;

/// The label the timer thread sends under.
const TICK_LABEL: u64 = 2;

/// What the two other threads need, which they cannot be told any other
/// way: a thread starts with nothing but the address of its own IPC
/// buffer.
struct Shared {
    device: AtomicU64,
    tick: AtomicU64,
    device_notification: AtomicU64,
    interrupt: AtomicU64,
    timer: AtomicU64,
}

/// What [`Shared`] holds, once it holds all of it.
#[derive(Clone, Copy)]
struct Handles {
    device: EndpointHandle,
    tick: EndpointHandle,
    device_notification: NotificationHandle,
    interrupt: InterruptHandle,
    timer: NotificationHandle,
}

impl Shared {
    const fn new() -> Self {
        Shared {
            device: AtomicU64::new(0),
            tick: AtomicU64::new(0),
            device_notification: AtomicU64::new(0),
            interrupt: AtomicU64::new(0),
            timer: AtomicU64::new(0),
        }
    }

    fn set(
        &self,
        device: EndpointHandle,
        tick: EndpointHandle,
        device_notification: NotificationHandle,
        interrupt: InterruptHandle,
        timer: NotificationHandle,
    ) {
        self.device.store(device.raw(), Ordering::SeqCst);
        self.tick.store(tick.raw(), Ordering::SeqCst);
        self.device_notification
            .store(device_notification.raw(), Ordering::SeqCst);
        self.interrupt.store(interrupt.raw(), Ordering::SeqCst);
        self.timer.store(timer.raw(), Ordering::SeqCst);
    }

    fn get(&self) -> Option<Handles> {
        Some(Handles {
            device: EndpointHandle::from_raw(self.device.load(Ordering::SeqCst))?,
            tick: EndpointHandle::from_raw(self.tick.load(Ordering::SeqCst))?,
            device_notification: NotificationHandle::from_raw(
                self.device_notification.load(Ordering::SeqCst),
            )?,
            interrupt: InterruptHandle::from_raw(self.interrupt.load(Ordering::SeqCst))?,
            timer: NotificationHandle::from_raw(self.timer.load(Ordering::SeqCst))?,
        })
    }

    /// The notification the serving thread signals when the deadline
    /// moves, or `None` before the timer thread was started.
    fn timer(&self) -> Option<NotificationHandle> {
        NotificationHandle::from_raw(self.timer.load(Ordering::SeqCst))
    }
}
