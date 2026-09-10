// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The graphical demonstration: a surface the size of the screen, a cursor
//! that follows the pointer, a stroke drawn while a button is held, and the
//! characters that are typed at it.
//!
//! It is both halves of the userland at once — the display protocol and the
//! input protocol in one loop — and it is the client the end-to-end tests
//! drive: what it does it also says on the console, so the runner knows
//! where to look in the picture it takes of the screen and never has to
//! work a position out for itself.
//!
//! Nothing is presented until the first event arrives (D-126). The program
//! that draws before this one holds a surface the size of the screen too,
//! and its picture is checked while the machine still runs; a canvas that
//! laid its background down at startup would decide by a race which of the
//! two the screen holds. Waiting for an event decides it by what happened.
//!
//! It ends when the key [`app_canvas::ENDS`] comes up, which is the last
//! thing the runner injects. The escape key does not end it: that clears
//! the canvas. A machine without a framebuffer has no screen to give it a
//! surface on, and there it says so and ends without waiting for anything.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds twelve programs and each uses a different part of what
// it depends on; these are the crates this one does not.
use driver_i8042 as _;
use driver_uart16550 as _;
use server_console as _;
use server_display as _;
use server_input as _;
use server_memory as _;
use server_name as _;
use user_loader as _;

use app_canvas::{Canvas, Step};
use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::{Error, Handle, Rights};
use gfx::{Damage, PixelFormat, Surface};
use user_programs::client::{lookup, write_line};
use user_programs::mapping::Mapping;
use user_proto::display::{Mode, Reply, Request, Surface as Given};
use user_proto::input::{Event, Reply as InputReply, Request as InputRequest, RingReader};
use user_proto::parent;
use user_rt::{EndpointHandle, MemoryHandle, Startup, Typed};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the display server registered itself under.
const DISPLAY: &[u8] = b"display";

/// The name the input server registered itself under.
const INPUT: &[u8] = b"input";

/// Where the pixels of the surface are mapped in this program.
const SURFACE: u64 = 0x3000_0000;

/// Where the ring of events is mapped in it.
const RING: u64 = 0x4000_0000;

/// What a notification handed to a server may be used for: signalling it,
/// and being handed over at all.
const SIGNAL_ONLY: Rights = Rights::SIGNAL.union(Rights::TRANSFER);

/// What a process handed to a server may be used for: being asked whether
/// it is still there, and being handed over at all.
const WATCHED: Rights = Rights::INFO.union(Rights::TRANSFER);

/// Draws what is done to it until the key that ends it comes up.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    match run(&mut gate, &startup) {
        Ok(()) => say(&mut gate, &startup, b"[canvas] done\n"),
        Err(error) => say(
            &mut gate,
            &startup,
            user_rt::Line::<128>::of(format_args!("[canvas] no screen: {}\n", error.message()))
                .as_bytes(),
        ),
    }
    report(&mut gate, &startup);
    gate.thread_exit()
}

/// Takes the surface and the ring, then draws until the end key comes up.
fn run(gate: &mut Gate, startup: &Startup) -> Result<(), Error> {
    let process = startup.own_process.ok_or(Error::NotFound)?;
    // Both capabilities the root task gave this program carry the badge the
    // servers know it by. Looking a server up under its name would give one
    // that carries none, and both refuse those: each keeps something per
    // client and cannot tell two of nobody apart.
    let display = match startup.display_server {
        Some(given) => given,
        None => lookup(gate, startup.name_server.ok_or(Error::NotFound)?, DISPLAY)?,
    };
    let input = match startup.input_server {
        Some(given) => given,
        None => lookup(gate, startup.name_server.ok_or(Error::NotFound)?, INPUT)?,
    };
    // The screen first: on a machine that has none this fails here, and
    // nothing is subscribed to that would then have to be given back.
    let mode = ask_mode(gate, display)?;
    let watched = gate.handle_duplicate(process.handle(), WATCHED)?;
    let given = ask_surface(gate, display, mode, watched)?;
    let memory = MemoryHandle::from_handle(given.memory);
    let bytes = u64::from(mode.width)
        .saturating_mul(u64::from(mode.height))
        .saturating_mul(4);
    let mut pixels = Mapping::new(gate, process, memory, SURFACE, round_up(bytes))?;

    let notification = gate.notification_create()?;
    let signal = gate.handle_duplicate(notification.handle(), SIGNAL_ONLY)?;
    let listener = gate.handle_duplicate(process.handle(), WATCHED)?;
    let ring = MemoryHandle::from_handle(subscribe(gate, input, signal, listener)?);
    gate.handle_close(signal)?;
    gate.handle_close(listener)?;
    let events = Mapping::new(gate, process, ring, RING, PAGE_SIZE)?;

    let mut canvas = Canvas::new(mode.width, mode.height);
    say(
        gate,
        startup,
        user_rt::Line::<64>::of(format_args!(
            "[canvas] ready {}x{}\n",
            mode.width, mode.height
        ))
        .as_bytes(),
    );

    loop {
        gate.notification_wait(notification)?;
        // A wake-up says that something is waiting and not how much, so the
        // ring is read until it is empty. One event is taken out at a time
        // and everything it costs — the drawing, the presentation, the
        // sprite, the line on the console — is done before the next, so
        // what the runner reads on the console is in the order it happened.
        loop {
            let taken = {
                // SAFETY: the mapping remains live and the server published
                // an initialized RingPage. Both processes use atomic access.
                let page = unsafe { events.ring() }.ok_or(Error::InvalidArgument)?;
                let mut reader = RingReader::new(page).ok_or(Error::InvalidArgument)?;
                reader.pop()
            };
            let Some(event) = taken else {
                break;
            };
            if act(
                gate,
                startup,
                &mut canvas,
                &mut pixels,
                display,
                given.id,
                mode,
                event,
            )? {
                return Ok(());
            }
        }
    }
}

/// Takes one event and answers whether it ended the program.
#[expect(
    clippy::too_many_arguments,
    reason = "one event needs the canvas, the pixels, the server, and the surface it belongs to; carrying them in a struct would name the same things once more"
)]
fn act(
    gate: &mut Gate,
    startup: &Startup,
    canvas: &mut Canvas,
    pixels: &mut Mapping,
    display: EndpointHandle,
    id: u32,
    mode: Mode,
    event: Event,
) -> Result<bool, Error> {
    let (step, damage) = {
        // SAFETY: the mapping was made in `run`, it is still standing, and
        // nothing else in this program holds a reference to it.
        let bytes = unsafe { pixels.bytes() };
        let mut surface = Surface::new(
            bytes,
            mode.width,
            mode.height,
            mode.width,
            PixelFormat::from_boot(mode.format),
        )
        .map_err(|_| Error::InvalidArgument)?;
        // The surface is made again for every event, so it carries the
        // damage of this one alone and a presentation copies no rectangle
        // twice.
        let step = canvas.feed(event, &mut surface);
        (step, *surface.damage())
    };
    if !damage.is_empty() {
        present(gate, display, id, &damage)?;
    }
    if matches!(event, Event::Pointer(_)) {
        let (x, y) = canvas.cursor();
        set_cursor(gate, display, x, y)?;
    }
    say(gate, startup, told(step, canvas).as_bytes());
    Ok(matches!(step, Step::Ended))
}

/// The line the console gets for `step`.
fn told(step: Step, canvas: &Canvas) -> user_rt::Line<96> {
    match step {
        Step::Nothing => user_rt::Line::of(format_args!("")),
        Step::Moved => {
            let (x, y) = canvas.cursor();
            user_rt::Line::of(format_args!("[canvas] cursor {x} {y}\n"))
        }
        Step::Drew { from, to } => user_rt::Line::of(format_args!(
            "[canvas] stroke {} {} {} {}\n",
            from.0, from.1, to.0, to.1
        )),
        Step::Typed { x, y, character } => {
            user_rt::Line::of(format_args!("[canvas] text {x} {y} {character}\n"))
        }
        Step::Cleared => user_rt::Line::of(format_args!("[canvas] cleared\n")),
        Step::Ended => user_rt::Line::of(format_args!("[canvas] ending\n")),
    }
}

/// `len` rounded up to whole pages, which is what a mapping covers.
const fn round_up(len: u64) -> u64 {
    len.saturating_add(PAGE_SIZE.saturating_sub(1))
        .wrapping_div(PAGE_SIZE)
        .saturating_mul(PAGE_SIZE)
}

/// Asks what the screen is.
fn ask_mode(gate: &mut Gate, display: EndpointHandle) -> Result<Mode, Error> {
    Request::Info.encode(&mut gate.writer())?;
    gate.ipc_call(display)?;
    match Reply::decode(gate.reader())? {
        Reply::Screen(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Asks for a surface of the size of the screen, handing over the
/// capability the server watches this program's end through.
fn ask_surface(
    gate: &mut Gate,
    display: EndpointHandle,
    mode: Mode,
    watched: Handle,
) -> Result<Given, Error> {
    let request = Request::CreateSurface {
        width: mode.width,
        height: mode.height,
        process: watched,
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(display)?;
    match Reply::decode(gate.reader())? {
        Reply::Created(outcome) => outcome,
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

/// Puts what was drawn on the screen.
fn present(
    gate: &mut Gate,
    display: EndpointHandle,
    id: u32,
    damage: &Damage,
) -> Result<(), Error> {
    let request = Request::Present {
        id,
        damage: *damage,
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(display)?;
    match Reply::decode(gate.reader())? {
        Reply::Presented(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Puts the sprite where the pointer now is.
fn set_cursor(gate: &mut Gate, display: EndpointHandle, x: u32, y: u32) -> Result<(), Error> {
    let request = Request::SetCursor {
        x,
        y,
        visible: true,
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(display)?;
    match Reply::decode(gate.reader())? {
        Reply::CursorSet(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Says one line on the console the root task gave this program, and
/// nothing at all for an event that changed nothing.
fn say(gate: &mut Gate, startup: &Startup, line: &[u8]) {
    if line.is_empty() {
        return;
    }
    if let Some(log) = startup.log {
        let _said = write_line(gate, log, line);
    }
}

/// Tells the process that started this one that the work is done.
fn report(gate: &mut Gate, startup: &Startup) {
    let Some(parent) = startup.parent else {
        return;
    };
    let finished = parent::Request::Finished {
        status: parent::SUCCESS,
    };
    if finished.encode(&mut gate.writer()).is_ok() {
        let _reported = gate.ipc_send(parent);
    }
}
