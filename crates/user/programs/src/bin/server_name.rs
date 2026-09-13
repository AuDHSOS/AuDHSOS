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

use audhsos_abi::Error;
use server_name::Registry;
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
        let answer = match Request::decode(gate.reader()) {
            Ok(request) => handle(&mut registry, serving.badge, &request),
            Err(error) => Reply::Registered(Err(Error::from(error))),
        };
        let _written = answer.encode(&mut gate.writer());
    }
}

/// What the registry says to one request.
fn handle(registry: &mut Registry, badge: u64, request: &Request) -> Reply {
    match request {
        Request::Register { name, endpoint } => {
            Reply::Registered(registry.register(badge, *name, *endpoint))
        }
        Request::Lookup { name } => Reply::Found(registry.lookup(name)),
    }
}
