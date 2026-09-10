// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The display server: the one program that writes to the framebuffer.
//!
//! It is given the device memory over the framebuffer and the mode the
//! firmware set, maps the first read and write, and hands every client a
//! memory object of its own to draw into. What a client presents is copied
//! out of that object and onto the screen, rectangle by rectangle.
//!
//! Everything it decides is in `server-display`; this is the loop around
//! it, and the system calls the loop needs: mapping the framebuffer,
//! asking the memory server for the pixels of a surface, and giving the
//! client the handle to them.
//!
//! A client that goes away gives its surface back without saying anything:
//! it hands over a capability to its own process when it asks for the
//! surface, the kernel signals a bit of a notification when that process
//! ends (D-106), and a second thread of this program turns that signal into
//! a message to the first one. So a client that exits and a client that
//! faults are the same thing here.
//!
//! A machine without a framebuffer starts this program all the same: it
//! answers `NotFound` to everything and stays where it is, because a client
//! that asks for a screen has to hear that there is none.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds twelve programs and each uses a different part of what it
// depends on; these are the crates this one does not.
use app_canvas as _;
use driver_i8042 as _;
use driver_uart16550 as _;
use server_console as _;
use server_input as _;
use server_memory as _;
use server_name as _;
use user_loader as _;

use core::sync::atomic::{AtomicU64, Ordering};

use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::{Error, Handle};
use gfx::{Damage, PixelFormat, Surface};
use server_display::Display;
use user_programs::client::{allocate, register, write_line};
use user_programs::mapping::Mapping;
use user_programs::serve::{Serving, receive};
use user_proto::display::{Mode, Reply, Request, Surface as Given};
use user_rt::{EndpointHandle, MemoryHandle, NotificationHandle, ProcessHandle, Startup, Typed};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the server registers itself under.
const NAME: &[u8] = b"display";

/// How many clients this process holds surfaces for. Each of them costs a
/// window of the address space and a mapping of it.
const CLIENTS: usize = 4;

/// Where the framebuffer is mapped.
const FRAMEBUFFER: u64 = 0x1000_0000;

/// Where the first client surface is mapped.
const SURFACES: u64 = 0x2000_0000;

/// How much address space one client surface gets, which is more than the
/// largest screen this system drives.
const SURFACE_SLOT: u64 = 0x0100_0000;

/// The badge the watcher thread sends under, which is how the serving
/// thread tells its message from a client's request.
const GONE_BADGE: u64 = 0x60_4E;

/// The label it sends the word of the notification under.
const GONE_LABEL: u64 = 1;

/// The top of the stack of the watcher thread, and how many pages it gets.
const WATCH_STACK_TOP: u64 = 0x0080_0000;
const WATCH_STACK_PAGES: u64 = 4;

/// What the watcher thread has to be told, in a place it can reach: it
/// starts with nothing but the address of its own IPC buffer, and this is a
/// process without a heap.
static SHARED: Shared = Shared::new();

/// The endpoint the watcher thread sends to and the notification it waits
/// on.
struct Shared {
    /// The endpoint of this server, badged as [`GONE_BADGE`].
    endpoint: AtomicU64,
    /// The notification the kernel signals the end of a client on.
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

/// What the serving thread holds between two messages.
struct Server<'a> {
    /// What the server decides, which is `server-display`.
    display: &'a mut Display<CLIENTS>,
    /// The mappings behind the surfaces it decides about.
    held: &'a mut [Option<Held>; CLIENTS],
    /// The notification the end of a client is signalled on, when there is
    /// a thread waiting for it.
    watcher: Option<NotificationHandle>,
}

/// The mapping of one client's pixels, and which surface it belongs to.
struct Held {
    /// The surface number the client knows it by.
    id: u32,
    /// The badge of the client that holds it.
    badge: u64,
    /// The memory object of its pixels.
    memory: MemoryHandle,
    /// The client itself, which this program watches the end of. The slot
    /// this entry stands in is the bit of the notification that end
    /// signals.
    process: ProcessHandle,
    /// Where the pixels are mapped in this process.
    mapping: Mapping,
    /// Visible columns.
    width: u32,
    /// Visible rows.
    height: u32,
}

/// Serves the screen until the endpoint is gone.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let Some(endpoint) = startup.own_endpoint else {
        gate.thread_exit()
    };
    if let Some(names) = startup.name_server {
        let outcome = register(&mut gate, names, NAME, endpoint);
        if let Err(error) = outcome {
            say(&mut gate, startup.log, "cannot register", error);
        }
    }

    let mut framebuffer = map_framebuffer(&mut gate, &startup);
    let mut screen: Option<Surface<'_>> = match (&mut framebuffer, startup.screen()) {
        (Some(mapping), Some(described)) => {
            // SAFETY: the mapping was made just now, it is still standing,
            // and nothing else in this program holds a reference to it.
            let bytes = unsafe { mapping.bytes() };
            Surface::new(
                bytes,
                described.width,
                described.height,
                described.stride,
                PixelFormat::from_boot(described.format),
            )
            .ok()
        }
        _ => None,
    };
    let mode = screen.as_ref().map(|surface| Mode {
        width: surface.width(),
        height: surface.height(),
        format: surface.format().to_boot(),
    });
    match mode {
        Some(found) => say_mode(&mut gate, startup.log, found),
        None => say_line(&mut gate, startup.log, b"[display] no framebuffer\n"),
    }

    let watcher = start_watcher(&mut gate, &startup, endpoint);
    if watcher.is_none() {
        say_line(
            &mut gate,
            startup.log,
            b"[display] nobody watches the clients\n",
        );
    }

    let mut display: Display<CLIENTS> = Display::new(mode);
    let mut held: [Option<Held>; CLIENTS] = [const { None }; CLIENTS];
    let mut serving = Serving::default();
    loop {
        if receive(&mut gate, endpoint, &mut serving).is_err() {
            gate.thread_exit()
        }
        // The watcher thread says which clients are gone; their surfaces go
        // back before anything else is answered, and the message is a send,
        // so there is nothing to reply to.
        if serving.badge == GONE_BADGE {
            let gone = gate.reader().word(0).unwrap_or(0);
            release_gone(&mut gate, &startup, &mut display, &mut held, gone);
            continue;
        }
        let answer = match Request::decode(gate.reader()) {
            Ok(request) => {
                let mut server = Server {
                    display: &mut display,
                    held: &mut held,
                    watcher,
                };
                handle(
                    &mut gate,
                    &startup,
                    &mut server,
                    screen.as_mut(),
                    serving.badge,
                    &request,
                )
            }
            Err(error) => Reply::Presented(Err(Error::from(error))),
        };
        let _written = answer.encode(&mut gate.writer());
    }
}

/// Maps the framebuffer, if this machine has one and it can be mapped.
fn map_framebuffer(gate: &mut Gate, startup: &Startup) -> Option<Mapping> {
    let process = startup.own_process?;
    let memory = startup.framebuffer?;
    let info = gate.memory_info(memory).ok()?;
    Mapping::new(gate, process, memory, FRAMEBUFFER, info.length).ok()
}

/// Says what went wrong, in one line.
fn say(gate: &mut Gate, log: Option<EndpointHandle>, what: &str, error: Error) {
    let line = user_rt::Line::<128>::of(format_args!("[display] {what}: {}\n", error.message()));
    let _said = sys::write_line(gate, log, line.as_bytes());
}

/// Says what the screen is, which is what the end-to-end test reads the
/// resolution out of instead of assuming one.
fn say_mode(gate: &mut Gate, log: Option<EndpointHandle>, mode: Mode) {
    let line = user_rt::Line::<128>::of(format_args!(
        "[display] screen={}x{} format={}\n",
        mode.width,
        mode.height,
        PixelFormat::from_boot(mode.format).name()
    ));
    say_line(gate, log, line.as_bytes());
}

/// Writes one line to the console the root task gave this program, and
/// nowhere when it was given none.
fn say_line(gate: &mut Gate, log: Option<EndpointHandle>, line: &[u8]) {
    if let Some(console) = log {
        let _said = write_line(gate, console, line);
    }
}

/// What the server answers to one request.
fn handle(
    gate: &mut Gate,
    startup: &Startup,
    server: &mut Server<'_>,
    screen: Option<&mut Surface<'_>>,
    badge: u64,
    request: &Request,
) -> Reply {
    let Server {
        display,
        held,
        watcher,
    } = server;
    let watcher = *watcher;
    match request {
        Request::Info => Reply::Screen(display.screen()),
        Request::CreateSurface {
            width,
            height,
            process,
        } => Reply::Created(create(
            gate, startup, display, held, watcher, badge, *width, *height, *process,
        )),
        Request::Present { id, damage } => {
            Reply::Presented(present(display, held, screen, badge, *id, damage))
        }
        Request::DestroySurface { id } => {
            Reply::Destroyed(destroy(gate, startup, display, held, badge, *id))
        }
        Request::SetCursor { x, y, visible } => Reply::CursorSet(match screen {
            Some(surface) => display.set_cursor(*x, *y, *visible, surface),
            None => Err(Error::NotFound),
        }),
    }
}

/// Makes a surface for a client: memory out of the memory server, mapped
/// here so the pixels can be read, and the handle to it in the answer.
#[expect(
    clippy::too_many_arguments,
    reason = "making a surface needs the client, its size, where it goes, and everything the server holds; a structure for them would be the argument list under another name"
)]
fn create(
    gate: &mut Gate,
    startup: &Startup,
    display: &mut Display<CLIENTS>,
    held: &mut [Option<Held>; CLIENTS],
    watcher: Option<NotificationHandle>,
    badge: u64,
    width: u32,
    height: u32,
    client: Handle,
) -> Result<Given, Error> {
    let (Some(process), Some(server)) = (startup.own_process, startup.memory_server) else {
        return Err(Error::NotFound);
    };
    let made = display.create(badge, width, height)?;
    let slot = free_slot(held).ok_or(Error::QuotaExceeded);
    let outcome = slot.and_then(|index| {
        // A mapping covers whole pages, so the window is the pixels rounded
        // up; the memory server rounds the object up the same way.
        let bytes = whole_pages(made.bytes());
        let memory = allocate(gate, server, bytes, PAGE_SIZE)?;
        let mapping = match Mapping::new(gate, process, memory, window_of(index), bytes) {
            Ok(mapping) => mapping,
            Err(error) => {
                let _released = user_programs::client::release(gate, server, memory);
                return Err(error);
            }
        };
        // The slot is the bit: whichever client of the four ends, the word
        // of the notification says which surface goes back.
        let client = ProcessHandle::from_handle(client);
        if let Some(notification) = watcher {
            let bit = u64::try_from(index).unwrap_or(0);
            let _watched = gate.process_watch(client, notification, bit);
        }
        put(
            held,
            index,
            Held {
                id: made.id,
                badge,
                memory,
                process: client,
                mapping,
                width,
                height,
            },
        );
        Ok(Given {
            id: made.id,
            memory: memory.handle(),
        })
    });
    if outcome.is_err() {
        let _gone = display.destroy(badge, made.id);
    }
    outcome
}

/// Starts the thread that waits for the end of a client, and answers with
/// the notification the kernel signals those ends on.
fn start_watcher(
    gate: &mut Gate,
    startup: &Startup,
    endpoint: EndpointHandle,
) -> Option<NotificationHandle> {
    let own = startup.own_process?;
    let server = startup.memory_server?;
    let notification = gate.notification_create().ok()?;
    let badged = gate.endpoint_badge(endpoint, GONE_BADGE).ok()?;
    let bytes = WATCH_STACK_PAGES.wrapping_mul(PAGE_SIZE);
    let object = allocate(gate, server, bytes, PAGE_SIZE).ok()?;
    let base = WATCH_STACK_TOP.wrapping_sub(bytes);
    // The mapping stays: the stack of a thread that never ends is never
    // taken back.
    let _stack = Mapping::new(gate, own, object, base, bytes).ok()?;
    SHARED.set(badged, notification);
    let thread = gate
        .thread_create(
            own,
            watcher_address(),
            WATCH_STACK_TOP,
            u64::from(user_programs::priority::DRIVER),
            u64::from(user_programs::priority::DRIVER),
            None,
        )
        .ok()?;
    gate.thread_start(thread).ok()?;
    Some(notification)
}

/// Where the watcher thread begins.
#[expect(
    clippy::as_conversions,
    reason = "a function has to become an address for `thread_create`, and there is no other way to write it"
)]
fn watcher_address() -> u64 {
    let pointer: unsafe extern "sysv64" fn(u64) -> ! = watcher;
    pointer as usize as u64
}

/// The watcher thread: wait for the kernel to say that a client has ended,
/// and hand the word to the thread that holds the surfaces.
///
/// # Safety
///
/// The kernel starts this once, with the address of the thread's IPC buffer
/// in the first argument register.
unsafe extern "sysv64" fn watcher(ipc_buffer: u64) -> ! {
    // SAFETY: the kernel started this thread with the address of its own
    // buffer, and this is the only gate over it.
    let mut gate = unsafe { Gate::adopt(ipc_buffer) };
    let Some((endpoint, notification)) = SHARED.get() else {
        gate.thread_exit()
    };
    loop {
        let Ok(word) = gate.notification_wait(notification) else {
            gate.thread_exit()
        };
        let mut writer = user_rt::message::Writer::new();
        {
            let mut buffer = gate.writer();
            let _written = writer.word(&mut buffer, word);
            let _finished = writer.finish(&mut buffer, GONE_LABEL);
        }
        let _sent = gate.ipc_send(endpoint);
    }
}

/// Gives back the surfaces of the clients whose bits stand in `gone`.
fn release_gone(
    gate: &mut Gate,
    startup: &Startup,
    display: &mut Display<CLIENTS>,
    held: &mut [Option<Held>; CLIENTS],
    gone: u64,
) {
    for index in 0..CLIENTS {
        let bit = 1_u64
            .checked_shl(u32::try_from(index).unwrap_or(0))
            .unwrap_or(0);
        if gone & bit == 0 {
            continue;
        }
        let Some(slot) = take(held, index) else {
            continue;
        };
        let _forgotten = display.forget(slot.badge);
        say_line(
            gate,
            startup.log,
            user_rt::Line::<96>::of(format_args!(
                "[display] client {} is gone: surface {} released\n",
                slot.badge, slot.id
            ))
            .as_bytes(),
        );
        give_back(gate, startup, slot);
    }
}

/// Unmaps a surface, gives its memory back, and lets go of the client.
fn give_back(gate: &mut Gate, startup: &Startup, slot: Held) {
    if let Some(process) = startup.own_process {
        let _unmapped = slot.mapping.unmap(gate, process);
    }
    if let Some(server) = startup.memory_server {
        let _released = user_programs::client::release(gate, server, slot.memory);
    }
    let _closed = gate.handle_close(slot.process.handle());
}

/// Copies what the client drew onto the screen.
fn present(
    display: &mut Display<CLIENTS>,
    held: &mut [Option<Held>; CLIENTS],
    screen: Option<&mut Surface<'_>>,
    badge: u64,
    id: u32,
    damage: &Damage,
) -> Result<(), Error> {
    let screen = screen.ok_or(Error::NotFound)?;
    let format = display.format()?;
    // Whose surface it is comes first: a client that names another's is
    // refused before this program touches the pixels of it.
    display.holding(badge, id)?;
    let slot = held
        .iter_mut()
        .flatten()
        .find(|entry| entry.id == id)
        .ok_or(Error::NotFound)?;
    // SAFETY: the mapping was made when the surface was created, nothing
    // has unmapped it, and this is the only reference to it while the copy
    // runs.
    let bytes = unsafe { slot.mapping.bytes() };
    let pixels = Surface::new(bytes, slot.width, slot.height, slot.width, format)
        .map_err(|_| Error::InvalidArgument)?;
    display.present(badge, id, damage, &pixels, screen)?;
    Ok(())
}

/// Gives a surface up: the mapping goes, and the memory with it.
fn destroy(
    gate: &mut Gate,
    startup: &Startup,
    display: &mut Display<CLIENTS>,
    held: &mut [Option<Held>; CLIENTS],
    badge: u64,
    id: u32,
) -> Result<(), Error> {
    display.destroy(badge, id)?;
    let Some(index) = held
        .iter()
        .position(|entry| entry.as_ref().is_some_and(|slot| slot.id == id))
    else {
        return Ok(());
    };
    let Some(slot) = take(held, index) else {
        return Ok(());
    };
    give_back(gate, startup, slot);
    Ok(())
}

/// `bytes` rounded up to whole pages, which is what a mapping covers.
const fn whole_pages(bytes: u64) -> u64 {
    bytes
        .saturating_add(PAGE_SIZE.saturating_sub(1))
        .wrapping_div(PAGE_SIZE)
        .saturating_mul(PAGE_SIZE)
}

/// The window of the address space slot `index` maps its surface in.
fn window_of(index: usize) -> u64 {
    let step = u64::try_from(index)
        .unwrap_or(0)
        .saturating_mul(SURFACE_SLOT);
    SURFACES.saturating_add(step)
}

/// The first slot that holds no surface.
fn free_slot(held: &[Option<Held>; CLIENTS]) -> Option<usize> {
    held.iter().position(Option::is_none)
}

/// Puts `entry` into slot `index`.
fn put(held: &mut [Option<Held>; CLIENTS], index: usize, entry: Held) {
    if let Some(slot) = held.get_mut(index) {
        *slot = Some(entry);
    }
}

/// Takes what stands in slot `index`.
fn take(held: &mut [Option<Held>; CLIENTS], index: usize) -> Option<Held> {
    held.get_mut(index)?.take()
}
