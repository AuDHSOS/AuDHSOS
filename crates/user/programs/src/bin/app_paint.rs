// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The program that draws: it asks the display server for the screen, for a
//! surface of that size, fills a rectangle in it, writes a line of text with
//! the font of this project, and presents both.
//!
//! What it draws it also says on the console, so the runner knows what to
//! look for in the picture it takes of the screen and never has to assume a
//! resolution. A machine without a framebuffer answers that there is none,
//! and the program says so and ends without drawing.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds nine programs and each uses a different part of what it
// depends on; these are the crates this one does not.
use driver_uart16550 as _;
use server_console as _;
use server_display as _;
use server_memory as _;
use server_name as _;
use user_loader as _;

use audhsos_abi::Error;
use audhsos_abi::layout::PAGE_SIZE;
use gfx::{Color, Damage, PixelFormat, Rect, Surface, draw_text};
use user_programs::client::{lookup, write_line};
use user_programs::mapping::Mapping;
use user_proto::display::{Mode, Reply, Request, Surface as Given};
use user_proto::parent;
use user_rt::{EndpointHandle, MemoryHandle, Startup, Typed};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the display server registered itself under.
const DISPLAY: &[u8] = b"display";

/// Where the pixels of the surface are mapped in this program.
const SURFACE: u64 = 0x3000_0000;

/// The rectangle this program fills, in pixels of the screen.
const BOX_X: u32 = 64;
const BOX_Y: u32 = 64;
const BOX_W: u32 = 96;
const BOX_H: u32 = 48;

/// The color it fills the rectangle with.
const PAINT: Color = Color::new(0x20, 0xC0, 0x40);

/// Where the line of text begins.
const TEXT_X: u32 = 64;
const TEXT_Y: u32 = 128;

/// What it writes there.
const TEXT: &str = "AuDHSOS";

/// Draws, says what it drew, and ends.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    match draw(&mut gate, &startup) {
        Ok(mode) => say(
            &mut gate,
            &startup,
            user_rt::Line::<128>::of(format_args!(
                "[paint] drawn on {}x{}\n",
                mode.width, mode.height
            )),
        ),
        Err(error) => say(
            &mut gate,
            &startup,
            user_rt::Line::<128>::of(format_args!("[paint] nothing drawn: {}\n", error.message())),
        ),
    }
    report(&mut gate, &startup);
    gate.thread_exit()
}

/// The whole of the drawing, from the name of the server to the
/// presentation.
fn draw(gate: &mut Gate, startup: &Startup) -> Result<Mode, Error> {
    let names = startup.name_server.ok_or(Error::NotFound)?;
    let process = startup.own_process.ok_or(Error::NotFound)?;
    let display = lookup(gate, names, DISPLAY)?;
    let mode = ask_mode(gate, display)?;
    let given = ask_surface(gate, display, mode)?;
    let memory = MemoryHandle::from_handle(given.memory);
    let bytes = u64::from(mode.width)
        .saturating_mul(u64::from(mode.height))
        .saturating_mul(4);
    let mut mapping = Mapping::new(gate, process, memory, SURFACE, round_up(bytes))?;
    // SAFETY: the mapping was made just now, it is still standing, and
    // nothing else in this program holds a reference to it.
    let pixels = unsafe { mapping.bytes() };
    let damage = paint(pixels, mode)?;
    present(gate, display, given.id, &damage)?;
    Ok(mode)
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

/// Asks for a surface of the size of the screen.
fn ask_surface(gate: &mut Gate, display: EndpointHandle, mode: Mode) -> Result<Given, Error> {
    let request = Request::CreateSurface {
        width: mode.width,
        height: mode.height,
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(display)?;
    match Reply::decode(gate.reader())? {
        Reply::Created(outcome) => outcome,
        _other => Err(Error::InvalidArgument),
    }
}

/// Fills the rectangle and writes the line, and answers with what changed.
fn paint(pixels: &mut [u8], mode: Mode) -> Result<Damage, Error> {
    let mut surface = Surface::new(
        pixels,
        mode.width,
        mode.height,
        mode.width,
        PixelFormat::from_boot(mode.format),
    )
    .map_err(|_| Error::InvalidArgument)?;
    surface.fill(surface.bounds(), Color::BLACK);
    surface.fill(Rect::new(BOX_X, BOX_Y, BOX_W, BOX_H), PAINT);
    draw_text(
        &mut surface,
        TEXT_X,
        TEXT_Y,
        TEXT,
        Color::WHITE,
        Some(Color::BLACK),
    );
    Ok(*surface.damage())
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

/// Says one line on the console the root task gave this program.
fn say<const N: usize>(gate: &mut Gate, startup: &Startup, line: user_rt::Line<N>) {
    if let Some(log) = startup.log {
        let _said = write_line(gate, log, line.as_bytes());
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
