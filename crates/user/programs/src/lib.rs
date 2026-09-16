// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![no_std]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![doc = include_str!("../README.md")]

// The library holds what the programs share; each of the sixteen uses a
// different part of the crates below, and the binaries are what reach
// them. Naming them here is what the unused-dependency check asks for.
use audhsos_abi::Rights;

use app_canvas as _;
use app_shell as _;
use audhsos_collections as _;
use audhsos_time as _;
use driver_i8042 as _;
use driver_uart16550 as _;
use driver_virtio_blk as _;
use fs_fat as _;
use gfx as _;
use pci as _;
use server_console as _;
use server_desk as _;
use server_display as _;
use server_fs as _;
use server_input as _;
use server_memory as _;
use server_name as _;
use user_loader as _;
use virtio_queue as _;

pub mod client;
pub mod config_space;
pub mod dma;
pub mod mapping;
pub mod registers;
pub mod serve;
pub mod socket;

pub use client::{allocate, lookup, register, release, write_line};
pub use mapping::{Mapping, SCRATCH};
pub use serve::{Serving, receive, reply};
pub use socket::{Idle, Listener, Received, Stream};

/// The rights each kind of handle goes out with.
///
/// It is here and not in the program that starts the servers, because two
/// programs of this system start others and both hand out the same rights.
pub struct ObjectRights;

impl ObjectRights {
    /// A program's own process: it maps memory into itself, manages its
    /// own threads, and may hand a capability to itself to a server that
    /// gives something back when it ends — which is `INFO` and, to be able
    /// to give it away at all, `DUPLICATE` and `TRANSFER` (D-106).
    pub const PROCESS: Rights = Rights::MAP
        .union(Rights::MANAGE)
        .union(Rights::INFO)
        .union(Rights::DUPLICATE)
        .union(Rights::TRANSFER);
    /// A program's own endpoint: it receives on it, badges it for the
    /// clients it hands it to, and passes it on — which is what registering
    /// a name is, and what needs `TRANSFER`.
    pub const RECV: Rights = Rights::RECV
        .union(Rights::BADGE)
        .union(Rights::SEND)
        .union(Rights::TRANSFER)
        .union(Rights::DUPLICATE);
    /// A capability to a server: the holder sends on it and hands it on.
    pub const SEND: Rights = Rights::SEND.union(Rights::TRANSFER);
    /// A memory object goes out with every right its type accepts but the
    /// duplication of the handle. `EXECUTE` is among them and has to be:
    /// memory the server hands back becomes the text of some program, and
    /// an object without the right cannot be mapped executable.
    pub const MEMORY: Rights = Rights::READ
        .union(Rights::WRITE)
        .union(Rights::EXECUTE)
        .union(Rights::MAP)
        .union(Rights::INFO)
        .union(Rights::TRANSFER);
    /// A range of I/O ports: the driver reads and writes them.
    pub const PORTS: Rights = Rights::READ.union(Rights::WRITE);
    /// The framebuffer: it is written to and mapped, and it is not code.
    pub const DEVICE: Rights = Rights::READ
        .union(Rights::WRITE)
        .union(Rights::MAP)
        .union(Rights::INFO);
    /// An interrupt: the driver acknowledges it and binds it, which is
    /// what managing it is.
    pub const MANAGE: Rights = Rights::MANAGE;
    /// A notification a driver was given: it waits on it, and what sets
    /// the bit is the interrupt the root task bound to it. The driver
    /// signals nothing and binds nothing, so it is given neither right.
    pub const NOTIFY: Rights = Rights::WAIT;
}

/// Where the programs of the archive are linked. It must equal
/// `PROGRAM_BASE` of `program.ld`, which the xtask checks.
pub const PROGRAM_BASE: u64 = 0x0100_0000;

/// The priorities the root task hands out.
///
/// They are here rather than in `server-init` alone because a program that
/// makes a thread of its own has to name one, and it may name no more than
/// it was given.
pub mod priority {
    /// A driver, which an interrupt wakes and which has to outrank whatever
    /// was running or the wake-up buys nothing.
    pub const DRIVER: u8 = 24;
    /// A system server: the name server, the memory server, and the root
    /// task's own children that serve.
    pub const SERVER: u8 = 16;
    /// An application.
    pub const APPLICATION: u8 = 8;
}

/// The permission bits of `memory_map`: readable everywhere, writable and
/// executable where the bits say so.
pub mod permissions {
    /// Read only.
    pub const READ: u64 = 0;
    /// Readable and writable.
    pub const WRITE: u64 = 0b1;
    /// Readable and executable.
    pub const EXECUTE: u64 = 0b10;
}
