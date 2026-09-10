// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![no_std]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![doc = include_str!("../README.md")]

// The library holds what the programs share; each of the twelve uses a
// different part of the crates below, and the binaries are what reach
// them. Naming them here is what the unused-dependency check asks for.
use app_canvas as _;
use driver_i8042 as _;
use driver_uart16550 as _;
use gfx as _;
use server_console as _;
use server_display as _;
use server_input as _;
use server_memory as _;
use server_name as _;
use user_loader as _;

pub mod client;
pub mod mapping;
pub mod serve;

pub use client::{allocate, lookup, register, release, write_line};
pub use mapping::{Mapping, SCRATCH};
pub use serve::{Serving, receive, reply};

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
