// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The name server: the one place where a program that holds a capability
//! to itself can leave it for a program that has no way of naming it.
//!
//! Everything it decides is in `server-name`; this is the loop around it.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds thirteen programs and each uses a different part of
// what it depends on; these are the crates this one does not.
use app_canvas as _;
use audhsos_collections as _;
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
use user_loader as _;
use virtio_queue as _;

use audhsos_abi::ipc_buffer::Buffer;
use audhsos_abi::layout::MAX_MESSAGE_HANDLES;
use audhsos_abi::{Error, Handle, Rights};
use server_name::{Handles, Registry};
use user_programs::serve::{Serving, receive};
use user_proto::name::{Reply, Request};
use user_rt::Startup;
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// Serves names until the endpoint is gone.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let Some(endpoint) = startup.own_endpoint else {
        gate.thread_exit()
    };
    let mut registry = Registry::new();
    let mut serving = Serving::default();
    loop {
        if receive(&mut gate, endpoint, &mut serving).is_err() {
            gate.thread_exit()
        }
        let carried = carried(gate.reader());
        let decoded = Request::decode(gate.reader());
        let taken = match decoded {
            Ok(Request::Register { endpoint, .. }) => Some(endpoint),
            _ => None,
        };
        let answer = match decoded {
            Ok(request) => handle(&mut registry, &mut gate, serving.badge, &request),
            Err(error) => Reply::Registered(Err(Error::from(error))),
        };
        close_rest(&mut gate, &carried, taken);
        let _written = answer.encode(&mut gate.writer());
    }
}

/// What the registry says to one request.
fn handle(registry: &mut Registry, gate: &mut Gate, badge: u64, request: &Request) -> Reply {
    match request {
        Request::Register { name, endpoint } => {
            let mut handles = GateHandles(gate);
            Reply::Registered(registry.accept(&mut handles, badge, *name, *endpoint))
        }
        Request::Lookup { name } => Reply::Found(registry.lookup(name)),
    }
}

/// The handles the message carried, read before any call of this program
/// writes the buffer.
fn carried(buffer: Buffer<'_>) -> [Option<Handle>; MAX_MESSAGE_HANDLES] {
    let mut held = [None; MAX_MESSAGE_HANDLES];
    let Ok(message) = buffer.message() else {
        return held;
    };
    for (index, slot) in held.iter_mut().enumerate().take(message.handle_count) {
        *slot = buffer.handle(index);
    }
    held
}

/// Gives up every handle the message carried but `taken`, which
/// [`Registry::accept`] gives up itself. A handle of a lookup, and one of a
/// message that did not decode, belongs to nobody here and would hold a
/// slot of this program's table for as long as it runs.
fn close_rest(
    gate: &mut Gate,
    carried: &[Option<Handle>; MAX_MESSAGE_HANDLES],
    taken: Option<Handle>,
) {
    for handle in carried.iter().flatten() {
        if Some(*handle) != taken {
            let _closed = gate.handle_close(*handle);
        }
    }
}

/// The handle calls of the registry, made on the gate.
struct GateHandles<'gate>(&'gate mut Gate);

impl Handles for GateHandles<'_> {
    fn duplicate(&mut self, handle: Handle, rights: Rights) -> Result<Handle, Error> {
        self.0.handle_duplicate(handle, rights)
    }

    fn close(&mut self, handle: Handle) {
        let _closed = self.0.handle_close(handle);
    }
}
