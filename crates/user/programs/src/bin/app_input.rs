// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The program that listens: it subscribes to the input server, waits on
//! the notification the server signals, reads its ring, and writes one line
//! per event through the console.
//!
//! It is the client side of the input protocol end to end: a notification of
//! its own reduced to `SIGNAL`, a ring of one page it never allocated, and a
//! loop that costs one system call per wake-up however many events arrived.
//!
//! One line is what an event costs most: a console line of this system goes
//! out byte by byte, and every byte is two system calls of the driver — one
//! to see that the transmitter is free, one to hand it the byte. A pointer
//! that is moving sends a hundred packets a second, and saying every one of
//! them costs more than the machine has. So a pointer event that another
//! pointer event of the same buttons already follows is not said: the one
//! that follows says where the pointer now is, and it says it sooner for
//! not having waited behind a line that is already out of date. Nothing
//! else is dropped — every key, every button, and the last event of every
//! wake-up are said — so a machine that keeps up says everything, and only
//! a backlog is thinned.
//!
//! It ends when the escape key comes up, which is the last thing the runner
//! injects. A machine on which nothing is injected therefore keeps it
//! waiting, which is what a program that listens does.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds thirteen programs and each uses a different part of what
// it depends on; these are the crates this one does not.
use app_canvas as _;
use audhsos_time as _;
use driver_i8042 as _;
use driver_uart16550 as _;
use driver_virtio_blk as _;
use fs_fat as _;
use gfx as _;
use pci as _;
use server_console as _;
use server_display as _;
use server_fs as _;
use server_input as _;
use server_memory as _;
use server_name as _;
use user_loader as _;
use virtio_queue as _;

use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::{Error, Rights};
use user_programs::client::{lookup, write_line};
use user_programs::mapping::Mapping;
use user_proto::input::{Event, KeyCode, Reply, Request, RingReader};
use user_proto::parent;
use user_rt::{EndpointHandle, MemoryHandle, Startup, Typed};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the input server registered itself under.
const INPUT: &[u8] = b"input";

/// Where the ring is mapped in this program.
const RING: u64 = 0x3000_0000;

/// What a notification handed to a server may be used for: signalling it,
/// and being handed over at all.
const SIGNAL_ONLY: Rights = Rights::SIGNAL.union(Rights::TRANSFER);

/// Listens until the escape key comes up, then reports and ends.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    match listen(&mut gate, &startup) {
        Ok(()) => say(&mut gate, &startup, b"[input] done\n"),
        Err(error) => say_error(&mut gate, &startup, error),
    }
    report(&mut gate, &startup);
    gate.thread_exit()
}

/// Subscribes, then reads until the escape key comes up.
fn listen(gate: &mut Gate, startup: &Startup) -> Result<(), Error> {
    let process = startup.own_process.ok_or(Error::NotFound)?;
    // The capability the root task gave this program carries the badge the
    // input server knows it by. Looking the server up under its name would
    // give one that carries none, and the server refuses those: it keeps a
    // ring per client and cannot tell two of nobody apart.
    let input = match startup.input_server {
        Some(given) => given,
        None => lookup(gate, startup.name_server.ok_or(Error::NotFound)?, INPUT)?,
    };
    let notification = gate.notification_create()?;
    let given = gate.handle_duplicate(notification.handle(), SIGNAL_ONLY)?;
    let watched = gate.handle_duplicate(process.handle(), Rights::INFO.union(Rights::TRANSFER))?;
    let memory = MemoryHandle::from_handle(subscribe(gate, input, given, watched)?);
    gate.handle_close(given)?;
    gate.handle_close(watched)?;
    let mapping = Mapping::new(gate, process, memory, RING, PAGE_SIZE)?;
    say(gate, startup, b"[input] ready\n");

    let mut done = false;
    while !done {
        gate.notification_wait(notification)?;
        // A wake-up says that something is waiting and not how much, so the
        // ring is read until it is empty. What one turn takes is bounded,
        // because saying it is a call on the console driver and the server
        // keeps writing while that call runs: the events are taken out
        // first and said afterwards, a handful at a time.
        loop {
            let mut lines = Lines::new();
            let lost;
            let emptied;
            {
                // SAFETY: the mapping remains live and the server published
                // an initialized RingPage. Both processes use atomic access.
                let page = unsafe { mapping.ring() }.ok_or(Error::InvalidArgument)?;
                let mut reader = RingReader::new(page).ok_or(Error::InvalidArgument)?;
                lost = reader.take_overflow();
                while !lines.is_full() {
                    let Some(event) = reader.pop() else {
                        break;
                    };
                    done |= ends(event);
                    lines.put(event);
                }
                emptied = reader.is_empty();
            }
            if lost != 0 {
                say(
                    gate,
                    startup,
                    user_rt::Line::<64>::of(format_args!("[input] lost {lost}\n")).as_bytes(),
                );
            }
            lines.say(gate, startup);
            if emptied {
                break;
            }
        }
    }
    Ok(())
}

/// How many events one turn of the loop says. A ring holds more, and what
/// does not fit is said on the next turn: nothing is taken out of the ring
/// that is not said.
const HELD: usize = 32;

/// The events of one wake-up, kept until the ring has been read out.
struct Lines {
    events: [Option<Event>; HELD],
    len: usize,
}

impl Lines {
    /// Nothing said yet.
    const fn new() -> Self {
        Lines {
            events: [None; HELD],
            len: 0,
        }
    }

    /// `true` when this many have arrived and the rest wait for the next
    /// turn.
    const fn is_full(&self) -> bool {
        self.len >= HELD
    }

    /// Notes one event, or drops it when this many have already arrived.
    fn put(&mut self, event: Event) {
        if let Some(slot) = self.events.get_mut(self.len) {
            *slot = Some(event);
            self.len = self.len.saturating_add(1);
        }
    }

    /// The event at `index`, if this many have arrived.
    fn at(&self, index: usize) -> Option<Event> {
        self.events.get(index).copied().flatten()
    }

    /// `true` for a pointer event the next one makes obsolete: both are
    /// pointer events, the buttons did not change between them, and this
    /// one turned no wheel. What is left of it is where the pointer was,
    /// and the next event says where it is.
    fn superseded(&self, index: usize) -> bool {
        let (Some(Event::Pointer(this)), Some(Event::Pointer(next))) =
            (self.at(index), self.at(index.saturating_add(1)))
        else {
            return false;
        };
        this.wheel == 0 && this.buttons == next.buttons
    }

    /// Writes one line per event, except for the pointer events the ones
    /// behind them already replaced.
    fn say(&self, gate: &mut Gate, startup: &Startup) {
        for index in 0..self.len {
            let Some(event) = self.at(index) else {
                continue;
            };
            if self.superseded(index) {
                continue;
            }
            let line = match event {
                Event::Key(key) => user_rt::Line::<96>::of(format_args!(
                    "[input] key {} {}\n",
                    key.code.code(),
                    u8::from(key.pressed)
                )),
                Event::Pointer(pointer) => user_rt::Line::<96>::of(format_args!(
                    "[input] pointer {} {} {} {}\n",
                    pointer.dx, pointer.dy, pointer.wheel, pointer.buttons
                )),
            };
            say(gate, startup, line.as_bytes());
        }
    }
}

/// `true` for the event that ends the run, which is the escape key coming
/// up.
const fn ends(event: Event) -> bool {
    match event {
        Event::Key(key) => matches!(key.code, KeyCode::Escape) && !key.pressed,
        Event::Pointer(_) => false,
    }
}

/// Asks the input server for a ring, handing over the notification it is to
/// signal.
fn subscribe(
    gate: &mut Gate,
    input: EndpointHandle,
    notification: audhsos_abi::Handle,
    process: audhsos_abi::Handle,
) -> Result<audhsos_abi::Handle, Error> {
    let request = Request::Subscribe {
        notification,
        process,
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(input)?;
    match Reply::decode(gate.reader())? {
        Reply::Subscribed(outcome) => outcome,
        Reply::Unsubscribed(_) => Err(Error::InvalidArgument),
    }
}

/// Says why it stopped listening.
fn say_error(gate: &mut Gate, startup: &Startup, error: Error) {
    let line =
        user_rt::Line::<128>::of(format_args!("[input] nothing heard: {}\n", error.message()));
    say(gate, startup, line.as_bytes());
}

/// Says one line on the console the root task gave this program.
fn say(gate: &mut Gate, startup: &Startup, line: &[u8]) {
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
