// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The compositor: the one program that holds the windows of the desktop.
//!
//! It is a client of the display server and a server to everything else
//! (D-151). Of
//! the display server it takes one surface the size of the screen, and it
//! subscribes to the input server for the keyboard and the pointer; to its
//! own clients it hands windows, carries their draw commands out in the
//! surface it keeps for each, and composes the screen out of those surfaces:
//! the desktop under everything, then every window back to front, then the
//! menu bar with its clock over all of them.
//!
//! Everything it decides is in `server-desk`; this is the loop around it,
//! and the system calls the loop needs: the surface of the screen, one
//! memory object per window, the ring of input events, and the presentation
//! of what changed.
//!
//! A thread of this kernel waits on one thing, and this program waits on
//! two (D-155): its endpoint, and the notification that carries both the input
//! events and the end of a client. So a second thread waits on the
//! notification with a deadline and sends what it heard to the first under
//! a badge of its own, which is also what makes the clock move: the
//! deadline passes every [`TICK`] microseconds whether anything happened or
//! not.
//!
//! Nothing is painted until the key [`server_desk::SHOWS`] comes up
//! (D-154, as D-126 says of the canvas). The programs that draw on the whole screen run before this one
//! and their pictures are checked while the machine still runs; a desktop
//! that laid its background down at startup would decide by a race which of
//! the two the screen holds.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds sixteen programs and each uses a different part of what
// it depends on; these are the crates this one does not.
use app_canvas as _;
use app_shell as _;
use driver_i8042 as _;
use driver_uart16550 as _;
use driver_virtio_blk as _;
use fs_fat as _;
use pci as _;
use server_console as _;
use server_display as _;
use server_fs as _;
use server_input as _;
use server_memory as _;
use server_name as _;
use user_loader as _;
use virtio_queue as _;

use core::sync::atomic::{AtomicU64, Ordering};

use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::{Error, Handle, Rights};
use audhsos_collections::ArrayVec;
use audhsos_time::UnixTime;
use gfx::{Damage, PixelFormat, Rect, Surface};
use server_desk::state::{Desk, Outcome, WINDOWS};
use user_programs::client::{allocate, lookup, register, release, write_line};
use user_programs::mapping::Mapping;
use user_programs::serve::{Serving, receive, reply};
use user_proto::display::{CursorShape, Mode, Reply as DisplayReply, Request as DisplayRequest};
use user_proto::input::{Reply as InputReply, Request as InputRequest, RingReader};
use user_proto::window::{Reply, Request, Window as Given};
use user_rt::{
    EndpointHandle, Line, MemoryHandle, NotificationHandle, ProcessHandle, Startup, Typed,
};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the compositor registers itself under.
const NAME: &[u8] = b"desk";

/// The name the display server registered itself under.
const DISPLAY: &[u8] = b"display";

/// The name the input server registered itself under.
const INPUT: &[u8] = b"input";

/// Where the surface of the screen is mapped.
const SCREEN: u64 = 0x1000_0000;

/// Where the surface of the first window is mapped.
const CONTENTS: u64 = 0x2000_0000;

/// How much address space one window surface gets.
const CONTENT_SLOT: u64 = 0x0100_0000;

/// Where the ring of input events is mapped.
const RING: u64 = 0x4000_0000;

/// The badge the waking thread sends under, which is how the serving
/// thread tells its message from a client's request.
const WAKE_BADGE: u64 = 0x000D_E54B;

/// The label it sends the word of the notification under.
const WAKE_LABEL: u64 = 1;

/// The top of the stack of the waking thread, and how many pages it gets.
const WAKE_STACK_TOP: u64 = 0x0080_0000;
const WAKE_STACK_PAGES: u64 = 4;

/// How long the waking thread waits before it wakes the desktop anyway, in
/// microseconds. It is what moves the clock.
const TICK: u64 = 200_000;

/// How many ticks the desktop goes on serving after it has ended, so that
/// its clients can take the news and give their windows back.
const GOODBYE: u32 = 40;

/// What a notification handed to a server may be used for: signalling it,
/// and being handed over at all.
const SIGNAL_ONLY: Rights = Rights::SIGNAL.union(Rights::TRANSFER);

/// What a process handed to a server may be used for: being asked whether
/// it is still there, and being handed over at all.
const WATCHED: Rights = Rights::INFO.union(Rights::TRANSFER);

/// How many microseconds one second has.
const SECOND: u64 = 1_000_000;

/// What the waking thread has to be told, in a place it can reach: it
/// starts with nothing but the address of its own IPC buffer, and this is a
/// process without a heap.
static SHARED: Shared = Shared::new();

/// The endpoint the waking thread sends to and the notification it waits
/// on.
struct Shared {
    /// The endpoint of this server, badged as [`WAKE_BADGE`].
    endpoint: AtomicU64,
    /// The notification the input server signals and the ends of clients
    /// are reported on.
    notification: AtomicU64,
}

impl Shared {
    const fn new() -> Self {
        Shared {
            endpoint: AtomicU64::new(0),
            notification: AtomicU64::new(0),
        }
    }

    fn set(&self, endpoint: EndpointHandle, notification: NotificationHandle) {
        self.endpoint.store(endpoint.raw(), Ordering::SeqCst);
        self.notification
            .store(notification.raw(), Ordering::SeqCst);
    }

    fn get(&self) -> Option<(EndpointHandle, NotificationHandle)> {
        let endpoint = EndpointHandle::from_raw(self.endpoint.load(Ordering::SeqCst))?;
        let notification = NotificationHandle::from_raw(self.notification.load(Ordering::SeqCst))?;
        Some((endpoint, notification))
    }
}

/// The surface of one window, and who holds it.
struct Held {
    /// The window the surface belongs to.
    id: u32,
    /// The badge of the client that opened it.
    badge: u64,
    /// The memory object of its pixels.
    memory: MemoryHandle,
    /// Where they are mapped in this process.
    mapping: Mapping,
    /// The notification the client waits on.
    notification: NotificationHandle,
    /// The client itself, whose end this program watches.
    process: ProcessHandle,
    /// The bit of the notification that end signals, which is the slot of
    /// the entry above the bit the input server signals.
    bit: u64,
    /// Columns of content.
    width: u32,
    /// Rows of it.
    height: u32,
}

/// Serves the desktop until the key that ends it has been answered.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let Some(endpoint) = startup.own_endpoint else {
        gate.thread_exit()
    };
    if let Some(names) = startup.name_server
        && let Err(error) = register(&mut gate, names, NAME, endpoint)
    {
        say(&mut gate, &startup, "cannot register", error);
    }
    match run(&mut gate, &startup, endpoint) {
        Ok(()) => {
            say_line(&mut gate, &startup, b"[desk] done\n");
            report_to_parent(&mut gate, &startup);
            gate.thread_exit()
        }
        // A machine without a screen has no desktop, and a client that
        // asks for a window has to hear that rather than wait for an
        // answer nobody will send. So this program reports that it is
        // done and goes on refusing, as `server-display` stays where it
        // is on such a machine.
        Err(error) => {
            say(&mut gate, &startup, "no desktop", error);
            report_to_parent(&mut gate, &startup);
            refuse(&mut gate, endpoint)
        }
    }
}

/// What the compositor took before it began to serve: the screen it
/// composes into and the events it composes for.
struct Attached {
    /// The display server, badged for this program.
    display: EndpointHandle,
    /// What the screen is.
    mode: Mode,
    /// The order of the channels of that screen.
    format: PixelFormat,
    /// The surface of the screen, as the display server named it.
    given: user_proto::display::Surface,
    /// Where its pixels are mapped here.
    screen: Mapping,
    /// Where the ring of input events is mapped.
    events: Mapping,
}

/// Takes the surface of the screen and the ring of input events, and
/// starts the thread that waits on the notification both the input server
/// and the ends of the clients signal.
fn attach(gate: &mut Gate, startup: &Startup, endpoint: EndpointHandle) -> Result<Attached, Error> {
    let process = startup.own_process.ok_or(Error::NotFound)?;
    let display = match startup.display_server {
        Some(given) => given,
        None => lookup(gate, startup.name_server.ok_or(Error::NotFound)?, DISPLAY)?,
    };
    let input = match startup.input_server {
        Some(given) => given,
        None => lookup(gate, startup.name_server.ok_or(Error::NotFound)?, INPUT)?,
    };
    let mode = ask_mode(gate, display)?;
    let watched = gate.handle_duplicate(process.handle(), WATCHED)?;
    let given = ask_surface(gate, display, mode, watched)?;
    let bytes = u64::from(mode.width)
        .saturating_mul(u64::from(mode.height))
        .saturating_mul(4);
    let screen = Mapping::new(
        gate,
        process,
        MemoryHandle::from_handle(given.memory),
        SCREEN,
        whole_pages(bytes),
    )?;

    let notification = gate.notification_create()?;
    let signal = gate.handle_duplicate(notification.handle(), SIGNAL_ONLY)?;
    let listener = gate.handle_duplicate(process.handle(), WATCHED)?;
    let ring = MemoryHandle::from_handle(subscribe(gate, input, signal, listener)?);
    gate.handle_close(signal)?;
    gate.handle_close(listener)?;
    let events = Mapping::new(gate, process, ring, RING, PAGE_SIZE)?;
    start_waker(gate, startup, endpoint, notification)?;
    Ok(Attached {
        display,
        mode,
        format: PixelFormat::from_boot(mode.format),
        given,
        screen,
        events,
    })
}

/// Serves the desktop until the key that ends it has been answered.
fn run(gate: &mut Gate, startup: &Startup, endpoint: EndpointHandle) -> Result<(), Error> {
    let Attached {
        display,
        mode,
        format,
        given,
        mut screen,
        events,
    } = attach(gate, startup, endpoint)?;
    let mut desk = Desk::new(mode.width, mode.height);
    let mut held: [Option<Held>; WINDOWS] = [const { None }; WINDOWS];
    let mut serving = Serving::default();
    let mut goodbye: Option<u32> = None;
    let mut shown = false;
    // A machine whose image carries no other program that draws has
    // nothing to race with, so the desktop is there from the start and
    // nobody has to press the key that shows it (D-156).
    let mut region = Rect::EMPTY;
    if startup.desk_alone == Some(1) {
        region = desk.show();
        // The sprite stands where the display server last put it, which is
        // the corner of a screen nobody has pointed at yet; the desktop
        // says where its own pointer is instead.
        let (x, y) = desk.pointer();
        let _moved = set_cursor(gate, display, x, y);
    }
    say_line(
        gate,
        startup,
        Line::<64>::of(format_args!(
            "[desk] ready {}x{}\n",
            mode.width, mode.height
        ))
        .as_bytes(),
    );
    loop {
        if paint(
            gate,
            startup,
            &desk,
            &mut held,
            &mut screen,
            display,
            given.id,
            format,
            region,
            &mut shown,
        ) {
            region = Rect::EMPTY;
        }
        if receive(gate, endpoint, &mut serving).is_err() {
            return Ok(());
        }
        if serving.badge == WAKE_BADGE {
            let word = gate.reader().word(0).unwrap_or(0);
            let ended = woken(
                gate,
                startup,
                &mut desk,
                &mut held,
                display,
                &events,
                word,
                &mut region,
            );
            if ended && goodbye.is_none() {
                say_line(gate, startup, b"[desk] ending\n");
                goodbye = Some(0);
            }
            if let Some(ticks) = goodbye {
                if desk.is_empty() || ticks >= GOODBYE {
                    break;
                }
                goodbye = Some(ticks.saturating_add(1));
            }
        } else {
            let answer = answer(
                gate,
                startup,
                &mut desk,
                &mut held,
                format,
                serving.badge,
                &mut region,
            );
            let _written = answer.encode(&mut gate.writer());
            // The answer goes out here and not with the next receive: a
            // presentation is a call of its own, and a call writes the
            // message area this answer stands in.
            let _replied = reply(gate, &mut serving);
        }
    }
    if let Some(process) = startup.own_process {
        let _unmapped = screen.unmap(gate, process);
    }
    let _destroyed = destroy_surface(gate, display, given.id);
    Ok(())
}

/// Composes `region` and puts it on the screen, and answers with whether
/// there was anything to paint.
///
/// The first painting is said on the console, because the run that drives
/// this machine waits for that line before it takes a picture.
#[expect(
    clippy::too_many_arguments,
    reason = "a painting needs the desktop, the surfaces of its windows, the surface of the screen, the display server, the surface it presents to, the format they are drawn in, the region, and whether anything was painted before"
)]
fn paint(
    gate: &mut Gate,
    startup: &Startup,
    desk: &Desk,
    held: &mut [Option<Held>; WINDOWS],
    screen: &mut Mapping,
    display: EndpointHandle,
    id: u32,
    format: PixelFormat,
    region: Rect,
    shown: &mut bool,
) -> bool {
    if region.is_empty() || !desk.is_shown() {
        return false;
    }
    let damage = compose(desk, held, screen, format, region);
    let _presented = present(gate, display, id, &damage);
    if !*shown {
        *shown = true;
        say_line(gate, startup, b"[desk] shown\n");
    }
    true
}

/// Takes what the waking thread heard: the ends of clients, the events in
/// the ring, and the clock.
///
/// Answers with whether the desktop has ended.
#[expect(
    clippy::too_many_arguments,
    reason = "one wake-up needs the desktop, the surfaces, the display server, the ring, the word that was heard, and the region it grows; a structure for them would be this list under another name"
)]
fn woken(
    gate: &mut Gate,
    startup: &Startup,
    desk: &mut Desk,
    held: &mut [Option<Held>; WINDOWS],
    display: EndpointHandle,
    events: &Mapping,
    word: u64,
    region: &mut Rect,
) -> bool {
    let mut ended = false;
    gone(gate, startup, desk, held, word, region);
    while let Some(event) = pop(events) {
        let outcome = desk.feed(event);
        *region = region.union(outcome.repaint);
        for id in &outcome.woken {
            wake_client(gate, held, *id);
        }
        if outcome.moved {
            let (x, y) = desk.pointer();
            let _moved = set_cursor(gate, display, x, y);
        }
        ended |= outcome.ended;
    }
    if let Ok((micros, _source)) = gate.clock_wall() {
        let seconds = i64::try_from(micros.wrapping_div(SECOND)).unwrap_or(0);
        if let Ok(time) = UnixTime::from_seconds(seconds).to_civil() {
            *region = region.union(desk.tick(time));
        }
    }
    ended
}

/// Gives back the windows of the clients whose bits stand in `word`.
fn gone(
    gate: &mut Gate,
    startup: &Startup,
    desk: &mut Desk,
    held: &mut [Option<Held>; WINDOWS],
    word: u64,
    region: &mut Rect,
) {
    for slot in 0..WINDOWS {
        let bit = 1_u64
            .checked_shl(u32::try_from(slot).unwrap_or(0).saturating_add(1))
            .unwrap_or(0);
        if word & bit == 0 {
            continue;
        }
        let Some(taken) = held.get_mut(slot).and_then(Option::take) else {
            continue;
        };
        let mut moved = Outcome::nothing();
        if let Some((_id, frame)) = desk.forget(taken.badge, &mut moved) {
            *region = region.union(frame);
        }
        *region = region.union(moved.repaint);
        for id in &moved.woken {
            wake_client(gate, held, *id);
        }
        say_line(
            gate,
            startup,
            Line::<96>::of(format_args!(
                "[desk] client {} is gone: window {} released\n",
                taken.badge, taken.id
            ))
            .as_bytes(),
        );
        give_back(gate, startup, taken);
    }
}

/// Signals the client of `id` that an event waits for it.
fn wake_client(gate: &mut Gate, held: &[Option<Held>; WINDOWS], id: u32) {
    let Some(entry) = held.iter().flatten().find(|entry| entry.id == id) else {
        return;
    };
    let _signalled = gate.notification_signal(entry.notification, 1);
}

/// Takes the next event out of the ring, if one waits there.
fn pop(events: &Mapping) -> Option<user_proto::input::Event> {
    // SAFETY: the mapping remains live and the server published an
    // initialized RingPage. Both processes use atomic access.
    let page = unsafe { events.ring() }?;
    RingReader::new(page)?.pop()
}

/// What the compositor answers to one request.
fn answer(
    gate: &mut Gate,
    startup: &Startup,
    desk: &mut Desk,
    held: &mut [Option<Held>; WINDOWS],
    format: PixelFormat,
    badge: u64,
    region: &mut Rect,
) -> Reply {
    match Request::decode(gate.reader()) {
        Ok(Request::Open {
            width,
            height,
            title,
            notification,
            process,
        }) => {
            let mut opening = Outcome::nothing();
            let outcome = open(
                gate,
                startup,
                desk,
                held,
                &mut opening,
                Client {
                    badge,
                    width,
                    height,
                    title,
                    notification,
                    process,
                },
            );
            if outcome.is_err() {
                let _closed = gate.handle_close(notification);
                let _also = gate.handle_close(process);
            }
            *region = region.union(opening.repaint);
            for id in &opening.woken {
                wake_client(gate, held, *id);
            }
            Reply::Opened(outcome)
        }
        Ok(Request::Draw { id, commands }) => {
            Reply::Drawn(draw(desk, held, format, badge, id, &commands, region))
        }
        Ok(Request::Close { id }) => {
            Reply::Closed(close(gate, startup, desk, held, badge, id, region))
        }
        Ok(Request::Poll { id }) => Reply::Polled(desk.poll(badge, id)),
        Err(error) => Reply::Drawn(Err(Error::from(error))),
    }
}

/// What a client asked for when it opened a window.
#[derive(Clone, Copy)]
struct Client {
    /// The badge its messages carry.
    badge: u64,
    /// Columns of content it asked for.
    width: u32,
    /// Rows of it.
    height: u32,
    /// What is to stand in its title bar.
    title: user_proto::window::Title,
    /// The notification it waits on.
    notification: Handle,
    /// The client itself, as something whose end may be watched.
    process: Handle,
}

/// Opens a window: the desktop decides where it stands, the memory server
/// gives its pixels, and this program maps them and watches the client.
fn open(
    gate: &mut Gate,
    startup: &Startup,
    desk: &mut Desk,
    held: &mut [Option<Held>; WINDOWS],
    opening: &mut Outcome,
    client: Client,
) -> Result<Given, Error> {
    let (Some(own), Some(server)) = (startup.own_process, startup.memory_server) else {
        return Err(Error::NotFound);
    };
    let Client {
        badge,
        width,
        height,
        title,
        notification,
        process,
    } = client;
    let id = desk.open(badge, width, height, title, opening)?;
    let outcome = take_on(
        gate,
        own,
        server,
        desk,
        held,
        badge,
        id,
        notification,
        process,
    );
    match outcome {
        Ok(window) => {
            let frame = desk
                .window(window.id)
                .map_or(Rect::EMPTY, server_desk::Window::frame);
            say_line(
                gate,
                startup,
                Line::<96>::of(format_args!("[desk] window {} of {badge}\n", window.id)).as_bytes(),
            );
            say_line(
                gate,
                startup,
                Line::<96>::of(format_args!(
                    "[desk] frame {} {} {} {}\n",
                    frame.x, frame.y, frame.w, frame.h
                ))
                .as_bytes(),
            );
            Ok(window)
        }
        Err(error) => {
            let _closed = desk.close(badge, id, opening);
            Err(error)
        }
    }
}

/// The memory, the mapping and the watch of one window.
#[expect(
    clippy::too_many_arguments,
    reason = "the same list as `open`, less what it has already answered for"
)]
fn take_on(
    gate: &mut Gate,
    own: ProcessHandle,
    server: EndpointHandle,
    desk: &Desk,
    held: &mut [Option<Held>; WINDOWS],
    badge: u64,
    id: u32,
    notification: Handle,
    process: Handle,
) -> Result<Given, Error> {
    let window = desk.window(id).ok_or(Error::NotFound)?;
    let (width, height) = (window.width(), window.height());
    let slot = free_slot(held).ok_or(Error::QuotaExceeded)?;
    let bytes = whole_pages(window.bytes());
    let memory = allocate(gate, server, bytes, PAGE_SIZE)?;
    let mapping = match Mapping::new(gate, own, memory, window_of(slot), bytes) {
        Ok(mapping) => mapping,
        Err(error) => {
            let _released = release(gate, server, memory);
            return Err(error);
        }
    };
    let client = ProcessHandle::from_handle(process);
    let bit = u64::try_from(slot).unwrap_or(0).saturating_add(1);
    if let Some(watcher) = watcher() {
        let _watched = gate.process_watch(client, watcher, bit);
    }
    if let Some(place) = held.get_mut(slot) {
        *place = Some(Held {
            id,
            badge,
            memory,
            mapping,
            notification: NotificationHandle::from_handle(notification),
            process: client,
            bit,
            width,
            height,
        });
    }
    Ok(Given { id, width, height })
}

/// Carries the commands of a client out in the surface of its window.
fn draw(
    desk: &Desk,
    held: &mut [Option<Held>; WINDOWS],
    format: PixelFormat,
    badge: u64,
    id: u32,
    commands: &gfx::draw::List,
    region: &mut Rect,
) -> Result<(), Error> {
    let window = desk.holding(badge, id)?;
    let frame = window.frame();
    let entry = held
        .iter_mut()
        .flatten()
        .find(|entry| entry.id == id)
        .ok_or(Error::NotFound)?;
    let (width, height) = (entry.width, entry.height);
    // SAFETY: the mapping was made when the window was opened, nothing has
    // unmapped it, and this is the only reference to it while the commands
    // are carried out.
    let bytes = unsafe { entry.mapping.bytes() };
    let mut surface =
        Surface::new(bytes, width, height, width, format).map_err(|_| Error::InvalidArgument)?;
    gfx::draw_all(&mut surface, commands);
    *region = region.union(frame);
    Ok(())
}

/// Closes a window and gives its pixels back.
fn close(
    gate: &mut Gate,
    startup: &Startup,
    desk: &mut Desk,
    held: &mut [Option<Held>; WINDOWS],
    badge: u64,
    id: u32,
    region: &mut Rect,
) -> Result<(), Error> {
    let mut moved = Outcome::nothing();
    let frame = desk.close(badge, id, &mut moved)?;
    *region = region.union(frame).union(moved.repaint);
    let slot = held
        .iter()
        .position(|entry| entry.as_ref().is_some_and(|held| held.id == id));
    if let Some(taken) = slot.and_then(|slot| held.get_mut(slot)?.take()) {
        give_back(gate, startup, taken);
    }
    // The window that is now in front heard that it has the focus; it is
    // woken after the one that left, because the entry of the one that
    // left is gone by then.
    for woken in &moved.woken {
        wake_client(gate, held, *woken);
    }
    Ok(())
}

/// Unmaps a window's pixels, gives the memory back, and lets go of the
/// client.
///
/// The watch goes first: the slot of this entry is the bit that reports
/// the end of its client, and a client that closed its window and exits
/// later must not take the window of whoever holds the slot by then.
fn give_back(gate: &mut Gate, startup: &Startup, taken: Held) {
    if let Some(watcher) = watcher() {
        let _unwatched = gate.process_unwatch(taken.process, watcher, taken.bit);
    }
    if let Some(process) = startup.own_process {
        let _unmapped = taken.mapping.unmap(gate, process);
    }
    if let Some(server) = startup.memory_server {
        let _released = release(gate, server, taken.memory);
    }
    let _closed = gate.handle_close(taken.notification.handle());
    let _also = gate.handle_close(taken.process.handle());
}

/// Paints the screen: the desktop over `region`, then every window back to
/// front, then the bar.
fn compose(
    desk: &Desk,
    held: &mut [Option<Held>; WINDOWS],
    screen: &mut Mapping,
    format: PixelFormat,
    region: Rect,
) -> Damage {
    let (width, height) = (desk.width(), desk.height());
    // SAFETY: the mapping was made in `run`, it is still standing, and
    // nothing else in this program holds a reference to it.
    let bytes = unsafe { screen.bytes() };
    let Ok(mut surface) = Surface::new(bytes, width, height, width, format) else {
        return Damage::new();
    };
    desk.paint_desktop(&mut surface, region);
    let mut order: ArrayVec<u32, WINDOWS> = ArrayVec::new();
    for window in desk.iter() {
        let _room = order.push(window.id());
    }
    for id in &order {
        let Some(window) = desk.window(*id) else {
            continue;
        };
        desk.paint_chrome(&mut surface, window);
        let at = window.content();
        let Some(entry) = held.iter_mut().flatten().find(|entry| entry.id == *id) else {
            continue;
        };
        let (content_width, content_height) = (entry.width, entry.height);
        // SAFETY: the mapping was made when the window was opened and
        // nothing has unmapped it; the surface of the screen is another
        // mapping and another slice.
        let content = unsafe { entry.mapping.bytes() };
        let Ok(pixels) = Surface::new(
            content,
            content_width,
            content_height,
            content_width,
            format,
        ) else {
            continue;
        };
        surface.blit(
            &pixels,
            Rect::new(0, 0, content_width, content_height),
            at.x,
            at.y,
        );
    }
    desk.paint_bar(&mut surface);
    *surface.damage()
}

/// Asks what the screen is.
fn ask_mode(gate: &mut Gate, display: EndpointHandle) -> Result<Mode, Error> {
    DisplayRequest::Info.encode(&mut gate.writer())?;
    gate.ipc_call(display)?;
    match DisplayReply::decode(gate.reader())? {
        DisplayReply::Screen(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Asks for a surface the size of the screen.
fn ask_surface(
    gate: &mut Gate,
    display: EndpointHandle,
    mode: Mode,
    watched: Handle,
) -> Result<user_proto::display::Surface, Error> {
    let request = DisplayRequest::CreateSurface {
        width: mode.width,
        height: mode.height,
        process: watched,
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(display)?;
    match DisplayReply::decode(gate.reader())? {
        DisplayReply::Created(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Gives the surface of the screen back.
fn destroy_surface(gate: &mut Gate, display: EndpointHandle, id: u32) -> Result<(), Error> {
    DisplayRequest::DestroySurface { id }.encode(&mut gate.writer())?;
    gate.ipc_call(display)?;
    match DisplayReply::decode(gate.reader())? {
        DisplayReply::Destroyed(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Puts what was composed on the screen.
fn present(
    gate: &mut Gate,
    display: EndpointHandle,
    id: u32,
    damage: &Damage,
) -> Result<(), Error> {
    let request = DisplayRequest::Present {
        id,
        damage: *damage,
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(display)?;
    match DisplayReply::decode(gate.reader())? {
        DisplayReply::Presented(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Puts the sprite where the pointer now is.
fn set_cursor(gate: &mut Gate, display: EndpointHandle, x: u32, y: u32) -> Result<(), Error> {
    let request = DisplayRequest::SetCursor {
        x,
        y,
        visible: true,
        shape: CursorShape::Arrow,
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(display)?;
    match DisplayReply::decode(gate.reader())? {
        DisplayReply::CursorSet(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Asks the input server for a ring, handing over the notification it is to
/// signal.
fn subscribe(
    gate: &mut Gate,
    input: EndpointHandle,
    notification: Handle,
    process: Handle,
) -> Result<Handle, Error> {
    let request = InputRequest::Subscribe {
        notification,
        process,
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(input)?;
    match InputReply::decode(gate.reader())? {
        InputReply::Subscribed(outcome) => outcome,
        InputReply::Unsubscribed(_) => Err(Error::InvalidArgument),
    }
}

/// Starts the thread that waits on the notification and wakes the desktop.
fn start_waker(
    gate: &mut Gate,
    startup: &Startup,
    endpoint: EndpointHandle,
    notification: NotificationHandle,
) -> Result<(), Error> {
    let own = startup.own_process.ok_or(Error::NotFound)?;
    let server = startup.memory_server.ok_or(Error::NotFound)?;
    let badged = gate.endpoint_badge(endpoint, WAKE_BADGE)?;
    let bytes = WAKE_STACK_PAGES.wrapping_mul(PAGE_SIZE);
    let object = allocate(gate, server, bytes, PAGE_SIZE)?;
    let base = WAKE_STACK_TOP.wrapping_sub(bytes);
    // The mapping stays: the stack of a thread that never ends is never
    // taken back.
    let _stack = Mapping::new(gate, own, object, base, bytes)?;
    SHARED.set(badged, notification);
    let thread = gate.thread_create(
        own,
        waker_address(),
        WAKE_STACK_TOP,
        u64::from(user_programs::priority::SERVER),
        u64::from(user_programs::priority::SERVER),
        None,
    )?;
    gate.thread_start(thread)?;
    Ok(())
}

/// Where the waking thread begins.
#[expect(
    clippy::as_conversions,
    reason = "a function has to become an address for `thread_create`, and there is no other way to write it"
)]
fn waker_address() -> u64 {
    let pointer: unsafe extern "sysv64" fn(u64) -> ! = waker;
    pointer as usize as u64
}

/// The waking thread: wait for the notification until the deadline, and
/// hand what was heard to the thread that holds the windows.
///
/// # Safety
///
/// The kernel starts this once, with the address of the thread's IPC buffer
/// in the first argument register.
unsafe extern "sysv64" fn waker(ipc_buffer: u64) -> ! {
    // SAFETY: the kernel started this thread with the address of its own
    // buffer, and this is the only gate over it.
    let mut gate = unsafe { Gate::adopt(ipc_buffer) };
    let Some((endpoint, notification)) = SHARED.get() else {
        gate.thread_exit()
    };
    loop {
        let now = gate.clock_now().unwrap_or(0);
        let word = gate
            .notification_wait_until(notification, now.saturating_add(TICK))
            .unwrap_or(0);
        let mut writer = user_rt::message::Writer::new();
        {
            let mut buffer = gate.writer();
            let _written = writer.word(&mut buffer, word);
            let _finished = writer.finish(&mut buffer, WAKE_LABEL);
        }
        let _sent = gate.ipc_send(endpoint);
    }
}

/// Answers every request with `NotFound` until the machine ends.
fn refuse(gate: &mut Gate, endpoint: EndpointHandle) -> ! {
    let mut serving = Serving::default();
    loop {
        if receive(gate, endpoint, &mut serving).is_err() {
            gate.thread_exit()
        }
        let answer = match Request::decode(gate.reader()) {
            Ok(Request::Open {
                notification,
                process,
                ..
            }) => {
                let _closed = gate.handle_close(notification);
                let _also = gate.handle_close(process);
                Reply::Opened(Err(Error::NotFound))
            }
            Ok(Request::Draw { .. }) => Reply::Drawn(Err(Error::NotFound)),
            Ok(Request::Close { .. }) => Reply::Closed(Err(Error::NotFound)),
            Ok(Request::Poll { .. }) => Reply::Polled(Err(Error::NotFound)),
            Err(error) => Reply::Drawn(Err(Error::from(error))),
        };
        let _written = answer.encode(&mut gate.writer());
    }
}

/// Tells the process that started this one that the desktop is down. The
/// machine ends when every program that reports has reported, and this one
/// reports because it is a server that ends.
fn report_to_parent(gate: &mut Gate, startup: &Startup) {
    let Some(parent) = startup.parent else {
        return;
    };
    let finished = user_proto::parent::Request::Finished {
        status: user_proto::parent::SUCCESS,
    };
    if finished.encode(&mut gate.writer()).is_ok() {
        let _reported = gate.ipc_send(parent);
    }
}

/// Says what went wrong, in one line.
fn say(gate: &mut Gate, startup: &Startup, what: &str, error: Error) {
    let line = Line::<128>::of(format_args!("[desk] {what}: {}\n", error.message()));
    say_line(gate, startup, line.as_bytes());
}

/// Writes one line to the console the root task gave this program.
fn say_line(gate: &mut Gate, startup: &Startup, line: &[u8]) {
    if let Some(console) = startup.log {
        let _said = write_line(gate, console, line);
    }
}

/// `bytes` rounded up to whole pages, which is what a mapping covers.
const fn whole_pages(bytes: u64) -> u64 {
    bytes
        .saturating_add(PAGE_SIZE.saturating_sub(1))
        .wrapping_div(PAGE_SIZE)
        .saturating_mul(PAGE_SIZE)
}

/// The window of the address space slot `index` maps its pixels in.
fn window_of(index: usize) -> u64 {
    let step = u64::try_from(index)
        .unwrap_or(0)
        .saturating_mul(CONTENT_SLOT);
    CONTENTS.saturating_add(step)
}

/// The notification the ends of the clients are reported on, which the
/// waking thread waits on.
fn watcher() -> Option<NotificationHandle> {
    NotificationHandle::from_raw(SHARED.notification.load(Ordering::SeqCst))
}

/// The first slot that holds no window.
fn free_slot(held: &[Option<Held>; WINDOWS]) -> Option<usize> {
    held.iter().position(Option::is_none)
}
