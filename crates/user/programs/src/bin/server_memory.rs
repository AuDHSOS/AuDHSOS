// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The memory server: everything the root task did not keep, handed out a
//! piece at a time and taken back again.
//!
//! The policy is in `server-memory` and is host-tested against a recording
//! double. What is here is the five operations that double stands in for —
//! map, zero, unmap, split, merge — over the gate of this thread.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

// The package holds thirteen programs and each uses a different part of
// what it depends on; these are the crates this one does not.
use app_canvas as _;
use driver_i8042 as _;
use driver_uart16550 as _;
use gfx as _;
use pci as _;
use server_console as _;
use server_display as _;
use server_input as _;
use server_name as _;
use user_loader as _;

use audhsos_abi::Error;
use server_memory::{Object, Pages, Store};
use user_programs::mapping::{Mapping, SCRATCH};
use user_programs::serve::{Serving, receive};
use user_proto::memory::{Reply, Request};
use user_rt::{MemoryHandle, ProcessHandle, Startup, Typed};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// How many pieces the free memory may be in, and how many objects may be
/// out at once.
type Memory = Store<256, 256>;

/// Serves memory until the endpoint is gone.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let (Some(endpoint), Some(own)) = (startup.own_endpoint, startup.own_process) else {
        gate.thread_exit()
    };
    let mut store = Memory::new();
    for region in &startup.ram {
        let Ok(object) = describe(&mut gate, *region) else {
            continue;
        };
        let mut pages = GatePages {
            gate: &mut gate,
            process: own,
        };
        let _adopted = store.adopt(&mut pages, object);
    }

    let mut serving = Serving::default();
    loop {
        if receive(&mut gate, endpoint, &mut serving).is_err() {
            gate.thread_exit()
        }
        let answer = match Request::decode(gate.reader()) {
            Ok(request) => handle(&mut gate, own, &mut store, serving.badge, &request),
            Err(error) => Reply::Released(Err(Error::from(error))),
        };
        let _written = answer.encode(&mut gate.writer());
    }
}

/// What the store says to one request.
fn handle(
    gate: &mut Gate,
    own: ProcessHandle,
    store: &mut Memory,
    badge: u64,
    request: &Request,
) -> Reply {
    let mut pages = GatePages { gate, process: own };
    match request {
        Request::Allocate { len, align } => Reply::Allocated(
            store
                .allocate(&mut pages, badge, *len, *align)
                .map(|object| object.handle),
        ),
        // What the client sent is a capability, and where the object lies
        // is what says which object it is; the server asks the kernel
        // rather than trusting a number the client chose.
        Request::Release { memory } => Reply::Released(
            describe(pages.gate, MemoryHandle::from_handle(*memory))
                .and_then(|object| store.release(&mut pages, badge, object))
                .map(|_object| ()),
        ),
    }
}

/// Where `object` lies and how large it is, which the store needs to know
/// before it can say whether two of them touch.
fn describe(gate: &mut Gate, object: MemoryHandle) -> Result<Object, Error> {
    let info = gate.memory_info(object)?;
    Ok(Object {
        handle: object.handle(),
        start: info.start,
        len: info.length,
    })
}

/// The five operations of the policy, over the gate.
struct GatePages<'a> {
    gate: &'a mut Gate,
    process: ProcessHandle,
}

impl Pages for GatePages<'_> {
    fn references(&mut self, object: audhsos_abi::Handle) -> Result<u64, Error> {
        self.gate
            .memory_references(MemoryHandle::from_handle(object))
    }
    fn map(&mut self, object: audhsos_abi::Handle, offset: u64, len: u64) -> Result<u64, Error> {
        // The window is always at the same address: it is used by one
        // operation at a time, and the page tables under it are built once
        // and then stand.
        let mapping = Mapping::window(
            self.gate,
            self.process,
            MemoryHandle::from_handle(object),
            SCRATCH,
            offset,
            len,
        )?;
        Ok(mapping.address())
    }

    fn zero(&mut self, address: u64, len: u64) -> Result<(), Error> {
        let start = usize::try_from(address).unwrap_or(0);
        let count = usize::try_from(len).unwrap_or(0);
        let pointer = core::ptr::without_provenance_mut::<u8>(start);
        // SAFETY: the kernel has just mapped `len` bytes there, readable and
        // writable, for this process alone; nothing else of this program
        // holds a reference to them, because the window is used by one
        // operation at a time.
        let bytes = unsafe { core::slice::from_raw_parts_mut(pointer, count) };
        bytes.fill(0);
        Ok(())
    }

    fn unmap(&mut self, address: u64, len: u64) -> Result<(), Error> {
        let mut done = 0u64;
        while done < len {
            let rest = len.wrapping_sub(done);
            let chunk = rest.min(
                audhsos_abi::layout::MAX_PAGES_PER_CALL
                    .wrapping_mul(audhsos_abi::layout::PAGE_SIZE),
            );
            let pages = self
                .gate
                .memory_unmap(self.process, address.wrapping_add(done), chunk)?;
            let moved = pages.wrapping_mul(audhsos_abi::layout::PAGE_SIZE);
            if moved == 0 {
                return Err(Error::NotMapped);
            }
            done = done.wrapping_add(moved);
        }
        Ok(())
    }

    fn split(
        &mut self,
        object: audhsos_abi::Handle,
        offset: u64,
    ) -> Result<audhsos_abi::Handle, Error> {
        self.gate
            .memory_split(MemoryHandle::from_handle(object), offset)
            .map(Typed::handle)
    }

    fn merge(
        &mut self,
        lower: audhsos_abi::Handle,
        upper: audhsos_abi::Handle,
    ) -> Result<(), Error> {
        self.gate.memory_merge(
            MemoryHandle::from_handle(lower),
            MemoryHandle::from_handle(upper),
        )
    }

    fn close(&mut self, handle: audhsos_abi::Handle) -> Result<(), Error> {
        self.gate.handle_close(handle)
    }
}
