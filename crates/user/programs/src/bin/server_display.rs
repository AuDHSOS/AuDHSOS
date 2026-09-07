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
//! A machine without a framebuffer starts this program all the same: it
//! answers `NotFound` to everything and stays where it is, because a client
//! that asks for a screen has to hear that there is none.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds nine programs and each uses a different part of what it
// depends on; these are the crates this one does not.
use driver_uart16550 as _;
use server_console as _;
use server_memory as _;
use server_name as _;
use user_loader as _;

use audhsos_abi::Error;
use audhsos_abi::layout::PAGE_SIZE;
use gfx::{Damage, PixelFormat, Surface};
use server_display::Display;
use user_programs::client::{allocate, register, write_line};
use user_programs::mapping::Mapping;
use user_programs::serve::{Serving, receive};
use user_proto::display::{Mode, Reply, Request, Surface as Given};
use user_rt::{EndpointHandle, MemoryHandle, Startup, Typed};
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

/// The mapping of one client's pixels, and which surface it belongs to.
struct Held {
    /// The surface number the client knows it by.
    id: u32,
    /// The memory object of its pixels.
    memory: MemoryHandle,
    /// Where they are mapped in this process.
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

    let mut display: Display<CLIENTS> = Display::new(mode);
    let mut held: [Option<Held>; CLIENTS] = [const { None }; CLIENTS];
    let mut serving = Serving::default();
    loop {
        if receive(&mut gate, endpoint, &mut serving).is_err() {
            gate.thread_exit()
        }
        let answer = match Request::decode(gate.reader()) {
            Ok(request) => handle(
                &mut gate,
                &startup,
                &mut display,
                &mut held,
                screen.as_mut(),
                serving.badge,
                &request,
            ),
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
    display: &mut Display<CLIENTS>,
    held: &mut [Option<Held>; CLIENTS],
    screen: Option<&mut Surface<'_>>,
    badge: u64,
    request: &Request,
) -> Reply {
    match request {
        Request::Info => Reply::Screen(display.screen()),
        Request::CreateSurface { width, height } => {
            Reply::Created(create(gate, startup, display, held, badge, *width, *height))
        }
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
fn create(
    gate: &mut Gate,
    startup: &Startup,
    display: &mut Display<CLIENTS>,
    held: &mut [Option<Held>; CLIENTS],
    badge: u64,
    width: u32,
    height: u32,
) -> Result<Given, Error> {
    let (Some(process), Some(server)) = (startup.own_process, startup.memory_server) else {
        return Err(Error::NotFound);
    };
    let made = display.create(badge, width, height)?;
    let slot = free_slot(held).ok_or(Error::QuotaExceeded);
    let outcome = slot.and_then(|index| {
        let memory = allocate(gate, server, made.bytes(), PAGE_SIZE)?;
        let address = window_of(index);
        let mapping = Mapping::new(gate, process, memory, address, made.bytes())?;
        put(
            held,
            index,
            Held {
                id: made.id,
                memory,
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
    let Some(process) = startup.own_process else {
        return Ok(());
    };
    let Some(index) = held
        .iter()
        .position(|entry| entry.as_ref().is_some_and(|slot| slot.id == id))
    else {
        return Ok(());
    };
    let Some(slot) = take(held, index) else {
        return Ok(());
    };
    let _unmapped = slot.mapping.unmap(gate, process);
    if let Some(server) = startup.memory_server {
        let _released = user_programs::client::release(gate, server, slot.memory);
    }
    Ok(())
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
