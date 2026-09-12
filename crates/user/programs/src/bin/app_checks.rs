// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The application that asks the servers the questions
//! [6.6.22](../../../../../docs/06-testing-strategy.md#6622-end-to-end-tests-in-qemu)
//! asks, and says what it got.
//!
//! Each check is one line on the console, and the line says the answer and
//! not whether it was the expected one: what is expected is written down in
//! the end-to-end run, so that a program which quietly changed its mind
//! about what it wants cannot also change what the run accepts.
//!
//! It writes its lines while `app-hello` writes its own, which is the other
//! thing this program is for: two clients of one console, and no line of
//! either torn by the other.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds thirteen programs and each uses a different part of what
// it depends on; these are the crates this one does not.
use app_canvas as _;
use driver_i8042 as _;
use driver_uart16550 as _;
use gfx as _;
use pci as _;
use server_console as _;
use server_display as _;
use server_input as _;
use server_memory as _;
use server_name as _;
use user_loader as _;

use audhsos_abi::Error;
use audhsos_abi::layout::PAGE_SIZE;
use user_programs::client::{allocate, lookup, release, write_line};
use user_programs::mapping::{Mapping, SCRATCH};
use user_proto::parent;
use user_rt::{EndpointHandle, Line, MemoryHandle, ProcessHandle, Startup, Typed};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The name the console driver registered itself under.
const CONSOLE: &[u8] = b"console";

/// A name nothing registers, which is what makes the answer to it worth
/// asking for.
const MISSING: &[u8] = b"nothing-is-here";

/// How much memory the third check asks for: more than any machine this
/// runs on has, so the answer is an error and the run goes on.
const TOO_MUCH: u64 = 1 << 42;

/// How many lines the interleaving check writes.
///
/// The end-to-end run looks for each of them by number and insists each
/// stands whole and once, so its `E2E_INTERLEAVED` says the same number;
/// a disagreement makes the run fail with the line it could not find.
const LINES: usize = 8;

/// Asks, says, and ends.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let (Some(names), Some(memory), Some(own)) = (
        startup.name_server,
        startup.memory_server,
        startup.own_process,
    ) else {
        gate.thread_exit()
    };
    let Ok(console) = lookup(&mut gate, names, CONSOLE) else {
        gate.thread_exit()
    };

    missing_name(&mut gate, names, console);
    zeroed_again(&mut gate, memory, own, console);
    too_much(&mut gate, memory, console);
    interleaved(&mut gate, console);
    let outcome = input_checks(&mut gate, &startup);
    let line = Line::<128>::of(format_args!(
        "[checks] input lifecycle and isolation: {}\n",
        answer(outcome)
    ));
    let _said = write_line(&mut gate, console, line.as_bytes());

    report(&mut gate, &startup);
    gate.thread_exit()
}

/// Exercises the actual kernel handle tables, page mappings and server
/// cleanup. Ends with a live subscription, so the runner must see its watch
/// cleanup even when it injects no subsequent input event.
fn input_checks(gate: &mut Gate, startup: &Startup) -> Result<(), Error> {
    use audhsos_abi::Rights;
    use user_proto::input::{Reply, Request};
    let input = startup.input_server.ok_or(Error::NotFound)?;
    let own = startup.own_process.ok_or(Error::NotFound)?;
    let notification = gate.notification_create()?;
    let signal = gate.handle_duplicate(notification.handle(), Rights::SIGNAL | Rights::TRANSFER)?;
    let process = gate.handle_duplicate(own.handle(), Rights::INFO | Rights::TRANSFER)?;
    // More rejected handles than the input server's entire handle table.
    for _ in 0..80 {
        Request::Unsubscribe.encode(&mut gate.writer())?;
        let mut buffer = gate.writer();
        buffer.set_handle(0, signal);
        buffer.set_counts(0, 1).map_err(Error::from)?;
        gate.ipc_call(input)?;
        if Reply::decode(gate.reader())? != Reply::Unsubscribed(Err(Error::InvalidArgument)) {
            return Err(Error::InvalidState);
        }
    }
    let subscribe = Request::Subscribe {
        notification: signal,
        process,
    };
    let first = input_ring(gate, input, subscribe)?;
    let mapping = Mapping::new(gate, own, first, SCRATCH, PAGE_SIZE)?;
    // Repeated subscription attempts must close both received handles.
    for _ in 0..40 {
        subscribe.encode(&mut gate.writer())?;
        gate.ipc_call(input)?;
        if Reply::decode(gate.reader())? != Reply::Subscribed(Err(Error::AlreadyExists)) {
            return Err(Error::InvalidState);
        }
    }
    input_unsubscribe(gate, input)?;
    let second = input_ring(gate, input, subscribe)?;
    if gate.memory_info(first)?.start == gate.memory_info(second)?.start {
        return Err(Error::InvalidState);
    }
    input_unsubscribe(gate, input)?;
    gate.handle_close(second.handle())?;
    // Leave only the old mapping: handle-count-only reclamation is unsafe.
    gate.handle_close(first.handle())?;
    // SAFETY: this is still the live ring mapping; all ring access is atomic.
    let page = unsafe { mapping.ring() }.ok_or(Error::InvalidState)?;
    page.overflow
        .store(123, core::sync::atomic::Ordering::Relaxed);
    for _ in 0..8 {
        let ring = input_ring(gate, input, subscribe)?;
        if page.overflow.load(core::sync::atomic::Ordering::Relaxed) != 123 {
            return Err(Error::InvalidState);
        }
        input_unsubscribe(gate, input)?;
        gate.handle_close(ring.handle())?;
    }
    mapping.unmap(gate, own)?;
    let _last = input_ring(gate, input, subscribe)?;
    gate.handle_close(signal)?;
    gate.handle_close(process)?;
    Ok(())
}

/// Subscribes and extracts the ring handle.
fn input_ring(
    gate: &mut Gate,
    input: EndpointHandle,
    request: user_proto::input::Request,
) -> Result<MemoryHandle, Error> {
    request.encode(&mut gate.writer())?;
    gate.ipc_call(input)?;
    match user_proto::input::Reply::decode(gate.reader())? {
        user_proto::input::Reply::Subscribed(result) => result.map(MemoryHandle::from_handle),
        user_proto::input::Reply::Unsubscribed(_) => Err(Error::InvalidState),
    }
}

/// Ends the current subscription without closing the client's ring handle.
fn input_unsubscribe(gate: &mut Gate, input: EndpointHandle) -> Result<(), Error> {
    user_proto::input::Request::Unsubscribe.encode(&mut gate.writer())?;
    gate.ipc_call(input)?;
    match user_proto::input::Reply::decode(gate.reader())? {
        user_proto::input::Reply::Unsubscribed(result) => result,
        user_proto::input::Reply::Subscribed(_) => Err(Error::InvalidState),
    }
}

/// What the name server says about a name nobody registered.
fn missing_name(gate: &mut Gate, names: EndpointHandle, console: EndpointHandle) {
    let outcome = lookup(gate, names, MISSING);
    let mut line = Line::<96>::new();
    line.put(b"[checks] missing name: ");
    line.put(answer(outcome.map(|_endpoint| ())).as_bytes());
    line.put(b"\n");
    let _said = write_line(gate, console, line.as_bytes());
}

/// Whether memory that was used, given back, and asked for again comes
/// back zeroed, which is what D-12 promises.
fn zeroed_again(
    gate: &mut Gate,
    memory: EndpointHandle,
    own: ProcessHandle,
    console: EndpointHandle,
) {
    let mut line = Line::<128>::new();
    line.put(b"[checks] memory comes back zeroed: ");
    match round_trip(gate, memory, own) {
        Ok(()) => line.put(b"ok"),
        Err((step, error)) => {
            line.put(step.as_bytes());
            line.put(b": ");
            line.put(error.message().as_bytes());
        }
    }
    line.put(b"\n");
    let _said = write_line(gate, console, line.as_bytes());
}

/// Takes a page, writes over it, gives it back, takes one again, and
/// answers whether every byte of it is zero.
///
/// The second page need not be the same page. What is checked is that
/// whatever the server hands out carries nothing of what it was used for
/// before, which is the promise; that the server does reuse the page is
/// what makes the check worth making, and its own tests say it does.
fn round_trip(
    gate: &mut Gate,
    memory: EndpointHandle,
    own: ProcessHandle,
) -> Result<(), (&'static str, Error)> {
    let at = |step: &'static str| move |error: Error| (step, error);
    let first = allocate(gate, memory, PAGE_SIZE, PAGE_SIZE).map_err(at("first allocate"))?;
    fill(gate, own, first, 0xA5).map_err(at("fill"))?;
    release(gate, memory, first).map_err(at("release"))?;
    let second = allocate(gate, memory, PAGE_SIZE, PAGE_SIZE).map_err(at("second allocate"))?;
    let clean = all_zero(gate, own, second).map_err(at("read back"))?;
    release(gate, memory, second).map_err(at("second release"))?;
    if clean {
        Ok(())
    } else {
        Err(("read back", Error::InvalidState))
    }
}

/// Writes `byte` over every byte of `object`.
fn fill(gate: &mut Gate, own: ProcessHandle, object: MemoryHandle, byte: u8) -> Result<(), Error> {
    let mut mapping = Mapping::new(gate, own, object, SCRATCH, PAGE_SIZE)?;
    // SAFETY: the kernel mapped the page there and nothing else holds a
    // reference to it.
    unsafe {
        mapping.bytes().fill(byte);
    }
    mapping.unmap(gate, own)
}

/// Whether every byte of `object` is zero.
fn all_zero(gate: &mut Gate, own: ProcessHandle, object: MemoryHandle) -> Result<bool, Error> {
    let mut mapping = Mapping::new(gate, own, object, SCRATCH, PAGE_SIZE)?;
    // SAFETY: as above.
    let clean = unsafe { mapping.bytes().iter().all(|byte| *byte == 0) };
    mapping.unmap(gate, own)?;
    Ok(clean)
}

/// What the memory server says to a request no machine can meet.
fn too_much(gate: &mut Gate, memory: EndpointHandle, console: EndpointHandle) {
    let outcome = allocate(gate, memory, TOO_MUCH, PAGE_SIZE);
    let mut line = Line::<96>::new();
    line.put(b"[checks] more than there is: ");
    line.put(answer(outcome.map(|_object| ())).as_bytes());
    line.put(b"\n");
    let _said = write_line(gate, console, line.as_bytes());
}

/// Writes a run of lines while the other application writes its own.
fn interleaved(gate: &mut Gate, console: EndpointHandle) {
    for index in 0..LINES {
        let line = Line::<96>::of(format_args!(
            "[checks] line {index} of {LINES}, and the whole of it\n"
        ));
        let _said = write_line(gate, console, line.as_bytes());
    }
}

/// The name of what an answer was: the error, or `ok`.
const fn answer(outcome: Result<(), Error>) -> &'static str {
    match outcome {
        Ok(()) => "ok",
        Err(error) => error.message(),
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
