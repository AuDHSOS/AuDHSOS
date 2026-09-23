// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![no_std]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![doc = include_str!("../README.md")]

// The library holds what the four programs share; each uses a different
// part of the crates below, and the binaries are what reach them. Naming
// them here is what the unused-dependency check asks for.
use audhsos_abi as _;
use audhsos_encoding as _;
use audhsos_ssh as _;
use audhsos_time as _;
use audhsos_x509 as _;
use crypto_rng as _;
use net_http as _;
use net_stack as _;
use net_wire as _;
use server_net as _;
use user_programs as _;
use user_proto as _;
use user_rt as _;
use user_sys_x86_64 as _;

pub mod net_dma;
pub mod net_registers;
pub mod reseed;

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests;
